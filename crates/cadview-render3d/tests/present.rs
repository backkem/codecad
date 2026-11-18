//! GPU evidence for the presentation render (the path tracer), measured
//! against closed forms, against the core's patterns and against the
//! working render. Every test prints SKIPPED and passes when the machine
//! has no GPU adapter. Set CODECAD_PRESENT_OUT to a folder to keep the
//! images.

use cadview_core::scene3d::{
    build_frame, pattern_albedo, shapes, srgb_to_linear, Background, Bands, Camera3d, Environment,
    Ground, LightFrame, Lighting, Material, NamedView, Pattern, PatternAxis, Projection, Scene3d,
    Sun,
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
    });
    s.add_mesh(
        &shapes::cuboid(Point3::ORIGIN, Point3::new(100.0, 100.0, 100.0)),
        Placement::IDENTITY,
        lambert(0.5),
    );
    s.ground = Some(Ground {
        height: Some(0.0),
        color: Some([230; 3]),
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
