//! CPU backend: rasterizes a [`DisplayList`] with `vello_cpu`, no GPU needed.
//!
//! Used when no GPU adapter exists and in tests/CI. It paints exactly what
//! the list says, in order, under the list's world-to-pixel transform: fills
//! with their fill rule, strokes with `kurbo::Stroke::new(width)` defaults
//! (round joins and caps) plus the optional dash pattern at offset 0.
//! Antialiasing is analytic area coverage (the GPU path uses Msaa16), and
//! blending is source-over in sRGB space, as in Vello on the GPU.

use cadview_core::render::{DisplayList, Paint};
use cadview_core::Color;
use kurbo::{Affine, Stroke};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::peniko::Fill;
use vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};

/// `vello_cpu` scenes are at most `u16::MAX` pixels on a side; larger
/// outputs are rendered in tiles of at most this size.
const MAX_TILE: u32 = u16::MAX as u32;

/// Rasterize `list` into straight (non-premultiplied) RGBA8, row-major,
/// top-left origin, `width * height * 4` bytes. Fully opaque when the
/// background is opaque. A zero width or height gives an empty buffer.
pub fn render_cpu(list: &DisplayList<'_>) -> Vec<u8> {
    render_tiled(list, MAX_TILE)
}

fn render_tiled(list: &DisplayList<'_>, tile: u32) -> Vec<u8> {
    let (width, height) = (list.width, list.height);
    let mut out = vec![0u8; width as usize * height as usize * 4];
    if out.is_empty() {
        return out;
    }

    if width <= tile && height <= tile {
        render_region(list, 0, 0, width as u16, height as u16, &mut out);
    } else {
        let mut buf = Vec::new();
        for ty in (0..height).step_by(tile as usize) {
            let th = tile.min(height - ty);
            for tx in (0..width).step_by(tile as usize) {
                let tw = tile.min(width - tx);
                buf.clear();
                buf.resize(tw as usize * th as usize * 4, 0);
                render_region(list, tx, ty, tw as u16, th as u16, &mut buf);
                let row_bytes = tw as usize * 4;
                for row in 0..th as usize {
                    let dst = ((ty as usize + row) * width as usize + tx as usize) * 4;
                    let src = row * row_bytes;
                    out[dst..dst + row_bytes].copy_from_slice(&buf[src..src + row_bytes]);
                }
            }
        }
    }

    unpremultiply(&mut out);
    out
}

/// Render the `tw` x `th` pixel region whose top-left is pixel (`tx`, `ty`)
/// into `buf` as premultiplied RGBA8.
fn render_region(list: &DisplayList<'_>, tx: u32, ty: u32, tw: u16, th: u16, buf: &mut [u8]) {
    let mut ctx = RenderContext::new(tw, th);
    ctx.set_transform(Affine::translate((-f64::from(tx), -f64::from(ty))) * list.transform);

    for item in &list.items {
        if item.color.a == 0 {
            continue;
        }
        ctx.set_paint(to_vello(item.color));
        match item.paint {
            Paint::Fill { even_odd } => {
                ctx.set_fill_rule(if even_odd {
                    Fill::EvenOdd
                } else {
                    Fill::NonZero
                });
                ctx.fill_path(item.path);
            }
            Paint::Stroke { width, dashes } => {
                ctx.set_stroke(stroke_style(width, dashes));
                ctx.stroke_path(item.path);
            }
        }
    }

    // Required before rendering when vello_cpu runs multi-threaded; a no-op otherwise.
    ctx.flush();
    let target = PixmapMut::new(tw, th, buf).expect("tile buffer is tw * th * 4 bytes");
    let settings = RasterizerSettings {
        target_init: TargetInit::Clear(to_vello(list.background)),
        ..RasterizerSettings::default()
    };
    ctx.render_with(target, &mut Resources::new(), settings);
}

/// `kurbo::Stroke::new(width)` with the dash pattern at offset 0. A pattern
/// kurbo cannot walk (empty, a negative or non-finite entry, or a zero
/// period) strokes solid instead of hanging or producing garbage.
fn stroke_style(width: f64, dashes: Option<&[f64]>) -> Stroke {
    let stroke = Stroke::new(width);
    match dashes {
        Some(pattern) if dashes_walkable(pattern) => {
            stroke.with_dashes(0.0, pattern.iter().copied())
        }
        _ => stroke,
    }
}

fn dashes_walkable(pattern: &[f64]) -> bool {
    pattern.iter().all(|d| d.is_finite() && *d >= 0.0) && pattern.iter().sum::<f64>() > 0.0
}

fn to_vello(c: Color) -> AlphaColor<Srgb> {
    AlphaColor::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Premultiplied to straight alpha, in place. Opaque pixels (all of them,
/// given an opaque background) are left untouched.
fn unpremultiply(rgba: &mut [u8]) {
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a != 255 && a != 0 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadview_core::render::DrawItem;
    use kurbo::{BezPath, Rect, Shape};

    const BG: Color = Color {
        r: 30,
        g: 30,
        b: 30,
        a: 255,
    };
    const RED: Color = Color {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };

    /// The viewer's camera transform: centre `(cx, cy)` at the middle of the
    /// image, `zoom` pixels per world unit, Y up.
    fn view(w: u32, h: u32, cx: f64, cy: f64, zoom: f64) -> Affine {
        Affine::translate((f64::from(w) / 2.0, f64::from(h) / 2.0))
            * Affine::scale_non_uniform(zoom, -zoom)
            * Affine::translate((-cx, -cy))
    }

    fn list<'a>(w: u32, h: u32, transform: Affine, items: Vec<DrawItem<'a>>) -> DisplayList<'a> {
        DisplayList {
            width: w,
            height: h,
            background: BG,
            transform,
            items,
        }
    }

    fn stroke<'a>(path: &'a BezPath, color: Color, width: f64) -> DrawItem<'a> {
        DrawItem {
            path,
            color,
            paint: Paint::Stroke {
                width,
                dashes: None,
            },
        }
    }

    fn fill<'a>(path: &'a BezPath, color: Color, even_odd: bool) -> DrawItem<'a> {
        DrawItem {
            path,
            color,
            paint: Paint::Fill { even_odd },
        }
    }

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x0, y0));
        p.line_to((x1, y1));
        p
    }

    fn px(img: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    fn rgba(c: Color) -> [u8; 4] {
        [c.r, c.g, c.b, c.a]
    }

    fn close(a: [u8; 4], b: [u8; 4], tol: u8) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
    }

    #[test]
    fn output_length_and_opacity() {
        let (w, h) = (37, 23);
        let l = line(-10.0, -5.0, 10.0, 6.0);
        let c = Rect::new(-4.0, -4.0, 4.0, 4.0).to_path(0.1);
        let translucent = Color { a: 35, ..RED };
        let dl = list(
            w,
            h,
            view(w, h, 0.0, 0.0, 1.5),
            vec![stroke(&l, RED, 1.0), fill(&c, translucent, false)],
        );
        let img = render_cpu(&dl);
        assert_eq!(img.len(), (w * h * 4) as usize);
        assert!(img.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }

    #[test]
    fn zero_size_is_empty() {
        let dl = list(0, 10, Affine::IDENTITY, vec![]);
        assert!(render_cpu(&dl).is_empty());
    }

    #[test]
    fn empty_list_is_background() {
        let (w, h) = (16, 9);
        let img = render_cpu(&list(w, h, Affine::IDENTITY, vec![]));
        assert!(img.as_chunks::<4>().0.iter().all(|p| *p == rgba(BG)));
    }

    #[test]
    fn horizontal_line_lands_on_row_under_y_flip() {
        // 40x40 px, zoom 2, centred on (5, 5). World y maps to pixel
        // y = 20 - (wy - 5) * 2, so wy = 13.25 is the centre of row 3 and
        // its mirror image would be row 36.
        let (w, h, zoom) = (40, 40, 2.0);
        let l = line(0.0, 13.25, 10.0, 13.25);
        let dl = list(
            w,
            h,
            view(w, h, 5.0, 5.0, zoom),
            vec![stroke(&l, RED, 1.0 / zoom)],
        );
        let img = render_cpu(&dl);
        let x = 20; // world x = 5, middle of the line
        assert!(
            close(px(&img, w, x, 3), rgba(RED), 2),
            "row 3: {:?}",
            px(&img, w, x, 3)
        );
        for y in [2, 4, 36] {
            assert_eq!(px(&img, w, x, y), rgba(BG), "row {y}");
        }
        // Nothing anywhere outside row 3.
        for y in (0..h).filter(|&y| y != 3) {
            for x in 0..w {
                assert_eq!(px(&img, w, x, y), rgba(BG), "pixel ({x}, {y})");
            }
        }
    }

    /// Outer square and inner square wound the same way.
    fn square_with_hole() -> BezPath {
        let mut p = Rect::new(-8.0, -8.0, 8.0, 8.0).to_path(0.1);
        p.extend(Rect::new(-3.0, -3.0, 3.0, 3.0).to_path(0.1));
        p
    }

    #[test]
    fn even_odd_leaves_hole_non_zero_fills_it() {
        let (w, h) = (20, 20);
        let p = square_with_hole();
        let t = view(w, h, 0.0, 0.0, 1.0);

        let eo = render_cpu(&list(w, h, t, vec![fill(&p, RED, true)]));
        assert_eq!(px(&eo, w, 10, 10), rgba(BG), "hole under even-odd");
        assert_eq!(px(&eo, w, 4, 10), rgba(RED), "ring under even-odd");
        assert_eq!(px(&eo, w, 0, 0), rgba(BG), "outside");

        let nz = render_cpu(&list(w, h, t, vec![fill(&p, RED, false)]));
        assert_eq!(px(&nz, w, 10, 10), rgba(RED), "hole under non-zero");
        assert_eq!(px(&nz, w, 4, 10), rgba(RED), "ring under non-zero");
    }

    #[test]
    fn translucent_fill_blends_over_what_is_beneath() {
        let (w, h) = (20, 20);
        let all = Rect::new(-100.0, -100.0, 100.0, 100.0).to_path(0.1);
        let left = Rect::new(-100.0, -100.0, 0.0, 100.0).to_path(0.1);
        let c = Color {
            r: 200,
            g: 100,
            b: 50,
            a: 35,
        };
        let blue = Color {
            r: 0,
            g: 0,
            b: 255,
            a: 255,
        };
        let dl = list(
            w,
            h,
            view(w, h, 0.0, 0.0, 1.0),
            vec![fill(&left, blue, false), fill(&all, c, false)],
        );
        let img = render_cpu(&dl);
        let over = |src: u8, dst: u8| {
            (f64::from(src) * 35.0 / 255.0 + f64::from(dst) * (1.0 - 35.0 / 255.0)).round() as u8
        };
        // Right half: over the background, rgb(30,30,30) -> about (53, 40, 33).
        let expect_bg = [over(200, 30), over(100, 30), over(50, 30), 255];
        assert!(
            close(px(&img, w, 15, 10), expect_bg, 2),
            "{:?} vs {expect_bg:?}",
            px(&img, w, 15, 10)
        );
        // Left half: over the opaque blue fill painted first.
        let expect_blue = [over(200, 0), over(100, 0), over(50, 255), 255];
        assert!(
            close(px(&img, w, 4, 10), expect_blue, 2),
            "{:?} vs {expect_blue:?}",
            px(&img, w, 4, 10)
        );
    }

    #[test]
    fn dashed_stroke_has_gaps() {
        // 100 px wide row, zoom 1, dashes 10 on / 10 off from world x = 0.
        // World y = 0 maps to pixel y = 5.5, the centre of row 5.
        let (w, h) = (100, 10);
        let t = Affine::translate((0.0, 5.5)) * Affine::scale_non_uniform(1.0, -1.0);
        let l = line(0.0, 0.0, 100.0, 0.0);
        let pattern = [10.0, 10.0];
        let dashed = DrawItem {
            path: &l,
            color: RED,
            paint: Paint::Stroke {
                width: 1.0,
                dashes: Some(&pattern),
            },
        };
        let img = render_cpu(&list(w, h, t, vec![dashed]));
        for period in 0..5 {
            let on = period * 20 + 5;
            let off = period * 20 + 15;
            assert!(
                close(px(&img, w, on, 5), rgba(RED), 2),
                "dash at x={on}: {:?}",
                px(&img, w, on, 5)
            );
            assert_eq!(px(&img, w, off, 5), rgba(BG), "gap at x={off}");
        }
    }

    #[test]
    fn degenerate_dash_pattern_strokes_solid() {
        let (w, h) = (40, 10);
        let t = Affine::translate((0.0, 5.5)) * Affine::scale_non_uniform(1.0, -1.0);
        let l = line(0.0, 0.0, 40.0, 0.0);
        for pattern in [&[][..], &[0.0, 0.0][..], &[f64::NAN, 1.0][..]] {
            let item = DrawItem {
                path: &l,
                color: RED,
                paint: Paint::Stroke {
                    width: 1.0,
                    dashes: Some(pattern),
                },
            };
            let img = render_cpu(&list(w, h, t, vec![item]));
            for x in 2..38 {
                assert!(close(px(&img, w, x, 5), rgba(RED), 2), "{pattern:?} x={x}");
            }
        }
    }

    /// Rows in column `x` that are mostly covered by the line colour.
    fn covered_rows(img: &[u8], w: u32, h: u32, x: u32) -> u32 {
        (0..h).filter(|&y| px(img, w, x, y)[0] > 140).count() as u32
    }

    #[test]
    fn stroke_width_scales_with_transform() {
        let (w, h) = (40, 40);
        // Horizontal line through world y = 0, which is the boundary between
        // pixel rows 19 and 20, width 1 world unit.
        let l = line(-100.0, 0.0, 100.0, 0.0);
        let at = |zoom: f64| {
            let dl = list(w, h, view(w, h, 0.0, 0.0, zoom), vec![stroke(&l, RED, 1.0)]);
            covered_rows(&render_cpu(&dl), w, h, 20)
        };
        assert_eq!(at(2.0), 2);
        assert_eq!(at(4.0), 4);
        assert_eq!(at(8.0), 8);
    }

    #[test]
    fn tiles_match_single_pass() {
        let (w, h) = (53, 31);
        let t = view(w, h, 0.0, 0.0, 1.7);
        let l = line(-14.0, -8.0, 13.0, 7.5);
        let circle = kurbo::Circle::new((2.0, 1.0), 6.3).to_path(0.01);
        let hole = square_with_hole();
        let pattern = [3.0, 1.5];
        let items = || {
            vec![
                fill(&hole, Color { a: 120, ..RED }, true),
                stroke(&l, Color::WHITE, 0.6),
                DrawItem {
                    path: &circle,
                    color: Color::WHITE,
                    paint: Paint::Stroke {
                        width: 0.5,
                        dashes: Some(&pattern),
                    },
                },
            ]
        };
        let whole = render_tiled(&list(w, h, t, items()), MAX_TILE);
        let tiled = render_tiled(&list(w, h, t, items()), 8);
        let worst = whole
            .iter()
            .zip(&tiled)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            worst <= 2,
            "max channel difference across tile seams: {worst}"
        );
    }

    #[test]
    fn unpremultiply_restores_straight_alpha() {
        let mut px = [50, 25, 0, 100, 0, 0, 0, 0, 9, 8, 7, 255];
        unpremultiply(&mut px);
        assert_eq!(px, [128, 64, 0, 100, 0, 0, 0, 0, 9, 8, 7, 255]);
    }
}
