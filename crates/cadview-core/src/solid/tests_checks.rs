//! Core tests for rung 3D1b on the closed-form fake kernel: evaluation that
//! continues from a cached prefix, `mirror3d`, and the set queries between
//! parts (`overlapping3d`, `pairsWithin3d`, `inside3d`, `distance3d`). The
//! same calls on arris are tested in `cadview-solid`.

use std::sync::Arc;

use serde_json::{json, Value};

use super::eval::{KernelEvaluator, SolidEnv, NO_KERNEL};
use super::fake::FakeKernel;
use crate::document::Document;

fn doc() -> Document {
    let mut d = Document::new();
    d.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(FakeKernel)));
    d
}

fn call(d: &mut Document, method: &str, args: Value) -> Value {
    let out = crate::cad_call(d, method, &args.to_string())
        .unwrap_or_else(|e| panic!("{method} {args}: {e}"));
    serde_json::from_str(&out).unwrap()
}

fn fails(d: &mut Document, method: &str, args: Value) -> String {
    match crate::cad_call(d, method, &args.to_string()) {
        Ok(v) => panic!("{method} {args} should fail, got {v}"),
        Err(e) => e,
    }
}

fn boxed(d: &mut Document, min: [f64; 3], max: [f64; 3], attrs: Value) -> String {
    let p = call(
        d,
        "extrude",
        json!({"profile": {"min": [min[0], min[1]], "max": [max[0], max[1]]},
               "at": [0, 0, min[2]], "distance": max[2] - min[2], "attrs": attrs}),
    );
    p["id"].as_str().unwrap().to_string()
}

fn counts(d: &Document) -> (u64, u64) {
    let s = d.solids.get().unwrap();
    (s.evaluations(), s.continued())
}

/// Each modifying call evaluates once, continuing from the definition as
/// it was (the cached prefix), not from scratch; the result is the one a
/// fresh cache gives.
#[test]
fn a_modifying_call_continues_from_the_cached_prefix() {
    let mut d = doc();
    let p = boxed(&mut d, [0.0, 0.0, 0.0], [100.0, 50.0, 18.0], json!({}));
    assert_eq!(counts(&d), (1, 0));
    for k in 0..4 {
        let x = 10.0 + 20.0 * k as f64;
        call(
            &mut d,
            "hole",
            json!({"target": p, "at": [x, 25, 18], "dia": 4, "depth": 6}),
        );
        assert_eq!(counts(&d), (2 + k, 1 + k), "after hole {k}");
    }
    let v = call(&mut d, "part", json!({"id": p}))["volume"]
        .as_f64()
        .unwrap();
    let want = 90_000.0 - 4.0 * std::f64::consts::PI * 4.0 * 6.0;
    assert!((v - want).abs() <= 1e-9 * want, "{v} vs {want}");
    // A fresh cache evaluates the same definition from scratch: same answer.
    d.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(FakeKernel)));
    let fresh = call(&mut d, "part", json!({"id": p}))["volume"]
        .as_f64()
        .unwrap();
    assert_eq!(fresh.to_bits(), v.to_bits());
    assert_eq!(counts(&d), (1, 0));
}

/// Changing a tool re-evaluates the part that uses it from the prefix
/// before the boolean, which does not reach the tool.
#[test]
fn a_changed_tool_reuses_the_prefix_before_it() {
    let mut d = doc();
    let plate = boxed(&mut d, [0.0, 0.0, 0.0], [100.0, 50.0, 18.0], json!({}));
    let tool = boxed(&mut d, [10.0, 10.0, 12.0], [20.0, 20.0, 18.0], json!({}));
    call(
        &mut d,
        "cut",
        json!({"target": plate, "tool": {"part": tool}}),
    );
    let (e0, c0) = counts(&d);
    // Changing the tool's definition re-evaluates the tool (continued) and
    // the plate that uses it (continued from its extrude).
    call(
        &mut d,
        "hole",
        json!({"target": tool, "at": [15, 15, 18], "dia": 2, "depth": 3}),
    );
    let (e1, c1) = counts(&d);
    assert_eq!((e1 - e0, c1 - c0), (2, 2));
}

#[test]
fn mirror3d_reflects_in_place_and_as_copies() {
    let mut d = doc();
    let a = boxed(
        &mut d,
        [0.0, 0.0, 0.0],
        [100.0, 50.0, 18.0],
        json!({"k": 1}),
    );
    let def = call(&mut d, "part", json!({"id": a}))["def"].clone();
    // A copy keeps the original definition in use.
    let keep = call(&mut d, "copy3d", json!({"target": a, "move": [0, 100, 0]}));
    // Across the plane x = 200: x in [300, 400].
    let r = call(
        &mut d,
        "mirror3d",
        json!({"target": a, "point": [200, 0, 0], "normal": [1, 0, 0]}),
    );
    assert_eq!(r["mirrored"], json!([a]));
    assert_eq!(r["defsMade"].as_array().unwrap().len(), 1);
    assert_eq!(r["defsRemoved"], json!([]));
    let made = r["defsMade"][0].clone();
    let p = call(&mut d, "part", json!({"id": a}));
    let b = &p["bbox"];
    for (i, (lo, hi)) in [(300.0, 400.0), (0.0, 50.0), (0.0, 18.0)]
        .iter()
        .enumerate()
    {
        assert!((b["min"][i].as_f64().unwrap() - lo).abs() < 1e-9, "{p}");
        assert!((b["max"][i].as_f64().unwrap() - hi).abs() < 1e-9, "{p}");
    }
    assert!((p["volume"].as_f64().unwrap() - 90_000.0).abs() < 1e-6);
    assert_eq!(p["attrs"]["k"], 1.0);
    // Mirroring back finds the original definition again (no new one).
    let r = call(
        &mut d,
        "mirror3d",
        json!({"target": a, "point": [200, 0, 0], "normal": [1, 0, 0]}),
    );
    assert_eq!(r["defsMade"], json!([]));
    assert_eq!(r["defsRemoved"], json!([made]));
    let p = call(&mut d, "part", json!({"id": a}));
    assert_eq!(p["def"], def);
    assert!((p["bbox"]["min"][0].as_f64().unwrap()).abs() < 1e-9, "{p}");

    // Copies: two parts of one definition mirror to two parts of one
    // mirrored definition; the originals stay.
    let both = json!([a, keep["id"]]);
    let images = call(
        &mut d,
        "mirror3d",
        json!({"target": both, "normal": [0, 1, 0], "copy": true, "attrs": {"side": "image"}}),
    );
    let images = images.as_array().unwrap();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0]["def"], images[1]["def"]);
    assert_ne!(images[0]["def"], def);
    assert_eq!(images[0]["attrs"]["side"], "image");
    assert_eq!(images[0]["instances"], 2);
    // Across y = 0: y in [-50, 0] and [-150, -100].
    assert!((images[0]["bbox"]["max"][1].as_f64().unwrap()).abs() < 1e-9);
    assert!((images[1]["bbox"]["min"][1].as_f64().unwrap() + 150.0).abs() < 1e-9);
    assert_eq!(
        call(&mut d, "parts", json!({})).as_array().unwrap().len(),
        4
    );

    // Bad input changes nothing.
    let before = call(&mut d, "parts", json!({}));
    let e = fails(
        &mut d,
        "mirror3d",
        json!({"target": a, "normal": [0, 0, 0]}),
    );
    assert!(e.contains("non-zero"), "{e}");
    let e = fails(&mut d, "mirror3d", json!({"target": a}));
    assert!(e.contains("normal"), "{e}");
    assert_eq!(call(&mut d, "parts", json!({})), before);
}

/// Three boxes: A and B overlap by 5 x 10 x 10, B and C are 5 apart, A and
/// C 10 apart; D sits on A (touching).
fn three_boxes(d: &mut Document) -> [String; 4] {
    [
        boxed(d, [0.0, 0.0, 0.0], [10.0, 10.0, 10.0], json!({"g": "x"})),
        boxed(d, [5.0, 0.0, 0.0], [15.0, 10.0, 10.0], json!({"g": "x"})),
        boxed(d, [20.0, 0.0, 0.0], [30.0, 10.0, 10.0], json!({"g": "y"})),
        boxed(d, [0.0, 0.0, 10.0], [4.0, 4.0, 14.0], json!({"g": "y"})),
    ]
}

#[test]
fn overlapping3d_lists_shared_volume_once_per_pair() {
    let mut d = doc();
    let [a, b, c, top] = three_boxes(&mut d);
    let r = call(&mut d, "overlapping3d", json!({"a": null}));
    assert_eq!(
        r["pairs"],
        json!([{"a": a, "b": b, "volume": 500.0, "depth": 5.0, "aInB": false, "bInA": false}])
    );
    assert_eq!(r["skipped"], json!([]));
    // touching: the box on top of A comes too, with volume 0.
    let r = call(
        &mut d,
        "overlapping3d",
        json!({"a": null, "touching": true}),
    );
    let pairs: Vec<(Value, Value)> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["a"].clone(), p["b"].clone()))
        .collect();
    assert!(pairs.contains(&(json!(a), json!(top))), "{r}");
    assert!(!pairs.contains(&(json!(b), json!(c))), "{r}");
    // A tolerance above the overlap's depth (5) makes it a touch.
    let r = call(&mut d, "overlapping3d", json!({"a": null, "tolerance": 6}));
    assert_eq!(r["pairs"], json!([]));
    // Two sets, differ and attrs.
    let r = call(
        &mut d,
        "overlapping3d",
        json!({"a": [a], "b": {"attrs": {"g": "x"}}, "attrs": true}),
    );
    assert_eq!(r["pairs"][0]["b"], json!(b));
    assert_eq!(r["pairs"][0]["bAttrs"]["g"], "x");
    let r = call(&mut d, "overlapping3d", json!({"a": null, "differ": ["g"]}));
    assert_eq!(r["pairs"], json!([]));
    let _ = c;
}

#[test]
fn pairs_within3d_and_distance3d_give_gaps_and_closest_points() {
    let mut d = doc();
    let [a, b, c, top] = three_boxes(&mut d);
    let r = call(&mut d, "pairsWithin3d", json!({"a": null, "distance": 6}));
    let got: Vec<(Value, Value, f64)> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["a"].clone(),
                p["b"].clone(),
                p["distance"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (json!(a), json!(b), 0.0),
            (json!(a), json!(top), 0.0),
            (json!(b), json!(c), 5.0),
            (json!(b), json!(top), 1.0),
        ]
    );
    // Strictly closer: exactly 5 apart is not within 5.
    let r = call(
        &mut d,
        "pairsWithin3d",
        json!({"a": [b], "b": [c], "distance": 5}),
    );
    assert_eq!(r["pairs"], json!([]));
    let r = call(&mut d, "distance3d", json!({"a": a, "b": c}));
    assert_eq!(r["distance"], 10.0);
    assert_eq!(r["pa"][0], 10.0);
    assert_eq!(r["pb"][0], 20.0);
    let e = fails(&mut d, "distance3d", json!({"a": a, "b": a}));
    assert!(e.contains("same part"), "{e}");
}

#[test]
fn inside3d_by_box_and_by_part() {
    let mut d = doc();
    let [a, b, c, top] = three_boxes(&mut d);
    let r = call(
        &mut d,
        "inside3d",
        json!({"a": null, "region": {"min": [0, 0, 0], "max": [15, 10, 10]}}),
    );
    let ins: Vec<&Value> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["a"])
        .collect();
    assert_eq!(ins, vec![&json!(a), &json!(b)]);
    assert_eq!(r["pairs"][0]["touches"], true);
    let r = call(
        &mut d,
        "inside3d",
        json!({"a": null, "region": {"min": [-1, -1, -1], "max": [16, 11, 11]}, "strict": true}),
    );
    assert_eq!(r["pairs"].as_array().unwrap().len(), 2);
    // A small box inside A: inside the solid of A, not of C.
    let small = boxed(&mut d, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0], json!({}));
    let r = call(
        &mut d,
        "inside3d",
        json!({"a": [small, top], "region": [a, c]}),
    );
    assert_eq!(r["pairs"], json!([{"a": small, "b": a}]));
}

#[test]
fn set_queries_need_a_kernel_and_say_so() {
    let mut d = Document::new();
    let e = fails(&mut d, "overlapping3d", json!({"a": null}));
    assert!(e.starts_with("overlapping3d: "), "{e}");
    assert!(e.contains(NO_KERNEL), "{e}");
}
