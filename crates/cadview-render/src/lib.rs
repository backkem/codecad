//! cadview-render: backends for `cadview_core::render` display lists.
//!
//! cadview-core decides what to draw (camera, culling, LOD, colours,
//! linetypes, paint order) and hands over a [`DisplayList`]. This crate
//! only turns that list into pixels; it never looks at a `Shape`.
//!
//! - `scene` (feature `gpu`, wasm32 + native): display list -> `vello::Scene`.
//!   The browser renderer draws through this.
//! - `gpu` (feature `gpu`, native): headless wgpu + Vello render to RGBA8,
//!   one device per process.
//! - CPU render (feature `cpu`, native).
//! - [`render_png`] (native): options -> display list -> backend -> PNG.

pub mod options;
pub use options::{Backend, Bounds, RenderOptions};

#[cfg(feature = "gpu")]
pub mod scene;

#[cfg(all(feature = "gpu", not(target_arch = "wasm32")))]
pub mod gpu;

#[cfg(all(feature = "cpu", not(target_arch = "wasm32")))]
pub mod cpu;

pub use cadview_core::render::{DisplayList, DrawItem, Paint};

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{Backend, RenderOptions};
    use cadview_core::render::{build_display_list, DisplayList, SceneCache};
    use cadview_core::Document;
    use std::time::Instant;

    /// A rendering failure, as text for the API.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RenderError(pub String);

    impl std::fmt::Display for RenderError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for RenderError {}

    impl From<String> for RenderError {
        fn from(s: String) -> Self {
            Self(s)
        }
    }

    /// Raw pixels from [`render_rgba`].
    #[derive(Debug, Clone)]
    pub struct RenderedRgba {
        /// Straight RGBA8, row-major, top-left origin.
        pub rgba: Vec<u8>,
        pub width: u32,
        pub height: u32,
        /// The backend that ran: `Gpu` or `Cpu`.
        pub backend: Backend,
        /// GPU adapter description, for GPU renders.
        pub adapter: Option<String>,
    }

    /// A PNG from [`render_png`].
    #[derive(Debug, Clone)]
    pub struct Rendered {
        pub png: Vec<u8>,
        pub width: u32,
        pub height: u32,
        /// The backend that ran: `Gpu` or `Cpu`.
        pub backend: Backend,
        pub adapter: Option<String>,
        /// Wall time for display list, render, readback and PNG encoding.
        pub ms: f64,
    }

    /// Render a display list on the GPU (shared device, created on first use).
    #[cfg(feature = "gpu")]
    pub fn render_gpu(list: &DisplayList<'_>) -> Result<(Vec<u8>, String), String> {
        // A wgpu validation error panics; keep it from taking the caller down.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::gpu::with_shared(|r| r.render(list).map(|px| (px, r.adapter().to_string())))
        }));
        match result {
            Ok(Ok(inner)) => inner,
            Ok(Err(init)) => Err(init),
            Err(_) => Err("GPU render panicked".to_string()),
        }
    }

    #[cfg(not(feature = "gpu"))]
    pub fn render_gpu(_list: &DisplayList<'_>) -> Result<(Vec<u8>, String), String> {
        Err("built without the GPU backend (feature `gpu`)".to_string())
    }

    /// Render a display list on the CPU.
    #[cfg(feature = "cpu")]
    pub fn render_cpu(list: &DisplayList<'_>) -> Result<Vec<u8>, String> {
        if list.width == 0 || list.height == 0 {
            return Err("render size must be positive".to_string());
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::cpu::render_cpu(list)
        }))
        .map_err(|_| "CPU render panicked".to_string())
    }

    #[cfg(not(feature = "cpu"))]
    pub fn render_cpu(_list: &DisplayList<'_>) -> Result<Vec<u8>, String> {
        Err("built without the CPU backend (feature `cpu`)".to_string())
    }

    /// Whether `Auto` already knows the GPU is unavailable.
    fn gpu_known_bad() -> bool {
        #[cfg(feature = "gpu")]
        {
            crate::gpu::shared_init_error().is_some()
        }
        #[cfg(not(feature = "gpu"))]
        {
            true
        }
    }

    /// Run a display list on the chosen backend. `Auto` tries the GPU and
    /// falls back to the CPU with a warning; an explicit `Gpu` or `Cpu` that
    /// fails is an error.
    pub fn render_list(
        list: &DisplayList<'_>,
        backend: Backend,
    ) -> Result<RenderedRgba, RenderError> {
        let done = |rgba: Vec<u8>, backend, adapter| RenderedRgba {
            rgba,
            width: list.width,
            height: list.height,
            backend,
            adapter,
        };
        match backend {
            Backend::Gpu => {
                let (px, adapter) = render_gpu(list).map_err(|e| format!("GPU render: {e}"))?;
                Ok(done(px, Backend::Gpu, Some(adapter)))
            }
            Backend::Cpu => {
                let px = render_cpu(list).map_err(|e| format!("CPU render: {e}"))?;
                Ok(done(px, Backend::Cpu, None))
            }
            Backend::Auto => {
                let gpu_err = if gpu_known_bad() {
                    None
                } else {
                    match render_gpu(list) {
                        Ok((px, adapter)) => return Ok(done(px, Backend::Gpu, Some(adapter))),
                        Err(e) => {
                            log::warn!("GPU render unavailable, falling back to CPU: {e}");
                            Some(e)
                        }
                    }
                };
                match render_cpu(list) {
                    Ok(px) => Ok(done(px, Backend::Cpu, None)),
                    Err(cpu_err) => Err(RenderError(match gpu_err {
                        Some(g) => format!("GPU render: {g}; CPU render: {cpu_err}"),
                        None => format!("CPU render: {cpu_err}"),
                    })),
                }
            }
        }
    }

    /// Render `doc` to straight RGBA8.
    pub fn render_rgba(doc: &Document, opts: &RenderOptions) -> Result<RenderedRgba, RenderError> {
        opts.validate()?;
        let view = opts.view(doc);
        let mut cache = SceneCache::new();
        let list = build_display_list(doc, &mut cache, &view);
        render_list(&list, opts.backend)
    }

    /// Render `doc` to a PNG.
    pub fn render_png(doc: &Document, opts: &RenderOptions) -> Result<Rendered, RenderError> {
        let start = Instant::now();
        let r = render_rgba(doc, opts)?;
        let png = encode_png(&r.rgba, r.width, r.height)?;
        Ok(Rendered {
            png,
            width: r.width,
            height: r.height,
            backend: r.backend,
            adapter: r.adapter,
            ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Encode straight RGBA8 as a PNG.
    pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(format!(
                "pixel buffer is {} bytes, expected {}x{}x4",
                rgba.len(),
                width,
                height
            ));
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, width, height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut writer = enc.write_header().map_err(|e| format!("png: {e}"))?;
            writer
                .write_image_data(rgba)
                .map_err(|e| format!("png: {e}"))?;
            writer.finish().map_err(|e| format!("png: {e}"))?;
        }
        Ok(out)
    }

    /// Decode a PNG to straight RGBA8 (for tests and tools).
    pub fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
        let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("png: {e}"))?;
        buf.truncate(info.buffer_size());
        let (w, h) = (info.width, info.height);
        let rgba = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::GrayscaleAlpha => buf
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            other => return Err(format!("png: unexpected color type {other:?}")),
        };
        if info.bit_depth != png::BitDepth::Eight {
            return Err(format!("png: unsupported bit depth {:?}", info.bit_depth));
        }
        Ok((rgba, w, h))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use cadview_core::Document;
    use kurbo::Point;

    /// A 100 x 60 box with a diagonal, so pixels are drawn in known places.
    fn synthetic() -> Document {
        let mut doc = Document::new();
        let p = |x, y| Point::new(x, y);
        doc.add_line(p(0.0, 0.0), p(100.0, 0.0), "0", None);
        doc.add_line(p(100.0, 0.0), p(100.0, 60.0), "0", None);
        doc.add_line(p(100.0, 60.0), p(0.0, 60.0), "0", None);
        doc.add_line(p(0.0, 60.0), p(0.0, 0.0), "0", None);
        doc.add_line(p(0.0, 0.0), p(100.0, 60.0), "0", None);
        doc
    }

    fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    /// Checks shared by both backends: size, background corners, a lit
    /// pixel on the box's left edge and on the diagonal, empty interior.
    fn check_box(r: &RenderedRgba) {
        assert_eq!((r.width, r.height), (240, 160));
        assert_eq!(r.rgba.len(), 240 * 160 * 4);
        let bg = [30, 30, 30, 255];
        assert_eq!(px(&r.rgba, 240, 0, 0), bg);
        assert_eq!(px(&r.rgba, 240, 239, 159), bg);

        // Fit: zoom = min(240/110, 160/66) = 2.1818; centre (50, 30).
        let zoom = 240.0 / 110.0;
        let to_px = |x: f64, y: f64| {
            (
                (120.0 + (x - 50.0) * zoom) as u32,
                (80.0 - (y - 30.0) * zoom) as u32,
            )
        };
        let bright = |p: [u8; 4]| p[0] > 100;
        // Left edge at its midpoint (the stroke straddles the pixel boundary,
        // so look at the neighbour too).
        let (lx, ly) = to_px(0.0, 30.0);
        assert!(
            bright(px(&r.rgba, 240, lx, ly)) || bright(px(&r.rgba, 240, lx.saturating_sub(1), ly)),
            "left edge drawn"
        );
        // Diagonal midpoint.
        let (dx, dy) = to_px(50.0, 30.0);
        let lit = (dx.saturating_sub(1)..=dx + 1)
            .flat_map(|x| (dy.saturating_sub(1)..=dy + 1).map(move |y| (x, y)))
            .any(|(x, y)| bright(px(&r.rgba, 240, x, y)));
        assert!(lit, "diagonal drawn");
        // Inside the box, away from the diagonal: background.
        let (ix, iy) = to_px(80.0, 10.0);
        assert_eq!(px(&r.rgba, 240, ix, iy), bg);
    }

    fn opts(backend: Backend) -> RenderOptions {
        RenderOptions {
            size: [240, 160],
            backend,
            ..Default::default()
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_renders_synthetic_box() {
        let doc = synthetic();
        match render_rgba(&doc, &opts(Backend::Gpu)) {
            Ok(r) => {
                assert_eq!(r.backend, Backend::Gpu);
                eprintln!("GPU adapter: {:?}", r.adapter);
                check_box(&r);
                let png = encode_png(&r.rgba, r.width, r.height).unwrap();
                let (back, w, h) = decode_png(&png).unwrap();
                assert_eq!((w, h), (240, 160));
                assert_eq!(back, r.rgba);
            }
            Err(e) if e.0.contains("no GPU adapter") => {
                eprintln!("SKIPPED gpu_renders_synthetic_box: {e}");
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[cfg(feature = "gpu")]
    #[test]
    fn gpu_render_png_reports_size_and_backend() {
        let doc = synthetic();
        match render_png(&doc, &opts(Backend::Auto)) {
            Ok(r) => {
                assert_eq!((r.width, r.height), (240, 160));
                assert_eq!(&r.png[1..4], b"PNG");
                assert!(r.ms >= 0.0);
            }
            Err(e) => eprintln!("SKIPPED gpu_render_png_reports_size_and_backend: {e}"),
        }
    }

    #[cfg(feature = "cpu")]
    #[test]
    fn cpu_renders_synthetic_box() {
        let doc = synthetic();
        match render_rgba(&doc, &opts(Backend::Cpu)) {
            Ok(r) => {
                assert_eq!(r.backend, Backend::Cpu);
                check_box(&r);
            }
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn png_roundtrip() {
        let rgba: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8).collect();
        let png = encode_png(&rgba, 4, 3).unwrap();
        assert_eq!(decode_png(&png).unwrap(), (rgba, 4, 3));
        assert!(encode_png(&[0; 5], 4, 3).is_err());
    }
}
