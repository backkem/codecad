//! GPU evidence for the working-quality renderer, measured against closed
//! forms and against the core's reference shading. Every test prints
//! SKIPPED and passes when the machine has no GPU adapter.

use cadview_core::scene3d::{
    build_frame, contrast_ratio, default_section_color, encode_srgb, shapes, srgb_to_linear,
    Background, Camera3d, ClipPlane, EdgeStyle, Frame3d, Ground, LightFrame, Lighting, Material,
    NamedView, Occlusion, Projection, RenderMesh, Scene3d, SceneItem, DEFAULT_CREASE_DEG,
    LIGHT_BACKGROUND, VIEWER_BACKGROUND,
};
use cadview_core::solid::{Placement, Point3, TriMesh, Vec3};
use cadview_render3d::raster::RasterRenderer;
use cadview_render3d::{
    contact_sheet, prepare_scene, render_frame_rgba, render_png, render_rgba, Gpu, Quality,
    Render3dError, RenderOptions3d, RenderedRgba3d,
};
use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Instant;

/// Render, or `None` (after printing SKIPPED) when there is no GPU.
fn render(test: &str, scene: &Scene3d, opts: &RenderOptions3d) -> Option<RenderedRgba3d> {
    match render_rgba(scene, opts) {
        Ok(r) => Some(r),
        Err(Render3dError::Gpu(e)) if e.contains("no GPU adapter") => {
            eprintln!("SKIPPED {test}: {e}");
            None
        }
        Err(e) => panic!("{test}: {e}"),
    }
}

/// The frame the renderer draws for `scene` with `opts` (for projecting
/// test points and computing expected colours).
fn frame_of<'a>(scene: &'a Scene3d, opts: &RenderOptions3d) -> Frame3d<'a> {
    let (w, h) = opts.pixel_size();
    let mut f = build_frame(scene, w, h);
    f.pixel_ratio = opts.pixel_ratio;
    f
}

fn px(r: &RenderedRgba3d, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * r.width + x) * 4) as usize;
    [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2]]
}

fn at(r: &RenderedRgba3d, frame: &Frame3d<'_>, cam: &Camera3d, p: Point3) -> [u8; 3] {
    let (x, y, _) = cam
        .project(p, frame.width, frame.height, frame.near_far)
        .unwrap();
    px(r, x as u32, y as u32)
}

fn lum(c: [u8; 3]) -> f64 {
    (c[0] as f64 + c[1] as f64 + c[2] as f64) / 3.0
}

fn assert_close(got: [u8; 3], want: [u8; 3], tol: u8, what: &str) {
    let ok = got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tol);
    assert!(ok, "{what}: got {got:?}, expected {want:?} (+-{tol})");
}

fn one(mesh: &TriMesh, material: Material) -> Scene3d {
    let mut s = Scene3d::default();
    s.add_mesh(mesh, Placement::IDENTITY, material);
    s
}

fn unit_box() -> TriMesh {
    shapes::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0))
}

/// Uniform ambient light only, black background, no edges: every covered
/// pixel has the same colour, so coverage is a threshold.
fn flat_lit(mut s: Scene3d) -> Scene3d {
    s.lighting = Lighting {
        key_intensity: 0.0,
        fill_intensity: 0.0,
        ambient: 1.0,
        ..Lighting::default()
    };
    s.background = Background::Solid([0, 0, 0]);
    s.edges = None;
    s
}

/// Mean of the channels in linear light.
fn lin(c: [u8; 3]) -> f64 {
    c.iter().map(|&v| srgb_to_linear(v)).sum::<f64>() / 3.0
}

/// Covered area in pixels: each pixel's linear brightness over a fully
/// covered one's (the background is black). MSAA resolves in linear light,
/// so this is the pixel's sample coverage.
fn covered(r: &RenderedRgba3d, full: [u8; 3]) -> f64 {
    let full = lin(full);
    r.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| (lin([p[0], p[1], p[2]]) / full).min(1.0))
        .sum()
}

/// Silhouette area of a unit box and a sphere from named views, against
/// the closed-form projected area. Tolerance: a quarter pixel along the
/// whole outline (4x MSAA quantises each outline pixel's coverage to
/// quarters), plus for the sphere the area its inscribed tessellation
/// loses.
#[test]
fn silhouette_area_matches_closed_form() {
    let opts = RenderOptions3d {
        size: [480, 320],
        ..Default::default()
    };
    let iso_edge = (2.0f64 / 3.0).sqrt();
    let box_cases = [
        (NamedView::Front, 1.0, 4.0),
        (NamedView::Top, 1.0, 4.0),
        (NamedView::Right, 1.0, 4.0),
        (NamedView::Isometric, 3f64.sqrt(), 6.0 * iso_edge),
    ];
    let (r, rings, segments) = (1.0, 96, 192);
    // Inscribed polygon: a 2 * rings-gon from the side, a segments-gon from
    // the top; take the larger deficit.
    let poly = |n: f64| n / 2.0 * (2.0 * PI / n).sin();
    let deficit = (PI - poly(2.0 * rings as f64)).max(PI - poly(segments as f64)) * r * r;
    let sphere_cases = [
        (NamedView::Front, PI * r * r, 2.0 * PI * r),
        (NamedView::Top, PI * r * r, 2.0 * PI * r),
        (NamedView::Isometric, PI * r * r, 2.0 * PI * r),
    ];
    let mesh_cases = box_cases
        .iter()
        .map(|c| (unit_box(), "box", *c, 0.0))
        .chain(
            sphere_cases
                .iter()
                .map(|c| (shapes::sphere(r, rings, segments), "sphere", *c, deficit)),
        );
    for (mesh, name, (view, area, perimeter), extra) in mesh_cases {
        let mut s = flat_lit(one(&mesh, Material::from_srgb8(200, 200, 200)));
        s.camera = Camera3d::new(view, Projection::Orthographic);
        let Some(img) = render("silhouette_area_matches_closed_form", &s, &opts) else {
            return;
        };
        let prepared = prepare_scene(&s, &opts);
        let f = frame_of(&prepared, &opts);
        let full = f.expected_srgb(Point3::ORIGIN, Vec3::Z, &s.items[0].material);
        let px_per_mm = 1.0 / f.pixel_world;
        let expect = area * px_per_mm * px_per_mm;
        let tol = 0.25 * perimeter * px_per_mm + extra * px_per_mm * px_per_mm;
        let got = covered(&img, full);
        eprintln!(
            "{name} {view:?}: {got} px covered, closed form {expect:.1}, error {:.2} px ({:.3} %), tolerance {tol:.1}",
            got - expect,
            (got - expect) / expect * 100.0
        );
        assert!((got - expect).abs() <= tol, "{name} {view:?}");
    }
}

/// A small near box hides the part of a far box behind it, and only that.
#[test]
fn nearer_box_hides_the_farther_one() {
    let red = Material::from_srgb8(200, 40, 40);
    let blue = Material::from_srgb8(40, 40, 200);
    let mut s = Scene3d::default();
    s.add_mesh(
        &shapes::cuboid(Point3::new(0.0, 10.0, 0.0), Point3::new(10.0, 20.0, 10.0)),
        Placement::IDENTITY,
        red,
    );
    s.add_mesh(
        &shapes::cuboid(Point3::new(3.0, 0.0, 3.0), Point3::new(6.0, 2.0, 6.0)),
        Placement::IDENTITY,
        blue,
    );
    s.edges = None;
    let opts = RenderOptions3d {
        size: [400, 300],
        ..Default::default()
    };
    for proj in [Projection::Orthographic, Projection::Perspective] {
        s.camera = Camera3d::new(NamedView::Front, proj);
        let Some(img) = render("nearer_box_hides_the_farther_one", &s, &opts) else {
            return;
        };
        let prepared = prepare_scene(&s, &opts);
        let f = frame_of(&prepared, &opts);
        let cam = &prepared.camera;
        // Behind the small box (on the far box's front face, y = 10): blue.
        let hidden = Point3::new(4.5, 10.0, 4.5);
        let want_blue = f.expected_srgb(Point3::new(4.5, 0.0, 4.5), -Vec3::Y, &blue);
        assert_close(
            at(&img, &f, cam, hidden),
            want_blue,
            2,
            "hidden point shows the near box",
        );
        // Beside it: the far box's own face.
        for p in [Point3::new(1.0, 10.0, 1.0), Point3::new(8.5, 10.0, 8.5)] {
            let want_red = f.expected_srgb(p, -Vec3::Y, &red);
            assert_close(at(&img, &f, cam, p), want_red, 2, "far box visible beside");
        }
        assert_eq!(px(&img, 2, 2), LIGHT_BACKGROUND, "corner is background");
    }
}

/// Facets facing the key light are brighter than those facing away, and
/// each face's pixel matches the core's reference shading within 2 levels.
#[test]
fn box_faces_follow_the_key_light() {
    let m = Material::from_srgb8(180, 180, 180);
    let mut s = one(&unit_box(), m);
    s.lighting = Lighting {
        key_direction: Vec3::new(-0.5, -1.0, 0.3),
        frame: LightFrame::World,
        fill_intensity: 0.0,
        ..Lighting::default()
    };
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
    let opts = RenderOptions3d::default();
    let Some(img) = render("box_faces_follow_the_key_light", &s, &opts) else {
        return;
    };
    let prepared = prepare_scene(&s, &opts);
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let faces = [
        ("front (-Y, lit)", Point3::new(0.5, 0.0, 0.5), -Vec3::Y),
        ("top (+Z, grazing)", Point3::new(0.5, 0.5, 1.0), Vec3::Z),
        ("right (+X, away)", Point3::new(1.0, 0.5, 0.5), Vec3::X),
    ];
    let mut l = Vec::new();
    for (name, p, n) in faces {
        let got = at(&img, &f, cam, p);
        let want = f.expected_srgb(p, n, &m);
        eprintln!("{name}: {got:?}, reference {want:?}");
        assert_close(got, want, 2, name);
        l.push(lum(got));
    }
    assert!(l[0] > l[1] && l[1] > l[2], "lit > grazing > away: {l:?}");
}

/// The largest jump between neighbouring pixels across the middle row of a
/// cylinder seen from the front, away from its outline.
fn max_row_jump(img: &RenderedRgba3d, f: &Frame3d<'_>, cam: &Camera3d, r: f64) -> f64 {
    let (x0, y, _) = cam
        .project(Point3::new(-r, 0.0, 1.0), f.width, f.height, f.near_far)
        .unwrap();
    let (x1, _, _) = cam
        .project(Point3::new(r, 0.0, 1.0), f.width, f.height, f.near_far)
        .unwrap();
    let (a, b) = (x0 as u32 + 4, x1 as u32 - 4);
    let l = |x: u32| lum(px(img, x, y as u32));
    // A seam is a step: a large second difference. A smooth gradient (even
    // the steep flank of a highlight) has a small one.
    (a..b)
        .map(|x| (l(x + 1) - 2.0 * l(x) + l(x - 1)).abs())
        .fold(0.0, f64::max)
}

/// A tessellated cylinder shades smooth at the default crease angle: along
/// a row across its side the second difference of brightness stays at or
/// below 3 levels, while the same mesh with flat facets (crease 0) shows
/// steps of at least twice that.
#[test]
fn cylinder_shows_no_facet_seams() {
    let r = 1.0;
    let mesh = shapes::cylinder(r, 2.0, 48);
    let m = Material::from_srgb8(200, 200, 200);
    let opts = RenderOptions3d::default();
    let mut jumps = Vec::new();
    for crease in [cadview_core::scene3d::DEFAULT_CREASE_DEG, 0.0] {
        let mut s = Scene3d {
            edges: None,
            camera: Camera3d::new(NamedView::Front, Projection::Orthographic),
            ..Scene3d::default()
        };
        s.items.push(SceneItem {
            mesh: Arc::new(RenderMesh::from_trimesh(&mesh, crease)),
            placement: Placement::IDENTITY,
            material: m,
            section: None,
        });
        let Some(img) = render("cylinder_shows_no_facet_seams", &s, &opts) else {
            return;
        };
        let prepared = prepare_scene(&s, &opts);
        let f = frame_of(&prepared, &opts);
        jumps.push(max_row_jump(&img, &f, &prepared.camera, r));
    }
    eprintln!(
        "largest second difference across the side: smooth {:.2}, flat facets {:.2} levels",
        jumps[0], jumps[1]
    );
    assert!(jumps[0] <= 3.0, "smooth cylinder has a seam");
    assert!(jumps[1] > 3.0 * 2.0, "flat facets should show seams");
}

/// Edges: a dark pixel where the geometry puts a box edge, none across a
/// cylinder's side, and dark pixels at its two outlines.
#[test]
fn edges_are_drawn_where_the_geometry_says() {
    let opts = RenderOptions3d::default();
    let ink = EdgeStyle::default().color;

    // The vertical edge nearest the eye in the isometric view (x = 1,
    // y = 0), between the lit front face and the right face.
    let mut s = one(&unit_box(), Material::from_srgb8(200, 200, 200));
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
    let Some(img) = render("edges_are_drawn_where_the_geometry_says", &s, &opts) else {
        return;
    };
    let prepared = prepare_scene(&s, &opts);
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let width = EdgeStyle::default().width_px;
    for z in [0.25, 0.5, 0.75] {
        let (x, y, _) = cam
            .project(Point3::new(1.0, 0.0, z), f.width, f.height, f.near_far)
            .unwrap();
        let (x, y) = (x.round() as u32, y as u32);
        // Ink across the row, in linear light: each pixel's share of the
        // way from its face colour to the ink colour. Sums to the line
        // width wherever the line falls within the pixel grid.
        let (left, right) = (px(&img, x - 5, y), px(&img, x + 5, y));
        let share = |p: [u8; 3], face: [u8; 3]| (lin(face) - lin(p)) / (lin(face) - lin(ink));
        let ink_px: f64 = (x - 3..x)
            .map(|xi| share(px(&img, xi, y), left))
            .chain((x..x + 3).map(|xi| share(px(&img, xi, y), right)))
            .sum();
        eprintln!("box edge at z = {z}: {ink_px:.2} px of ink (line width {width})");
        assert!((ink_px - width).abs() <= 0.5, "box edge drawn at z = {z}");
    }

    // Cylinder from the front, with and without edges: identical across
    // the side (no line on a smooth face), ink at the two outlines.
    let r = 1.0;
    let mut s = one(
        &shapes::cylinder(r, 2.0, 48),
        Material::from_srgb8(200, 200, 200),
    );
    s.camera = Camera3d::new(NamedView::Front, Projection::Orthographic);
    let Some(with) = render("edges_are_drawn_where_the_geometry_says", &s, &opts) else {
        return;
    };
    let bare = render(
        "edges_are_drawn_where_the_geometry_says",
        &Scene3d {
            edges: None,
            ..s.clone()
        },
        &opts,
    )
    .unwrap();
    let prepared = prepare_scene(&s, &opts);
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let (x0, y, _) = cam
        .project(Point3::new(-r, 0.0, 1.0), f.width, f.height, f.near_far)
        .unwrap();
    let (x1, _, _) = cam
        .project(Point3::new(r, 0.0, 1.0), f.width, f.height, f.near_far)
        .unwrap();
    let (x0, x1, y) = (x0.round() as u32, x1.round() as u32, y as u32);
    for x in x0 + 2..x1 - 2 {
        assert_eq!(
            px(&with, x, y),
            px(&bare, x, y),
            "line across the side at x = {x}"
        );
    }
    // Inside each outline, the line's inner half darkens the shading.
    let share = |x: u32| {
        let (a, b) = (lin(px(&with, x, y)), lin(px(&bare, x, y)));
        ((b - a) / (b - lin(ink))).max(0.0)
    };
    let left: f64 = (x0..x0 + 2).map(share).sum();
    let right: f64 = (x1 - 2..x1).map(share).sum();
    eprintln!(
        "cylinder outline ink inside: left {left:.2} px, right {right:.2} px (half width {})",
        width / 2.0
    );
    assert!(left >= 0.4 && right >= 0.4, "outline lines drawn");
}

/// Two renders of one scene are byte-identical, and the PNG decodes to
/// the same pixels.
#[test]
fn renders_are_deterministic() {
    let mut s = Scene3d::default();
    s.add_mesh(
        &shapes::plate_with_square_hole(10.0, 4.0, 2.0),
        Placement::IDENTITY,
        Material::from_srgb8(170, 190, 210),
    );
    s.add_mesh(
        &shapes::sphere(2.0, 24, 48),
        Placement::translation(0.0, 0.0, 4.0),
        Material {
            metallic: 1.0,
            ..Material::default()
        },
    );
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Perspective);
    let opts = RenderOptions3d::default();
    let Some(a) = render("renders_are_deterministic", &s, &opts) else {
        return;
    };
    let b = render("renders_are_deterministic", &s, &opts).unwrap();
    assert_eq!(a.rgba, b.rgba);
    let png = render_png(&s, &opts).unwrap();
    let (back, w, h) = cadview_render::decode_png(&png.png).unwrap();
    assert_eq!((w, h), (960, 640));
    assert_eq!(back, a.rgba);
    // The one shared device: the 3D renderer opened none of its own.
    assert_eq!(cadview_render::context::devices_created(), 1);
}

/// Size and pixel ratio behave like the 2D renderer's: the output is
/// `size * pixel_ratio` pixels and shows the same picture; a non-square
/// size still frames the whole scene.
#[test]
fn size_and_pixel_ratio() {
    let mut s = one(&unit_box(), Material::from_srgb8(200, 200, 200));
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
    let o1 = RenderOptions3d {
        size: [240, 160],
        ..Default::default()
    };
    let o2 = RenderOptions3d {
        pixel_ratio: 2.0,
        ..o1.clone()
    };
    let Some(a) = render("size_and_pixel_ratio", &s, &o1) else {
        return;
    };
    let b = render("size_and_pixel_ratio", &s, &o2).unwrap();
    assert_eq!((a.width, a.height), (240, 160));
    assert_eq!((b.width, b.height), (480, 320));
    // Box-filter the 2x image down and compare.
    let mut total = 0.0;
    let mut worst_px = 0usize;
    for y in 0..160 {
        for x in 0..240 {
            let mut sum = [0.0; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(&b, 2 * x + dx, 2 * y + dy);
                for c in 0..3 {
                    sum[c] += p[c] as f64 / 4.0;
                }
            }
            let p = px(&a, x, y);
            let d = (0..3)
                .map(|c| (sum[c] - p[c] as f64).abs())
                .fold(0.0, f64::max);
            total += d;
            if d > 32.0 {
                worst_px += 1;
            }
        }
    }
    let mean = total / (240.0 * 160.0);
    eprintln!("1x vs 2x downsampled: mean difference {mean:.2} levels, {worst_px} pixels over 32");
    assert!(mean < 1.5, "same picture at both ratios");
    assert!(worst_px < 240, "differences only along edges");

    // Tall image: the whole box still fits, centred.
    let tall = RenderOptions3d {
        size: [100, 300],
        ..Default::default()
    };
    let t = render("size_and_pixel_ratio", &s, &tall).unwrap();
    assert_eq!((t.width, t.height), (100, 300));
    let bg = LIGHT_BACKGROUND;
    for x in 0..100 {
        assert_eq!(px(&t, x, 0), bg);
        assert_eq!(px(&t, x, 299), bg);
    }
    for y in 0..300 {
        assert_eq!(px(&t, 0, y), bg);
        assert_eq!(px(&t, 99, y), bg);
    }
    assert_ne!(px(&t, 50, 150), bg);
}

#[test]
fn presentation_quality_is_not_available_yet() {
    let s = one(&unit_box(), Material::default());
    let opts = RenderOptions3d {
        quality: Quality::Presentation,
        ..Default::default()
    };
    assert_eq!(
        render_rgba(&s, &opts).unwrap_err(),
        Render3dError::NotAvailable(Quality::Presentation)
    );
}

// ── Outline, depth cues, cutaway, many parts, contact sheet ───────────

fn iso(mut s: Scene3d) -> Scene3d {
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
    s
}

/// Convex hull of pixel points (monotone chain).
fn hull(mut p: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    p.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let half = |pts: &mut dyn Iterator<Item = (f64, f64)>| {
        let mut h: Vec<(f64, f64)> = Vec::new();
        for q in pts {
            while h.len() >= 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
        h
    };
    let mut lower = half(&mut p.clone().into_iter());
    lower.extend(half(&mut p.into_iter().rev()));
    lower
}

/// The silhouette of a box contrasts with the background: on its outline
/// (from the silhouette itself to 3 px outside, at 18 points around it)
/// the best pixel has a WCAG contrast ratio of at least 3 with the
/// background, on the light default and on the dark viewer background.
/// Control: on the dark background with the outline switched off it stays
/// below 3.
#[test]
fn outline_contrasts_with_dark_and_light_backgrounds() {
    let opts = RenderOptions3d::default();
    for (name, bg, outline) in [
        ("light", LIGHT_BACKGROUND, true),
        ("dark", VIEWER_BACKGROUND, true),
        ("dark, outline off", VIEWER_BACKGROUND, false),
    ] {
        let mut s = iso(one(&unit_box(), Material::from_srgb8(200, 200, 200)));
        s.background = Background::Solid(bg);
        s.edges = Some(EdgeStyle {
            outline,
            ..EdgeStyle::default()
        });
        let Some(img) = render(
            "outline_contrasts_with_dark_and_light_backgrounds",
            &s,
            &opts,
        ) else {
            return;
        };
        let prepared = prepare_scene(&s, &opts);
        let f = frame_of(&prepared, &opts);
        let corners: Vec<(f64, f64)> = (0..8)
            .map(|i| {
                let p = Point3::new((i & 1) as f64, ((i >> 1) & 1) as f64, (i >> 2) as f64);
                let (x, y, _) = prepared
                    .camera
                    .project(p, f.width, f.height, f.near_far)
                    .unwrap();
                (x, y)
            })
            .collect();
        let h = hull(corners);
        let c = (
            h.iter().map(|p| p.0).sum::<f64>() / h.len() as f64,
            h.iter().map(|p| p.1).sum::<f64>() / h.len() as f64,
        );
        let mut worst = f64::INFINITY;
        let mut points = 0;
        for i in 0..h.len() {
            let (a, b) = (h[i], h[(i + 1) % h.len()]);
            let len = (b.0 - a.0).hypot(b.1 - a.1);
            // Corners that project onto each other leave tiny hull edges.
            if len < 1.0 {
                continue;
            }
            for t in [0.25, 0.5, 0.75] {
                points += 1;
                let m = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                let mut n = (-(b.1 - a.1) / len, (b.0 - a.0) / len);
                if (m.0 - c.0) * n.0 + (m.1 - c.1) * n.1 < 0.0 {
                    n = (-n.0, -n.1);
                }
                let best = (0..=6)
                    .map(|k| {
                        let d = k as f64 * 0.5;
                        let p = px(&img, (m.0 + n.0 * d) as u32, (m.1 + n.1 * d) as u32);
                        contrast_ratio(p, bg)
                    })
                    .fold(1.0, f64::max);
                worst = worst.min(best);
            }
        }
        assert_eq!(points, 18, "six outline edges");
        eprintln!("{name}: least outline contrast {worst:.2}:1 at {points} points");
        if outline {
            assert!(worst >= 3.0, "{name}: outline contrast {worst:.2}");
        } else {
            assert!(worst < 3.0, "{name}: control should not read");
        }
    }
}

/// Seen from the top, the floor of a 12 x 12 x 12 mm pocket in a
/// 60 x 60 x 30 block has the same normal as the top face, so without
/// occlusion both shade the same; with the default occlusion the floor's
/// centre is at least 20 levels darker, while the open top face keeps the
/// reference shading.
#[test]
fn pocket_floor_is_darker_than_the_face_around_it() {
    let test = "pocket_floor_is_darker_than_the_face_around_it";
    let m = Material::from_srgb8(200, 200, 200);
    let mut s = one(&shapes::block_with_pocket(60.0, 12.0, 30.0, 12.0), m);
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    let opts = RenderOptions3d::default();
    let Some(img) = render(test, &s, &opts) else {
        return;
    };
    let bare_scene = Scene3d {
        occlusion: None,
        ..s.clone()
    };
    let bare = render(test, &bare_scene, &opts).unwrap();
    let prepared = prepare_scene(&s, &opts);
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let (floor_p, top_p) = (Point3::new(0.0, 0.0, 18.0), Point3::new(18.0, 18.0, 30.0));
    let want = f.expected_srgb(top_p, Vec3::Z, &m);
    let (floor, top) = (at(&img, &f, cam, floor_p), at(&img, &f, cam, top_p));
    assert_close(top, want, 2, "open top face keeps the reference shading");
    assert_close(
        at(&bare, &f, cam, floor_p),
        want,
        2,
        "without occlusion the floor shades like the top",
    );
    let darker = lum(top) - lum(floor);
    eprintln!("pocket floor {floor:?} against top {top:?}: {darker:.1} levels darker");
    assert!(
        darker >= 20.0,
        "pocket floor only {darker:.1} levels darker"
    );
}

/// A 20 mm cube on a shadow-catching ground, isometric: the ground 0.5 mm
/// in front of the cube is at least 15 levels darker than the ground 8 mm
/// away, which shows the plain background colour.
#[test]
fn ground_next_to_a_box_is_darker_than_far_from_it() {
    let test = "ground_next_to_a_box_is_darker_than_far_from_it";
    let mut s = iso(one(
        &shapes::cuboid(Point3::ORIGIN, Point3::new(20.0, 20.0, 20.0)),
        Material::from_srgb8(200, 200, 200),
    ));
    s.ground = Some(Ground::default());
    let opts = RenderOptions3d::default();
    let Some(img) = render(test, &s, &opts) else {
        return;
    };
    let prepared = prepare_scene(&s, &opts);
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let near = at(&img, &f, cam, Point3::new(10.0, -0.5, 0.0));
    let far = at(&img, &f, cam, Point3::new(10.0, -8.0, 0.0));
    let darker = lum(far) - lum(near);
    eprintln!("ground next to the cube {near:?}, far {far:?}: {darker:.1} levels darker");
    assert_close(far, LIGHT_BACKGROUND, 1, "far ground is the background");
    assert!(darker >= 15.0, "contact shadow only {darker:.1} levels");
}

/// With neither ground nor occlusion the depth-cue passes change nothing:
/// the image is byte-identical to one with occlusion at strength 0, faces
/// keep the reference shading, and switching occlusion on does change
/// pixels (so the comparison is not vacuous).
#[test]
fn without_ground_or_occlusion_renders_as_before() {
    let test = "without_ground_or_occlusion_renders_as_before";
    let m = Material::from_srgb8(170, 190, 210);
    let mut s = iso(one(&shapes::block_with_pocket(60.0, 20.0, 30.0, 15.0), m));
    s.add_mesh(
        &shapes::cylinder(8.0, 40.0, 48),
        Placement::translation(45.0, 0.0, 0.0),
        Material::from_srgb8(214, 160, 92),
    );
    let off = Scene3d {
        occlusion: None,
        ground: None,
        ..s.clone()
    };
    let zero = Scene3d {
        occlusion: Some(Occlusion {
            strength: 0.0,
            ..Occlusion::default()
        }),
        ..off.clone()
    };
    let opts = RenderOptions3d::default();
    let Some(a) = render(test, &off, &opts) else {
        return;
    };
    let b = render(test, &zero, &opts).unwrap();
    assert_eq!(a.rgba, b.rgba, "occlusion at strength 0 is the identity");
    let on = render(test, &s, &opts).unwrap();
    let changed = a
        .rgba
        .chunks(4)
        .zip(on.rgba.chunks(4))
        .filter(|(x, y)| x != y)
        .count();
    eprintln!("occlusion on changes {changed} pixels");
    assert!(changed > 1000);
    let prepared = prepare_scene(&off, &opts);
    let f = frame_of(&prepared, &opts);
    let p = Point3::new(-20.0, -20.0, 30.0);
    assert_close(
        at(&a, &f, &prepared.camera, p),
        f.expected_srgb(p, Vec3::Z, &m),
        2,
        "top face",
    );
}

/// A box with an inner cavity, cut in half by a clip plane, isometric:
/// on the cut, where there is material, the pixel is the flat section
/// colour (given, or the derived default; within 1 level); where the cut
/// opens the cavity, the pixel shows the cavity's side wall with its
/// reference shading, where the uncut box shows its top face. Also with a
/// ground that touches the cut faces from below.
#[test]
fn cutaway_shows_section_and_cavity() {
    let test = "cutaway_shows_section_and_cavity";
    let m = Material::from_srgb8(176, 190, 204);
    let mesh = shapes::box_with_cavity(
        Point3::new(-40.0, -30.0, 0.0),
        Point3::new(40.0, 30.0, 40.0),
        Point3::new(-30.0, -20.0, 8.0),
        Point3::new(30.0, 20.0, 32.0),
    );
    let mut s = iso(one(&mesh, m));
    s.occlusion = None;
    let opts = RenderOptions3d::default();
    let Some(uncut) = render(test, &s, &opts) else {
        return;
    };
    let uncut_scene = prepare_scene(&s, &opts);
    let uncut_f = frame_of(&uncut_scene, &opts);
    s.clip_planes.push(ClipPlane {
        point: Point3::ORIGIN,
        normal: -Vec3::Y,
    });
    let material = [Point3::new(-35.0, 0.0, 20.0), Point3::new(0.0, 0.0, 4.0)];
    // On the cavity's x = -30 wall (facing +X); the view ray from it leaves
    // through the cut at (-25, 0, 25), inside the opening. Without the cut
    // it leaves through the top face at (-10, -15, 40).
    let wall = Point3::new(-30.0, 5.0, 20.0);
    let top = uncut_f.expected_srgb(Point3::new(-10.0, -15.0, 40.0), Vec3::Z, &m);
    let (x, y, _) = uncut_scene
        .camera
        .project(wall, uncut_f.width, uncut_f.height, uncut_f.near_far)
        .unwrap();
    assert_close(px(&uncut, x as u32, y as u32), top, 2, "uncut: top face");
    for (section, ground) in [
        (Some([200u8, 60, 60]), false),
        (None, false),
        (Some([60, 120, 200]), true),
    ] {
        s.items[0].section = section;
        s.ground = ground.then(Ground::default);
        let img = render(test, &s, &opts).unwrap();
        let prepared = prepare_scene(&s, &opts);
        let f = frame_of(&prepared, &opts);
        let cam = &prepared.camera;
        let want = section.unwrap_or_else(|| default_section_color(&m).map(encode_srgb));
        for p in material {
            assert_close(at(&img, &f, cam, p), want, 1, "section colour on material");
        }
        let seen = at(&img, &f, cam, wall);
        let want_wall = f.expected_srgb(wall, Vec3::X, &m);
        assert_close(seen, want_wall, 2, "cavity wall through the cut");
        assert!((lum(top) - lum(want_wall)).abs() > 10.0);
        eprintln!(
            "section {section:?}, ground {ground}: material {:?}, cavity wall {seen:?} (uncut: top face {top:?})",
            at(&img, &f, cam, material[0])
        );
    }
}

/// A quarter cutaway: two clip planes with `ClipMode::All` take a wedge
/// out of a 100 mm cube (x > 50 and y < 50), isometric. Where the view
/// ray crosses the wedge it meets the cut face in the section colour
/// (within 1 level); a point beyond only one plane keeps its face's
/// reference shading (within 2), where `ClipMode::Any` removes it. The
/// camera fit is the uncut one: the wedge takes nothing off the box.
#[test]
fn quarter_cutaway_removes_only_the_wedge() {
    use cadview_core::scene3d::ClipMode;
    let test = "quarter_cutaway_removes_only_the_wedge";
    let m = Material::from_srgb8(176, 190, 204);
    let mesh = shapes::cuboid(Point3::ORIGIN, Point3::new(100.0, 100.0, 100.0));
    let mut s = iso(one(&mesh, m));
    s.occlusion = None;
    let opts = RenderOptions3d::default();
    let uncut = prepare_scene(&s, &opts);
    for normal in [Vec3::X, -Vec3::Y] {
        s.clip_planes.push(ClipPlane {
            point: Point3::new(50.0, 50.0, 0.0),
            normal,
        });
    }
    s.clip_mode = ClipMode::All;
    let Some(img) = render(test, &s, &opts) else {
        return;
    };
    let prepared = prepare_scene(&s, &opts);
    assert_eq!(prepared.camera, uncut.camera, "the fit keeps the whole box");
    let f = frame_of(&prepared, &opts);
    let cam = &prepared.camera;
    let section = default_section_color(&m).map(encode_srgb);
    // The view ray through (90, 20, 100) runs in the wedge until it meets
    // the cut face y = 50 at (60, 50, 70).
    assert_close(
        at(&img, &f, cam, Point3::new(90.0, 20.0, 100.0)),
        section,
        1,
        "cut face seen through the wedge",
    );
    // Beyond one plane only: kept.
    let top = Point3::new(75.0, 75.0, 100.0);
    let front = Point3::new(25.0, 0.0, 50.0);
    let kept = [(top, Vec3::Z, "top face"), (front, -Vec3::Y, "front face")];
    for (p, n, what) in kept {
        assert_close(at(&img, &f, cam, p), f.expected_srgb(p, n, &m), 2, what);
    }
    // The same planes with ClipMode::Any remove both.
    s.clip_mode = ClipMode::Any;
    let any = render(test, &s, &opts).unwrap();
    let pa = prepare_scene(&s, &opts);
    let fa = frame_of(&pa, &opts);
    for (p, n, what) in kept {
        let seen = at(&any, &fa, &pa.camera, p);
        let face = fa.expected_srgb(p, n, &m);
        assert!(
            seen.iter().zip(face).any(|(a, b)| a.abs_diff(b) > 8),
            "any: {what} is removed, got {seen:?}"
        );
    }
    let cut = img
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| (0..3).all(|i| p[i].abs_diff(section[i]) <= 1))
        .count();
    eprintln!("quarter cutaway: {cut} section pixels");
    assert!(cut > 1000, "{cut}");
}

/// 500 boxes: with one shared mesh the renderer prepares one mesh, with a
/// mesh each it prepares 500; both give the same bytes, warm renders equal
/// the first, and a warm render at 960x640 takes under a second.
#[test]
fn shared_meshes_are_prepared_once() {
    let board = shapes::cuboid(Point3::ORIGIN, Point3::new(90.0, 18.0, 45.0));
    let scene = |shared: bool| {
        let one = Arc::new(RenderMesh::from_trimesh(&board, DEFAULT_CREASE_DEG));
        let mut s = iso(Scene3d {
            ground: Some(Ground::default()),
            ..Scene3d::default()
        });
        for i in 0..500usize {
            let mesh = if shared {
                one.clone()
            } else {
                Arc::new(RenderMesh::from_trimesh(&board, DEFAULT_CREASE_DEG))
            };
            let p = Placement::translation((i % 25) as f64 * 110.0, (i / 25) as f64 * 40.0, 0.0);
            s.add_shared(mesh, p, Material::from_srgb8(200, 180, 150));
        }
        s
    };
    let opts = RenderOptions3d::default();
    let (w, h) = opts.pixel_size();
    let mut images = Vec::new();
    for (shared, expect) in [(true, 1), (false, 500)] {
        let s = prepare_scene(&scene(shared), &opts);
        let frame = build_frame(&s, w, h);
        let run = cadview_render::gpu::with_shared(|r| {
            let ctx = r.context();
            let gpu = Gpu {
                device: &ctx.device,
                queue: &ctx.queue,
            };
            let mut rr = RasterRenderer::new(&ctx.device);
            let t = Instant::now();
            let first = render_frame_rgba(gpu, &mut rr, &frame).unwrap();
            let first_ms = t.elapsed().as_secs_f64() * 1e3;
            let cached = rr.cached_meshes();
            let mut warm = Vec::new();
            for _ in 0..3 {
                let t = Instant::now();
                assert_eq!(render_frame_rgba(gpu, &mut rr, &frame).unwrap(), first);
                warm.push(t.elapsed().as_secs_f64() * 1e3);
            }
            (first, first_ms, cached, warm)
        });
        let (first, first_ms, cached, warm) = match run {
            Ok(r) => r,
            Err(e) if e.contains("no GPU adapter") => {
                eprintln!("SKIPPED shared_meshes_are_prepared_once: {e}");
                return;
            }
            Err(e) => panic!("{e}"),
        };
        eprintln!(
            "500 boxes, shared {shared}: {cached} meshes prepared, first {first_ms:.1} ms (incl. pipelines), warm {warm:.1?} ms"
        );
        assert_eq!(cached, expect);
        assert!(warm.iter().all(|&t| t < 1000.0));
        images.push(first);
    }
    assert_eq!(
        images[0], images[1],
        "shared and distinct meshes draw the same"
    );
}

/// The contact sheet places each tile at its cell (bytes copied as they
/// are at scale 1), adds a caption bar with light text when there are
/// labels, and halves tiles at scale 0.5.
#[test]
fn contact_sheet_lays_out_tiles() {
    let test = "contact_sheet_lays_out_tiles";
    let opts = RenderOptions3d {
        size: [120, 80],
        ..Default::default()
    };
    let Some(a) = render(test, &iso(one(&unit_box(), Material::default())), &opts) else {
        return;
    };
    let mut cyl = one(
        &shapes::cylinder(1.0, 2.0, 32),
        Material::from_srgb8(214, 160, 92),
    );
    cyl.camera = Camera3d::new(NamedView::Front, Projection::Orthographic);
    let b = render(test, &cyl, &opts).unwrap();
    let gap = cadview_render3d::SHEET_GAP;
    let sheet = contact_sheet(&[(&a, ""), (&b, "")], 2, 1.0).unwrap();
    assert_eq!(
        (sheet.width, sheet.height),
        (2 * 120 + 3 * gap, 80 + 2 * gap)
    );
    for (y, x) in [(0, 0), (40, 60), (79, 119)] {
        assert_eq!(px(&sheet, gap + x, gap + y), px(&a, x, y));
        assert_eq!(px(&sheet, 2 * gap + 120 + x, gap + y), px(&b, x, y));
    }
    let labelled = contact_sheet(&[(&a, "box"), (&b, "cylinder")], 1, 0.5).unwrap();
    let cap = cadview_render3d::SHEET_CAPTION;
    assert_eq!(
        (labelled.width, labelled.height),
        (60 + 2 * gap, 2 * (40 + cap) + 3 * gap)
    );
    let bar_y = gap + 40;
    let text = (gap..gap + 60)
        .flat_map(|x| (bar_y..bar_y + cap).map(move |y| (x, y)))
        .filter(|&(x, y)| lum(px(&labelled, x, y)) > 128.0)
        .count();
    eprintln!("caption 'box': {text} light pixels");
    assert!(
        lum(px(&labelled, gap + 58, bar_y + 1)) < 64.0,
        "dark caption bar"
    );
    assert!(text > 10, "caption text drawn");
}
