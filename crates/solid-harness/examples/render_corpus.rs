//! Render every corpus part, evaluated and tessellated by arris, to a PNG.
//!
//! ```text
//! cargo run -p solid-harness --release --example render_corpus -- <out-dir>
//! ```
//!
//! Each part is tessellated at a quarter pixel of chord for a 960x640
//! isometric view (design section 6, headless budget), drawn on a ground
//! in its palette colour and written to `<out-dir>/<entry>-<part>.png`,
//! with a half cutaway (front half removed) next to it as
//! `<entry>-<part>-cut.png`. `<out-dir>/corpus-sheet.png` shows them all
//! in one labelled grid. A part the kernel refuses is listed and skipped.

use cadview_core::scene3d::{
    palette_color, Camera3d, ClipPlane, Ground, Material, NamedView, Projection, Scene3d,
};
use cadview_core::solid::corpus::{curved, prismatic};
use cadview_core::solid::{Placement, Point3, SolidKernel, Vec3};
use cadview_render3d::{contact_sheet, render_rgba, RenderOptions3d, RenderedRgba3d};
use solid_harness::kernels::ArrisKernel;
use std::path::PathBuf;
use std::time::Instant;

fn file_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: render_corpus <out-dir>"),
    );
    std::fs::create_dir_all(&dir).expect("create out dir");
    let kernel = ArrisKernel;
    let opts = RenderOptions3d::default();
    let write = |name: &str, r: &RenderedRgba3d| {
        let png = r.png().expect("png");
        let path = dir.join(name);
        std::fs::write(&path, png).expect("write png");
        path
    };
    let mut sheet: Vec<(RenderedRgba3d, String)> = Vec::new();
    let entries = prismatic::entries().into_iter().chain(curved::entries());
    for (i, entry) in entries.enumerate() {
        for part in &entry.parts {
            let t = Instant::now();
            let brep = match kernel.evaluate(&part.def, &entry) {
                Ok(e) => e.brep,
                Err(e) => {
                    println!("{} / {}: refused: {e}", entry.name, part.def.name);
                    continue;
                }
            };
            let eval_ms = t.elapsed().as_secs_f64() * 1e3;
            let b = part.bbox;
            let diagonal = (b.max - b.min).length();
            let chord = 0.25 * diagonal * 1.1 / opts.size[1] as f64;
            let t = Instant::now();
            let mesh = match kernel.tessellate(&brep, chord) {
                Ok(m) => m,
                Err(e) => {
                    println!("{} / {}: no mesh: {e}", entry.name, part.def.name);
                    continue;
                }
            };
            let tess_ms = t.elapsed().as_secs_f64() * 1e3;
            let [r, g, bl] = palette_color(i);
            let mut scene = Scene3d {
                camera: Camera3d::new(NamedView::Isometric, Projection::Orthographic),
                ground: Some(Ground::default()),
                ..Scene3d::default()
            };
            scene.add_mesh(&mesh, Placement::IDENTITY, Material::from_srgb8(r, g, bl));
            let t = Instant::now();
            let whole = render_rgba(&scene, &opts).expect("render");
            let render_ms = t.elapsed().as_secs_f64() * 1e3;
            // Half cutaway: remove the front half (toward -Y) of the box.
            scene.clip_planes.push(ClipPlane {
                point: Point3::new(0.0, (b.min.y + b.max.y) * 0.5, 0.0),
                normal: -Vec3::Y,
            });
            let cut = render_rgba(&scene, &opts).expect("render");
            let name = format!("{}-{}", file_name(entry.name), file_name(&part.def.name));
            let path = write(&format!("{name}.png"), &whole);
            write(&format!("{name}-cut.png"), &cut);
            println!(
                "{}: {} triangles (chord {chord:.3} mm), eval {eval_ms:.1} ms, tessellate {tess_ms:.1} ms, render {render_ms:.1} ms",
                path.display(),
                mesh.triangles.len(),
            );
            sheet.push((whole, part.def.name.to_string()));
            sheet.push((cut, format!("{} (cut)", part.def.name)));
        }
    }
    let tiles: Vec<(&RenderedRgba3d, &str)> = sheet.iter().map(|(r, l)| (r, l.as_str())).collect();
    let grid = contact_sheet(&tiles, 4, 0.4).expect("contact sheet");
    println!("{}", write("corpus-sheet.png", &grid).display());
}
