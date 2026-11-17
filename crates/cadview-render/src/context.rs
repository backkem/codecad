//! One GPU context: wgpu instance, adapter, device, queue and the
//! `vello::Renderer`, built by one constructor for every host.
//!
//! - Headless (native): [`crate::gpu`] keeps one context per process.
//! - Browser: [`shared_local`] gives every canvas on the page the same
//!   context. Each canvas keeps only its own surface, surface config,
//!   intermediate texture and blitter.
//!
//! Every render uses `AaConfig::Msaa16`, so only that pipeline is compiled.
//! Each device creation logs `[CodeCAD] GPU device created (#N)`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::task::{Context, Poll, Waker};
use std::thread::LocalKey;
use vello::peniko;

/// Devices created by [`GpuContext::new`] in this process (or page).
static DEVICES_CREATED: AtomicU32 = AtomicU32::new(0);

/// How many GPU devices this process (or page) has created so far.
pub fn devices_created() -> u32 {
    DEVICES_CREATED.load(Ordering::Relaxed)
}

/// Adapter, device, queue and Vello renderer, shared by every render
/// target of one host.
pub struct GpuContext {
    /// Surfaces (browser canvases) must come from this instance.
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    renderer: vello::Renderer,
    adapter_name: String,
}

impl GpuContext {
    /// Find an adapter, open a device and build the Vello pipelines
    /// (Msaa16 only). Slow (shader compilation); keep the result.
    ///
    /// Native uses the primary backends (Vulkan, Metal, DX12; Vello needs
    /// compute shaders, so not GL), the browser uses WebGPU.
    pub async fn new() -> Result<Self, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = if cfg!(target_arch = "wasm32") {
            wgpu::Backends::BROWSER_WEBGPU
        } else {
            wgpu::Backends::PRIMARY
        };
        let instance = wgpu::Instance::new(desc);
        // WebGPU has no surface compatibility check at adapter request time,
        // so no surface is needed here.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);

        // Vello needs no optional features. The browser has always asked
        // for everything the adapter offers; native asks for none.
        let required_features = if cfg!(target_arch = "wasm32") {
            adapter.features()
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cadview-gpu"),
                required_features,
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("request_device on {adapter_name}: {e}"))?;
        let n = DEVICES_CREATED.fetch_add(1, Ordering::Relaxed) + 1;
        log_line(&format!("[CodeCAD] GPU device created (#{n})"));

        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport {
                    area: false,
                    msaa8: false,
                    msaa16: true,
                },
                // A heuristic thread count on native; ignored on wasm32.
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| format!("vello renderer on {adapter_name}: {e}"))?;

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            renderer,
            adapter_name,
        })
    }

    /// Adapter name, backend and type, e.g. "NVIDIA ... (Vulkan, DiscreteGpu)".
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// An Rgba8Unorm texture Vello can render into (storage binding), plus
    /// `usage` (`COPY_SRC` for readback, `TEXTURE_BINDING` for a blit).
    pub fn create_target(
        &self,
        width: u32,
        height: u32,
        usage: wgpu::TextureUsages,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cadview_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            usage: wgpu::TextureUsages::STORAGE_BINDING | usage,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Render `scene` into `target` (from [`Self::create_target`], `width`
    /// x `height`) with Msaa16 on a black base.
    pub fn render_to_texture(
        &mut self,
        scene: &vello::Scene,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let params = vello::RenderParams {
            base_color: peniko::Color::BLACK,
            width,
            height,
            antialiasing_method: vello::AaConfig::Msaa16,
        };
        self.renderer
            .render_to_texture(&self.device, &self.queue, scene, target, &params)
            .map_err(|e| format!("vello render: {e}"))
    }
}

fn log_line(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&msg.into());
    #[cfg(not(target_arch = "wasm32"))]
    log::info!("{msg}");
}

// ── One context per thread (the browser page) ─────────────────────────

/// A context shared by every user on this thread.
pub type SharedGpu = Rc<RefCell<GpuContext>>;

thread_local! {
    static LOCAL: SlotCell<SharedGpu> = const { RefCell::new(None) };
}

/// This thread's context, created on first use. Callers that arrive while
/// the first one is still initialising wait for its result, so concurrent
/// callers never create a second device. A failed init is handed to every
/// waiting caller and retried by the next new caller.
///
/// The browser is single-threaded, so this is one device per page.
pub async fn shared_local() -> Result<SharedGpu, String> {
    get_or_init(&LOCAL, || async {
        GpuContext::new()
            .await
            .map(|ctx| Rc::new(RefCell::new(ctx)))
    })
    .await
}

/// A once-per-thread async value: the slot in the thread-local, if any.
type SlotCell<T> = RefCell<Option<Rc<RefCell<Slot<T>>>>>;

enum Slot<T> {
    /// Init is running; these callers wait for it.
    Pending(Vec<Waker>),
    Ready(Result<T, String>),
}

/// The value in `key`, running `init` only if no other caller has started
/// it; everyone else waits for that one result.
async fn get_or_init<T, F>(
    key: &'static LocalKey<SlotCell<T>>,
    init: impl FnOnce() -> F,
) -> Result<T, String>
where
    T: Clone + 'static,
    F: Future<Output = Result<T, String>>,
{
    let (slot, owner) = key.with(|cell| {
        let mut cell = cell.borrow_mut();
        match &*cell {
            Some(slot) => (Rc::clone(slot), false),
            None => {
                let slot = Rc::new(RefCell::new(Slot::Pending(Vec::new())));
                *cell = Some(Rc::clone(&slot));
                (slot, true)
            }
        }
    });
    if !owner {
        return WaitSlot(slot).await;
    }
    let mut guard = InitGuard {
        key,
        slot,
        done: false,
    };
    let result = init().await;
    guard.finish(result.clone());
    result
}

/// Waits for the owner of a pending slot to finish.
struct WaitSlot<T>(Rc<RefCell<Slot<T>>>);

impl<T: Clone> Future for WaitSlot<T> {
    type Output = Result<T, String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut *self.0.borrow_mut() {
            Slot::Ready(result) => Poll::Ready(result.clone()),
            Slot::Pending(wakers) => {
                if !wakers.iter().any(|w| w.will_wake(cx.waker())) {
                    wakers.push(cx.waker().clone());
                }
                Poll::Pending
            }
        }
    }
}

/// Publishes the owner's result, and fails the waiters if the owner's
/// future is dropped before init finished.
struct InitGuard<T: 'static> {
    key: &'static LocalKey<SlotCell<T>>,
    slot: Rc<RefCell<Slot<T>>>,
    done: bool,
}

impl<T> InitGuard<T> {
    fn finish(&mut self, result: Result<T, String>) {
        self.done = true;
        if result.is_err() {
            // Let the next caller try again.
            let _ = self.key.try_with(|cell| *cell.borrow_mut() = None);
        }
        let prev = std::mem::replace(&mut *self.slot.borrow_mut(), Slot::Ready(result));
        if let Slot::Pending(wakers) = prev {
            wakers.into_iter().for_each(Waker::wake);
        }
    }
}

impl<T> Drop for InitGuard<T> {
    fn drop(&mut self) {
        if !self.done {
            self.finish(Err("GPU init was cancelled".to_string()));
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Pending on the first poll (waking itself), ready on the second, like
    /// a browser GPU request.
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    /// Poll all futures on one task until each is done, like several
    /// `spawn_local` tasks on the browser's single thread.
    fn join_all<F: Future>(futs: Vec<F>) -> Vec<F::Output> {
        let mut futs: Vec<_> = futs.into_iter().map(Box::pin).collect();
        let mut out: Vec<Option<F::Output>> = futs.iter().map(|_| None).collect();
        pollster::block_on(std::future::poll_fn(|cx| {
            for (f, o) in futs.iter_mut().zip(out.iter_mut()) {
                if o.is_none() {
                    if let Poll::Ready(v) = f.as_mut().poll(cx) {
                        *o = Some(v);
                    }
                }
            }
            if out.iter().all(Option::is_some) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }));
        out.into_iter().map(Option::unwrap).collect()
    }

    thread_local! {
        static TEST_SLOT: SlotCell<Rc<u32>> = const { RefCell::new(None) };
        static INITS: Cell<u32> = const { Cell::new(0) };
    }

    async fn counted(fail: bool) -> Result<Rc<u32>, String> {
        INITS.with(|n| n.set(n.get() + 1));
        YieldOnce(false).await;
        if fail {
            Err("no device".to_string())
        } else {
            Ok(Rc::new(INITS.with(Cell::get)))
        }
    }

    #[test]
    fn concurrent_callers_share_one_init() {
        // Each test runs on its own thread, so the thread-locals start empty.
        let results = join_all(
            (0..3)
                .map(|_| get_or_init(&TEST_SLOT, || counted(false)))
                .collect(),
        );
        assert_eq!(INITS.with(Cell::get), 1, "init ran once");
        let first = results[0].clone().unwrap();
        for r in &results {
            assert!(Rc::ptr_eq(&first, r.as_ref().unwrap()));
        }
        // A later caller gets the same value without another init.
        let later = pollster::block_on(get_or_init(&TEST_SLOT, || counted(false))).unwrap();
        assert!(Rc::ptr_eq(&first, &later));
        assert_eq!(INITS.with(Cell::get), 1);
    }

    #[test]
    fn failed_init_reaches_waiters_and_is_retried() {
        let results = join_all(
            (0..2)
                .map(|_| get_or_init(&TEST_SLOT, || counted(true)))
                .collect(),
        );
        assert!(results
            .iter()
            .all(|r| r.as_ref().unwrap_err() == "no device"));
        assert_eq!(INITS.with(Cell::get), 1);
        let retry = pollster::block_on(get_or_init(&TEST_SLOT, || counted(false)));
        assert_eq!(*retry.unwrap(), 2);
    }

    #[test]
    fn cancelled_init_fails_waiters() {
        let waiter = {
            let mut owner = Box::pin(get_or_init(&TEST_SLOT, || counted(false)));
            let waker = std::task::Waker::noop();
            let mut cx = Context::from_waker(waker);
            assert!(owner.as_mut().poll(&mut cx).is_pending());
            let mut waiter = Box::pin(get_or_init(&TEST_SLOT, || counted(false)));
            assert!(waiter.as_mut().poll(&mut cx).is_pending());
            waiter
            // The owner is dropped here, before its init finished.
        };
        let r = pollster::block_on(waiter);
        assert_eq!(r.unwrap_err(), "GPU init was cancelled");
    }

    #[test]
    fn shared_local_is_one_device() {
        let results = join_all(vec![shared_local(), shared_local()]);
        match (&results[0], &results[1]) {
            (Ok(a), Ok(b)) => assert!(Rc::ptr_eq(a, b)),
            (Err(e), _) if e.contains("no GPU adapter") => {
                eprintln!("SKIPPED shared_local_is_one_device: {e}");
            }
            (a, b) => panic!(
                "shared_local: {:?} / {:?}",
                a.as_ref().err(),
                b.as_ref().err()
            ),
        }
    }
}
