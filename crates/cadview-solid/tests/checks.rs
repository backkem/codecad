//! Rung 3D1b on arris, through `cad_call` (the API scripts use): evaluation
//! that continues from a cached prefix gives what a from-scratch
//! evaluation gives; boxes of parts where two cylinders meet; the `near`
//! edge rule at a revolve's seam; `mirror3d`; and the set queries between
//! parts against closed forms, with timings for a hundred parts.
//!
//! Run with `-- --nocapture` to see the numbers.

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use cadview_core::document::Document;
use cadview_core::solid::eval::{DefTools, Evaluator, KernelEvaluator, SolidEnv};
use cadview_core::solid::kernel::SolidKernel;
use cadview_core::solid::part::parse_part_id;
use cadview_core::solid::Placement;
use cadview_solid::ArrisKernel;
use serde_json::{json, Value};

fn doc() -> Document {
    let mut d = Document::new();
    d.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(ArrisKernel)));
    d
}

fn call(d: &mut Document, method: &str, args: Value) -> Value {
    let out = cadview_core::cad_call(d, method, &args.to_string())
        .unwrap_or_else(|e| panic!("{method} {args}: {e}"));
    serde_json::from_str(&out).unwrap()
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs()
}

/// An axis-aligned box part.
fn boxed(d: &mut Document, min: [f64; 3], max: [f64; 3]) -> String {
    id(&call(
        d,
        "extrude",
        json!({"profile": {"min": [min[0], min[1]], "max": [max[0], max[1]]},
               "at": [0, 0, min[2]], "distance": max[2] - min[2]}),
    ))
}

/// A w x h rectangle at (x, y) with corners rounded to r: vertices with
/// bulges (quarter arcs), as examples/3d/lib/shapes.js makes it.
fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> Value {
    let q = (PI / 8.0).tan();
    json!([
        [x + r, y],
        [x + w - r, y, q],
        [x + w, y + r],
        [x + w, y + h - r, q],
        [x + w - r, y + h],
        [x + r, y + h, q],
        [x, y + h - r],
        [x, y + r, q],
    ])
}

/// The base of examples/3d/04-enclosure.js: 20 features. Returns the part
/// and the time of each call (ms).
fn enclosure_base(d: &mut Document) -> (String, Vec<f64>) {
    let (l, w, split, wall) = (100.0, 60.0, 17.0, 2.0);
    let mut times = Vec::new();
    let mut timed = |d: &mut Document, m: &str, a: Value| {
        let t = Instant::now();
        let v = call(d, m, a);
        times.push(ms(t));
        v
    };
    let base = id(&timed(
        d,
        "extrude",
        json!({"profile": rounded_rect(0.0, 0.0, l, w, 3.0), "distance": split}),
    ));
    timed(
        d,
        "cut",
        json!({"target": base, "tool": {"extrude": rounded_rect(wall, wall, l - 2.0 * wall, w - 2.0 * wall, 1.0), "at": [0, 0, wall], "distance": split}}),
    );
    for [x, y] in [[5.0, 5.0], [95.0, 5.0], [5.0, 55.0], [95.0, 55.0]] {
        timed(
            d,
            "union",
            json!({"target": base, "tool": {"extrude": {"center": [x, y], "radius": 3.5}, "at": [0, 0, wall - 1.0], "distance": split - wall + 1.0}}),
        );
        timed(
            d,
            "hole",
            json!({"target": base, "at": [x, y, split], "dia": 2.5, "depth": 10}),
        );
    }
    for [x, y] in [[15.0, 15.0], [85.0, 15.0], [15.0, 45.0], [85.0, 45.0]] {
        timed(
            d,
            "union",
            json!({"target": base, "tool": {"extrude": {"center": [x, y], "radius": 2.5}, "at": [0, 0, wall - 1.0], "distance": 6.0 - wall + 1.0}}),
        );
        timed(
            d,
            "hole",
            json!({"target": base, "at": [x, y, 6], "dia": 2.5, "depth": 5}),
        );
    }
    timed(
        d,
        "cut",
        json!({"target": base, "tool": {"extrude": rounded_rect(45.0, 7.6, 10.0, 4.5, 1.0), "plane": "xz", "at": [0, wall + 1.0, 0], "distance": wall + 2.0}}),
    );
    timed(
        d,
        "hole",
        json!({"target": base, "at": [l, 30, 11.6], "dir": [-1, 0, 0], "dia": 8.4, "depth": wall + 1.0}),
    );
    (base, times)
}

/// The cached solid of a part (built call by call, each continuing from
/// the one before) against the same definition evaluated from scratch:
/// equal volume, centroid, box and mesh, bit for bit.
fn assert_same_as_from_scratch(d: &Document, part: &str, what: &str) -> f64 {
    let p = d.part(parse_part_id(part).unwrap()).unwrap();
    let def = &d.part_defs[&p.def];
    let solids = d.solids.get().unwrap();
    let cached = solids.solid(&d.part_defs, def).unwrap();
    let t = Instant::now();
    let fresh = KernelEvaluator::new(ArrisKernel)
        .evaluate(def, &DefTools(&d.part_defs))
        .unwrap();
    let scratch_ms = ms(t);
    let (a, b) = (cached.mass().unwrap(), fresh.mass().unwrap());
    assert_eq!(a.volume.to_bits(), b.volume.to_bits(), "{what}: volume");
    assert_eq!(a.centroid, b.centroid, "{what}: centroid");
    for frame in [Placement::IDENTITY, p.placement] {
        assert_eq!(
            cached.bbox(&frame).unwrap(),
            fresh.bbox(&frame).unwrap(),
            "{what}: box"
        );
    }
    let (ma, mb) = (
        cached.tessellate(0.05).unwrap(),
        fresh.tessellate(0.05).unwrap(),
    );
    assert_eq!(ma.triangles, mb.triangles, "{what}: mesh triangles");
    assert_eq!(ma.positions, mb.positions, "{what}: mesh positions");
    println!(
        "{what}: {} features, volume {:.6}, {} triangles; from scratch {scratch_ms:.0} ms",
        def.features.len(),
        a.volume,
        ma.triangles.len()
    );
    scratch_ms
}

#[test]
fn evaluation_continued_from_a_prefix_equals_from_scratch() {
    let mut d = doc();
    let (base, times) = enclosure_base(&mut d);
    let solids = d.solids.get().unwrap();
    // The first call evaluates from scratch; each later one continues.
    assert_eq!(solids.continued(), times.len() as u64 - 1);
    let scratch = assert_same_as_from_scratch(&d, &base, "enclosure base");
    println!(
        "enclosure base, per call (ms): {}",
        times
            .iter()
            .map(|t| format!("{t:.0}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "enclosure base: all calls {:.0} ms, the whole list from scratch {scratch:.0} ms",
        times.iter().sum::<f64>()
    );

    // A side of examples/3d/03-bookshelf.js: a round by a near point, then
    // dados and a rabbet cut by other parts (tool definitions).
    let shelf = boxed(&mut d, [12.0, 0.0, 60.0], [788.0, 294.0, 78.0]);
    let back = boxed(&mut d, [12.0, 294.0, 0.0], [788.0, 300.0, 1782.0]);
    let side = boxed(&mut d, [0.0, 0.0, 0.0], [18.0, 300.0, 1782.0]);
    call(
        &mut d,
        "fillet3d",
        json!({"target": side, "edges": {"near": [[0, 0, 891]]}, "radius": 3}),
    );
    call(
        &mut d,
        "cut",
        json!({"target": side, "tool": {"part": shelf}}),
    );
    call(
        &mut d,
        "cut",
        json!({"target": side, "tool": {"part": back}}),
    );
    assert_same_as_from_scratch(&d, &side, "bookshelf side");

    // Curved: a revolved body, a rounded crown, a boss and a cross bore.
    let piston = id(&call(
        &mut d,
        "revolve",
        json!({"profile": [[0, 0], [40, 0], [40, 60], [0, 60]], "plane": "xz", "axis": [[0, 0], [0, 1]]}),
    ));
    call(
        &mut d,
        "fillet3d",
        json!({"target": piston, "edges": {"near": [[0, 40, 60]]}, "radius": 2}),
    );
    call(
        &mut d,
        "hole",
        json!({"target": piston, "at": [0, -50, 25], "dir": [0, 1, 0], "dia": 16}),
    );
    call(
        &mut d,
        "hole",
        json!({"target": piston, "at": [0, 0, 60], "dia": 10, "depth": 20}),
    );
    assert_same_as_from_scratch(&d, &piston, "revolved and cross-bored");
}

/// The box a part reports, its tolerance, and the box it should have.
fn assert_box(p: &Value, min: [f64; 3], max: [f64; 3], what: &str) -> f64 {
    let tol = p["bboxTol"].as_f64().unwrap();
    let b = &p["bbox"];
    let mut worst: f64 = 0.0;
    for i in 0..3 {
        let (lo, hi) = (b["min"][i].as_f64().unwrap(), b["max"][i].as_f64().unwrap());
        // The box contains the solid: never inside the closed form by more
        // than rounding, and outside it by at most the tolerance (plus the
        // kernel's own fit of the intersection curves, 1e-7 at most).
        assert!(
            lo <= min[i] + 1e-9 && hi >= max[i] - 1e-9,
            "{what}: {b} vs {min:?} {max:?}"
        );
        worst = worst.max(min[i] - lo).max(hi - max[i]);
    }
    assert!(
        worst <= tol + 1e-7,
        "{what}: box {b} off by {worst:e}, tol {tol:e}"
    );
    println!("{what}: box off the closed form by {worst:.2e} mm, reported tolerance {tol:e}");
    worst
}

#[test]
fn parts_where_two_cylinders_meet_have_tight_boxes() {
    let mut d = doc();
    // A cross-bored cylinder: r 20, z 0..100, a Ø10 bore along x at z 50.
    let c = id(&call(
        &mut d,
        "extrude",
        json!({"profile": {"center": [0, 0], "radius": 20}, "distance": 100}),
    ));
    let p = call(
        &mut d,
        "hole",
        json!({"target": c, "at": [-30, 0, 50], "dir": [1, 0, 0], "dia": 10}),
    );
    assert_box(
        &p,
        [-20.0, -20.0, 0.0],
        [20.0, 20.0, 100.0],
        "cross-bored cylinder",
    );
    // A second through hole on the cross-bored part (its reach needs the
    // box of a part that already has NURBS edges).
    let p = call(
        &mut d,
        "hole",
        json!({"target": c, "at": [0, -30, 20], "dir": [0, 1, 0], "dia": 6}),
    );
    assert_box(
        &p,
        [-20.0, -20.0, 0.0],
        [20.0, 20.0, 100.0],
        "twice cross-bored",
    );
    let want = PI * 400.0 * 100.0;
    assert!(p["volume"].as_f64().unwrap() < want);

    // A tee: a pipe along x (r 30, x -100..100) with a branch up (r 20, to
    // z 80), bored through (r 25 and r 15).
    let main = id(&call(
        &mut d,
        "extrude",
        json!({"profile": {"center": [0, 0], "radius": 30}, "plane": "yz", "at": [-100, 0, 0], "distance": 200}),
    ));
    call(
        &mut d,
        "union",
        json!({"target": main, "tool": {"extrude": {"center": [0, 0], "radius": 20}, "distance": 80}}),
    );
    call(
        &mut d,
        "hole",
        json!({"target": main, "at": [-100, 0, 0], "dir": [1, 0, 0], "dia": 50}),
    );
    let tee = call(
        &mut d,
        "hole",
        json!({"target": main, "at": [0, 0, 80], "dir": [0, 0, -1], "dia": 30, "depth": 80}),
    );
    assert_box(&tee, [-100.0, -30.0, -30.0], [100.0, 30.0, 80.0], "tee");

    // Two crossing cylinders, kept where both are (r 10 along z, r 20 along
    // x): the box sides at y = +-10 and z = +-20 lie on the intersection
    // curves, so the NURBS bracket decides them.
    let a = id(&call(
        &mut d,
        "extrude",
        json!({"profile": {"center": [0, 0], "radius": 10}, "at": [0, 0, -30], "distance": 60}),
    ));
    let both = call(
        &mut d,
        "intersect",
        json!({"target": a, "tool": {"extrude": {"center": [0, 0], "radius": 20}, "plane": "yz", "at": [-30, 0, 0], "distance": 60}}),
    );
    let worst = assert_box(
        &both,
        [-10.0, -10.0, -20.0],
        [10.0, 10.0, 20.0],
        "crossing cylinders",
    );
    assert!(both["bboxTol"].as_f64().unwrap() > 0.0, "{both}");
    assert!(worst > 0.0 || both["bboxTol"].as_f64().unwrap() > 0.0);

    // The exact-only box (what the test harness asks for) still refuses.
    let p = d.part(parse_part_id(&a).unwrap()).unwrap();
    let def = &d.part_defs[&p.def];
    let brep = ArrisKernel
        .evaluate(def, &DefTools(&d.part_defs))
        .unwrap()
        .brep;
    let e = ArrisKernel.bbox(&brep, &Placement::IDENTITY).unwrap_err();
    assert!(e.to_string().contains("NURBS edge"), "{e}");
}

/// A chamfer by a point on a revolve's sketch at +x picks the circular edge
/// there, not the revolve's seam (which meets it at that point).
#[test]
fn near_on_a_revolve_seam_picks_the_real_edge() {
    let mut d = doc();
    let disc = id(&call(
        &mut d,
        "revolve",
        json!({"profile": {"min": [0, 0], "max": [82.5, 18]}, "plane": "xz", "axis": [[0, 0], [0, 1]]}),
    ));
    let p = call(
        &mut d,
        "chamfer3d",
        json!({"target": disc, "edges": {"near": [[82.5, 0, 18]]}, "d": 1}),
    );
    // Pappus: the triangle d^2 / 2 at radius R - d / 3.
    let want = PI * 82.5 * 82.5 * 18.0 - 2.0 * PI * (82.5 - 1.0 / 3.0) * 0.5;
    let got = p["volume"].as_f64().unwrap();
    assert!(rel(got, want) <= 1e-9, "{got} vs {want}");
    // The same point at -x was never ambiguous; it gives the same part.
    let disc2 = id(&call(
        &mut d,
        "revolve",
        json!({"profile": {"min": [0, 0], "max": [82.5, 18]}, "plane": "xz", "axis": [[0, 0], [0, 1]]}),
    ));
    let q = call(
        &mut d,
        "chamfer3d",
        json!({"target": disc2, "edges": {"near": [[-82.5, 0, 18]]}, "d": 1}),
    );
    assert!(rel(q["volume"].as_f64().unwrap(), want) <= 1e-9);
}

#[test]
fn mirror3d_gives_the_mirror_image_on_the_kernel() {
    let mut d = doc();
    // An L-shaped plate with a hole off centre and a rounded corner: no
    // symmetry, so a wrong reflection shows in the box or the volume.
    let l = id(&call(
        &mut d,
        "extrude",
        json!({"profile": [[0, 0], [60, 0], [60, 20], [20, 20], [20, 50], [0, 50]], "distance": 8}),
    ));
    call(
        &mut d,
        "hole",
        json!({"target": l, "at": [45, 10, 8], "dia": 6}),
    );
    let p = call(
        &mut d,
        "fillet3d",
        json!({"target": l, "edges": {"near": [[60, 0, 4]]}, "radius": 4}),
    );
    let v = p["volume"].as_f64().unwrap();
    // Across the plane x = 100 (normal along x), as a copy.
    let m = call(
        &mut d,
        "mirror3d",
        json!({"target": l, "point": [100, 0, 0], "normal": [1, 0, 0], "copy": true}),
    );
    assert!(rel(m["volume"].as_f64().unwrap(), v) <= 1e-12, "{m}");
    let b = &m["bbox"];
    for (i, (lo, hi)) in [(140.0, 200.0), (0.0, 50.0), (0.0, 8.0)].iter().enumerate() {
        assert!((b["min"][i].as_f64().unwrap() - lo).abs() <= 1e-9, "{m}");
        assert!((b["max"][i].as_f64().unwrap() - hi).abs() <= 1e-9, "{m}");
    }
    // The hole's mirror image is at x = 155: a point there is outside the
    // material, its mirror x = 45 on the original too; the centroid
    // mirrors.
    let c0 = p["centroid"][0].as_f64().unwrap();
    let c1 = m["centroid"][0].as_f64().unwrap();
    assert!((c0 + c1 - 200.0).abs() <= 1e-9, "{c0} {c1}");
    // The two never overlap; mirrored across x = 60 (the plate's edge)
    // they touch.
    let t = call(
        &mut d,
        "mirror3d",
        json!({"target": l, "point": [60, 0, 0], "normal": [1, 0, 0], "copy": true}),
    );
    let r = call(
        &mut d,
        "overlapping3d",
        json!({"a": [l], "b": [t["id"]], "touching": true}),
    );
    assert_eq!(r["pairs"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["pairs"][0]["volume"], 0.0, "{r}");
}

/// The ladder's gate for rung 3D1b on the bookshelf geometry: a shelf 2 mm
/// too long gives exactly one interference record, the right pair, with
/// the overlap volume equal to the closed form; the correct shelf gives
/// none; the gap between two parallel plates comes back to 1e-9 mm.
#[test]
fn interference_and_clearance_gates() {
    let mut d = doc();
    // Sides with 6 deep dados, a shelf that fills them, and a top on them.
    let (w, t, depth, dado, z) = (800.0, 18.0, 294.0, 6.0, 400.0);
    let left = boxed(&mut d, [0.0, 0.0, 0.0], [t, 300.0, 1782.0]);
    let right = boxed(&mut d, [w - t, 0.0, 0.0], [w, 300.0, 1782.0]);
    let shelf = boxed(&mut d, [t - dado, 0.0, z], [w - t + dado, depth, z + t]);
    let top = boxed(&mut d, [0.0, 0.0, 1782.0], [w, 300.0, 1800.0]);
    for s in [&left, &right] {
        call(&mut d, "cut", json!({"target": s, "tool": {"part": shelf}}));
    }
    let t0 = Instant::now();
    let r = call(&mut d, "overlapping3d", json!({"a": null}));
    println!(
        "bookshelf, correct shelf: {} records, {:.1} ms",
        r["pairs"].as_array().unwrap().len(),
        ms(t0)
    );
    assert_eq!(r["pairs"], json!([]), "{r}");
    assert_eq!(r["skipped"], json!([]), "{r}");
    // The shelf made 2 mm longer at its right end, after the dados were
    // cut: a definition of its own (the sides keep the dados the shelf's
    // definition cut), in the shelf's place.
    let long = id(&call(
        &mut d,
        "copy3d",
        json!({"target": shelf, "unique": true}),
    ));
    call(&mut d, "remove3d", json!({"target": shelf}));
    call(
        &mut d,
        "union",
        json!({"target": long, "tool": {"extrude": {"min": [w - t + dado, 0], "max": [w - t + dado + 2.0, depth]}, "at": [0, 0, z], "distance": t}}),
    );
    let shelf = long;
    let r = call(&mut d, "overlapping3d", json!({"a": null}));
    let pairs = r["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "{r}");
    let (a, b) = (
        pairs[0]["a"].as_str().unwrap(),
        pairs[0]["b"].as_str().unwrap(),
    );
    let mut pair = [a.to_string(), b.to_string()];
    pair.sort();
    let mut want_pair = [shelf.clone(), right.clone()];
    want_pair.sort();
    assert_eq!(pair, want_pair, "{r}");
    let want = 2.0 * depth * t;
    let got = pairs[0]["volume"].as_f64().unwrap();
    println!(
        "bookshelf, shelf 2 mm long: overlap {got} mm^3, closed form {want}, rel err {:.1e}",
        rel(got, want)
    );
    assert!(rel(got, want) <= 1e-6, "{got} vs {want}");
    let _ = top;

    // Two parallel plates 3.5 apart (one turned a quarter about z, so the
    // gap is between a face and a face of a rotated part).
    let p = boxed(&mut d, [0.0, 1000.0, 0.0], [200.0, 1100.0, 10.0]);
    let q = boxed(&mut d, [-50.0, -50.0, 0.0], [50.0, 50.0, 6.0]);
    call(&mut d, "rotate3d", json!({"target": q, "angle": 90}));
    call(
        &mut d,
        "move3d",
        json!({"target": q, "by": [100, 1050, 13.5]}),
    );
    let r = call(&mut d, "distance3d", json!({"a": p, "b": q}));
    let gap = r["distance"].as_f64().unwrap();
    println!(
        "parallel plates: gap {gap:.15} (closed form 3.5), error {:.1e}",
        (gap - 3.5).abs()
    );
    assert!((gap - 3.5).abs() <= 1e-9, "{r}");
    let r = call(
        &mut d,
        "pairsWithin3d",
        json!({"a": [p], "b": [q], "distance": 3.6}),
    );
    assert_eq!(r["pairs"].as_array().unwrap().len(), 1, "{r}");
    let r = call(
        &mut d,
        "pairsWithin3d",
        json!({"a": [p], "b": [q], "distance": 3.4}),
    );
    assert_eq!(r["pairs"], json!([]), "{r}");
}

/// A hundred parts: a 10 x 10 grid of 40 x 40 x 20 blocks 10 apart, three
/// of them moved to overlap a neighbour, as one query each.
#[test]
fn a_hundred_parts_stay_fast() {
    let mut d = doc();
    let first = boxed(&mut d, [0.0, 0.0, 0.0], [40.0, 40.0, 20.0]);
    call(
        &mut d,
        "copy3d",
        json!({"target": first, "move": [50, 0, 0], "count": 9}),
    );
    let row = call(&mut d, "parts", json!({"geometry": false}));
    let ids: Vec<String> = row.as_array().unwrap().iter().map(id).collect();
    call(
        &mut d,
        "copy3d",
        json!({"target": ids, "move": [0, 50, 0], "count": 9}),
    );
    let all = call(&mut d, "parts", json!({}));
    let all: Vec<String> = all.as_array().unwrap().iter().map(id).collect();
    assert_eq!(all.len(), 100);
    // Three blocks moved 15 toward their right-hand neighbour: overlap
    // 5 x 40 x 20 each.
    for k in [3, 47, 82] {
        call(
            &mut d,
            "move3d",
            json!({"target": all[k], "by": [15, 0, 0]}),
        );
    }
    let t = Instant::now();
    let r = call(&mut d, "overlapping3d", json!({"a": null}));
    let overlap_ms = ms(t);
    assert_eq!(r["pairs"].as_array().unwrap().len(), 3, "{r}");
    for p in r["pairs"].as_array().unwrap() {
        assert!(rel(p["volume"].as_f64().unwrap(), 4000.0) <= 1e-9, "{p}");
    }
    // The blocks are axis-aligned boxes, so the expected answers follow
    // from their boxes by brute force.
    let boxes: Vec<[[f64; 3]; 2]> = call(&mut d, "parts", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let c = |k: &str, i: usize| p["bbox"][k][i].as_f64().unwrap();
            [
                [c("min", 0), c("min", 1), c("min", 2)],
                [c("max", 0), c("max", 1), c("max", 2)],
            ]
        })
        .collect();
    let gap = |a: &[[f64; 3]; 2], b: &[[f64; 3]; 2]| {
        (0..3)
            .map(|i| (b[0][i] - a[1][i]).max(a[0][i] - b[1][i]).max(0.0).powi(2))
            .sum::<f64>()
            .sqrt()
    };
    let mut want_within = 0;
    for i in 0..100 {
        for j in i + 1..100 {
            if gap(&boxes[i], &boxes[j]) < 10.5 {
                want_within += 1;
            }
        }
    }
    let t = Instant::now();
    let r = call(
        &mut d,
        "pairsWithin3d",
        json!({"a": null, "distance": 10.5}),
    );
    let within_ms = ms(t);
    let n = r["pairs"].as_array().unwrap().len();
    assert_eq!(n, want_within);
    let region = [[0.0, 0.0, 0.0], [240.0, 240.0, 20.0]];
    let want_inside = boxes
        .iter()
        .filter(|b| (0..3).all(|i| b[0][i] >= region[0][i] && b[1][i] <= region[1][i]))
        .count();
    let t = Instant::now();
    let r = call(
        &mut d,
        "inside3d",
        json!({"a": null, "region": {"min": region[0], "max": region[1]}}),
    );
    let inside_ms = ms(t);
    assert_eq!(r["pairs"].as_array().unwrap().len(), want_inside, "{r}");
    println!(
        "100 parts: overlapping3d {overlap_ms:.0} ms (3 pairs), pairsWithin3d {within_ms:.0} ms ({n} pairs), inside3d {inside_ms:.1} ms"
    );
}
