//! Render a prebaked 3D scene (`cad.bake3d`'s file) natively, with no
//! kernel: the reference the browser's 3D view is compared with. The scene
//! is built by the same core code the browser uses (the bake's document and
//! render options through `solid::scene::parts_scene`).
//!
//! ```text
//! cargo run -p cadview-render3d --release --example bake -- scene.bake3d out.png \
//!     [--size 960x640] [--quality working|presentation] [--spp 64] [--seed 0] [--raw]
//! ```

use cadview_core::solid::baked::Baked;
use cadview_render3d::{render_png, Quality, RenderOptions3d};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        return Err(
            "usage: bake <scene.bake3d> <out.png> [--size WxH] [--quality q] [--spp n] [--seed n]"
                .into(),
        );
    }
    let mut opts = RenderOptions3d::default();
    // As `cad.render`: presentation renders are denoised unless `--raw`.
    let mut raw = false;
    let mut rest = args[2..].iter();
    while let Some(a) = rest.next() {
        let mut val = || rest.next().cloned().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--size" => opts.size = cadview_render::parse_size(&val()?)?,
            "--quality" => opts.quality = val()?.parse::<Quality>()?,
            "--spp" => opts.spp = val()?.parse().map_err(|e| format!("--spp: {e}"))?,
            "--seed" => opts.seed = val()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--raw" => raw = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let bytes = std::fs::read(&args[0]).map_err(|e| format!("{}: {e}", args[0]))?;
    let started = std::time::Instant::now();
    let baked = Baked::from_bytes(&bytes)?;
    let doc = baked.document();
    let built = cadview_core::solid::scene::parts_scene(&doc, &baked.options, opts.pixel_size())?;
    let load_ms = started.elapsed().as_secs_f64() * 1e3;
    opts.denoise = opts.quality == Quality::Presentation && !raw;
    // A camera the options place is used as given.
    opts.fit = built.fit;
    let r = render_png(&built.scene, &opts).map_err(|e| e.to_string())?;
    std::fs::write(&args[1], &r.png).map_err(|e| format!("{}: {e}", args[1]))?;
    println!(
        "{}: {} parts, {} triangles, chord {} mm; load {load_ms:.1} ms, render {:.1} ms ({}){}",
        args[1],
        built.parts,
        built.triangles,
        built.chord,
        r.ms,
        r.adapter,
        r.trace
            .map(|t| format!(", {} spp in {:.0} ms", t.samples, t.trace_ms))
            .unwrap_or_default()
    );
    Ok(())
}
