//! GPU evidence for the presentation render (the path tracer), measured
//! against closed forms, against the core's patterns and against the
//! working render. Every test prints SKIPPED and passes when the machine
//! has no GPU adapter. Set CODECAD_PRESENT_OUT to a folder to keep the
//! images.

use cadview_core::scene3d::{
    build_frame, pattern_albedo, shapes, srgb_to_linear, Background, Bands, Brushed, Camera3d,
    Environment, Ground, LightFrame, Lighting, Material, NamedView, Pattern, PatternAxis,
    Projection, Scene3d, Sun,
};
use cadview_core::solid::{Placement, Point3, TriMesh, Vec3};
use cadview_render3d::trace::{PathTracer, TraceSettings};
use cadview_render3d::{
    prepare_scene, render_frame_rgba, render_png, render_radiance, render_rgba, render_rgba_stats,
    Gpu, Quality, Radiance3d, Render3dError, RenderOptions3d, RenderedRgba3d,
};
use std::f64::consts::PI;

fn skip(test: &str, e: &Render3dError) -> bool {
    match e {
        Render3dError::Gpu(m) if m.contains("no GPU adapter") => {
            eprintln!("SKIPPED {test}: {m}");
            true
        }
        _ => false,
    }
}

/// Linear radiance, or `None` (after printing SKIPPED) without a GPU.
fn radiance(test: &str, scene: &Scene3d, opts: &RenderOptions3d) -> Option<Radiance3d> {
    match render_radiance(scene, opts) {
        Ok(r) => {
            eprintln!(
                "{test}: {}x{} at {} spp, {} passes, {:.0} ms tracing",
                r.width, r.height, r.stats.samples, r.stats.passes, r.stats.trace_ms
            );
            Some(r)
        }
        Err(e) if skip(test, &e) => None,
        Err(e) => panic!("{test}: {e}"),
    }
}

fn image(test: &str, scene: &Scene3d, opts: &RenderOptions3d) -> Option<RenderedRgba3d> {
    match render_rgba(scene, opts) {
        Ok(r) => Some(r),
        Err(e) if skip(test, &e) => None,
        Err(e) => panic!("{test}: {e}"),
    }
}

/// Keep `img` as a PNG when CODECAD_PRESENT_OUT names a folder.
fn keep(name: &str, img: &RenderedRgba3d) {
    if let Ok(dir) = std::env::var("CODECAD_PRESENT_OUT") {
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, img.png().unwrap()).unwrap();
    }
}

fn opts(w: u32, h: u32, spp: u32) -> RenderOptions3d {
    RenderOptions3d {
        size: [w, h],
        quality: Quality::Presentation,
        spp,
        ..Default::default()
    }
}

/// A Lambert surface with albedo `rho` and no specular reflection
/// (index of refraction 1).
fn lambert(rho: f64) -> Material {
    Material {
        base_color: [rho; 3],
        roughness: 1.0,
        ior: 1.0,
        ..Material::default()
    }
}

fn lin_mean(p: &[f32; 3]) -> f64 {
    (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0
}

/// A scene lit only by `env`, no edges or occlusion, no ground.
fn lit_by(env: Environment) -> Scene3d {
    Scene3d {
        environment: env,
        edges: None,
        occlusion: None,
        ..Scene3d::default()
    }
}

fn mean_where(r: &Radiance3d, keep: impl Fn(u32, u32) -> bool) -> (f64, usize) {
    let mut sum = 0.0;
    let mut n = 0;
    for y in 0..r.height {
        for x in 0..r.width {
            if keep(x, y) {
                sum += lin_mean(&r.pixels[(y * r.width + x) as usize]);
                n += 1;
            }
        }
    }
    (sum / n.max(1) as f64, n)
}

/// The furnace: white objects in a uniform white environment come out at
/// exactly the environment's radiance when the renderer neither loses nor
/// gains energy. A Lambert block with a pocket (light bounces many times
/// inside it), a smooth glass sphere (Fresnel splits every ray) and a
/// white mirror sphere. Tolerance: 0.5 % on each object's mean.
#[test]
fn furnace_keeps_energy() {
    let test = "furnace_keeps_energy";
    let l = 1.0;
    let glass = Material {
        base_color: [1.0; 3],
        roughness: 0.0,
        transmission: 1.0,
        ..Material::default()
    };
    let mirror = Material {
        base_color: [1.0; 3],
        roughness: 0.0,
        metallic: 1.0,
        ..Material::default()
    };
    let cases: [(&str, TriMesh, Material, f64); 3] = [
        (
            "lambert pocket",
            shapes::block_with_pocket(10.0, 6.0, 6.0, 5.0),
            lambert(1.0),
            0.005,
        ),
        ("glass sphere", shapes::sphere(5.0, 48, 96), glass, 0.005),
        ("mirror sphere", shapes::sphere(5.0, 48, 96), mirror, 0.005),
    ];
    for (name, mesh, m, tol) in cases {
        let mut s = lit_by(Environment::uniform([l; 3]));
        s.add_mesh(&mesh, Placement::IDENTITY, m);
        s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
        let o = opts(160, 120, 256);
        let Some(r) = radiance(test, &s, &o) else {
            return;
        };
        let (mean, n) = mean_where(&r, |_, _| true);
        let worst = r
            .pixels
            .iter()
            .map(|p| (lin_mean(p) - l).abs())
            .fold(0.0, f64::max);
        eprintln!(
            "{name}: mean {mean:.5} over {n} px (environment {l}), error {:.3} %, worst pixel {worst:.3}",
            (mean - l) / l * 100.0
        );
        assert!((mean - l).abs() <= tol * l, "{name}: {mean}");
    }
}

/// The camera looking straight at a plate whose normal is `n` (unit).
fn facing(n: Vec3) -> Camera3d {
    let mut c = Camera3d::new(NamedView::Top, Projection::Orthographic);
    c.pitch = n.z.clamp(-1.0, 1.0).asin();
    c.yaw = n.y.atan2(n.x);
    if n.x == 0.0 && n.y == 0.0 {
        c.yaw = -PI / 2.0;
    }
    c
}

/// A thin square plate centred at the origin with normal `n`.
fn plate(s: &mut Scene3d, n: Vec3, m: Material) {
    let mesh = shapes::cuboid(
        Point3::new(-50.0, -50.0, -0.5),
        Point3::new(50.0, 50.0, 0.0),
    );
    // Rotate +Z onto n.
    let axis = Vec3::Z.cross(n);
    let angle = n.z.clamp(-1.0, 1.0).acos();
    let p = match axis.normalized() {
        Some(a) => Placement::axis_angle(a, angle, Vec3::new(0.0, 0.0, 0.0)),
        None => Placement::IDENTITY,
    };
    s.add_mesh(&mesh, p, m);
}

/// Mean radiance of the central quarter of the image.
fn centre(r: &Radiance3d) -> f64 {
    let (w, h) = (r.width, r.height);
    mean_where(r, |x, y| {
        x >= w * 3 / 8 && x < w * 5 / 8 && y >= h * 3 / 8 && y < h * 5 / 8
    })
    .0
}

/// A Lambert plate tilted by beta under a uniform upper hemisphere (and
/// nothing below the horizon) receives `pi L (1 + cos beta) / 2`, so it
/// reflects `rho L (1 + cos beta) / 2`. Under a small sun of irradiance E
/// at angle theta from its normal, and nothing else, it reflects
/// `rho E cos(theta) / pi`. Tolerance 1 %.
#[test]
fn plates_match_closed_form_irradiance() {
    let test = "plates_match_closed_form_irradiance";
    let rho = 0.5;
    let dome = Environment {
        zenith: [1.0; 3],
        horizon: [1.0; 3],
        ground: [0.0; 3],
        sun: None,
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    };
    for beta_deg in [0.0f64, 60.0, 90.0] {
        let b: f64 = beta_deg.to_radians();
        let n = Vec3::new(0.0, -b.sin(), b.cos());
        let mut s = lit_by(dome);
        plate(&mut s, n, lambert(rho));
        s.camera = facing(n);
        let Some(r) = radiance(test, &s, &opts(96, 96, 256)) else {
            return;
        };
        let got = centre(&r);
        let want = rho * (1.0 + b.cos()) / 2.0;
        eprintln!(
            "dome, tilt {beta_deg} deg: {got:.5}, closed form {want:.5} ({:+.2} %)",
            (got - want) / want * 100.0
        );
        assert!((got - want).abs() <= 0.01 * want, "tilt {beta_deg}");
    }
    let e = 2.0;
    for theta_deg in [0.0f64, 60.0] {
        let t: f64 = theta_deg.to_radians();
        let mut s = lit_by(Environment {
            zenith: [0.0; 3],
            horizon: [0.0; 3],
            ground: [0.0; 3],
            sun: Some(Sun {
                direction: Vec3::new(t.sin(), 0.0, t.cos()),
                frame: LightFrame::World,
                angle_deg: 0.53,
                irradiance: e,
                color: [1.0; 3],
            }),
            intensity: 1.0,
            sweep: false,
            haze: 0.0,
        });
        plate(&mut s, Vec3::Z, lambert(rho));
        s.camera = facing(Vec3::Z);
        let Some(r) = radiance(test, &s, &opts(96, 96, 64)) else {
            return;
        };
        let got = centre(&r);
        let want = rho * e * t.cos() / PI;
        eprintln!(
            "sun at {theta_deg} deg: {got:.5}, cosine law {want:.5} ({:+.2} %)",
            (got - want) / want * 100.0
        );
        assert!((got - want).abs() <= 0.01 * want, "sun at {theta_deg}");
    }
}

/// Pixel of world point `p` in a frame of `scene` (already fitted).
fn pixel_of(scene: &Scene3d, w: u32, h: u32, p: Point3) -> (u32, u32) {
    let f = build_frame(scene, w, h);
    let (x, y, _) = scene.camera.project(p, w, h, f.near_far).unwrap();
    (x as u32, y as u32)
}

/// A 100 mm cube on the ground under a sun 45 degrees from the zenith
/// toward +X: the ground in its shadow (toward -X) is darker than open
/// ground by at least 4 times, and ground far away matches the closed form
/// for open ground within 2 %.
#[test]
fn a_box_shadows_the_ground() {
    let test = "a_box_shadows_the_ground";
    let rho = 0.8;
    let (sky, e) = (0.1, 2.0);
    let mut s = lit_by(Environment {
        zenith: [sky; 3],
        horizon: [sky; 3],
        ground: [0.0; 3],
        sun: Some(Sun {
            direction: Vec3::new(1.0, 0.0, 1.0),
            frame: LightFrame::World,
            angle_deg: 2.0,
            irradiance: e,
            color: [1.0; 3],
        }),
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    });
    s.add_mesh(
        &shapes::cuboid(Point3::ORIGIN, Point3::new(100.0, 100.0, 100.0)),
        Placement::IDENTITY,
        lambert(0.5),
    );
    s.ground = Some(Ground {
        height: Some(0.0),
        color: Some([230; 3]),
        ..Ground::default()
    });
    assert!((srgb_to_linear(230) - rho).abs() < 0.01);
    let rho = srgb_to_linear(230);
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    s.camera.target = Point3::new(0.0, 50.0, 0.0);
    s.camera.distance = 400.0 / (s.camera.fov_y * 0.5).tan();
    let mut o = opts(200, 200, 256);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let at = |p: Point3| {
        let (x, y) = pixel_of(&s, 200, 200, p);
        // Mean of a 5 x 5 block.
        let mut sum = 0.0;
        for dy in 0..5 {
            for dx in 0..5 {
                sum += lin_mean(&r.pixels[((y + dy - 2) * 200 + x + dx - 2) as usize]);
            }
        }
        sum / 25.0
    };
    let shadow = at(Point3::new(-50.0, 50.0, 0.0));
    let open = at(Point3::new(-50.0, 330.0, 0.0));
    let want = rho * (sky + e * (0.5f64).sqrt() / PI);
    eprintln!(
        "ground in the shadow {shadow:.4}, far away {open:.4} (closed form {want:.4}), ratio {:.2}",
        open / shadow
    );
    assert!(open >= 4.0 * shadow, "shadow");
    assert!((open - want).abs() <= 0.02 * want, "open ground");
}

/// A ray through a flat glass slab comes out parallel to itself and
/// displaced. An orthographic front view of a black block's top edge
/// against a white environment, through a 20 mm slab of index 1.5 tilted
/// 45 degrees: the edge moves by the closed-form displacement
/// `t sin(i - r) / cos r` (6.58 mm), within 0.1 px (0.05 mm).
#[test]
fn glass_slab_displaces_by_the_closed_form() {
    let test = "glass_slab_displaces_by_the_closed_form";
    let (t, ior, i) = (20.0f64, 1.5f64, 45f64.to_radians());
    let black = Material {
        base_color: [0.0; 3],
        ior: 1.0,
        roughness: 1.0,
        ..Material::default()
    };
    let glass = Material {
        base_color: [1.0; 3],
        roughness: 0.0,
        transmission: 1.0,
        ior,
        ..Material::default()
    };
    let size = 160u32;
    let half = 40.0; // mm, half the view's height: 0.5 mm per pixel
    let edge_row = |with_glass: bool| -> Option<f64> {
        let mut s = lit_by(Environment::uniform([1.0; 3]));
        s.add_mesh(
            &shapes::cuboid(
                Point3::new(-80.0, 100.0, -80.0),
                Point3::new(80.0, 110.0, 0.0),
            ),
            Placement::IDENTITY,
            black,
        );
        if with_glass {
            // Slab normal turned from -Y by 45 degrees about X.
            s.add_mesh(
                &shapes::cuboid(
                    Point3::new(-60.0, -t / 2.0, -60.0),
                    Point3::new(60.0, t / 2.0, 60.0),
                ),
                Placement::axis_angle(Vec3::X, i, Vec3::new(0.0, 40.0, 0.0)),
                glass,
            );
        }
        s.camera = Camera3d::new(NamedView::Front, Projection::Orthographic);
        s.camera.target = Point3::ORIGIN;
        s.camera.distance = half / (s.camera.fov_y * 0.5).tan();
        let mut o = opts(size, size, 512);
        o.fit = false;
        let r = radiance(test, &s, &o)?;
        // Per column: the bright share of the column (black below the
        // edge, bright above) gives the edge's height in pixels.
        let rows = r.height as usize;
        // The profile down the middle half of the columns, averaged.
        let cols = (size / 4..size * 3 / 4).collect::<Vec<_>>();
        let prof: Vec<f64> = (0..rows)
            .map(|y| {
                cols.iter()
                    .map(|&x| lin_mean(&r.pixels[y * size as usize + x as usize]))
                    .sum::<f64>()
                    / cols.len() as f64
            })
            .collect();
        // The edge row is the first below half way between the levels
        // either side; the share of it that is bright places the edge
        // inside it.
        let level = |a: usize, b: usize| prof[a..b].iter().sum::<f64>() / (b - a) as f64;
        let mid = 0.5 * (level(10, 30) + level(rows - 30, rows - 10));
        let k = (0..rows).find(|&y| prof[y] < mid).unwrap();
        let (hi, lo) = (level(k - 6, k - 1), level(k + 2, k + 7));
        Some(k as f64 + (prof[k] - lo) / (hi - lo))
    };
    let Some(plain) = edge_row(false) else {
        return;
    };
    let through = edge_row(true).unwrap();
    // The closed form, and its sign from tracing one ray by hand: a ray
    // along +Y meets the slab, whose normal (toward the camera) is
    // R_x(45 deg) (0, -1, 0) = (0, -cos i, -sin i).
    let r = (i.sin() / ior).asin();
    let shift = t * (i - r).sin() / r.cos();
    let n = Vec3::new(0.0, -i.cos(), -i.sin());
    let d = Vec3::Y;
    let eta = 1.0 / ior;
    let c = -n.dot(d);
    let k = 1.0 - eta * eta * (1.0 - c * c);
    let inside = d.scale(eta) + n.scale(eta * c - k.sqrt());
    // Lateral offset of the exit ray from the entry ray, perpendicular to d.
    let travel = inside.scale(t / (-n.dot(inside)));
    let lateral_z = travel.z;
    assert!((lateral_z.abs() - shift).abs() < 1e-9);
    // A ray displaced by +dz meets the edge when it started dz lower: the
    // edge appears dz lower, i.e. dz / 0.5 mm more bright rows.
    let want_px = lateral_z / (2.0 * half / size as f64);
    let got_px = through - plain;
    eprintln!(
        "edge {plain:.3} px bright without glass (view centre at {}), {through:.3} with: shift {got_px:.3} px, closed form {want_px:.3} px ({shift:.3} mm)",
        size / 2
    );
    assert!(
        (plain - size as f64 / 2.0).abs() < 0.1,
        "edge at the centre"
    );
    assert!((got_px - want_px).abs() <= 0.1, "displacement");
}

/// The same scene, samples and seed give the same bytes; another seed
/// does not.
#[test]
fn renders_are_deterministic() {
    let test = "presentation_renders_are_deterministic";
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::block_with_pocket(40.0, 20.0, 20.0, 12.0),
        Placement::IDENTITY,
        Material::from_srgb8(180, 150, 110),
    );
    s.add_mesh(
        &shapes::sphere(10.0, 24, 48),
        Placement::translation(35.0, 0.0, 10.0),
        Material {
            metallic: 1.0,
            roughness: 0.3,
            ..Material::default()
        },
    );
    let o = opts(240, 160, 32);
    let Some(a) = image(test, &s, &o) else {
        return;
    };
    let b = image(test, &s, &o).unwrap();
    assert_eq!(a.rgba, b.rgba, "same seed, same bytes");
    let c = image(
        test,
        &s,
        &RenderOptions3d {
            seed: 7,
            ..o.clone()
        },
    )
    .unwrap();
    assert_ne!(a.rgba, c.rgba, "another seed");
    let png = render_png(&s, &o).unwrap();
    let (back, _, _) = cadview_render::decode_png(&png.png).unwrap();
    assert_eq!(back, a.rgba);
    let stats = png.trace.unwrap();
    assert_eq!(stats.samples, 32);
    // The one shared device.
    assert_eq!(cadview_render::context::devices_created(), 1);
}

/// Monte Carlo error falls as one over the square root of the samples:
/// against a 4096-sample reference with another seed, the root mean
/// square error at 16, 64 and 256 samples falls by about 2 per step.
/// Tolerance: each step between 1.6 and 2.5.
#[test]
fn noise_falls_as_one_over_root_samples() {
    let test = "noise_falls_as_one_over_root_samples";
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::block_with_pocket(40.0, 20.0, 20.0, 12.0),
        Placement::IDENTITY,
        Material::from_srgb8(180, 150, 110),
    );
    s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
    let Some(reference) = radiance(
        test,
        &s,
        &RenderOptions3d {
            seed: 99,
            ..opts(96, 64, 4096)
        },
    ) else {
        return;
    };
    let mut errs = Vec::new();
    for spp in [16, 64, 256] {
        let r = radiance(test, &s, &opts(96, 64, spp)).unwrap();
        let mse: f64 = r
            .pixels
            .iter()
            .zip(&reference.pixels)
            .map(|(a, b)| {
                (0..3)
                    .map(|c| (a[c] as f64 - b[c] as f64).powi(2))
                    .sum::<f64>()
                    / 3.0
            })
            .sum::<f64>()
            / r.pixels.len() as f64;
        errs.push(mse.sqrt());
    }
    let ratios = [errs[0] / errs[1], errs[1] / errs[2]];
    eprintln!("rms error at 16/64/256 spp: {errs:.5?}; ratios {ratios:.3?} (expect about 2)");
    for q in ratios {
        assert!((1.6..=2.5).contains(&q), "{ratios:?}");
    }
}

/// Coverage of each pixel in a render whose background is white and whose
/// object is black (presentation) or the other way round (working).
fn coverage_trace(r: &Radiance3d) -> Vec<f64> {
    r.pixels
        .iter()
        .map(|p| (1.0 - lin_mean(p)).clamp(0.0, 1.0))
        .collect()
}

/// The presentation render draws the same geometry as the working render:
/// a box and a sphere, isometric and front, orthographic and perspective.
/// Per pixel, coverage is the share of the pixel the parts cover (the
/// working render's 4x MSAA counts quarters, the tracer's 256 samples
/// spread over the pixel). The summed difference along the outline stays
/// within a quarter pixel per pixel of outline, and the covered areas agree
/// to the same.
#[test]
fn silhouette_matches_the_working_render() {
    let test = "silhouette_matches_the_working_render";
    let (w, h) = (240u32, 160u32);
    let meshes = [
        (
            "box",
            shapes::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)),
        ),
        ("sphere", shapes::sphere(1.0, 64, 128)),
    ];
    for (name, mesh) in &meshes {
        for (view, proj) in [
            (NamedView::Isometric, Projection::Orthographic),
            (NamedView::Front, Projection::Perspective),
            (NamedView::IsometricSw, Projection::Perspective),
        ] {
            // Working: ambient light only on a black background.
            let mut work = Scene3d {
                edges: None,
                occlusion: None,
                background: Background::Solid([0, 0, 0]),
                lighting: Lighting {
                    key_intensity: 0.0,
                    fill_intensity: 0.0,
                    ambient: 1.0,
                    ..Lighting::default()
                },
                ..Scene3d::default()
            };
            work.add_mesh(
                mesh,
                Placement::IDENTITY,
                Material::from_srgb8(200, 200, 200),
            );
            work.camera = Camera3d::new(view, proj);
            let wo = RenderOptions3d {
                size: [w, h],
                ..Default::default()
            };
            let Some(wimg) = image(test, &work, &wo) else {
                return;
            };
            let prepared = prepare_scene(&work, &wo);
            let f = build_frame(&prepared, w, h);
            let full = f.expected_srgb(Point3::ORIGIN, Vec3::Z, &work.items[0].material);
            let full = full.iter().map(|&c| srgb_to_linear(c)).sum::<f64>() / 3.0;
            let wcov: Vec<f64> = wimg
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| {
                    let l = [p[0], p[1], p[2]]
                        .iter()
                        .map(|&c| srgb_to_linear(c))
                        .sum::<f64>()
                        / 3.0;
                    (l / full).min(1.0)
                })
                .collect();
            // Presentation: a black part in a white environment.
            let mut pres = lit_by(Environment::uniform([1.0; 3]));
            pres.add_mesh(mesh, Placement::IDENTITY, lambert(0.0));
            pres.camera = work.camera;
            let Some(r) = radiance(test, &pres, &opts(w, h, 256)) else {
                return;
            };
            let pcov = coverage_trace(&r);
            let outline: usize = wcov.iter().filter(|&&c| c > 0.0 && c < 1.0).count();
            let diff: f64 = wcov.iter().zip(&pcov).map(|(a, b)| (a - b).abs()).sum();
            let (aw, ap) = (wcov.iter().sum::<f64>(), pcov.iter().sum::<f64>());
            // Pixels that are fully in or fully out in one render and the
            // opposite in the other.
            let flipped = wcov
                .iter()
                .zip(&pcov)
                .filter(|(a, b)| (**a - **b).abs() > 0.75)
                .count();
            eprintln!(
                "{name} {view:?} {proj:?}: area {aw:.1} px working, {ap:.1} presentation; summed difference {diff:.1} px over {outline} outline px ({:.3} per px); {flipped} flipped",
                diff / outline as f64
            );
            assert!(diff <= 0.25 * outline as f64, "{name} {view:?}");
            assert!(
                (aw - ap).abs() <= 0.25 * outline as f64,
                "{name} {view:?} area"
            );
            assert_eq!(flipped, 0, "{name} {view:?}");
        }
    }
}

/// The tracer's albedo is the core's pattern: a banded Lambert plate in a
/// uniform environment of radiance 1 reflects exactly its albedo, which is
/// compared with `pattern_albedo` at each pixel centre. Tolerance: median
/// difference 0.005, 90 % of pixels within 0.03 (pixels straddling a ring
/// edge average across it).
#[test]
fn patterns_match_the_core() {
    let test = "patterns_match_the_core";
    let m = Material {
        base_color: [0.7, 0.5, 0.3],
        roughness: 1.0,
        ior: 1.0,
        pattern: Pattern::Bands(Bands {
            axis: PatternAxis::Vector(Vec3::X),
            spacing: 10.0,
            color: [0.2, 0.1, 0.05],
            strength: 0.9,
            warp: 4.0,
            distance: 30.0,
        }),
        ..Material::default()
    };
    let mut s = lit_by(Environment::uniform([1.0; 3]));
    let mesh = shapes::cuboid(
        Point3::new(-100.0, -60.0, -1.0),
        Point3::new(100.0, 60.0, 0.0),
    );
    s.add_mesh(&mesh, Placement::IDENTITY, m);
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    s.camera.target = Point3::ORIGIN;
    s.camera.distance = 50.0 / (s.camera.fov_y * 0.5).tan();
    let (w, h) = (300u32, 200u32);
    let mut o = opts(w, h, 64);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let f = build_frame(&s, w, h);
    let item = &f.items[0];
    let half_h = 50.0;
    let half_w = half_h * w as f64 / h as f64;
    let mut diffs: Vec<f64> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let wx = -half_w + (x as f64 + 0.5) / w as f64 * 2.0 * half_w;
            let wy = half_h - (y as f64 + 0.5) / h as f64 * 2.0 * half_h;
            let a = pattern_albedo(&item.material, &item.pattern, Point3::new(wx, wy, 0.0));
            let p = r.pixels[(y * w + x) as usize];
            let d = (0..3)
                .map(|c| (p[c] as f64 - a[c]).abs())
                .fold(0.0, f64::max);
            diffs.push(d);
        }
    }
    diffs.sort_by(f64::total_cmp);
    let median = diffs[diffs.len() / 2];
    let p90 = diffs[diffs.len() * 9 / 10];
    eprintln!("pattern albedo vs core: median {median:.4}, 90th percentile {p90:.4}");
    assert!(median <= 0.005 && p90 <= 0.03);
}

/// Path trace with explicit tracer settings (the clamp is not a render
/// option), on the shared device.
fn radiance_with(
    test: &str,
    scene: &Scene3d,
    o: &RenderOptions3d,
    settings: TraceSettings,
) -> Option<Vec<[f32; 3]>> {
    let (w, h) = o.pixel_size();
    let scene = prepare_scene(scene, o);
    let frame = build_frame(&scene, w, h);
    let r = cadview_render::gpu::with_shared(|r| {
        let ctx = r.context();
        let gpu = Gpu {
            device: &ctx.device,
            queue: &ctx.queue,
        };
        let mut t = PathTracer::new(&ctx.device);
        t.set_settings(settings);
        render_frame_rgba(gpu, &mut t, &frame).unwrap();
        t.radiance(gpu).unwrap()
    });
    match r {
        Ok(px) => Some(px),
        Err(e) if e.contains("no GPU adapter") => {
            eprintln!("SKIPPED {test}: {e}");
            None
        }
        Err(e) => panic!("{test}: {e}"),
    }
}

/// The clamp on indirect light costs no visible energy where there are no
/// caustics: a scene like the examples (a wooden block with a pocket,
/// polished and brushed metal, a floor) under the studio light, 2048
/// samples with and without the clamp: mean radiance within 0.5 %, and the
/// clamp takes the brightest pixel from a firefly down to the scene's
/// range. A glass sphere in sunlight focuses a caustic onto the floor,
/// which plain path tracing reaches only through rare paths; there the
/// clamp removes most of it (reported, not limited: a known bias).
#[test]
fn clamp_removes_fireflies_without_visible_bias() {
    let test = "clamp_removes_fireflies_without_visible_bias";
    let polished = Material {
        metallic: 1.0,
        roughness: 0.05,
        ..Material::from_srgb8(220, 220, 225)
    };
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::block_with_pocket(40.0, 20.0, 20.0, 12.0),
        Placement::IDENTITY,
        Material::from_srgb8(180, 140, 90),
    );
    s.add_mesh(
        &shapes::sphere(10.0, 32, 64),
        Placement::translation(35.0, 0.0, 10.0),
        polished,
    );
    s.add_mesh(
        &shapes::cylinder(8.0, 30.0, 64),
        Placement::translation(-35.0, 10.0, 0.0),
        Material {
            metallic: 1.0,
            roughness: 0.3,
            ..Material::default()
        },
    );
    let o = opts(160, 108, 2048);
    let on = TraceSettings {
        spp: 2048,
        ..TraceSettings::default()
    };
    let off = TraceSettings { clamp: 0.0, ..on };
    let Some(a) = radiance_with(test, &s, &o, on) else {
        return;
    };
    let b = radiance_with(test, &s, &o, off).unwrap();
    let mean = |px: &[[f32; 3]]| px.iter().map(lin_mean).sum::<f64>() / px.len() as f64;
    let peak = |px: &[[f32; 3]]| px.iter().map(lin_mean).fold(0.0, f64::max);
    let (ma, mb) = (mean(&a), mean(&b));
    eprintln!(
        "studio: mean {ma:.5} clamped, {mb:.5} not ({:+.3} %); brightest pixel {:.2} clamped, {:.2} not",
        (ma - mb) / mb * 100.0,
        peak(&a),
        peak(&b)
    );
    assert!((ma - mb).abs() <= 0.005 * mb);

    // Glass in the sun: the caustic.
    let mut g = Scene3d {
        ground: Some(Ground::default()),
        environment: Environment::sky(),
        ..Scene3d::default()
    };
    g.add_mesh(
        &shapes::sphere(10.0, 48, 96),
        Placement::translation(0.0, 0.0, 10.0),
        Material {
            transmission: 1.0,
            roughness: 0.0,
            ..Material::from_srgb8(255, 255, 255)
        },
    );
    let a = radiance_with(test, &g, &o, on).unwrap();
    let b = radiance_with(test, &g, &o, off).unwrap();
    let (ma, mb) = (mean(&a), mean(&b));
    eprintln!(
        "glass sphere in the sun: mean {ma:.5} clamped, {mb:.5} not ({:+.3} %); brightest pixel {:.2} clamped, {:.2} not",
        (ma - mb) / mb * 100.0,
        peak(&a),
        peak(&b)
    );
}

/// The presentation quality renders, reports its samples, and draws a
/// lit ground under a studio environment.
#[test]
fn presentation_quality_renders() {
    let test = "presentation_quality_renders";
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::cuboid(Point3::ORIGIN, Point3::new(1.0, 1.0, 1.0)),
        Placement::IDENTITY,
        Material::default(),
    );
    let o = opts(320, 240, 64);
    match render_rgba_stats(&s, &o) {
        Ok((img, stats)) => {
            let st = stats.unwrap();
            assert_eq!(st.samples, 64);
            assert_eq!((st.instances, st.triangles), (1, 12));
            keep("presentation_quality_renders", &img);
        }
        Err(e) if skip(test, &e) => {}
        Err(e) => panic!("{e}"),
    }
    let bad = RenderOptions3d { spp: 0, ..o };
    assert!(matches!(
        render_rgba(&s, &bad),
        Err(Render3dError::InvalidOptions(_))
    ));
}

// ── The denoiser ──────────────────────────────────────────────────────

/// What the image shows for a linear radiance: exposure 0, the core's tone
/// curve, 8-bit sRGB.
fn display(p: &[f32; 3]) -> [u8; 3] {
    cadview_core::scene3d::tone_map([p[0] as f64, p[1] as f64, p[2] as f64])
        .map(cadview_core::scene3d::encode_srgb)
}

/// Keep a radiance image as a PNG when CODECAD_PRESENT_OUT names a folder.
fn keep_radiance(name: &str, px: &[[f32; 3]], w: u32, h: u32) {
    let rgba = px
        .iter()
        .flat_map(|p| {
            let [r, g, b] = display(p);
            [r, g, b, 255]
        })
        .collect();
    keep(
        name,
        &RenderedRgba3d {
            rgba,
            width: w,
            height: h,
            adapter: String::new(),
        },
    );
}

/// Root mean square difference of two radiance images in 8-bit display
/// levels, and the largest difference of any channel.
fn display_diff(a: &[[f32; 3]], b: &[[f32; 3]]) -> (f64, u8) {
    let (mut sq, mut worst) = (0.0f64, 0u8);
    for (p, q) in a.iter().zip(b) {
        let (p, q) = (display(p), display(q));
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            sq += (d as f64).powi(2);
            worst = worst.max(d);
        }
    }
    ((sq / (3 * a.len()) as f64).sqrt(), worst)
}

/// Root mean square difference of two linear radiance images.
fn linear_rms(a: &[[f32; 3]], b: &[[f32; 3]]) -> f64 {
    let se: f64 = a
        .iter()
        .zip(b)
        .map(|(p, q)| {
            (0..3)
                .map(|c| (p[c] as f64 - q[c] as f64).powi(2))
                .sum::<f64>()
                / 3.0
        })
        .sum();
    (se / a.len() as f64).sqrt()
}

/// A scene like the examples: a banded wooden block with a pocket, a
/// polished and a brushed metal part, on a floor, in the studio light.
fn studio_still_life() -> Scene3d {
    let mut s = Scene3d {
        ground: Some(Ground::default()),
        ..Scene3d::default()
    };
    s.add_mesh(
        &shapes::block_with_pocket(40.0, 20.0, 20.0, 12.0),
        Placement::IDENTITY,
        Material {
            pattern: Pattern::Bands(Bands {
                axis: PatternAxis::Longest,
                spacing: 2.0,
                color: [0.25, 0.13, 0.05],
                strength: 0.6,
                warp: 1.2,
                distance: 60.0,
            }),
            ..Material::from_srgb8(184, 137, 79)
        },
    );
    s.add_mesh(
        &shapes::sphere(10.0, 32, 64),
        Placement::translation(35.0, 0.0, 10.0),
        Material {
            metallic: 1.0,
            roughness: 0.05,
            ..Material::from_srgb8(220, 220, 225)
        },
    );
    s.add_mesh(
        &shapes::cylinder(8.0, 30.0, 64),
        Placement::translation(-35.0, 10.0, 0.0),
        Material {
            metallic: 1.0,
            roughness: 0.3,
            pattern: Pattern::Brushed(Brushed {
                axis: PatternAxis::Symmetry,
                anisotropy: 0.75,
                streaks: 0.5,
                scale: 0.08,
                circular: false,
            }),
            ..Material::default()
        },
    );
    s
}

/// The error of `x` against the true image, estimated from two independent
/// references `a` and `b` of the same scene: the mean square difference
/// from `a` minus `a`'s own noise (half the mean square difference of `a`
/// and `b`), as a root mean square.
fn error_to_truth(x: &[[f32; 3]], a: &[[f32; 3]], b: &[[f32; 3]]) -> f64 {
    let own = linear_rms(a, b).powi(2) / 2.0;
    (linear_rms(x, a).powi(2) - own).max(0.0).sqrt()
}

/// The denoiser on the studio scene, against the truth estimated from two
/// 4096-sample references with other seeds ([`error_to_truth`]):
///
/// - On a converged image (4096 samples) it has next to nothing to do:
///   filtered and not differ by at most 0.6 display levels (root mean
///   square) and by more than 2 levels on at most 1 % of the pixels, and
///   what it changes brings the image closer to the truth, not further.
///   Filtering one accumulation again gives the same pixels. (Two
///   renders of 4096 samples can differ in the last bit of the raw sums in
///   a few hundred pixels: the GPU, not the sampling; no 8-bit pixel
///   changed in the four compared while this was written.)
/// - At 64 samples it cuts the error in display levels by at least 1.5
///   times and the error against the truth in linear radiance by at least
///   1.1 times. The linear error left is mostly rare bright paths (the key
///   light reflected by the polished sphere onto the floor, a caustic),
///   which it spreads but does not remove; at this small size most pixels
///   are near an edge. The sharp-edge test below, with direct light and
///   sky only, shows the factor where noise is ordinary.
#[test]
fn denoiser_leaves_a_converged_image_alone() {
    let test = "denoiser_leaves_a_converged_image_alone";
    let s = studio_still_life();
    let o = RenderOptions3d {
        denoise: true,
        ..opts(160, 108, 4096)
    };
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let refs: Vec<Vec<[f32; 3]>> = [99, 77]
        .iter()
        .map(|&seed| {
            radiance(
                test,
                &s,
                &RenderOptions3d {
                    seed,
                    ..opts(160, 108, 4096)
                },
            )
            .unwrap()
            .pixels
        })
        .collect();
    let truth = |x: &[[f32; 3]]| error_to_truth(x, &refs[0], &refs[1]);
    let raw = r.raw.as_ref().expect("raw kept");
    eprintln!(
        "4096 spp: error to the truth {:.5} raw, {:.5} denoised (references' own noise {:.5})",
        truth(raw),
        truth(&r.pixels),
        linear_rms(&refs[0], &refs[1]) / 2f64.sqrt()
    );
    assert!(truth(&r.pixels) <= truth(raw), "toward the truth");
    for spp in [16u32, 64] {
        let low = radiance(
            test,
            &s,
            &RenderOptions3d {
                denoise: true,
                ..opts(160, 108, spp)
            },
        )
        .unwrap();
        let low_raw = low.raw.as_ref().unwrap();
        let (er, ed) = (truth(low_raw), truth(&low.pixels));
        let (dr, _) = display_diff(low_raw, &refs[0]);
        let (dd, _) = display_diff(&low.pixels, &refs[0]);
        eprintln!(
            "{spp} spp: error to the truth {er:.5} raw, {ed:.5} denoised (factor {:.2}); display rms against a reference {dr:.2} and {dd:.2} levels (factor {:.2})",
            er / ed,
            dr / dd
        );
        keep_radiance(&format!("denoise_studio_{spp}_raw"), low_raw, 160, 108);
        keep_radiance(&format!("denoise_studio_{spp}"), &low.pixels, 160, 108);
        if spp == 64 {
            assert!(er / ed >= 1.1, "{er} / {ed}");
            assert!(dr / dd >= 1.5, "{dr} / {dd}");
        }
    }
    let (rms, worst) = display_diff(&r.pixels, raw);
    let changed = r
        .pixels
        .iter()
        .zip(raw)
        .filter(|(a, b)| {
            let (a, b) = (display(a), display(b));
            (0..3).any(|c| a[c].abs_diff(b[c]) > 2)
        })
        .count();
    eprintln!(
        "converged (4096 spp): denoised against raw rms {rms:.3} levels, worst {worst}, {:.3} % of pixels change by more than 2",
        changed as f64 / r.pixels.len() as f64 * 100.0
    );
    keep_radiance("denoise_converged_raw", raw, r.width, r.height);
    keep_radiance("denoise_converged", &r.pixels, r.width, r.height);
    // The filter on one accumulation, twice: the same pixels.
    let (w, h) = (160u32, 108u32);
    let prepared = prepare_scene(&s, &o);
    let frame = build_frame(&prepared, w, h);
    let twice = cadview_render::gpu::with_shared(|g| {
        let ctx = g.context();
        let gpu = Gpu {
            device: &ctx.device,
            queue: &ctx.queue,
        };
        let mut t = PathTracer::new(&ctx.device);
        t.set_settings(TraceSettings {
            spp: 256,
            denoise: true,
            ..TraceSettings::default()
        });
        render_frame_rgba(gpu, &mut t, &frame).unwrap();
        let a = t.denoised_radiance(gpu).unwrap().unwrap();
        // Done already: this call only filters and resolves again.
        render_frame_rgba(gpu, &mut t, &frame).unwrap();
        let b = t.denoised_radiance(gpu).unwrap().unwrap();
        (a, b)
    })
    .unwrap();
    assert_eq!(twice.0, twice.1, "deterministic");
    assert!(rms <= 0.6, "{rms}");
    assert!(changed as f64 <= 0.01 * r.pixels.len() as f64, "{changed}");
}

/// The width in pixels over which `profile` rises (or falls) from 10 % to
/// 90 % of the way between its two ends, by linear interpolation.
fn edge_width(profile: &[f64]) -> f64 {
    let n = profile.len();
    let lo = profile[..3].iter().sum::<f64>() / 3.0;
    let hi = profile[n - 3..].iter().sum::<f64>() / 3.0;
    let at = |frac: f64| {
        let level = lo + (hi - lo) * frac;
        let past = |v: f64| if hi > lo { v >= level } else { v <= level };
        let k = (1..n).find(|&k| past(profile[k])).unwrap_or(n - 1);
        let (a, b) = (profile[k - 1], profile[k]);
        (k - 1) as f64 + ((level - a) / (b - a)).clamp(0.0, 1.0)
    };
    (at(0.9) - at(0.1)).abs()
}

/// At 64 samples the denoiser brings the image at least 2.3 times closer
/// to a 4096-sample reference (another seed) and keeps sharp edges sharp.
/// Top view of a box on a floor under a small sun at 45 degrees. Across the
/// edge of the box's top against its own shadow (geometry and light change)
/// the 10-90 % width of the edge stays within 0.1 px of the reference's.
/// Across the far end of the shadow on the floor (a penumbra 1.7 px wide;
/// only the light changes, the guides do not) it grows by at most 0.1 px
/// and shrinks by at most 0.3 px: an edge-stopping filter steepens a soft
/// edge a little, it must not blur it.
#[test]
fn denoiser_cuts_noise_and_keeps_edges_sharp() {
    let test = "denoiser_cuts_noise_and_keeps_edges_sharp";
    let mut s = lit_by(Environment {
        zenith: [0.3; 3],
        horizon: [0.3; 3],
        ground: [0.0; 3],
        sun: Some(Sun {
            direction: Vec3::new(1.0, 0.0, 1.0),
            frame: LightFrame::World,
            angle_deg: 0.53,
            irradiance: 3.0,
            color: [1.0; 3],
        }),
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    });
    s.add_mesh(
        &shapes::cuboid(Point3::ORIGIN, Point3::new(40.0, 40.0, 40.0)),
        Placement::IDENTITY,
        lambert(0.6),
    );
    s.ground = Some(Ground {
        height: Some(0.0),
        color: Some([200; 3]),
        ..Ground::default()
    });
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    s.camera.target = Point3::new(-10.0, 20.0, 0.0);
    s.camera.distance = 40.0 / (s.camera.fov_y * 0.5).tan();
    let (w, h) = (240u32, 160u32);
    let mut o = opts(w, h, 4096);
    o.fit = false;
    o.seed = 99;
    let Some(reference) = radiance(test, &s, &o) else {
        return;
    };
    let mut lo = opts(w, h, 64);
    lo.fit = false;
    lo.denoise = true;
    let low = radiance(test, &s, &lo).unwrap();
    let raw = low.raw.as_ref().unwrap();
    let (e_raw, e_den) = (
        linear_rms(raw, &reference.pixels),
        linear_rms(&low.pixels, &reference.pixels),
    );
    let (d_raw, _) = display_diff(raw, &reference.pixels);
    let (d_den, _) = display_diff(&low.pixels, &reference.pixels);
    eprintln!(
        "64 spp against 4096: rms {e_raw:.5} raw, {e_den:.5} denoised (factor {:.2}); display rms {d_raw:.2} and {d_den:.2} levels",
        e_raw / e_den
    );
    keep_radiance("denoise_edges_reference", &reference.pixels, w, h);
    keep_radiance("denoise_edges_64_raw", raw, w, h);
    keep_radiance("denoise_edges_64", &low.pixels, w, h);
    // Profiles along x, averaged over the rows across the middle of the box.
    let (x0, _) = pixel_of(&s, w, h, Point3::new(0.0, 20.0, 40.0));
    let (xs, _) = pixel_of(&s, w, h, Point3::new(-40.0, 20.0, 0.0));
    let (_, ya) = pixel_of(&s, w, h, Point3::new(0.0, 32.0, 0.0));
    let (_, yb) = pixel_of(&s, w, h, Point3::new(0.0, 8.0, 0.0));
    let profile = |px: &[[f32; 3]], xc: u32| -> Vec<f64> {
        (xc - 8..=xc + 8)
            .map(|x| {
                (ya..=yb)
                    .map(|y| lin_mean(&px[(y * w + x) as usize]))
                    .sum::<f64>()
                    / (yb - ya + 1) as f64
            })
            .collect()
    };
    for (name, xc) in [
        ("box top against its shadow", x0),
        ("end of the shadow", xs),
    ] {
        let (wr, wl, wd) = (
            edge_width(&profile(&reference.pixels, xc)),
            edge_width(&profile(raw, xc)),
            edge_width(&profile(&low.pixels, xc)),
        );
        eprintln!(
            "{name}: 10-90 % width {wr:.2} px at 4096 spp, {wl:.2} at 64 raw, {wd:.2} at 64 denoised"
        );
        if xc == x0 {
            assert!((wd - wr).abs() <= 0.1, "{name}: {wd} against {wr}");
        } else {
            assert!(
                wd <= wr + 0.1 && wd >= wr - 0.3,
                "{name}: {wd} against {wr}"
            );
        }
    }
    assert!(e_raw / e_den >= 2.3, "{e_raw} / {e_den}");
}

// ── Clear coat, rough glass, emission ─────────────────────────────────

/// The furnace for the material paths the first slice left unchecked: a
/// white sphere in a uniform environment of radiance 1 must not come out
/// brighter than 1 (no energy made) and loses only what single-scattering
/// microfacets lose (rough glass returns that loss along its lobe: see
/// trace.wgsl). A white dielectric (index 1.5) under a clear coat,
/// smooth and rough, and glass of roughness 0.3 and 0.6; a white
/// dielectric without a coat as the control.
#[test]
fn clear_coat_and_rough_glass_keep_energy() {
    let test = "clear_coat_and_rough_glass_keep_energy";
    let white = |roughness: f64| Material {
        base_color: [1.0; 3],
        roughness,
        ..Material::default()
    };
    let cases: [(&str, Material, f64); 6] = [
        (
            "white dielectric, roughness 0.5 (control)",
            white(0.5),
            0.03,
        ),
        (
            "under a clear coat of roughness 0.05",
            Material {
                clearcoat: 1.0,
                clearcoat_roughness: 0.05,
                ..white(0.5)
            },
            0.03,
        ),
        (
            "under a clear coat of roughness 0.5",
            Material {
                clearcoat: 1.0,
                clearcoat_roughness: 0.5,
                ..white(0.5)
            },
            0.03,
        ),
        (
            "smooth glass",
            Material {
                transmission: 1.0,
                ..white(0.0)
            },
            0.005,
        ),
        (
            "rough glass 0.3",
            Material {
                transmission: 1.0,
                ..white(0.3)
            },
            0.005,
        ),
        (
            "rough glass 0.6",
            Material {
                transmission: 1.0,
                ..white(0.6)
            },
            0.005,
        ),
    ];
    for (name, m, loss) in cases {
        let mut s = lit_by(Environment::uniform([1.0; 3]));
        s.add_mesh(&shapes::sphere(5.0, 48, 96), Placement::IDENTITY, m);
        s.camera = Camera3d::new(NamedView::Isometric, Projection::Orthographic);
        let Some(r) = radiance(test, &s, &opts(128, 96, 1024)) else {
            return;
        };
        // The sphere only: pixels whose centre ray meets it.
        let (w, h) = (r.width, r.height);
        let rpx = 5.0 / build_frame(&prepare_scene(&s, &opts(w, h, 1)), w, h).pixel_world;
        let (mean, n) = mean_where(&r, |x, y| {
            let (dx, dy) = (
                x as f64 + 0.5 - w as f64 / 2.0,
                y as f64 + 0.5 - h as f64 / 2.0,
            );
            dx.hypot(dy) < 0.95 * rpx
        });
        eprintln!(
            "{name}: mean {mean:.5} over {n} px ({:+.2} %; allowed loss {:.1} %)",
            (mean - 1.0) * 100.0,
            loss * 100.0
        );
        assert!(mean <= 1.005, "{name}: gains energy: {mean}");
        assert!(mean >= 1.0 - loss, "{name}: loses too much: {mean}");
    }
}

/// An emitter is seen at its own radiance: a black plate (index 1, so no
/// reflection) with emission (2, 1, 0.5) in a black environment, to float
/// precision.
#[test]
fn an_emitter_is_seen_at_its_radiance() {
    let test = "an_emitter_is_seen_at_its_radiance";
    let e = [2.0, 1.0, 0.5];
    let mut s = lit_by(Environment::uniform([0.0; 3]));
    plate(
        &mut s,
        Vec3::Z,
        Material {
            emission: e,
            ..lambert(0.0)
        },
    );
    s.camera = facing(Vec3::Z);
    let Some(r) = radiance(test, &s, &opts(64, 64, 16)) else {
        return;
    };
    let p = r.pixels[(32 * 64 + 32) as usize];
    eprintln!("emitter seen at {p:?}, emission {e:?}");
    for c in 0..3 {
        assert!((p[c] as f64 - e[c]).abs() <= 1e-6 * e[c], "{p:?}");
    }
}

/// A white Lambert plate lit only by a sphere of radiance L and radius R
/// whose centre is h above it: at distance d from the centre (angle theta
/// from the plate's normal) the plate receives `pi L (R / d)^2 cos(theta)`
/// and reflects `rho L R^2 h / d^3`. Seen from above beside the sphere, at
/// 16384 samples, averaged over 9 x 9 pixels: within 2.5 % (five standard
/// deviations of the noise at the farthest point). The sphere is
/// found by BSDF sampling only (no next-event estimation of emitters).
#[test]
fn a_sphere_light_lights_a_plate_by_the_closed_form() {
    let test = "a_sphere_light_lights_a_plate_by_the_closed_form";
    let (rho, l, rad, h) = (0.5, 4.0, 20.0, 60.0);
    let mut s = lit_by(Environment::uniform([0.0; 3]));
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(-200.0, -200.0, -1.0),
            Point3::new(200.0, 200.0, 0.0),
        ),
        Placement::IDENTITY,
        lambert(rho),
    );
    s.add_mesh(
        &shapes::sphere(rad, 96, 192),
        Placement::translation(0.0, 0.0, h),
        Material {
            emission: [l; 3],
            ..lambert(0.0)
        },
    );
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    s.camera.target = Point3::new(60.0, 0.0, 0.0);
    s.camera.distance = 50.0 / (s.camera.fov_y * 0.5).tan();
    let (w, hh) = (200u32, 100u32);
    let mut o = opts(w, hh, 16384);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let f = build_frame(&s, w, hh);
    let at = |p: Point3| s.camera.project(p, w, hh, f.near_far).unwrap();
    for x in [30.0f64, 50.0, 80.0, 110.0] {
        // The closed form at the centres of the pixels averaged (the top
        // view maps x and y to columns and rows, scaled).
        let (x0, y0, _) = at(Point3::new(x, 0.0, 0.0));
        let bx = at(Point3::new(x + 1.0, 0.0, 0.0)).0 - x0;
        let by = at(Point3::new(x, 1.0, 0.0)).1 - y0;
        let (px, py) = (x0 as u32, y0 as u32);
        let (mut sum, mut want) = (0.0, 0.0);
        for dy in 0..9 {
            for dx in 0..9 {
                let (c, row) = (px + dx - 4, py + dy - 4);
                sum += lin_mean(&r.pixels[(row * w + c) as usize]);
                let wx = x + (c as f64 + 0.5 - x0) / bx;
                let wy = (row as f64 + 0.5 - y0) / by;
                let d = (wx * wx + wy * wy + h * h).sqrt();
                want += rho * l * rad * rad * h / d.powi(3);
            }
        }
        let got = sum / 81.0;
        let want = want / 81.0;
        eprintln!(
            "plate at {x} mm: {got:.5}, closed form {want:.5} ({:+.2} %)",
            (got - want) / want * 100.0
        );
        assert!((got - want).abs() <= 0.025 * want, "at {x}");
    }
}

// ── Portals ───────────────────────────────────────────────────────────

/// A closed room 200 x 200 x 100 with black walls and ceiling and a grey
/// floor, lit only by a uniform sky (radiance 1 above the horizon) through
/// a 40 x 40 opening in the 1 mm ceiling. The floor under the opening's
/// centre sees the sky through the opening's top edge (101 above it) and
/// reflects `rho L F`, with F the form factor of the opening: four corner
/// rectangles, each `(X atan(Y / sqrt(1 + X^2)) / sqrt(1 + X^2) + Y atan(X /
/// sqrt(1 + Y^2)) / sqrt(1 + Y^2)) / (2 pi)` with X = Y = 20 / 101. Seen
/// from inside, looking down: with the opening named as a portal, 256
/// samples land within 1 % of the closed form; without it, the dome is
/// found by BSDF sampling only, and the floor's pixels scatter at least 10
/// times more. Both converge to the same value: 16384 samples without the
/// portal agree within 2 %.
#[test]
fn a_portal_lights_a_room_through_its_opening() {
    let test = "a_portal_lights_a_room_through_its_opening";
    let (rho, a, t, hgt) = (0.5, 40.0, 1.0, 100.0);
    let room = |portal: bool| {
        let mut s = lit_by(Environment {
            zenith: [1.0; 3],
            horizon: [1.0; 3],
            ground: [0.0; 3],
            sun: None,
            intensity: 1.0,
            sweep: false,
            haze: 0.0,
        });
        s.add_mesh(
            &shapes::cuboid(
                Point3::new(-102.0, -102.0, -1.0),
                Point3::new(102.0, 102.0, 0.0),
            ),
            Placement::IDENTITY,
            lambert(rho),
        );
        let black = lambert(0.0);
        for (lo, hi) in [
            ([-102.0, -102.0], [-100.0, 102.0]),
            ([100.0, -102.0], [102.0, 102.0]),
            ([-100.0, -102.0], [100.0, -100.0]),
            ([-100.0, 100.0], [100.0, 102.0]),
        ] {
            s.add_mesh(
                &shapes::cuboid(
                    Point3::new(lo[0], lo[1], 0.0),
                    Point3::new(hi[0], hi[1], hgt),
                ),
                Placement::IDENTITY,
                black,
            );
        }
        s.add_mesh(
            &shapes::plate_with_square_hole(204.0, a, t),
            Placement::translation(0.0, 0.0, hgt),
            black,
        );
        if portal {
            s.portals = vec![cadview_core::scene3d::Portal::checked(
                Point3::new(-a / 2.0, -a / 2.0, hgt + t),
                Vec3::new(a, 0.0, 0.0),
                Vec3::new(0.0, a, 0.0),
            )
            .unwrap()];
        }
        s.camera = Camera3d::new(NamedView::Top, Projection::Perspective);
        s.camera.target = Point3::ORIGIN;
        s.camera.distance = 50.0;
        s
    };
    let corner = |x: f64, y: f64| {
        let (sx, sy) = ((1.0 + x * x).sqrt(), (1.0 + y * y).sqrt());
        (x / sx * (y / sx).atan() + y / sy * (x / sy).atan()) / (2.0 * PI)
    };
    let c = a / 2.0 / (hgt + t);
    let want = rho * 4.0 * corner(c, c);
    let (w, h) = (96u32, 96u32);
    // Mean and relative spread of the pixels within 9 px of the centre.
    let stats = |r: &Radiance3d| {
        let v: Vec<f64> = (40..56)
            .flat_map(|y| (40..56).map(move |x| (x, y)))
            .map(|(x, y)| lin_mean(&r.pixels[(y * w + x) as usize]))
            .collect();
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        (m, sd / m)
    };
    let run = |portal: bool, spp: u32| {
        let mut o = opts(w, h, spp);
        o.fit = false;
        radiance(test, &room(portal), &o)
    };
    let Some(with) = run(true, 256) else {
        return;
    };
    let without = run(false, 256).unwrap();
    let long = run(false, 16384).unwrap();
    let (mw, sw) = stats(&with);
    let (mo, so) = stats(&without);
    let (ml, _) = stats(&long);
    eprintln!(
        "floor under the opening: closed form {want:.5}; portal, 256 spp: {mw:.5} ({:+.2} %, pixels spread {:.2} %); no portal, 256 spp: {mo:.5} (spread {:.1} %); no portal, 16384 spp: {ml:.5} ({:+.2} %)",
        (mw - want) / want * 100.0,
        sw * 100.0,
        so * 100.0,
        (ml - want) / want * 100.0
    );
    assert!((mw - want).abs() <= 0.01 * want, "with the portal");
    assert!(so >= 10.0 * sw, "noise {so} against {sw}");
    assert!((ml - want).abs() <= 0.02 * want, "without the portal");
}

// ── The studio sweep ──────────────────────────────────────────────────

/// In a level view the studio floor meets its backdrop without a line: the
/// rows just below and just above the horizon (away from the part) differ
/// by less than 3 % with the sweep. Without it the lit floor ends in a hard
/// line against the dark dome (reported).
#[test]
fn the_studio_floor_meets_its_backdrop_without_a_line() {
    let test = "the_studio_floor_meets_its_backdrop_without_a_line";
    let row_means = |sweep: bool| -> Option<(f64, f64)> {
        let mut s = Scene3d {
            ground: Some(Ground::default()),
            environment: Environment {
                sweep,
                ..Environment::studio()
            },
            ..Scene3d::default()
        };
        s.add_mesh(
            &shapes::cuboid(
                Point3::new(-50.0, -50.0, 0.0),
                Point3::new(50.0, 50.0, 100.0),
            ),
            Placement::IDENTITY,
            Material::default(),
        );
        s.camera = Camera3d::new(NamedView::Front, Projection::Perspective);
        s.camera.target = Point3::new(0.0, 0.0, 60.0);
        s.camera.distance = 1500.0;
        let (w, h) = (240u32, 160u32);
        let mut o = opts(w, h, 256);
        o.fit = false;
        let r = radiance(test, &s, &o)?;
        keep_radiance(
            if sweep {
                "studio_level_sweep"
            } else {
                "studio_level_no_sweep"
            },
            &r.pixels,
            w,
            h,
        );
        // The horizon is the middle row; columns left of the box.
        let row = |y: u32| {
            (0..40)
                .map(|x| lin_mean(&r.pixels[(y * w + x) as usize]))
                .sum::<f64>()
                / 40.0
        };
        Some((row(h / 2 + 2), row(h / 2 - 3)))
    };
    let Some((below, above)) = row_means(true) else {
        return;
    };
    let (b0, a0) = row_means(false).unwrap();
    eprintln!(
        "level view: floor below the horizon {below:.4}, backdrop above {above:.4} ({:+.1} %); without the sweep {b0:.4} and {a0:.4} ({:+.1} %)",
        (above - below) / below * 100.0,
        (a0 - b0) / b0 * 100.0
    );
    assert!((above - below).abs() <= 0.03 * below);
}

// ── Shadows through glass ─────────────────────────────────────────────

/// A clear glass pane (index 1.5, 4 mm) 50 mm above a dark floor, the sun
/// straight overhead and nothing else: the shadow ray crosses two faces at
/// normal incidence, each letting through `1 - ((n - 1) / (n + 1))^2`, so
/// the floor under the pane gets 0.9216 of the open floor's light. Seen
/// from a low angle (no camera ray passes through the pane), within 1.5 %
/// (the floor's light reflected back down by the pane adds about 0.8 %).
#[test]
fn a_glass_pane_casts_its_fresnel_shadow() {
    let test = "a_glass_pane_casts_its_fresnel_shadow";
    let rho = 0.1;
    let mut s = lit_by(Environment {
        zenith: [0.0; 3],
        horizon: [0.0; 3],
        ground: [0.0; 3],
        sun: Some(Sun {
            direction: Vec3::Z,
            frame: LightFrame::World,
            angle_deg: 0.53,
            irradiance: 3.0,
            color: [1.0; 3],
        }),
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    });
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(-300.0, -300.0, -1.0),
            Point3::new(300.0, 300.0, 0.0),
        ),
        Placement::IDENTITY,
        lambert(rho),
    );
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(-30.0, -30.0, 50.0),
            Point3::new(30.0, 30.0, 54.0),
        ),
        Placement::IDENTITY,
        Material {
            base_color: [1.0; 3],
            roughness: 0.0,
            transmission: 1.0,
            ..Material::default()
        },
    );
    s.camera = Camera3d::new(NamedView::Front, Projection::Orthographic);
    s.camera.pitch = 15f64.to_radians();
    s.camera.target = Point3::new(30.0, 0.0, 0.0);
    s.camera.distance = 60.0 / (s.camera.fov_y * 0.5).tan();
    let (w, h) = (200u32, 120u32);
    let mut o = opts(w, h, 256);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let block = |p: Point3| {
        let (x, y) = pixel_of(&s, w, h, p);
        let mut sum = 0.0;
        for dy in 0..5 {
            for dx in 0..5 {
                sum += lin_mean(&r.pixels[((y + dy - 2) * w + x + dx - 2) as usize]);
            }
        }
        sum / 25.0
    };
    let (under, open) = (
        block(Point3::new(0.0, 0.0, 0.0)),
        block(Point3::new(80.0, 0.0, 0.0)),
    );
    let t = (1.0 - ((1.5 - 1.0) / (1.5 + 1.0f64)).powi(2)).powi(2);
    eprintln!(
        "floor under the pane {under:.5}, open {open:.5}: ratio {:.4}, closed form {t:.4}",
        under / open
    );
    keep_radiance("glass_pane_shadow", &r.pixels, w, h);
    assert!((under / open - t).abs() <= 0.015, "{}", under / open);
}

// ── Placed cameras and the ground ─────────────────────────────────────

/// The closed-form pixel of world point `p` for a pinhole (or parallel)
/// camera at `eye` looking at `target` with world up +Z: perspective with
/// vertical field of view `fov` (radians), or orthographic `height` mm high.
fn closed_form_pixel(
    eye: Point3,
    target: Point3,
    lens: Result<f64, f64>,
    p: Point3,
    w: u32,
    h: u32,
) -> (f64, f64) {
    let f = (target - eye).normalized().unwrap();
    let r = f.cross(Vec3::Z).normalized().unwrap();
    let u = r.cross(f);
    let d = p - eye;
    let (x, y, z) = (d.dot(r), d.dot(u), d.dot(f));
    let (sx, sy) = match lens {
        Ok(fov) => {
            let t = (fov / 2.0).tan();
            (x / (z * t), y / (z * t))
        }
        Err(height) => (2.0 * x / height, 2.0 * y / height),
    };
    let aspect = w as f64 / h as f64;
    (
        w as f64 / 2.0 * (1.0 + sx / aspect),
        h as f64 / 2.0 * (1.0 - sy),
    )
}

/// The centroid (in pixels) of the dark pixels of a radiance or sRGB image.
fn dark_centroid(lum: &[f64], w: u32) -> (f64, f64) {
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for (i, l) in lum.iter().enumerate() {
        let wgt = (1.0 - l).clamp(0.0, 1.0);
        sx += wgt * ((i as u32 % w) as f64 + 0.5);
        sy += wgt * ((i as u32 / w) as f64 + 0.5);
        sw += wgt;
    }
    (sx / sw, sy / sw)
}

/// A placed camera puts a known point at its closed-form pixel in both
/// renders: a black 3 mm ball at a known place, seen from an eye point
/// toward a target, perspective and orthographic. The centroid of the
/// ball's dark pixels (working render: lit grey on black is inverted; the
/// presentation render: a black ball in a white environment) lands within
/// 0.25 px of the pinhole (or parallel) projection of its centre.
#[test]
fn a_placed_camera_projects_points_to_their_closed_form_pixels() {
    let test = "a_placed_camera_projects_points_to_their_closed_form_pixels";
    let (w, h) = (240u32, 160u32);
    let eye = Point3::new(-400.0, -900.0, 500.0);
    let target = Point3::new(20.0, 30.0, 40.0);
    let ball = Point3::new(60.0, -20.0, 90.0);
    for lens in [Ok(30f64.to_radians()), Err(300.0)] {
        let proj = if lens.is_ok() {
            Projection::Perspective
        } else {
            Projection::Orthographic
        };
        let cam = Camera3d::placed(eye, target, proj, lens.unwrap_or_else(|x| x), Vec3::Z).unwrap();
        let want = closed_form_pixel(eye, target, lens, ball, w, h);
        // Presentation: a black ball in a white environment.
        let mut pres = lit_by(Environment::uniform([1.0; 3]));
        pres.add_mesh(
            &shapes::sphere(3.0, 32, 64),
            Placement::translation(ball.x, ball.y, ball.z),
            lambert(0.0),
        );
        pres.camera = cam;
        let mut o = opts(w, h, 64);
        o.fit = false;
        let Some(r) = radiance(test, &pres, &o) else {
            return;
        };
        let lum: Vec<f64> = r.pixels.iter().map(lin_mean).collect();
        let got_p = dark_centroid(&lum, w);
        // Working: a light ball on black, ambient only, no edges.
        let mut work = Scene3d {
            edges: None,
            occlusion: None,
            background: Background::Solid([255, 255, 255]),
            lighting: Lighting {
                key_intensity: 0.0,
                fill_intensity: 0.0,
                ambient: 0.0,
                ..Lighting::default()
            },
            ..Scene3d::default()
        };
        work.add_mesh(
            &shapes::sphere(3.0, 32, 64),
            Placement::translation(ball.x, ball.y, ball.z),
            Material::from_srgb8(0, 0, 0),
        );
        work.camera = cam;
        let wo = RenderOptions3d {
            size: [w, h],
            fit: false,
            ..Default::default()
        };
        let img = image(test, &work, &wo).unwrap();
        let lum: Vec<f64> = img
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| srgb_to_linear(p[0]))
            .collect();
        let got_w = dark_centroid(&lum, w);
        eprintln!(
            "{proj:?}: closed form ({:.3}, {:.3}); presentation ({:.3}, {:.3}); working ({:.3}, {:.3})",
            want.0, want.1, got_p.0, got_p.1, got_w.0, got_w.1
        );
        for got in [got_p, got_w] {
            assert!(
                (got.0 - want.0).abs() <= 0.25 && (got.1 - want.1).abs() <= 0.25,
                "{proj:?}: {got:?} vs {want:?}"
            );
        }
    }
}

/// From eye height, the horizon of an endless ground sits where the closed
/// form puts it: a camera 1600 mm up, looking 4 degrees down with a 40
/// degree field of view, sees the horizon (the row where the ground gives
/// way to the sky) `h/2 (1 - tan 4 / tan 20)` from the top, within 1 px.
#[test]
fn the_horizon_sits_at_its_closed_form_row() {
    let test = "the_horizon_sits_at_its_closed_form_row";
    let (w, h) = (240u32, 200u32);
    let (down, fov) = (4f64.to_radians(), 40f64.to_radians());
    let eye = Point3::new(0.0, 0.0, 1600.0);
    let target = Point3::new(0.0, 10000.0, 1600.0 - 10000.0 * down.tan());
    let mut s = lit_by(Environment {
        zenith: [1.0; 3],
        horizon: [1.0; 3],
        ground: [0.0; 3],
        sun: None,
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    });
    s.ground = Some(Ground {
        height: Some(0.0),
        color: Some([0, 0, 0]),
        ..Ground::default()
    });
    // Something to draw; far off to the side, out of view.
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(-1e5, -1e5, 0.0),
            Point3::new(-1e5 + 1.0, -1e5 + 1.0, 1.0),
        ),
        Placement::IDENTITY,
        lambert(0.0),
    );
    s.camera = Camera3d::placed(eye, target, Projection::Perspective, fov, Vec3::Z).unwrap();
    let mut o = opts(w, h, 64);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    // The share of each row that is sky, down the middle columns.
    let sky: Vec<f64> = (0..h)
        .map(|y| {
            (w / 4..3 * w / 4)
                .map(|x| lin_mean(&r.pixels[(y * w + x) as usize]))
                .sum::<f64>()
                / (w / 2) as f64
        })
        .collect();
    // Rows above the horizon are 1, below 0: the summed sky is the horizon row.
    let got: f64 = sky.iter().sum();
    let want = h as f64 / 2.0 * (1.0 - down.tan() / (fov / 2.0).tan());
    eprintln!("horizon at row {got:.3} from the top, closed form {want:.3}");
    assert!((got - want).abs() <= 1.0);
}

/// The ground takes a material like a part: a Lambert ground with a soft
/// mottle, in a uniform upper hemisphere of radiance 1, reflects exactly
/// its albedo, which is compared with the core's `pattern_albedo` in the
/// ground's pattern frame at each pixel centre (median within 0.005, 90 %
/// within 0.02), and the albedo varies (standard deviation above 4 % of
/// its mean) so the even surface is not flat.
#[test]
fn the_ground_takes_a_material_with_a_soft_variation() {
    let test = "the_ground_takes_a_material_with_a_soft_variation";
    let m = Material {
        base_color: [0.15, 0.35, 0.08],
        roughness: 1.0,
        ior: 1.0,
        pattern: Pattern::Mottle(cadview_core::scene3d::Mottle {
            scale: 120.0,
            color: [0.5, 0.45, 0.1],
            strength: 0.8,
        }),
        ..Material::default()
    };
    let mut s = lit_by(Environment {
        zenith: [1.0; 3],
        horizon: [1.0; 3],
        ground: [0.0; 3],
        sun: None,
        intensity: 1.0,
        sweep: false,
        haze: 0.0,
    });
    // Something to draw, out of view.
    s.add_mesh(
        &shapes::cuboid(
            Point3::new(5000.0, 5000.0, 0.0),
            Point3::new(5001.0, 5001.0, 1.0),
        ),
        Placement::IDENTITY,
        lambert(0.0),
    );
    s.ground = Some(Ground {
        height: Some(0.0),
        material: Some(m),
        ..Ground::default()
    });
    s.camera = Camera3d::new(NamedView::Top, Projection::Orthographic);
    s.camera.target = Point3::ORIGIN;
    s.camera.distance = 200.0 / (s.camera.fov_y * 0.5).tan();
    let (w, h) = (300u32, 200u32);
    let mut o = opts(w, h, 64);
    o.fit = false;
    let Some(r) = radiance(test, &s, &o) else {
        return;
    };
    let f = build_frame(&s, w, h);
    let (gm, pf) = f.ground.unwrap().material.unwrap();
    let (half_h, half_w) = (200.0, 300.0);
    let mut diffs = Vec::new();
    let mut lum = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let wx = -half_w + (x as f64 + 0.5) / w as f64 * 2.0 * half_w;
            let wy = half_h - (y as f64 + 0.5) / h as f64 * 2.0 * half_h;
            let a = pattern_albedo(&gm, &pf, Point3::new(wx, wy, 0.0));
            let p = r.pixels[(y * w + x) as usize];
            diffs.push(
                (0..3)
                    .map(|c| (p[c] as f64 - a[c]).abs())
                    .fold(0.0, f64::max),
            );
            lum.push(0.2126 * a[0] + 0.7152 * a[1] + 0.0722 * a[2]);
        }
    }
    diffs.sort_by(f64::total_cmp);
    let (median, p90) = (diffs[diffs.len() / 2], diffs[diffs.len() * 9 / 10]);
    let mean = lum.iter().sum::<f64>() / lum.len() as f64;
    let sd = (lum.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / lum.len() as f64).sqrt();
    eprintln!("ground albedo vs core: median {median:.4}, 90th percentile {p90:.4}; albedo luminance {mean:.3} +- {sd:.3}");
    keep_radiance("ground_mottle", &r.pixels, w, h);
    assert!(median <= 0.005 && p90 <= 0.02, "{median} {p90}");
    assert!(sd > 0.04 * mean, "{sd}");
}

/// Aerial haze softens the horizon: in daylight, from eye height looking
/// level over an endless green ground, the rows just below and just above
/// the horizon differ by less than 8 % with the sky's haze; without it the
/// ground ends in a line (reported).
#[test]
fn haze_softens_the_horizon() {
    let test = "haze_softens_the_horizon";
    let rows = |haze: f64| -> Option<(f64, f64)> {
        let mut s = lit_by(Environment {
            haze,
            ..Environment::sky()
        });
        s.add_mesh(
            &shapes::cuboid(
                Point3::new(-1e5, -1e5, 0.0),
                Point3::new(-1e5 + 1.0, -1e5 + 1.0, 1.0),
            ),
            Placement::IDENTITY,
            lambert(0.0),
        );
        s.ground = Some(Ground {
            height: Some(0.0),
            color: Some([90, 130, 60]),
            ..Ground::default()
        });
        let eye = Point3::new(0.0, 0.0, 1600.0);
        s.camera = Camera3d::placed(
            eye,
            Point3::new(0.0, 10000.0, 1600.0),
            Projection::Perspective,
            40f64.to_radians(),
            Vec3::Z,
        )
        .unwrap();
        let (w, h) = (240u32, 200u32);
        let mut o = opts(w, h, 128);
        o.fit = false;
        let r = radiance(test, &s, &o)?;
        keep_radiance(
            if haze > 0.0 {
                "horizon_haze"
            } else {
                "horizon_no_haze"
            },
            &r.pixels,
            w,
            h,
        );
        let row = |y: u32| {
            (0..w)
                .map(|x| lin_mean(&r.pixels[(y * w + x) as usize]))
                .sum::<f64>()
                / w as f64
        };
        Some((row(h / 2 + 1), row(h / 2 - 2)))
    };
    let Some((below, above)) = rows(cadview_core::scene3d::SKY_HAZE) else {
        return;
    };
    let (b0, a0) = rows(0.0).unwrap();
    eprintln!(
        "level view: ground below the horizon {below:.4}, sky above {above:.4} ({:+.1} %); without haze {b0:.4} and {a0:.4} ({:+.1} %)",
        (below - above) / above * 100.0,
        (b0 - a0) / a0 * 100.0
    );
    assert!((below - above).abs() <= 0.08 * above);
}
