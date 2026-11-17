//! Compare two PNGs pixel by pixel.
//!
//! ```text
//! cargo run -p cadview-render --example pngdiff -- a.png b.png [diff.png]
//! ```
//!
//! Prints both image sizes, the fraction of pixels that differ by more than
//! 8/255 in any RGBA channel, and the largest channel difference. With a
//! third path it writes a diff image: `a` dimmed to grey, differing pixels
//! in red. Exits 1 on a size mismatch, 2 on a usage or I/O error, and 0
//! otherwise (whatever the difference; callers judge the fraction).

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::process::ExitCode;

/// A pixel differs when any channel differs by more than this.
const THRESHOLD: u8 = 8;

struct Image {
    width: u32,
    height: u32,
    /// Straight RGBA8, row-major.
    rgba: Vec<u8>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        eprintln!("usage: pngdiff <a.png> <b.png> [diff.png]");
        return ExitCode::from(2);
    }
    let (a, b) = match (load(&args[0]), load(&args[1])) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    println!("a: {} {}x{}", args[0], a.width, a.height);
    println!("b: {} {}x{}", args[1], b.width, b.height);
    if (a.width, a.height) != (b.width, b.height) {
        eprintln!("size mismatch");
        return ExitCode::from(1);
    }

    let total = a.width as usize * a.height as usize;
    let mut differing = 0usize;
    let mut max_diff = 0u8;
    let mut diff_img = Vec::with_capacity(total * 4);
    for (pa, pb) in a
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.rgba.as_chunks::<4>().0)
    {
        let d = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        max_diff = max_diff.max(d);
        if d > THRESHOLD {
            differing += 1;
            diff_img.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            let luma =
                (u32::from(pa[0]) * 299 + u32::from(pa[1]) * 587 + u32::from(pa[2]) * 114) / 1000;
            let g = (luma / 3) as u8;
            diff_img.extend_from_slice(&[g, g, g, 255]);
        }
    }
    let fraction = if total == 0 {
        0.0
    } else {
        differing as f64 / total as f64
    };
    println!("differing pixels (> {THRESHOLD}/255 in any channel): {differing} / {total} = {fraction:.6}");
    println!("max channel difference: {max_diff}");

    if let Some(path) = args.get(2) {
        if let Err(e) = save(path, a.width, a.height, &diff_img) {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
        println!("diff image: {path}");
    }
    ExitCode::SUCCESS
}

/// Decode any PNG to 8-bit straight RGBA.
fn load(path: &str) -> Result<Image, String> {
    let file = File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    // Palette -> RGB, tRNS -> alpha, low bit depths -> 8, 16-bit -> 8.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{path}: {e}"))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("{path}: image too large"))?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("{path}: {e}"))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{path}: unexpected bit depth {:?}", info.bit_depth));
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => return Err(format!("{path}: unexpected colour type {other:?}")),
    };
    let (w, h) = (info.width as usize, info.height as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in buf.chunks(info.line_size).take(h) {
        for p in row[..w * channels].chunks_exact(channels) {
            rgba.extend_from_slice(&match channels {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
    }
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn save(path: &str, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("{path}: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("{path}: {e}"))
}
