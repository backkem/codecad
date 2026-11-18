//! Evidence for the optional `SolidKernel` capabilities of rung 3D1b, over
//! the whole catalogue, on arris:
//!
//! - `evaluate_from`: continuing from every evaluated prefix gives what a
//!   from-scratch evaluation gives (volume and centroid bit for bit, box,
//!   and the mesh of the last continuation), or the same refusal.
//! - `bbox_within`: equal to the exact `bbox` with tolerance 0 wherever
//!   that exists; where it refuses (NURBS edges), a box that contains the
//!   solid's closed-form box and lies within the stated tolerance of it.
//! - `interferences` and `distances`: a solid against itself in the same
//!   place shares its whole volume; moved past its own box it is apart, at
//!   least the gap away, with closest points that are that far apart.
//!
//! - `topology`, `silhouettes` and `ray_hits` (rung 3D3a, projection),
//!   against the certified reference meaning of each part: every edge
//!   and every silhouette lies on the boundary; a silhouette's normal
//!   (closed form, from its face's surface) is across the view; a ray's
//!   hits lie on the boundary, and between them the ray is alternately
//!   outside and inside.
//!
//! These capabilities are not scoreboard layers (the scoreboards are
//! unchanged by them). `cargo test -p solid-harness --release --test
//! capabilities -- --nocapture` prints the counts.

use cadview_core::solid::kernel::{Curve3, Interference, Ray3, TightBox};
use cadview_core::solid::{Placement, Point3, SolidKernel, Vec3};
use solid_harness::cases::catalogue;
use solid_harness::kernels::ArrisKernel;
use solid_harness::reference::{Class, Reference};

/// How far (mm) a point said to be on the boundary may be from it: the
/// kernel's linear tolerance is 1e-7.
const ON: f64 = 1e-6;

/// Off the boundary by more than `ON`, certainly.
fn off(r: &Reference, p: Point3) -> Option<f64> {
    match r.classify(p) {
        Class::Inside { margin } | Class::Outside { margin } if margin > ON => Some(margin),
        _ => None,
    }
}

fn samples(c: &Curve3, range: [f64; 2]) -> impl Iterator<Item = Point3> + '_ {
    [0.0, 0.17, 0.5, 0.83, 1.0]
        .into_iter()
        .map(move |f| c.point(range[0] + (range[1] - range[0]) * f))
}

/// Every edge of every part lies on the part's boundary, at its ends and
/// at three points between.
#[test]
fn edges_lie_on_the_boundary() {
    let k = ArrisKernel;
    let (mut parts, mut edges, mut seams, mut smooth) = (0, 0, 0, 0);
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for case in catalogue().parts {
        let (Ok(e), Ok(r)) = (
            k.evaluate(&case.def, &case.tools),
            Reference::build(&case.def, &case.tools),
        ) else {
            continue;
        };
        let t = k.topology(&e.brep).expect("arris gives edges");
        parts += 1;
        for edge in &t.edges {
            edges += 1;
            seams += usize::from(edge.seam);
            smooth += usize::from(edge.smooth && !edge.seam);
            *kinds.entry(edge.curve.kind()).or_default() += 1;
            assert!(
                (1..=2).contains(&edge.faces.len())
                    && edge.faces.iter().all(|&f| f < t.faces.len()),
                "{}: {edge:?}",
                case.name
            );
            for p in samples(&edge.curve, edge.range) {
                if let Some(m) = off(&r, p) {
                    panic!(
                        "{}: edge {:?} is {m:e} mm off at {p:?}",
                        case.name, edge.curve
                    );
                }
            }
        }
    }
    println!(
        "topology: {parts} parts, {edges} edges on the boundary ({kinds:?}), {seams} seams, \
         {smooth} smooth"
    );
    assert!(parts > 100 && edges > 1000, "{parts} {edges}");
}

/// Silhouettes in four views lie on the boundary, and the normal of their
/// face (closed form) is across the view there.
#[test]
fn silhouettes_lie_on_the_boundary_across_the_view() {
    let k = ArrisKernel;
    let s = 1.0 / 3f64.sqrt();
    let views = [
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(s, -s, s),
    ];
    let (mut parts, mut curves, mut skipped) = (0, 0, 0);
    let mut worst: f64 = 0.0;
    let mut worst_at = String::new();
    for case in catalogue().parts {
        let (Ok(e), Ok(r)) = (
            k.evaluate(&case.def, &case.tools),
            Reference::build(&case.def, &case.tools),
        ) else {
            continue;
        };
        let t = k.topology(&e.brep).expect("edges");
        parts += 1;
        for v in views {
            let sil = k.silhouettes(&e.brep, v).expect("silhouettes");
            skipped += sil.skipped.len();
            for c in &sil.curves {
                curves += 1;
                assert!(c.range[1] > c.range[0], "{}", case.name);
                for p in samples(&c.curve, c.range) {
                    if let Some(m) = off(&r, p) {
                        panic!("{}: silhouette {:?} is {m:e} mm off", case.name, c.curve);
                    }
                    let n = t.faces[c.face]
                        .surface
                        .normal_at(p)
                        .expect("a closed-form face");
                    // In units of 1e-9 at coordinates up to 1 m; beyond,
                    // the closed-form normal itself loses digits to
                    // cancellation in proportion to the coordinates.
                    let size = (p - Point3::ORIGIN).length().max(1e3) / 1e3;
                    let dev = n.dot(v).abs() / size;
                    if dev > worst {
                        worst = dev;
                        worst_at = format!("{} ({})", case.name, t.faces[c.face].surface.kind());
                    }
                }
            }
        }
    }
    println!(
        "silhouettes: {parts} parts in 4 views, {curves} curves on the boundary, largest |n . v| \
         {worst:e} ({worst_at}); {skipped} faces without a closed form (oblique tori, B-spline \
         faces)"
    );
    assert!(worst <= 1e-9, "{worst:e}");
    assert!(curves > 100, "{curves}");
}

/// Rays through a grid over each part's box from four directions: every
/// hit lies on the boundary, and between hits that cross the surface the
/// ray is outside, inside, outside... as the reference says.
#[test]
fn ray_hits_agree_with_the_reference() {
    let k = ArrisKernel;
    let s = 1.0 / 3f64.sqrt();
    let dirs = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, -1.0),
        Vec3::new(-s, s, -s),
    ];
    // Fractions chosen to miss edges and vertices of the catalogue parts.
    let fr = [0.1234, 0.3719, 0.5103, 0.6871, 0.9012];
    let (mut rays, mut hits, mut segments, mut in_face) = (0, 0, 0, 0);
    for case in catalogue().parts {
        let (Ok(e), Ok(r)) = (
            k.evaluate(&case.def, &case.tools),
            Reference::build(&case.def, &case.tools),
        ) else {
            continue;
        };
        let Ok(b) = k.bbox_within(&e.brep, &Placement::IDENTITY, 1e-9) else {
            continue;
        };
        let (lo, hi) = (b.aabb.min, b.aabb.max);
        let size = (hi - lo).length();
        let centre = Point3::new(
            0.5 * (lo.x + hi.x),
            0.5 * (lo.y + hi.y),
            0.5 * (lo.z + hi.z),
        );
        let mut batch = Vec::new();
        for d in dirs {
            let a = cadview_core::solid::silhouette::perpendicular(d);
            let c = d.cross(a);
            for &u in &fr {
                for &w in &fr {
                    let off = a.scale((u - 0.5) * size) + c.scale((w - 0.5) * size);
                    let o = Point3::new(
                        centre.x + off.x - d.x * size,
                        centre.y + off.y - d.y * size,
                        centre.z + off.z - d.z * size,
                    );
                    batch.push(Ray3 { origin: o, dir: d });
                }
            }
        }
        let answers = k.ray_hits(&e.brep, &batch).expect("ray hits");
        for (ray, ans) in batch.iter().zip(answers) {
            let Ok(h) = ans else { continue };
            rays += 1;
            if h.in_face {
                in_face += 1;
                continue;
            }
            let at = |t: f64| {
                Point3::new(
                    ray.origin.x + ray.dir.x * t,
                    ray.origin.y + ray.dir.y * t,
                    ray.origin.z + ray.dir.z * t,
                )
            };
            let mut crossings = Vec::new();
            for x in &h.hits {
                hits += 1;
                if let Some(m) = off(&r, at(x.t)) {
                    panic!("{}: a hit {m:e} mm off the boundary", case.name);
                }
                if !x.tangent {
                    crossings.push(x.t);
                }
            }
            for (i, w) in crossings.windows(2).enumerate() {
                let inside_expected = i % 2 == 0;
                match r.classify(at(0.5 * (w[0] + w[1]))) {
                    Class::Inside { .. } => assert!(inside_expected, "{}", case.name),
                    Class::Outside { .. } => assert!(!inside_expected, "{}", case.name),
                    Class::Band => continue,
                }
                segments += 1;
            }
        }
    }
    println!(
        "ray hits: {rays} rays, {hits} hits on the boundary, {segments} segments between crossings \
         inside or outside as the reference says, {in_face} rays inside a face"
    );
    assert!(rays > 10_000 && segments > 2_000, "{rays} {segments}");
}

#[test]
fn continued_evaluation_equals_from_scratch() {
    let k = ArrisKernel;
    let (mut checked, mut refused_alike) = (0, 0);
    for case in catalogue().parts {
        let n = case.def.features.len();
        let full = k.evaluate(&case.def, &case.tools);
        for done in 1..n {
            let mut prefix = case.def.clone();
            prefix.features.truncate(done);
            let Ok(base) = k.evaluate(&prefix, &case.tools) else {
                continue;
            };
            let cont = k.evaluate_from(&case.def, &case.tools, &base.brep, done);
            match (&full, &cont) {
                (Ok(a), Ok(b)) => {
                    let (ma, mb) = (k.mass(&a.brep).unwrap(), k.mass(&b.brep).unwrap());
                    assert_eq!(ma.volume.to_bits(), mb.volume.to_bits(), "{}", case.name);
                    assert_eq!(ma.centroid, mb.centroid, "{}", case.name);
                    let bx = |x| k.bbox_within(x, &Placement::IDENTITY, 1e-9).ok();
                    assert_eq!(bx(&a.brep), bx(&b.brep), "{}", case.name);
                    if done == n - 1 {
                        let (ta, tb) = (k.tessellate(&a.brep, 0.05), k.tessellate(&b.brep, 0.05));
                        assert_eq!(ta.ok(), tb.ok(), "{}: mesh", case.name);
                    }
                    checked += 1;
                }
                (Err(a), Err(b)) => {
                    assert_eq!(a.to_string(), b.to_string(), "{}", case.name);
                    refused_alike += 1;
                }
                (a, b) => panic!(
                    "{} from {done}: scratch {:?} but continued {:?}",
                    case.name,
                    a.as_ref().err(),
                    b.as_ref().err()
                ),
            }
        }
    }
    println!(
        "evaluate_from: {checked} continuations equal to from scratch, {refused_alike} refused alike"
    );
    assert!(checked > 50, "{checked}");
}

#[test]
fn boxes_within_a_tolerance_agree_with_exact_boxes() {
    let k = ArrisKernel;
    let tol = 1e-9;
    let (mut exact, mut bracketed) = (0, 0);
    let frames = [Placement::IDENTITY, solid_harness::cases::generic_motion()];
    for case in catalogue().parts {
        let Ok(e) = k.evaluate(&case.def, &case.tools) else {
            continue;
        };
        for frame in &frames {
            match k.bbox(&e.brep, frame) {
                Ok(b) => {
                    assert_eq!(
                        k.bbox_within(&e.brep, frame, tol).unwrap(),
                        TightBox::exact(b),
                        "{}",
                        case.name
                    );
                    exact += 1;
                }
                Err(_) => {
                    let t = k.bbox_within(&e.brep, frame, tol).unwrap();
                    assert!(t.tol > 0.0, "{}", case.name);
                    if let (Some(want), true) = (case.bbox, frame.is_identity()) {
                        let d = t.aabb.max_abs_diff(&want);
                        assert!(d <= t.tol + 1e-7, "{}: {d:e}", case.name);
                    }
                    bracketed += 1;
                }
            }
        }
    }
    println!("bbox_within: {exact} equal to the exact box, {bracketed} bracketed");
    assert!(exact > 100, "{exact}");
}

#[test]
fn pair_queries_on_a_solid_and_its_copies() {
    let k = ArrisKernel;
    let mut checked = 0;
    for case in catalogue().parts {
        let Ok(e) = k.evaluate(&case.def, &case.tools) else {
            continue;
        };
        let (Ok(m), Ok(b)) = (k.mass(&e.brep), k.bbox(&e.brep, &Placement::IDENTITY)) else {
            continue;
        };
        let gap = 2.5;
        let away = Placement::translation(b.max.x - b.min.x + gap, 0.0, 0.0);
        let solids = [
            (&e.brep, Placement::IDENTITY),
            (&e.brep, Placement::IDENTITY),
            (&e.brep, away),
        ];
        let pairs = [(0, 1), (0, 2)];
        let (Ok(i), Ok(d)) = (
            k.interferences(&solids, &pairs),
            k.distances(&solids, &pairs),
        ) else {
            continue;
        };
        match &i[0] {
            Ok(Interference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
                ..
            }) => {
                assert!(
                    (volume - m.volume).abs() <= 1e-9 * m.volume,
                    "{}",
                    case.name
                );
                assert!(*a_in_b && *b_in_a, "{}", case.name);
            }
            other => panic!("{}: a solid with itself: {other:?}", case.name),
        }
        if let (Ok(Interference::Apart(c)), Ok(c2)) = (&i[1], &d[1]) {
            assert!(c.distance >= gap - 1e-9, "{}: {}", case.name, c.distance);
            assert_eq!(c.distance.to_bits(), c2.distance.to_bits(), "{}", case.name);
            assert!(
                (c.a.distance(c.b) - c.distance).abs() <= 1e-9,
                "{}",
                case.name
            );
            checked += 1;
        } else if let Ok(other) = &i[1] {
            panic!("{}: moved past its box: {other:?}", case.name);
        }
    }
    println!("interferences and distances: {checked} solids checked against copies");
    assert!(checked > 50, "{checked}");
}
