//! Noise of a render against a reference of the same scene: the root mean
//! square difference in 8-bit levels over all channels, and the share of
//! pixels where some channel differs by more than 4 and by more than 8
//! levels. For judging how many samples a presentation render needs.
//!
//! ```text
//! cargo run -p cadview-render3d --release --example noise -- reference.png render.png...
//! ```

fn load(path: &str) -> Result<(Vec<u8>, u32, u32), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    cadview_render::decode_png(&bytes).map_err(|e| format!("{path}: {e}"))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err("usage: noise <reference.png> <render.png>...".into());
    }
    let (reference, w, h) = load(&args[0])?;
    for path in &args[1..] {
        let (img, iw, ih) = load(path)?;
        if (iw, ih) != (w, h) {
            return Err(format!("{path}: {iw}x{ih}, the reference is {w}x{h}"));
        }
        let (mut sq, mut over4, mut over8) = (0.0f64, 0usize, 0usize);
        for (a, b) in img
            .as_chunks::<4>()
            .0
            .iter()
            .zip(reference.as_chunks::<4>().0)
        {
            let mut worst = 0u8;
            for c in 0..3 {
                let d = a[c].abs_diff(b[c]);
                sq += (d as f64).powi(2);
                worst = worst.max(d);
            }
            over4 += usize::from(worst > 4);
            over8 += usize::from(worst > 8);
        }
        let n = (w * h) as f64;
        println!(
            "{path}: rms {:.2} levels, {:.2} % of pixels over 4, {:.3} % over 8",
            (sq / (3.0 * n)).sqrt(),
            over4 as f64 / n * 100.0,
            over8 as f64 / n * 100.0
        );
    }
    Ok(())
}
