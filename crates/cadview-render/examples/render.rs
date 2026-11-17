//! Render DWGs to PNGs and print timings.
//!
//! ```text
//! cargo run -p cadview-render --release --example render -- \
//!     [--backend auto|gpu|cpu] [--size 960x640] [--out DIR] a.dwg [b.dwg ...]
//! ```
//!
//! Writes `<out>/<stem>.png` (default out: the current directory) and prints
//! one line per file: backend, size, load ms, render ms (display list,
//! render, readback and PNG encoding). Repeats the first file at the end so
//! the first-render cost (GPU device and pipeline creation) shows up
//! separately from a warm render.

use cadview_render::{render_png, Backend, RenderOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let mut backend = Backend::Auto;
    let mut size = [960u32, 640];
    let mut out = PathBuf::from(".");
    let mut inputs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--backend" => match args.next().map(|s| s.parse()) {
                Some(Ok(b)) => backend = b,
                _ => return usage("--backend needs auto, gpu or cpu"),
            },
            "--size" => {
                let s = args.next().unwrap_or_default();
                match cadview_render::parse_size(&s) {
                    Ok(v) => size = v,
                    Err(_) => return usage("--size needs WxH"),
                }
            }
            "--out" => match args.next() {
                Some(d) => out = PathBuf::from(d),
                None => return usage("--out needs a directory"),
            },
            _ => inputs.push(a),
        }
    }
    if inputs.is_empty() {
        return usage("no input DWGs");
    }
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("error: {}: {e}", out.display());
        return ExitCode::from(2);
    }

    let opts = RenderOptions {
        size,
        backend,
        ..Default::default()
    };
    let mut failed = false;
    let mut runs: Vec<(&str, bool)> = inputs.iter().map(|s| (s.as_str(), true)).collect();
    runs.push((inputs[0].as_str(), false)); // warm repeat, not written
    for (i, (input, write)) in runs.into_iter().enumerate() {
        let t = Instant::now();
        let doc = match cadview_core::load_dwg(input) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("FAIL {input}: load: {e}");
                failed = true;
                continue;
            }
        };
        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
        match render_png(&doc, &opts) {
            Ok(r) => {
                let label = if write { "" } else { " (warm repeat)" };
                println!(
                    "ok   {input}{label}: backend={} {}x{} entities={} load={load_ms:.1}ms render={:.1}ms{}{}",
                    r.backend.as_str(),
                    r.width,
                    r.height,
                    doc.entities.len(),
                    r.ms,
                    if i == 0 { " (first in process)" } else { "" },
                    r.adapter.map(|a| format!(" adapter={a}")).unwrap_or_default(),
                );
                if write {
                    let stem = Path::new(input)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "render".to_string());
                    let path = out.join(format!("{stem}.png"));
                    if let Err(e) = std::fs::write(&path, &r.png) {
                        eprintln!("FAIL {}: {e}", path.display());
                        failed = true;
                    }
                }
            }
            Err(e) => {
                eprintln!("FAIL {input}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn usage(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    eprintln!("usage: render [--backend auto|gpu|cpu] [--size WxH] [--out DIR] a.dwg [b.dwg ...]");
    ExitCode::from(2)
}
