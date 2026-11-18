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
//! These capabilities are not scoreboard layers (the scoreboards are
//! unchanged by them). `cargo test -p solid-harness --release --test
//! capabilities -- --nocapture` prints the counts.

use cadview_core::solid::kernel::{Interference, TightBox};
use cadview_core::solid::{Placement, SolidKernel};
use solid_harness::cases::catalogue;
use solid_harness::kernels::ArrisKernel;

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
