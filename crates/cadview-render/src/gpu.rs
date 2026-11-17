//! Headless GPU render (native): wgpu device + `vello::Renderer`, render to
//! an Rgba8Unorm texture with Msaa16, read back to RGBA8.

use crate::scene::build_scene;
use cadview_core::render::DisplayList;
use std::sync::{Mutex, OnceLock};
use vello::peniko;

/// Largest output edge accepted, in pixels (also capped by the device).
pub const MAX_EDGE: u32 = 16384;

/// One wgpu device and Vello renderer, reused across renders.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: vello::Renderer,
    adapter: String,
    max_edge: u32,
    target: Option<Target>,
}

/// Render target and readback buffer for one output size.
struct Target {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    padded_row: u32,
}

impl GpuRenderer {
    /// Find an adapter, open a device and build the Vello pipelines.
    /// Slow (shader compilation); keep the result.
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, String> {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        // Vello needs compute shaders; skip the GL backend.
        desc.backends = wgpu::Backends::PRIMARY;
        let instance = wgpu::Instance::new(desc);
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let adapter_name = format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type);

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cadview-render"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("request_device on {adapter_name}: {e}"))?;
        let max_edge = device.limits().max_texture_dimension_2d.min(MAX_EDGE);

        let renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport {
                    area: false,
                    msaa8: false,
                    msaa16: true,
                },
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| format!("vello renderer on {adapter_name}: {e}"))?;

        Ok(Self {
            device,
            queue,
            renderer,
            adapter: adapter_name,
            max_edge,
            target: None,
        })
    }

    /// Adapter name, backend and type, e.g. "NVIDIA ... (Vulkan, DiscreteGpu)".
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    fn target(&mut self, width: u32, height: u32) -> &Target {
        let reuse = self
            .target
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if !reuse {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cadview_render_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                format: wgpu::TextureFormat::Rgba8Unorm,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("cadview_render_readback"),
                size: padded_row as u64 * height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target {
                width,
                height,
                texture,
                view,
                buffer,
                padded_row,
            });
        }
        self.target.as_ref().unwrap()
    }

    /// Render `list` and return straight RGBA8, row-major, top-left origin.
    pub fn render(&mut self, list: &DisplayList<'_>) -> Result<Vec<u8>, String> {
        let (width, height) = (list.width, list.height);
        if width == 0 || height == 0 {
            return Err("render size must be positive".to_string());
        }
        if width > self.max_edge || height > self.max_edge {
            return Err(format!(
                "render size {width}x{height} exceeds the GPU limit of {} px per edge",
                self.max_edge
            ));
        }
        let scene = build_scene(list);
        self.target(width, height);
        let target = self.target.as_ref().unwrap();

        let params = vello::RenderParams {
            base_color: peniko::Color::BLACK,
            width,
            height,
            antialiasing_method: vello::AaConfig::Msaa16,
        };
        self.renderer
            .render_to_texture(&self.device, &self.queue, &scene, &target.view, &params)
            .map_err(|e| format!("vello render: {e}"))?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("cadview_render_readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = target.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU poll: {e}"))?;
        rx.recv()
            .map_err(|e| format!("readback: {e}"))?
            .map_err(|e| format!("readback map: {e}"))?;

        let row = (width * 4) as usize;
        let mut rgba = Vec::with_capacity(row * height as usize);
        {
            let data = slice.get_mapped_range();
            for y in 0..height as usize {
                let start = y * target.padded_row as usize;
                rgba.extend_from_slice(&data[start..start + row]);
            }
        }
        target.buffer.unmap();
        Ok(rgba)
    }
}

/// The process-wide renderer: created on first use, reused afterwards. A
/// failed init is remembered and not retried.
static SHARED: OnceLock<Result<Mutex<GpuRenderer>, String>> = OnceLock::new();

/// Run `f` on the shared renderer, creating it on first use.
pub fn with_shared<T>(f: impl FnOnce(&mut GpuRenderer) -> T) -> Result<T, String> {
    let shared = SHARED.get_or_init(|| {
        let start = std::time::Instant::now();
        match GpuRenderer::new() {
            Ok(r) => {
                log::info!(
                    "GPU renderer ready on {} in {:.0} ms",
                    r.adapter(),
                    start.elapsed().as_secs_f64() * 1000.0
                );
                Ok(Mutex::new(r))
            }
            Err(e) => Err(e),
        }
    });
    match shared {
        Ok(m) => {
            let mut guard = m
                .lock()
                .map_err(|_| "GPU renderer unusable after an earlier panic".to_string())?;
            Ok(f(&mut guard))
        }
        Err(e) => Err(e.clone()),
    }
}

/// The shared renderer's init error, if init was attempted and failed.
pub fn shared_init_error() -> Option<String> {
    match SHARED.get() {
        Some(Err(e)) => Some(e.clone()),
        _ => None,
    }
}
