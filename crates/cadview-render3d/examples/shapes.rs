//! Render procedural meshes to PNGs, showing each working-render feature,
//! and print timings.
//!
//! ```text
//! cargo run -p cadview-render3d --release --example shapes -- <out-dir> [--fallback]
//! ```
//!
//! Writes to `<out-dir>` (960x640 unless noted):
//!
//! - `shapes-iso.png`, `shapes-iso-persp.png`, `shapes-front.png`,
//!   `shapes-top.png`, `shapes-iso@2x.png` (pixel ratio 2): the default look
//!   (light background, edges, ambient occlusion);
//! - `shapes-ground.png`: the same on a ground with its contact shadow;
//! - `shapes-dark.png`: on the 2D viewer's dark background, where the
//!   outline turns light;
//! - `pocket.png`: a block with a blind pocket, on the ground;
//! - `cutaway.png`: a box with an inner cavity and a pin through it, cut in
//!   half by a clip plane;
//! - `assembly.png` and `exploded.png`: a small assembly in palette
//!   colours, assembled and exploded;
//! - `many-shared.png` and `many-distinct.png`: 500 boxes sharing one mesh,
//!   and 500 boxes with a mesh each, with first and warm render times;
//! - `contact-sheet.png`: all of the above in one labelled grid.
//!
//! With `--fallback` it also renders on wgpu's fallback (software) adapter,
//! if the platform has one, and writes `shapes-iso-fallback.png`.

use cadview_core::scene3d::{
    build_frame, palette_color, shapes, Background, Camera3d, ClipPlane, Ground, Material,
    NamedView, Projection, RenderMesh, Scene3d, DEFAULT_CREASE_DEG,
};
use cadview_core::solid::{Placement, Point3, Vec3};
use cadview_render3d::raster::RasterRenderer;
use cadview_render3d::{
    contact_sheet, prepare_scene, render_frame_rgba, render_png, render_rgba, Gpu, RenderOptions3d,
    RenderedRgba3d,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

fn shapes_scene() -> Scene3d {
    let mut s = Scene3d::default();
    // A 120 x 120 x 20 plate with a 50 mm square hole.
    s.add_mesh(
        &shapes::plate_with_square_hole(120.0, 50.0, 20.0),
        Placement::IDENTITY,
        Material::from_srgb8(176, 190, 204),
    );
    // A cylinder, a box and a sphere on top of it.
    s.add_mesh(
        &shapes::cylinder(18.0, 60.0, 64),
        Placement::translation(-35.0, 35.0, 20.0),
        Material::from_srgb8(214, 160, 92),
    );
    s.add_mesh(
        &shapes::cuboid(Point3::new(0.0, 0.0, 0.0), Point3::new(30.0, 30.0, 30.0)),
        Placement::axis_angle(Vec3::Z, 0.4, Vec3::new(25.0, 20.0, 20.0)),
        Material::from_srgb8(120, 168, 120),
    );
    s.add_mesh(
        &shapes::sphere(16.0, 32, 64),
        Placement::translation(35.0, -35.0, 36.0),
        Material {
            metallic: 1.0,
            roughness: 0.35,
            ..Material::from_srgb8(200, 200, 205)
        },
    );
    s
}

fn pocket_scene() -> Scene3d {
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::block_with_pocket(80.0, 24.0, 30.0, 20.0),
        Placement::IDENTITY,
        Material::from_srgb8(176, 190, 204),
    );
    s.add_mesh(
        &shapes::cylinder(8.0, 40.0, 48),
        Placement::translation(60.0, 0.0, 0.0),
        Material::from_srgb8(214, 160, 92),
    );
    s
}

fn cutaway_scene() -> Scene3d {
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::box_with_cavity(
            Point3::new(-40.0, -30.0, 0.0),
            Point3::new(40.0, 30.0, 40.0),
            Point3::new(-30.0, -20.0, 8.0),
            Point3::new(30.0, 20.0, 32.0),
        ),
        Placement::IDENTITY,
        Material::from_srgb8(176, 190, 204),
    );
    // A pin through the top into the cavity.
    s.add_mesh(
        &shapes::cylinder(6.0, 50.0, 48),
        Placement::translation(0.0, 0.0, 10.0),
        Material::from_srgb8(214, 160, 92),
    );
    // Remove the front half (y < 0).
    s.clip_planes.push(ClipPlane {
        point: Point3::ORIGIN,
        normal: -Vec3::Y,
    });
    s
}

/// A base, four posts and a top, in palette colours.
fn assembly_scene() -> Scene3d {
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    let mat = |i: usize| {
        let [r, g, b] = palette_color(i);
        Material::from_srgb8(r, g, b)
    };
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(-60.0, -40.0, 0.0),
            Point3::new(60.0, 40.0, 10.0),
        ),
        Placement::IDENTITY,
        mat(0),
    );
    let post = Arc::new(RenderMesh::from_trimesh(
        &shapes::cylinder(6.0, 50.0, 48),
        DEFAULT_CREASE_DEG,
    ));
    for (k, (x, y)) in [(-45.0, -25.0), (45.0, -25.0), (45.0, 25.0), (-45.0, 25.0)]
        .into_iter()
        .enumerate()
    {
        s.add_shared(post.clone(), Placement::translation(x, y, 10.0), mat(1 + k));
    }
    s.add_mesh(
        &shapes::block_with_pocket(100.0, 40.0, 12.0, 6.0),
        Placement::translation(0.0, 0.0, 60.0),
        mat(5),
    );
    s
}

/// 500 boxes in a 25 x 20 grid; `shared`: one mesh for all, else one each.
fn many_scene(shared: bool) -> Scene3d {
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    let board = shapes::cuboid(Point3::ORIGIN, Point3::new(90.0, 18.0, 45.0));
    let one = Arc::new(RenderMesh::from_trimesh(&board, DEFAULT_CREASE_DEG));
    for i in 0..500usize {
        let (x, y) = ((i % 25) as f64 * 110.0, (i / 25) as f64 * 40.0);
        let z = ((i * 7) % 5) as f64 * 12.0;
        let mesh = if shared {
            one.clone()
        } else {
            Arc::new(RenderMesh::from_trimesh(&board, DEFAULT_CREASE_DEG))
        };
        let [r, g, b] = palette_color(i % 12);
        s.add_shared(
            mesh,
            Placement::translation(x, y, z),
            Material::from_srgb8(r, g, b),
        );
    }
    s
}

fn write(dir: &Path, name: &str, png: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, png).expect("write png");
    path
}

fn save(dir: &Path, name: &str, r: &RenderedRgba3d) {
    println!("{}", write(dir, name, &r.png().expect("png")).display());
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.iter().find(|a| !a.starts_with("--")).expect("out dir"));
    let fallback = args.iter().any(|a| a == "--fallback");
    std::fs::create_dir_all(&dir).expect("create out dir");
    let opts = RenderOptions3d::default();
    let iso = |mut s: Scene3d| {
        s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
        s
    };

    // The default look from several views.
    let base = shapes_scene();
    let shots = [
        (
            "shapes-iso.png",
            NamedView::Isometric,
            Projection::Orthographic,
            1.0,
        ),
        (
            "shapes-iso-persp.png",
            NamedView::Isometric,
            Projection::Perspective,
            1.0,
        ),
        (
            "shapes-front.png",
            NamedView::Front,
            Projection::Orthographic,
            1.0,
        ),
        (
            "shapes-top.png",
            NamedView::Top,
            Projection::Orthographic,
            1.0,
        ),
        (
            "shapes-iso@2x.png",
            NamedView::Isometric,
            Projection::Orthographic,
            2.0,
        ),
    ];
    for (name, view, proj, ratio) in shots {
        let mut s = base.clone();
        s.camera = Camera3d::new(view, proj);
        let opts = RenderOptions3d {
            pixel_ratio: ratio,
            ..Default::default()
        };
        let r = render_png(&s, &opts).expect("render");
        let path = write(&dir, name, &r.png);
        println!(
            "{} {}x{} in {:.1} ms on {}",
            path.display(),
            r.width,
            r.height,
            r.ms,
            r.adapter
        );
    }

    // Every capability once, kept for the contact sheet.
    let mut sheet: Vec<(RenderedRgba3d, String)> = Vec::new();
    let mut shot = |name: &str, label: &str, s: &Scene3d| {
        let r = render_rgba(s, &opts).expect("render");
        save(&dir, name, &r);
        sheet.push((r, label.to_string()));
    };
    shot(
        "shapes-iso.png",
        "default: edges, occlusion",
        &iso(base.clone()),
    );
    shot(
        "shapes-ground.png",
        "ground and contact shadow",
        &iso(Scene3d {
            ground: Some(Ground::default()),
            ..base.clone()
        }),
    );
    shot(
        "shapes-dark.png",
        "dark background: light outline",
        &iso(Scene3d {
            background: Background::VIEWER,
            ..base.clone()
        }),
    );
    shot("pocket.png", "blind pocket", &iso(pocket_scene()));
    shot(
        "cutaway.png",
        "cutaway: cavity and pin",
        &iso(cutaway_scene()),
    );
    let assembly = iso(assembly_scene());
    shot("assembly.png", "assembly, palette colours", &assembly);
    let mut exploded = assembly.clone();
    exploded.explode(0.8, [false, false, true]);
    shot("exploded.png", "exploded along Z", &exploded);

    // Many parts: first (cold mesh cache) and warm render times.
    for (shared, name, label) in [
        (true, "many-shared.png", "500 boxes, one shared mesh"),
        (false, "many-distinct.png", "500 boxes, a mesh each"),
    ] {
        let s = iso(many_scene(shared));
        let t = Instant::now();
        let first = render_rgba(&s, &opts).expect("render");
        let first_ms = ms(t);
        let mut warm = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let r = render_rgba(&s, &opts).expect("render");
            warm.push(ms(t));
            assert_eq!(r.rgba, first.rgba, "warm render differs from the first");
        }
        println!("{label}: first {first_ms:.1} ms, warm {warm:.1?} ms (960x640, byte-identical)");
        save(&dir, name, &first);
        sheet.push((first, label.to_string()));
    }

    // The contact sheet.
    let tiles: Vec<(&RenderedRgba3d, &str)> = sheet.iter().map(|(r, l)| (r, l.as_str())).collect();
    let t = Instant::now();
    let grid = contact_sheet(&tiles, 3, 0.5).expect("contact sheet");
    println!(
        "contact sheet {}x{} in {:.1} ms",
        grid.width,
        grid.height,
        ms(t)
    );
    save(&dir, "contact-sheet.png", &grid);

    // Warm timings of the default scene at 960x640.
    let base = iso(base);
    let mut times = Vec::new();
    for _ in 0..5 {
        times.push(render_png(&base, &opts).expect("render").ms);
    }
    println!("warm 960x640 shapes-iso incl. PNG (ms): {times:.1?}");

    if fallback {
        render_fallback(&base, &opts, &dir);
    }
}

/// Render on wgpu's fallback adapter with a device of its own (a
/// measurement only: shipped code uses the shared device).
fn render_fallback(scene: &Scene3d, opts: &RenderOptions3d, dir: &Path) {
    let start = Instant::now();
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::PRIMARY;
    let instance = wgpu::Instance::new(desc);
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        force_fallback_adapter: true,
        ..Default::default()
    })) {
        Ok(a) => a,
        Err(e) => {
            println!("fallback adapter: none ({e})");
            return;
        }
    };
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("render3d-fallback"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("fallback device");
    let mut renderer = RasterRenderer::new(&device);
    println!(
        "fallback adapter {} ({:?}, {:?}) ready in {:.0} ms",
        info.name,
        info.backend,
        info.device_type,
        ms(start)
    );
    let (w, h) = opts.pixel_size();
    let s = prepare_scene(scene, opts);
    let frame = build_frame(&s, w, h);
    let gpu = Gpu {
        device: &device,
        queue: &queue,
    };
    let mut times = Vec::new();
    let mut last = Vec::new();
    for _ in 0..3 {
        let t = Instant::now();
        last = render_frame_rgba(gpu, &mut renderer, &frame).expect("fallback render");
        times.push(ms(t));
    }
    println!("fallback 960x640 render + readback (ms): {times:.1?}");
    let png = cadview_render::encode_png(&last, w, h).expect("png");
    let path = write(dir, "shapes-iso-fallback.png", &png);
    println!("{}", path.display());
}
