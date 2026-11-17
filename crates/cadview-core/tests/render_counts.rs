//! Display-list counts for a committed example, so a change in culling, LOD
//! or block/text expansion shows up as a test failure.

use cadview_core::render::{build_display_list, Camera, Paint, SceneCache, View};

fn counts(path: &str, w: u32, h: u32, zoom_factor: f64) -> (usize, usize, usize) {
    let doc = cadview_core::load_dwg(path).expect("example DWG loads");
    let mut cam = Camera::fit_document(&doc, w as f64, h as f64).unwrap();
    cam.zoom *= zoom_factor;
    let mut cache = SceneCache::new();
    let list = build_display_list(&doc, &mut cache, &View::new(w, h, cam));
    let fills = list
        .items
        .iter()
        .filter(|i| matches!(i.paint, Paint::Fill { .. }))
        .count();
    let strokes = list.items.len() - fills;
    let dashed = list
        .items
        .iter()
        .filter(|i| {
            matches!(
                i.paint,
                Paint::Stroke {
                    dashes: Some(_),
                    ..
                }
            )
        })
        .count();
    (fills, strokes, dashed)
}

#[test]
fn flange_plate_display_list_counts() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/01-flange-plate.dwg"
    );
    let fit = counts(path, 960, 640, 1.0);
    // Zoomed in 8x on the centre: frustum culling drops what is off screen.
    let zoomed = counts(path, 960, 640, 8.0);
    eprintln!("01-flange-plate fit (fills, strokes, dashed) = {fit:?}, zoomed 8x = {zoomed:?}");
    assert_eq!(fit, EXPECTED_FIT);
    assert_eq!(zoomed, EXPECTED_ZOOMED);
}

/// (fills, strokes, dashed strokes), recorded 2026-11-17 from the browser
/// renderer's behaviour (render_frame before the move to core).
const EXPECTED_FIT: (usize, usize, usize) = (15, 132, 14);
const EXPECTED_ZOOMED: (usize, usize, usize) = (0, 7, 2);
