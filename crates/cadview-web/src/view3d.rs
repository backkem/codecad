//! The 3D viewport: a session's solid parts on a canvas, in the working
//! render (raster, interactive) or the presentation render (progressive
//! path tracing), on the page's one GPU device.
//!
//! What is drawn is decided in cadview-core: `solid::scene::parts_scene`
//! builds the scene from the document's parts and the render options (the
//! same code and options as the server's `cad.render`), `scene3d` holds the
//! orbit camera and builds each frame. `cadview-render3d` draws the frame
//! into a texture; this file only owns the canvas: its surface on the
//! shared `GpuContext`, a target texture and a blit, pointer and wheel
//! input on the camera, and the frame loop.
//!
//! Frames are demand-driven, as in the 2D view: a frame is drawn when the
//! camera, the scene or the canvas size changes, and none while idle. In
//! presentation mode the view draws the working render while the camera
//! moves; once it is still, it adds batches of samples (each batch waits
//! until the GPU finished the previous one, and is sized to about
//! [`BATCH_MS`]) until the sample target, then stops. On a slow GPU (the
//! first sample takes longer than [`SLOW_SAMPLE_MS`]) it traces at half
//! resolution, and it lowers the sample target so the image converges in
//! about [`TRACE_BUDGET_MS`].

use crate::session::{RepaintFn, SESSIONS};
use cadview_core::scene3d::{build_frame, Camera3d, NamedView, Projection, Scene3d};
use cadview_core::Document;
use cadview_render::context::{shared_local, SharedGpu};
use cadview_render3d::raster::RasterRenderer;
use cadview_render3d::trace::{PathTracer, TraceSettings};
use cadview_render3d::{Gpu, Progress, Quality, Renderer3d, Target, TARGET_FORMAT};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

/// Aim for trace batches of about this much GPU time.
pub const BATCH_MS: f64 = 24.0;
/// A first sample (at full resolution) slower than this marks a slow GPU:
/// the view traces at half resolution.
pub const SLOW_SAMPLE_MS: f64 = 40.0;
/// The sample target is lowered so the image converges in about this long.
pub const TRACE_BUDGET_MS: f64 = 30_000.0;
/// Sample target of the presentation view (3D5a's browser aim).
pub const DEFAULT_SPP: u32 = 256;
/// The sample target is never lowered below this.
pub const MIN_SPP: u32 = 16;
/// Most samples per batch.
const MAX_BATCH: u32 = 32;
/// After the last input, the view waits this long before tracing.
const SETTLE_MS: f64 = 150.0;

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// GPU completion of the last submitted work, written by wgpu's callback.
#[derive(Default)]
struct Done {
    /// Work submitted and not yet reported done.
    pending: bool,
    submitted_at: f64,
    /// GPU time of the last completed submission (ms, submit to done).
    last_ms: f64,
    /// Samples in it (0 for a raster frame).
    last_samples: u32,
    /// A completion arrived that the loop has not looked at yet.
    fresh: bool,
}

/// Counters a page or a test reads (`view3d_stats`).
#[derive(Default, Clone)]
struct Stats {
    /// Frames drawn (raster frames and trace batches presented).
    frames: u64,
    raster_frames: u64,
    trace_batches: u64,
    /// Animation-frame callbacks run (drawn or not).
    ticks: u64,
    started_at: f64,
    first_frame_ms: Option<f64>,
    /// CPU time of the last raster frame (ms), and its GPU time.
    raster_cpu_ms: f64,
    raster_gpu_ms: f64,
    /// When the current accumulation started and reached its target.
    trace_started_at: f64,
    trace_done_ms: Option<f64>,
    /// GPU time per sample at the current scale (ms).
    sample_ms: f64,
    resets: u64,
    scene_builds: u64,
    parts: usize,
    triangles: usize,
    error: Option<String>,
}

struct View3d {
    gpu: SharedGpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    blitter: wgpu::util::TextureBlitter,
    /// Render target at canvas size: drawn through an sRGB view, blitted
    /// through a plain view (the bytes are already sRGB-encoded).
    full: Option<TargetTex>,
    /// Half-size target for tracing on a slow GPU.
    half: Option<TargetTex>,
    raster: Option<RasterRenderer>,
    tracer: Option<PathTracer>,
    session_id: String,
    key: String,
    canvas: HtmlCanvasElement,
    quality: Quality,
    camera: Option<Camera3d>,
    /// Viewer options over the bake's (or the defaults).
    options: Value,
    scene: Option<Scene3d>,
    /// What the scene was built from: (doc_rev, bake_rev, options rev).
    built: Option<(u64, u64, u64)>,
    options_rev: u64,
    spp: u32,
    /// The user set the sample target; do not adapt it.
    spp_fixed: bool,
    /// Presentation: filter the image with the tracer's denoiser (the
    /// `denoise` render option; on unless it says false, as in `cad.render`).
    denoise: bool,
    /// 1 or 2: tracing resolution divisor.
    trace_div: u32,
    scale_decided: bool,
    batch: u32,
    done: Arc<Mutex<Done>>,
    last_input: f64,
    stats: Stats,
}

struct TargetTex {
    _texture: wgpu::Texture,
    srgb: wgpu::TextureView,
    plain: wgpu::TextureView,
    width: u32,
    height: u32,
}

fn make_target(device: &wgpu::Device, width: u32, height: u32) -> TargetTex {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("view3d_target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
    });
    let srgb = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let plain = texture.create_view(&wgpu::TextureViewDescriptor {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..Default::default()
    });
    TargetTex {
        _texture: texture,
        srgb,
        plain,
        width,
        height,
    }
}

/// The animation-frame callback, held in a cell so it can schedule itself.
type FrameCb = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

thread_local! {
    static VIEWS: RefCell<HashMap<String, Rc<RefCell<View3d>>>> = RefCell::new(HashMap::new());
}

fn with_view<R>(key: &str, f: impl FnOnce(&mut View3d) -> R) -> Option<R> {
    let v = VIEWS.with(|m| m.borrow().get(key).cloned())?;
    let mut v = v.borrow_mut();
    Some(f(&mut v))
}

/// Start a 3D view of `session_id` on `canvas`. `options` holds `cad.render`
/// options (view, projection, colours, materials, ...) and `quality`.
pub fn start(canvas: HtmlCanvasElement, session_id: &str, key: &str, options: Value) {
    let session_id = session_id.to_string();
    let key = key.to_string();
    let started_at = now_ms();
    wasm_bindgen_futures::spawn_local(async move {
        let gpu = match shared_local().await {
            Ok(g) => g,
            Err(e) => {
                web_sys::console::error_1(&format!("3D view: {e}").into());
                return;
            }
        };
        let (surface, config, blitter) = {
            let ctx = gpu.borrow();
            let surface = match ctx
                .instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            {
                Ok(s) => s,
                Err(e) => {
                    web_sys::console::error_1(&format!("3D view: create_surface: {e}").into());
                    return;
                }
            };
            let (w, h) = canvas_size(&canvas);
            let caps = surface.get_capabilities(&ctx.adapter);
            let Some(format) = caps.formats.first().copied() else {
                web_sys::console::error_1(&"3D view: no surface format".into());
                return;
            };
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: w,
                height: h,
                present_mode: wgpu::PresentMode::AutoVsync,
                desired_maximum_frame_latency: 2,
                alpha_mode: wgpu::CompositeAlphaMode::Auto,
                color_space: wgpu::SurfaceColorSpace::Auto,
                view_formats: vec![],
            };
            surface.configure(&ctx.device, &config);
            let blitter = wgpu::util::TextureBlitterBuilder::new(&ctx.device, format)
                .sample_type(wgpu::FilterMode::Linear)
                .build();
            (surface, config, blitter)
        };
        let quality = match options.get("quality").and_then(Value::as_str) {
            Some("presentation") => Quality::Presentation,
            _ => Quality::Working,
        };
        let spp = options
            .get("spp")
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 65536) as u32);
        let view = Rc::new(RefCell::new(View3d {
            gpu,
            surface,
            config,
            blitter,
            full: None,
            half: None,
            raster: None,
            tracer: None,
            session_id: session_id.clone(),
            key: key.clone(),
            canvas: canvas.clone(),
            quality,
            camera: None,
            options: render_options(&options),
            scene: None,
            built: None,
            options_rev: 0,
            spp: spp.unwrap_or(DEFAULT_SPP),
            spp_fixed: spp.is_some(),
            denoise: true,
            trace_div: 1,
            scale_decided: false,
            batch: 1,
            done: Arc::new(Mutex::new(Done::default())),
            last_input: f64::NEG_INFINITY,
            stats: Stats {
                started_at,
                ..Default::default()
            },
        }));
        VIEWS.with(|m| m.borrow_mut().insert(key.clone(), view.clone()));

        // Demand-driven frames: at most one animation frame queued.
        let pending = Rc::new(Cell::new(false));
        let cb: FrameCb = Rc::new(RefCell::new(None));
        let schedule: Rc<dyn Fn()> = {
            let cb = cb.clone();
            let pending = pending.clone();
            Rc::new(move || {
                if pending.get() {
                    return;
                }
                pending.set(true);
                if let Some(f) = cb.borrow().as_ref() {
                    let _ = web_sys::window()
                        .unwrap()
                        .request_animation_frame(f.as_ref().unchecked_ref());
                }
            })
        };
        attach_input(&canvas, view.clone(), schedule.clone());
        {
            let sched = schedule.clone();
            let mut reg = SESSIONS.lock().unwrap();
            reg.renderers.insert(key.clone(), session_id.clone());
            reg.repaint_fns
                .insert(key.clone(), RepaintFn(Box::new(move || sched())));
        }
        {
            let sched = schedule.clone();
            let on_resize =
                Closure::<dyn FnMut(js_sys::Array)>::new(move |_e: js_sys::Array| sched());
            if let Ok(observer) = web_sys::ResizeObserver::new(on_resize.as_ref().unchecked_ref()) {
                observer.observe(&canvas);
                std::mem::forget(observer);
            }
            on_resize.forget();
        }
        let sched = schedule.clone();
        *cb.borrow_mut() = Some(Closure::new(move || {
            pending.set(false);
            let active = {
                let reg = SESSIONS.lock().unwrap();
                reg.renderers.get(&key).is_some_and(|s| *s == session_id)
            };
            if !active {
                VIEWS.with(|m| m.borrow_mut().remove(&key));
                return;
            }
            let again = view.borrow_mut().tick();
            if again {
                sched();
            }
        }));
        schedule();
    });
}

/// The canvas's size in device pixels.
fn canvas_size(c: &HtmlCanvasElement) -> (u32, u32) {
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    (
        ((c.client_width() as f64 * dpr) as u32).max(1),
        ((c.client_height() as f64 * dpr) as u32).max(1),
    )
}

/// The `cad.render` options a viewer passes on (viewer-only keys dropped).
fn render_options(o: &Value) -> Value {
    let mut o = if o.is_object() { o.clone() } else { json!({}) };
    if let Some(m) = o.as_object_mut() {
        for k in [
            "quality",
            "spp",
            "out",
            "size",
            "pixelRatio",
            "backend",
            "mode",
        ] {
            m.remove(k);
        }
    }
    o
}

/// Merge `over` into `base` (top-level keys).
fn merged(base: &Value, over: &Value) -> Value {
    let mut out = if base.is_object() {
        base.clone()
    } else {
        json!({})
    };
    if let (Some(o), Some(m)) = (out.as_object_mut(), over.as_object()) {
        for (k, v) in m {
            o.insert(k.clone(), v.clone());
        }
    }
    out
}

impl View3d {
    /// One animation frame. Returns whether another one is needed.
    fn tick(&mut self) -> bool {
        self.stats.ticks += 1;
        let t0 = now_ms();
        let resized = self.fit_canvas();
        if let Err(e) = self.ensure_scene() {
            if self.stats.error.as_deref() != Some(e.as_str()) {
                web_sys::console::warn_1(&format!("[CodeCAD] 3D view: {e}").into());
            }
            self.stats.error = Some(e);
            return false;
        }
        self.stats.error = None;
        let moving = t0 - self.last_input < SETTLE_MS;
        if self.quality == Quality::Working || moving || resized {
            self.draw_raster(t0);
            // Keep ticking until the input has settled, so presentation
            // mode starts tracing once the camera is still.
            return self.quality == Quality::Presentation;
        }
        self.trace_step()
    }

    /// Match the surface to the canvas; true if the size changed.
    fn fit_canvas(&mut self) -> bool {
        let (w, h) = canvas_size(&self.canvas);
        // Only on a change: setting a canvas's size clears what it shows,
        // and a tick that waits for the GPU presents nothing new.
        if self.canvas.width() != w {
            self.canvas.set_width(w);
        }
        if self.canvas.height() != h {
            self.canvas.set_height(h);
        }
        if w == self.config.width && h == self.config.height && self.full.is_some() {
            return false;
        }
        self.config.width = w;
        self.config.height = h;
        let gpu = self.gpu.borrow();
        self.surface.configure(&gpu.device, &self.config);
        self.full = Some(make_target(&gpu.device, w, h));
        self.half = None;
        if let Some(t) = self.tracer.as_mut() {
            t.reset();
        }
        true
    }

    /// Build the scene from the session when the document, its bake or the
    /// options changed.
    fn ensure_scene(&mut self) -> Result<(), String> {
        let (w, h) = (self.config.width, self.config.height);
        let reg = SESSIONS.lock().unwrap();
        let session = reg
            .sessions
            .get(&self.session_id)
            .ok_or_else(|| format!("session '{}' is gone", self.session_id))?;
        let bake_rev = session.bake.as_ref().map_or(0, |b| b.rev);
        let stamp = (session.doc_rev, bake_rev, self.options_rev);
        if self.built == Some(stamp) && self.scene.is_some() {
            return Ok(());
        }
        // The document's own kernel when it has one; else its bake.
        let doc_with_bake;
        let (doc, base): (&Document, Value) = if crate::kernel::is_kernel(&session.doc.solids) {
            (&session.doc, json!({}))
        } else if let Some(b) = &session.bake {
            let mut d = Document::new();
            if session.doc.parts.is_empty() {
                d.part_defs = b.baked.defs.clone();
                d.parts = b.baked.parts.clone();
            } else {
                d.part_defs = session.doc.part_defs.clone();
                d.parts = session.doc.parts.clone();
            }
            d.solids = b.env.clone();
            doc_with_bake = d;
            (&doc_with_bake, b.baked.options.clone())
        } else if session.doc.parts.is_empty() {
            return Err("no parts to show".into());
        } else {
            return Err(
                "this page has no geometry for these parts yet (no prebaked scene and no kernel)"
                    .into(),
            );
        };
        let opts = merged(&base, &self.options);
        let built = cadview_core::solid::scene::parts_scene(doc, &opts, (w, h))?;
        drop(reg);
        self.denoise = opts.get("denoise").and_then(Value::as_bool).unwrap_or(true);
        let fit = built.fit;
        let mut scene = built.scene;
        // The bake's chord, or the first build's, stays: a resize must not
        // re-tessellate.
        if opts.get("chord").is_none() {
            self.options["chord"] = json!(built.chord);
        }
        match self.camera {
            Some(c) => scene.camera = c,
            None => {
                // A camera the options place is used as given (as in
                // `cad.render`); a named view is fitted.
                if fit {
                    scene.fit_camera(w as f64 / h as f64);
                }
                self.camera = Some(scene.camera);
            }
        }
        self.stats.parts = built.parts;
        self.stats.triangles = built.triangles;
        self.stats.scene_builds += 1;
        self.scene = Some(scene);
        self.built = Some(stamp);
        Ok(())
    }

    fn draw_raster(&mut self, t0: f64) {
        let (w, h) = (self.config.width, self.config.height);
        let dpr = web_sys::window().unwrap().device_pixel_ratio();
        let gpu_rc = self.gpu.clone();
        let gpu = gpu_rc.borrow();
        let raster = self
            .raster
            .get_or_insert_with(|| RasterRenderer::new(&gpu.device));
        let scene = {
            let cam = self.camera;
            let s = self.scene.as_mut().expect("built");
            if let Some(c) = cam {
                s.camera = c;
            }
            &*s
        };
        let mut frame = build_frame(scene, w, h);
        frame.pixel_ratio = dpr;
        let full = self.full.as_ref().expect("sized");
        let g = Gpu {
            device: &gpu.device,
            queue: &gpu.queue,
        };
        let target = Target {
            view: &full.srgb,
            width: w,
            height: h,
        };
        if let Err(e) = raster.render(g, &frame, target) {
            self.stats.error = Some(e.to_string());
            return;
        }
        drop(frame);
        self.present(&gpu, false, 0);
        self.stats.raster_frames += 1;
        self.stats.raster_cpu_ms = now_ms() - t0;
        if let Ok(d) = self.done.lock() {
            if d.last_samples == 0 && d.last_ms > 0.0 {
                self.stats.raster_gpu_ms = d.last_ms;
            }
        }
    }

    /// Blit the full or half target to the surface, present, and ask the
    /// GPU to report when the work submitted so far is done.
    fn present(&mut self, gpu: &cadview_render::context::GpuContext, half: bool, samples: u32) {
        let src = if half {
            self.half.as_ref().map(|t| &t.plain)
        } else {
            self.full.as_ref().map(|t| &t.plain)
        };
        let Some(src) = src else { return };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("view3d_blit"),
            });
        self.blitter.copy(&gpu.device, &mut enc, src, &view);
        gpu.queue.submit(Some(enc.finish()));
        gpu.queue.present(frame);
        let submitted_at = now_ms();
        if let Ok(mut d) = self.done.lock() {
            d.pending = true;
            d.submitted_at = submitted_at;
        }
        let done = self.done.clone();
        gpu.queue.on_submitted_work_done(move || {
            if let Ok(mut d) = done.lock() {
                d.pending = false;
                d.last_ms = now_ms() - d.submitted_at;
                d.last_samples = samples;
                d.fresh = true;
            }
        });
        self.stats.frames += 1;
        if self.stats.first_frame_ms.is_none() {
            self.stats.first_frame_ms = Some(now_ms() - self.stats.started_at);
        }
    }

    /// One step of progressive tracing. Returns whether to come back.
    fn trace_step(&mut self) -> bool {
        // Wait for the GPU to finish the last batch: never queue more than
        // one batch of samples.
        let (pending, fresh) = match self.done.lock() {
            Ok(mut d) => {
                let f = if d.fresh && d.last_samples > 0 {
                    Some((d.last_ms, d.last_samples))
                } else {
                    None
                };
                d.fresh = false;
                (d.pending, f)
            }
            Err(_) => (false, None),
        };
        if pending {
            return true;
        }
        if let Some((ms, n)) = fresh {
            self.adapt(ms, n);
        }
        let (w, h) = (self.config.width, self.config.height);
        let div = self.trace_div;
        let (tw, th) = ((w / div).max(1), (h / div).max(1));
        let dpr = web_sys::window().unwrap().device_pixel_ratio();
        let gpu_rc = self.gpu.clone();
        let gpu = gpu_rc.borrow();
        if div > 1
            && self
                .half
                .as_ref()
                .is_none_or(|t| (t.width, t.height) != (tw, th))
        {
            self.half = Some(make_target(&gpu.device, tw, th));
        }
        let spp = self.spp;
        let tracer = self
            .tracer
            .get_or_insert_with(|| PathTracer::new(&gpu.device));
        let denoise = self.denoise;
        let s = tracer.settings();
        if s.spp != spp || s.denoise != denoise {
            tracer.set_settings(TraceSettings { spp, denoise, ..s });
        }
        let before = tracer.samples();
        let scene = {
            let cam = self.camera;
            let sc = self.scene.as_mut().expect("built");
            if let Some(c) = cam {
                sc.camera = c;
            }
            &*sc
        };
        let mut frame = build_frame(scene, tw, th);
        frame.pixel_ratio = dpr / div as f64;
        let tex = if div > 1 {
            self.half.as_ref()
        } else {
            self.full.as_ref()
        }
        .expect("sized");
        let g = Gpu {
            device: &gpu.device,
            queue: &gpu.queue,
        };
        let target = Target {
            view: &tex.srgb,
            width: tw,
            height: th,
        };
        let mut progress = Progress::Done;
        for _ in 0..self.batch.max(1) {
            match tracer.render(g, &frame, target) {
                Ok(p) => {
                    progress = p;
                    if p == Progress::Done {
                        break;
                    }
                }
                Err(e) => {
                    self.stats.error = Some(e.to_string());
                    return false;
                }
            }
        }
        let after = tracer.samples();
        drop(frame);
        if after < before || (before == 0 && after > 0) {
            // A new accumulation (scene, camera or size changed).
            self.stats.resets += 1;
            self.stats.trace_started_at = now_ms();
            self.stats.trace_done_ms = None;
        }
        let added = after.saturating_sub(if after < before { 0 } else { before });
        // Present even when converged: the working render may have drawn
        // over the target since (a mode switch), and the tracer's resolve
        // just wrote the traced image back.
        self.present(&gpu, div > 1, added);
        self.stats.trace_batches += 1;
        match progress {
            Progress::Done => {
                if self.stats.trace_done_ms.is_none() {
                    self.stats.trace_done_ms = Some(now_ms() - self.stats.trace_started_at);
                }
                false
            }
            Progress::Partial { .. } => true,
        }
    }

    /// Size the next batch from the last one's GPU time, and on the first
    /// batch decide the resolution and the sample target.
    fn adapt(&mut self, ms: f64, samples: u32) {
        let per = (ms / samples.max(1) as f64).max(0.01);
        self.stats.sample_ms = per;
        if !self.scale_decided {
            self.scale_decided = true;
            if self.trace_div == 1 && per > SLOW_SAMPLE_MS {
                self.trace_div = 2;
                // About a quarter of the pixels: guess, then measure again.
                self.stats.sample_ms = per / 4.0;
                if let Some(t) = self.tracer.as_mut() {
                    t.reset();
                }
                self.batch = 1;
                return;
            }
        }
        if !self.spp_fixed {
            let fit = (TRACE_BUDGET_MS / per) as u32;
            let reached = self.tracer.as_ref().map_or(0, |t| t.samples());
            self.spp = fit
                .clamp(MIN_SPP, DEFAULT_SPP)
                .max(reached.min(DEFAULT_SPP));
        }
        self.batch = ((BATCH_MS / per) as u32).clamp(1, MAX_BATCH);
    }

    fn command(&mut self, c: &Value) -> Result<(), String> {
        if let Some(q) = c.get("quality").and_then(Value::as_str) {
            self.quality = q.parse::<Quality>()?;
        }
        if let Some(n) = c.get("spp") {
            match n.as_u64() {
                Some(n) => {
                    self.spp = n.clamp(1, 65536) as u32;
                    self.spp_fixed = true;
                }
                None => self.spp_fixed = false,
            }
        }
        if let Some(d) = c.get("traceScale").and_then(Value::as_u64) {
            self.trace_div = if d >= 2 { 2 } else { 1 };
            self.scale_decided = true;
            self.half = None;
            if let Some(t) = self.tracer.as_mut() {
                t.reset();
            }
        }
        if let Some(d) = c.get("denoise").and_then(Value::as_bool) {
            self.options["denoise"] = json!(d);
            self.options_rev += 1;
        }
        if let Some(o) = c.get("options") {
            self.options = render_options(&merged(&self.options, o));
            self.options_rev += 1;
        }
        let aspect = self.config.width as f64 / self.config.height.max(1) as f64;
        if let Some(cam) = self.camera.as_mut() {
            if let Some(v) = c.get("view").and_then(Value::as_str) {
                let v = match v {
                    "iso" | "iso-se" => NamedView::Isometric,
                    other => other.parse::<NamedView>()?,
                };
                cam.set_view(v);
            }
            match c.get("projection").and_then(Value::as_str) {
                None => {}
                Some("orthographic") => cam.projection = Projection::Orthographic,
                Some("perspective") => cam.projection = Projection::Perspective,
                Some(p) => return Err(format!("projection '{p}'")),
            }
            if let Some(o) = c.get("orbit").and_then(Value::as_array) {
                let f = |i: usize| o.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                orbit(cam, f(0), f(1));
            }
            if c.get("fit").and_then(Value::as_bool) == Some(true) || c.get("view").is_some() {
                if let Some(s) = self.scene.as_mut() {
                    s.camera = *cam;
                    s.fit_camera(aspect);
                    *cam = s.camera;
                }
            }
        } else if c.get("view").is_some() || c.get("projection").is_some() {
            // Not built yet: start from these.
            for k in ["view", "projection"] {
                if let Some(v) = c.get(k) {
                    self.options[k] = v.clone();
                }
            }
            self.options_rev += 1;
        }
        Ok(())
    }

    fn stats_json(&self) -> Value {
        let s = &self.stats;
        let (samples, target) = match &self.tracer {
            Some(t) => (t.samples(), self.spp),
            None => (0, self.spp),
        };
        let gpu = self.gpu.borrow();
        json!({
            "key": self.key,
            "session": self.session_id,
            "quality": self.quality.as_str(),
            "frames": s.frames,
            "rasterFrames": s.raster_frames,
            "traceBatches": s.trace_batches,
            "ticks": s.ticks,
            "firstFrameMs": s.first_frame_ms,
            "rasterCpuMs": s.raster_cpu_ms,
            "rasterGpuMs": s.raster_gpu_ms,
            "samples": samples,
            "target": target,
            "traceDoneMs": s.trace_done_ms,
            "sampleMs": s.sample_ms,
            "traceScale": self.trace_div,
            "denoise": self.denoise,
            "batch": self.batch,
            "resets": s.resets,
            "sceneBuilds": s.scene_builds,
            "parts": s.parts,
            "triangles": s.triangles,
            "width": self.config.width,
            "height": self.config.height,
            "adapter": gpu.adapter_name(),
            "camera": self.camera.map(|c| json!({
                "yaw": c.yaw, "pitch": c.pitch, "distance": c.distance,
                "target": [c.target.x, c.target.y, c.target.z],
                "projection": match c.projection { Projection::Orthographic => "orthographic", Projection::Perspective => "perspective" },
            })),
            "error": s.error,
        })
    }
}

fn orbit(cam: &mut Camera3d, dyaw: f64, dpitch: f64) {
    let lim = 89.9f64.to_radians();
    cam.yaw += dyaw;
    cam.pitch = (cam.pitch + dpitch).clamp(-lim, lim);
}

fn attach_input(canvas: &HtmlCanvasElement, view: Rc<RefCell<View3d>>, schedule: Rc<dyn Fn()>) {
    #[derive(Default)]
    struct Drag {
        down: bool,
        pan: bool,
        x: f64,
        y: f64,
    }
    let drag = Rc::new(RefCell::new(Drag::default()));
    let target = canvas.clone();
    {
        let drag = drag.clone();
        let canvas = canvas.clone();
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                let mut d = drag.borrow_mut();
                d.down = true;
                d.pan = e.button() != 0 || e.shift_key();
                d.x = e.offset_x() as f64;
                d.y = e.offset_y() as f64;
                let _ = canvas.set_pointer_capture(e.pointer_id());
            });
        target
            .add_event_listener_with_callback("pointerdown", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
    {
        let drag = drag.clone();
        let view = view.clone();
        let schedule = schedule.clone();
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                let mut d = drag.borrow_mut();
                if !d.down {
                    return;
                }
                let (x, y) = (e.offset_x() as f64, e.offset_y() as f64);
                let (dx, dy) = (x - d.x, y - d.y);
                d.x = x;
                d.y = y;
                let pan = d.pan;
                drop(d);
                let mut v = view.borrow_mut();
                let h = (v.config.height as f64).max(1.0);
                let dpr = web_sys::window().unwrap().device_pixel_ratio();
                if let Some(cam) = v.camera.as_mut() {
                    if pan {
                        // World units per CSS pixel at the target's depth.
                        let k = 2.0 * cam.half_height() / h * dpr;
                        let (r, u) = (cam.right(), cam.up());
                        cam.target = cam.target + r.scale(-dx * k) + u.scale(dy * k);
                    } else {
                        orbit(cam, -dx * 0.008, dy * 0.008);
                    }
                }
                v.last_input = now_ms();
                drop(v);
                schedule();
            });
        target
            .add_event_listener_with_callback("pointermove", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
    for name in ["pointerup", "pointercancel"] {
        let drag = drag.clone();
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |_e: web_sys::PointerEvent| {
                drag.borrow_mut().down = false;
            });
        target
            .add_event_listener_with_callback(name, cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
    {
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            e.prevent_default();
        });
        target
            .add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
    {
        let cb = Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |e: web_sys::WheelEvent| {
            e.prevent_default();
            let factor = if e.delta_y() < 0.0 { 1.0 / 1.1 } else { 1.1 };
            let mut v = view.borrow_mut();
            if let Some(cam) = v.camera.as_mut() {
                cam.distance = (cam.distance * factor).max(1e-6);
            }
            v.last_input = now_ms();
            drop(v);
            schedule();
        });
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "wheel",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }
}

/// Apply a command to a view and wake it.
pub fn command(key: &str, c: &Value) -> Result<(), String> {
    let r = with_view(key, |v| {
        let r = v.command(c);
        v.last_input = f64::NEG_INFINITY;
        r
    })
    .ok_or_else(|| format!("no 3D view '{key}'"))?;
    r?;
    let reg = SESSIONS.lock().unwrap();
    if let Some(f) = reg.repaint_fns.get(key) {
        f.call();
    }
    Ok(())
}

pub fn stats(key: &str) -> Result<Value, String> {
    with_view(key, |v| v.stats_json()).ok_or_else(|| format!("no 3D view '{key}'"))
}

/// A stopped view: free its GPU resources now (its frame callback, which
/// would notice on its next tick, may never run again).
pub fn release(key: &str) {
    if let Some(v) = VIEWS.with(|m| m.borrow_mut().remove(key)) {
        let mut v = v.borrow_mut();
        v.full = None;
        v.half = None;
        v.raster = None;
        v.tracer = None;
        v.scene = None;
    }
}
