//! Headless GPU render (native): the shared [`GpuContext`] renders to an
//! Rgba8Unorm texture with Msaa16, read back to RGBA8.

use crate::context::GpuContext;
use crate::scene::build_scene;
use cadview_core::render::DisplayList;
use std::sync::{Mutex, OnceLock};

/// A GPU context plus a readback target, reused across renders.
pub struct GpuRenderer {
    ctx: GpuContext,
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
        let ctx = pollster::block_on(GpuContext::new())?;
        let max_edge = ctx
            .device
            .limits()
            .max_texture_dimension_2d
            .min(crate::options::MAX_EDGE);
        Ok(Self {
            ctx,
            max_edge,
            target: None,
        })
    }

    /// Adapter name, backend and type, e.g. "NVIDIA ... (Vulkan, DiscreteGpu)".
    pub fn adapter(&self) -> &str {
        self.ctx.adapter_name()
    }

    fn target(&mut self, width: u32, height: u32) -> &Target {
        let reuse = self
            .target
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height);
        if !reuse {
            let (texture, view) =
                self.ctx
                    .create_target(width, height, wgpu::TextureUsages::COPY_SRC);
            let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let buffer = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
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
        self.ctx
            .render_to_texture(&scene, &target.view, width, height)?;

        let mut encoder = self
            .ctx
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
        self.ctx.queue.submit(Some(encoder.finish()));

        let slice = target.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.ctx
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("GPU poll: {e}"))?;
        rx.recv()
            .map_err(|e| format!("readback: {e}"))?
            .map_err(|e| format!("readback map: {e}"))?;

        let row = (width * 4) as usize;
        let rgba = slice.get_mapped_range().map(|data| {
            let mut rgba = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                let start = y * target.padded_row as usize;
                rgba.extend_from_slice(&data[start..start + row]);
            }
            rgba
        });
        target.buffer.unmap();
        rgba.map_err(|e| format!("readback view: {e}"))
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
