//! Display list -> `vello::Scene`. Used by the browser renderer and the
//! headless GPU path, so both draw the same scene.

use cadview_core::render::{DisplayList, Paint};
use cadview_core::Color;
use kurbo::{Affine, Rect};
use vello::peniko;

fn to_peniko(c: Color) -> peniko::Color {
    peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Append `list` to `scene`: the background rectangle, then every item in
/// order. Render the result with `base_color` black and `AaConfig::Msaa16`.
pub fn append_display_list(scene: &mut vello::Scene, list: &DisplayList<'_>) {
    let bg = list.background;
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        peniko::Color::from_rgb8(bg.r, bg.g, bg.b),
        None,
        &Rect::new(0.0, 0.0, list.width as f64, list.height as f64),
    );
    for item in &list.items {
        let color = to_peniko(item.color);
        match item.paint {
            Paint::Fill { even_odd } => {
                let rule = if even_odd {
                    peniko::Fill::EvenOdd
                } else {
                    peniko::Fill::NonZero
                };
                scene.fill(rule, list.transform, color, None, item.path);
            }
            Paint::Stroke { width, dashes } => {
                let mut stroke = kurbo::Stroke::new(width);
                if let Some(pattern) = dashes {
                    stroke = stroke.with_dashes(0.0, pattern.iter().copied());
                }
                scene.stroke(&stroke, list.transform, color, None, item.path);
            }
        }
    }
}

/// A new scene holding `list`.
pub fn build_scene(list: &DisplayList<'_>) -> vello::Scene {
    let mut scene = vello::Scene::new();
    append_display_list(&mut scene, list);
    scene
}
