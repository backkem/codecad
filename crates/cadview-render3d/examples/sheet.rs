//! Lay PNG renders out as one labelled contact sheet (`contact_sheet`).
//!
//! ```text
//! cargo run -p cadview-render3d --release --example sheet -- \
//!     <out.png> <columns> <scale> <label=image.png> [<label=image.png> ...]
//! ```
//!
//! Each tile is captioned with its label (empty: no caption). Used to
//! review the 3D examples' working renders side by side.

use cadview_render3d::{contact_sheet, RenderedRgba3d};

fn main() {
    if let Err(e) = run() {
        eprintln!("sheet: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err("usage: sheet <out.png> <columns> <scale> <label=image.png>...".into());
    }
    let out = &args[0];
    let columns: usize = args[1].parse().map_err(|e| format!("columns: {e}"))?;
    let scale: f64 = args[2].parse().map_err(|e| format!("scale: {e}"))?;
    let mut tiles = Vec::new();
    for spec in &args[3..] {
        let (label, path) = spec
            .split_once('=')
            .ok_or_else(|| format!("'{spec}' is not label=image.png"))?;
        let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        let (rgba, width, height) =
            cadview_render::decode_png(&bytes).map_err(|e| format!("{path}: {e}"))?;
        tiles.push((
            RenderedRgba3d {
                rgba,
                width,
                height,
                adapter: String::new(),
            },
            label.to_string(),
        ));
    }
    let refs: Vec<(&RenderedRgba3d, &str)> = tiles.iter().map(|(t, l)| (t, l.as_str())).collect();
    let sheet = contact_sheet(&refs, columns, scale).map_err(|e| e.to_string())?;
    let png = sheet.png().map_err(|e| e.to_string())?;
    std::fs::write(out, &png).map_err(|e| format!("{out}: {e}"))?;
    println!(
        "{out}: {}x{} from {} images",
        sheet.width,
        sheet.height,
        tiles.len()
    );
    Ok(())
}
