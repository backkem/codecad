//! Headless render (native): scene -> frame -> backend -> RGBA8 / PNG, on
//! the process-wide device of `cadview_render::gpu` (the one the 2D
//! renderer uses; never a second one).

use crate::raster::RasterRenderer;
use crate::{
    Gpu, Progress, Quality, Render3dError, RenderOptions3d, Renderer3d, Target, TARGET_FORMAT,
};
use cadview_core::scene3d::{build_frame, Background, Frame3d, Scene3d};
use std::sync::Mutex;
use std::time::Instant;

/// Raw pixels from [`render_rgba`].
#[derive(Debug, Clone)]
pub struct RenderedRgba3d {
    /// sRGB RGBA8, row-major, top-left origin, alpha 255.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// GPU adapter description.
    pub adapter: String,
}

impl RenderedRgba3d {
    /// The pixels as a PNG.
    pub fn png(&self) -> Result<Vec<u8>, Render3dError> {
        cadview_render::encode_png(&self.rgba, self.width, self.height).map_err(Render3dError::Gpu)
    }
}

/// A PNG from [`render_png`].
#[derive(Debug, Clone)]
pub struct Rendered3d {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub adapter: String,
    /// Wall time for frame building, render, readback and PNG encoding.
    pub ms: f64,
}

/// Most calls a progressive renderer gets before the image is taken as is.
const MAX_PASSES: u32 = 1 << 16;

/// Run `renderer` on `frame` until it is done, on any device, and read the
/// target back as RGBA8 (sRGB, top-left origin).
pub fn render_frame_rgba(
    gpu: Gpu<'_>,
    renderer: &mut dyn Renderer3d,
    frame: &Frame3d<'_>,
) -> Result<Vec<u8>, Render3dError> {
    let (width, height) = (frame.width, frame.height);
    let max = gpu.device.limits().max_texture_dimension_2d;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(Render3dError::InvalidOptions(format!(
            "render size {width}x{height} outside 1..={max} px per edge"
        )));
    }
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render3d_target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let target = Target {
        view: &view,
        width,
        height,
    };
    let mut passes = 0;
    while let Progress::Partial { .. } = renderer.render(gpu, frame, target)? {
        passes += 1;
        if passes >= MAX_PASSES {
            break;
        }
    }

    let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("render3d_readback"),
        size: padded_row as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render3d_readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| Render3dError::Gpu(format!("poll: {e}")))?;
    rx.recv()
        .map_err(|e| Render3dError::Gpu(format!("readback: {e}")))?
        .map_err(|e| Render3dError::Gpu(format!("readback map: {e}")))?;
    let row = (width * 4) as usize;
    let rgba = slice.get_mapped_range().map(|data| {
        let mut rgba = Vec::with_capacity(row * height as usize);
        for y in 0..height as usize {
            let start = y * padded_row as usize;
            rgba.extend_from_slice(&data[start..start + row]);
        }
        rgba
    });
    buffer.unmap();
    rgba.map_err(|e| Render3dError::Gpu(format!("readback view: {e}")))
}

/// The scene as the options say to draw it: background override and fit.
pub fn prepare_scene(scene: &Scene3d, opts: &RenderOptions3d) -> Scene3d {
    let (w, h) = opts.pixel_size();
    let mut scene = scene.clone();
    if let Some(bg) = opts.background {
        scene.background = Background::Solid(bg);
    }
    if opts.fit {
        scene.fit_camera(w.max(1) as f64 / h.max(1) as f64);
    }
    scene
}

/// Pipelines for the shared device, built on first use.
static RASTER: Mutex<Option<RasterRenderer>> = Mutex::new(None);

/// Render `scene` to RGBA8 on the shared GPU device.
pub fn render_rgba(
    scene: &Scene3d,
    opts: &RenderOptions3d,
) -> Result<RenderedRgba3d, Render3dError> {
    opts.validate()?;
    if opts.quality != Quality::Working {
        return Err(Render3dError::NotAvailable(opts.quality));
    }
    let (width, height) = opts.pixel_size();
    let scene = prepare_scene(scene, opts);
    let mut frame = build_frame(&scene, width, height);
    frame.pixel_ratio = opts.pixel_ratio;

    // A wgpu validation error panics; keep it from taking the caller down.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cadview_render::gpu::with_shared(|r| {
            let ctx = r.context();
            let mut guard = RASTER.lock().unwrap_or_else(|poisoned| {
                // A panic mid-render may have left the pipelines half used.
                let mut g = poisoned.into_inner();
                *g = None;
                g
            });
            let renderer = guard.get_or_insert_with(|| RasterRenderer::new(&ctx.device));
            let gpu = Gpu {
                device: &ctx.device,
                queue: &ctx.queue,
            };
            render_frame_rgba(gpu, renderer, &frame).map(|px| (px, r.adapter().to_string()))
        })
    }));
    let (rgba, adapter) = match result {
        Ok(Ok(inner)) => inner?,
        Ok(Err(init)) => return Err(Render3dError::Gpu(init)),
        Err(_) => return Err(Render3dError::Gpu("3D render panicked".to_string())),
    };
    Ok(RenderedRgba3d {
        rgba,
        width,
        height,
        adapter,
    })
}

/// Background of a contact sheet between its tiles (sRGB).
pub const SHEET_BACKGROUND: [u8; 3] = [200, 201, 204];
/// Gap between contact-sheet tiles, pixels.
pub const SHEET_GAP: u32 = 8;
/// Height of a tile's caption bar, pixels.
pub const SHEET_CAPTION: u32 = 24;

/// Lay `tiles` out as a grid of `columns`, each scaled by `scale` (in
/// (0, 1]; area-averaged in linear light), with a caption bar under each
/// tile whose label is not empty. Captions are drawn with the core's text
/// outlines through the 2D renderer (no font or text dependency here).
/// Cells take the size of the largest tile; smaller tiles sit top-left.
pub fn contact_sheet(
    tiles: &[(&RenderedRgba3d, &str)],
    columns: usize,
    scale: f64,
) -> Result<RenderedRgba3d, Render3dError> {
    if tiles.is_empty() || columns == 0 || !(scale > 0.0 && scale <= 1.0) {
        return Err(Render3dError::InvalidOptions(
            "contact sheet needs tiles, columns > 0 and a scale in (0, 1]".to_string(),
        ));
    }
    let shrunk: Vec<(Vec<u8>, u32, u32)> = tiles
        .iter()
        .map(|(t, _)| downscale(&t.rgba, t.width, t.height, scale))
        .collect();
    let cell_w = shrunk.iter().map(|t| t.1).max().unwrap_or(1);
    let caption = if tiles.iter().any(|(_, l)| !l.is_empty()) {
        SHEET_CAPTION
    } else {
        0
    };
    let cell_h = shrunk.iter().map(|t| t.2).max().unwrap_or(1) + caption;
    let columns = columns.min(tiles.len()) as u32;
    let rows = (tiles.len() as u32).div_ceil(columns);
    let width = columns * cell_w + (columns + 1) * SHEET_GAP;
    let height = rows * cell_h + (rows + 1) * SHEET_GAP;
    if width > cadview_render::MAX_EDGE || height > cadview_render::MAX_EDGE {
        return Err(Render3dError::InvalidOptions(format!(
            "contact sheet {width}x{height} exceeds {} px per edge",
            cadview_render::MAX_EDGE
        )));
    }
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        rgba.extend_from_slice(&[
            SHEET_BACKGROUND[0],
            SHEET_BACKGROUND[1],
            SHEET_BACKGROUND[2],
            255,
        ]);
    }
    let mut blit = |src: &[u8], w: u32, h: u32, x0: u32, y0: u32| {
        for y in 0..h {
            let s = (y * w * 4) as usize;
            let d = (((y0 + y) * width + x0) * 4) as usize;
            rgba[d..d + (w * 4) as usize].copy_from_slice(&src[s..s + (w * 4) as usize]);
        }
    };
    for (i, ((px, w, h), (_, label))) in shrunk.iter().zip(tiles).enumerate() {
        let (col, row) = (i as u32 % columns, i as u32 / columns);
        let x0 = SHEET_GAP + col * (cell_w + SHEET_GAP);
        let y0 = SHEET_GAP + row * (cell_h + SHEET_GAP);
        blit(px, *w, *h, x0, y0);
        if caption > 0 {
            let bar = caption_bar(label, cell_w, caption)?;
            blit(&bar.rgba, bar.width, bar.height, x0, y0 + cell_h - caption);
        }
    }
    Ok(RenderedRgba3d {
        rgba,
        width,
        height,
        adapter: tiles[0].0.adapter.clone(),
    })
}

/// Area-average `rgba` to `scale` of its size, in linear light.
fn downscale(rgba: &[u8], w: u32, h: u32, scale: f64) -> (Vec<u8>, u32, u32) {
    if scale >= 1.0 {
        return (rgba.to_vec(), w, h);
    }
    let (nw, nh) = (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    );
    let lin: Vec<f64> = rgba
        .iter()
        .map(|&c| cadview_core::scene3d::srgb_to_linear(c))
        .collect();
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for y in 0..nh {
        let (y0, y1) = (y * h / nh, ((y + 1) * h / nh).max(y * h / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (x * w / nw, ((x + 1) * w / nw).max(x * w / nw + 1));
            let mut sum = [0.0; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * w + sx) * 4) as usize;
                    for c in 0..3 {
                        sum[c] += lin[i + c];
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as f64;
            for s in sum {
                out.push(cadview_core::scene3d::encode_srgb(s / n));
            }
            out.push(255);
        }
    }
    (out, nw, nh)
}

/// A dark bar `w` x `h` with `label` in light text, left-aligned: one TEXT
/// entity in a scratch document, drawn by the 2D renderer at one drawing
/// unit per pixel.
fn caption_bar(label: &str, w: u32, h: u32) -> Result<cadview_render::RenderedRgba, Render3dError> {
    use cadview_core::render::FIT_MARGIN;
    let mut doc = cadview_core::Document::new();
    let text: String = label.chars().filter(|c| !c.is_control()).collect();
    if !text.is_empty() {
        let args =
            format!(r#"{{"text": {text:?}, "at": [0, 0], "height": 11, "valign": "middle"}}"#);
        cadview_core::cad_call(&mut doc, "addText", &args)
            .map_err(Render3dError::InvalidOptions)?;
    }
    // The fit leaves a margin of FIT_MARGIN: a box of (w, h) / FIT_MARGIN
    // drawing units fills the bar at exactly one unit per pixel.
    let (bw, bh) = (w as f64 / FIT_MARGIN, h as f64 / FIT_MARGIN);
    let pad = 8.0 - (w as f64 - bw) * 0.5;
    let opts = cadview_render::RenderOptions {
        size: [w, h],
        bounds: Some(cadview_render::Bounds {
            min: [-pad, -bh * 0.5],
            max: [bw - pad, bh * 0.5],
        }),
        ..Default::default()
    };
    cadview_render::render_rgba(&doc, &opts).map_err(|e| Render3dError::Gpu(e.to_string()))
}

/// Render `scene` to a PNG on the shared GPU device.
pub fn render_png(scene: &Scene3d, opts: &RenderOptions3d) -> Result<Rendered3d, Render3dError> {
    let start = Instant::now();
    let r = render_rgba(scene, opts)?;
    let png = cadview_render::encode_png(&r.rgba, r.width, r.height).map_err(Render3dError::Gpu)?;
    Ok(Rendered3d {
        png,
        width: r.width,
        height: r.height,
        adapter: r.adapter,
        ms: start.elapsed().as_secs_f64() * 1000.0,
    })
}
