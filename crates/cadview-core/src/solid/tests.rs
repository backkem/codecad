//! Core tests for parts in the document (rung 3D1a): every `cad_call`
//! method, degenerate input, refusals that leave the document unchanged,
//! undo, JSON, DWG, two Yrs peers and the render scene. They run on the
//! closed-form [`FakeKernel`], so core needs no real kernel.

use std::sync::Arc;

use serde_json::{json, Value};

use super::eval::{KernelEvaluator, SolidEnv, NO_KERNEL};
use super::fake::FakeKernel;
use crate::document::{document_from_json, document_to_json, Document};
use crate::sync::SyncDoc;

const EPS: f64 = 1e-9;

fn fake_env() -> SolidEnv {
    SolidEnv::new(Arc::new(KernelEvaluator::new(FakeKernel)))
}

fn doc() -> Document {
    let mut d = Document::new();
    d.solids = fake_env();
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

/// Everything about parts that a failed call must leave as it was.
fn state(d: &Document) -> (String, u64) {
    (
        serde_json::to_string(&(d.part_defs.values().collect::<Vec<_>>(), &d.parts)).unwrap(),
        d.next_id(),
    )
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-300)
}

fn close3(v: &Value, want: [f64; 3]) -> bool {
    (0..3).all(|i| (v[i].as_f64().unwrap() - want[i]).abs() <= 1e-9)
}

/// A 100 x 50 x 18 plate on XY.
fn plate(d: &mut Document) -> Value {
    call(
        d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [100, 50]}, "distance": 18, "name": "plate", "attrs": {"role": "plate"}}),
    )
}

#[test]
fn extrude_reports_exact_box_volume_and_centroid() {
    let mut d = doc();
    let p = plate(&mut d);
    assert_eq!(p["id"], "p_2");
    assert_eq!(p["def"], "d_1");
    assert_eq!(p["name"], "plate");
    assert_eq!(p["attrs"]["role"], "plate");
    assert!(rel(p["volume"].as_f64().unwrap(), 90_000.0) < EPS);
    assert!(close3(&p["bbox"]["min"], [0.0, 0.0, 0.0]));
    assert!(close3(&p["bbox"]["max"], [100.0, 50.0, 18.0]));
    assert!(close3(&p["centroid"], [50.0, 25.0, 9.0]));
    assert_eq!(p["features"], json!([{"id": 1, "kind": "extrude"}]));

    // The named planes: yz is u along Y, v along Z, normal +X.
    let s = call(
        &mut d,
        "extrude",
        json!({"profile": [[0, 0], [300, 0], [300, 600], [0, 600]], "plane": "yz", "at": [10, 0, 0], "distance": 18}),
    );
    assert!(close3(&s["bbox"]["min"], [10.0, 0.0, 0.0]));
    assert!(close3(&s["bbox"]["max"], [28.0, 300.0, 600.0]));
    // xz has normal -Y: a positive distance goes toward -Y.
    let f = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [40, 20]}, "plane": "xz", "distance": 5}),
    );
    assert!(close3(&f["bbox"]["min"], [0.0, -5.0, 0.0]));
    assert!(close3(&f["bbox"]["max"], [40.0, 0.0, 20.0]));
    // A general plane.
    let g = call(
        &mut d,
        "extrude",
        json!({"profile": {"center": [0, 0], "radius": 5}, "plane": {"origin": [0, 0, 100], "x": [1, 0, 0], "normal": [0, 0, 1]}, "distance": -10}),
    );
    assert!(rel(g["volume"].as_f64().unwrap(), std::f64::consts::PI * 250.0) < EPS);
    assert!(close3(&g["bbox"]["min"], [-5.0, -5.0, 90.0]));
}

#[test]
fn cut_hole_and_tool_parts_subtract_closed_forms() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap().to_string();
    // A 6 deep slot across the top: 100 x 10 x 6.
    let c = call(
        &mut d,
        "cut",
        json!({"target": id, "tool": {"extrude": {"min": [0, 20], "max": [100, 30]}, "at": [0, 0, 12], "distance": 6}}),
    );
    assert!(rel(c["volume"].as_f64().unwrap(), 90_000.0 - 6_000.0) < EPS);
    assert_eq!(c["features"][1], json!({"id": 2, "kind": "extrude"}));
    // A blind hole and a through hole.
    let h = call(
        &mut d,
        "hole",
        json!({"target": id, "at": [20, 10, 18], "dir": [0, 0, -1], "dia": 8, "depth": 12}),
    );
    let pi = std::f64::consts::PI;
    let v1 = 84_000.0 - pi * 16.0 * 12.0;
    assert!(rel(h["volume"].as_f64().unwrap(), v1) < EPS);
    let t = call(
        &mut d,
        "hole",
        json!({"target": id, "at": [80, 40, 18], "dia": 6, "depth": "through"}),
    );
    let v2 = v1 - pi * 9.0 * 18.0;
    assert!(rel(t["volume"].as_f64().unwrap(), v2) < EPS);

    // Another part as the tool, as it is placed now.
    let tool = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [10, 10]}, "at": [40, 0, 0], "distance": 5}),
    );
    let tid = tool["id"].as_str().unwrap();
    let r = call(&mut d, "cut", json!({"target": id, "tool": {"part": tid}}));
    assert!(rel(r["volume"].as_f64().unwrap(), v2 - 500.0) < EPS);
    // Moving the tool later does not move the cut.
    call(&mut d, "move3d", json!({"target": tid, "by": [500, 0, 0]}));
    let again = call(&mut d, "part", json!({"id": id}));
    assert!(rel(again["volume"].as_f64().unwrap(), v2 - 500.0) < EPS);
    // A part cannot cut itself, directly or through a tool.
    let before = state(&d);
    assert!(fails(&mut d, "cut", json!({"target": id, "tool": {"part": id}})).contains("own tool"));
    let e = fails(&mut d, "cut", json!({"target": tid, "tool": {"part": id}}));
    assert!(e.contains("uses the part itself"), "{e}");
    assert_eq!(state(&d), before);
}

#[test]
fn copies_share_the_definition_and_evaluate_once() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap().to_string();
    let solids = d.solids.get().unwrap();
    let n0 = solids.evaluations();
    let c = call(&mut d, "copy3d", json!({"target": id, "move": [0, 0, 100]}));
    assert!(c.is_object(), "one target, one copy: an object");
    assert_eq!(c["def"], p["def"]);
    assert_eq!(c["instances"], 2);
    assert!(close3(&c["bbox"]["min"], [0.0, 0.0, 100.0]));
    // A linear array: three more, 100 apart.
    let arr = call(
        &mut d,
        "copy3d",
        json!({"target": [c["id"]], "move": [0, 0, 100], "count": 3}),
    );
    assert_eq!(arr.as_array().unwrap().len(), 3);
    assert!(close3(&arr[2]["bbox"]["min"], [0.0, 0.0, 400.0]));
    let all = call(&mut d, "parts", json!({}));
    assert_eq!(all.as_array().unwrap().len(), 5);
    assert_eq!(
        d.solids.get().unwrap().evaluations(),
        n0,
        "copies reuse the evaluation"
    );

    // Editing the shared definition changes every instance.
    call(
        &mut d,
        "hole",
        json!({"target": arr[0]["id"], "at": [50, 25, 218], "dia": 10, "depth": "through"}),
    );
    let vols: Vec<f64> = call(&mut d, "parts", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["volume"].as_f64().unwrap())
        .collect();
    let want = 90_000.0 - std::f64::consts::PI * 25.0 * 18.0;
    assert!(vols.iter().all(|v| rel(*v, want) < EPS), "{vols:?}");

    // unique: a definition of its own.
    let u = call(
        &mut d,
        "copy3d",
        json!({"target": id, "move": [200, 0, 0], "unique": true, "attrs": {"copy": true}}),
    );
    assert_ne!(u["def"], p["def"]);
    assert_eq!(u["instances"], 1);
    assert_eq!(u["attrs"], json!({"role": "plate", "copy": true}));
    // A cut given in world coordinates lands on the moved copy's material.
    let cut = call(
        &mut d,
        "cut",
        json!({"target": u["id"], "tool": {"extrude": {"min": [200, 0], "max": [210, 50]}, "distance": 18}}),
    );
    assert!(rel(cut["volume"].as_f64().unwrap(), want - 9_000.0) < EPS);
    // The original is untouched by the unique copy's cut.
    let orig = call(&mut d, "part", json!({"id": id}));
    assert!(rel(orig["volume"].as_f64().unwrap(), want) < EPS);
}

#[test]
fn move_rotate_remove_and_gc() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap().to_string();
    call(&mut d, "move3d", json!({"target": id, "by": [10, 20, 30]}));
    let m = call(&mut d, "part", json!({"id": id}));
    assert!(close3(&m["bbox"]["min"], [10.0, 20.0, 30.0]));
    assert!(close3(&m["centroid"], [60.0, 45.0, 39.0]));
    // A quarter turn about Z through the origin.
    call(
        &mut d,
        "rotate3d",
        json!({"target": {"attrs": {"role": "plate"}}, "angle": 90}),
    );
    let r = call(&mut d, "part", json!({"id": id}));
    assert!(close3(&r["bbox"]["min"], [-70.0, 10.0, 30.0]), "{r}");
    assert!(close3(&r["bbox"]["max"], [-20.0, 110.0, 48.0]), "{r}");
    assert!(close3(&r["localBbox"]["max"], [100.0, 50.0, 18.0]));
    assert!(rel(r["volume"].as_f64().unwrap(), 90_000.0) < EPS);

    // A tool definition stays while a part uses it.
    let t = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [1, 1]}, "distance": 1}),
    );
    let s = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [10, 10]}, "distance": 10}),
    );
    call(
        &mut d,
        "cut",
        json!({"target": s["id"], "tool": {"part": t["id"]}}),
    );
    let gone = call(&mut d, "remove3d", json!({"target": t["id"]}));
    assert_eq!(gone["removed"], json!([t["id"]]));
    assert_eq!(gone["defsRemoved"], json!([]));
    assert!(d.part_defs.len() == 3);
    let gone = call(&mut d, "remove3d", json!({"target": s["id"]}));
    assert_eq!(gone["defsRemoved"], json!([t["def"], s["def"]]));
    assert_eq!(d.part_defs.len(), 1);
}

#[test]
fn selection_by_ids_attrs_has_def_and_bounds() {
    let mut d = doc();
    let a = plate(&mut d);
    let b = call(
        &mut d,
        "copy3d",
        json!({"target": a["id"], "move": [0, 0, 500], "attrs": {"level": 2}}),
    );
    let c = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [5, 5]}, "distance": 5, "attrs": {"role": "pin"}}),
    );
    let ids = |v: Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    };
    let (ia, ib, ic) = (
        a["id"].as_str().unwrap(),
        b["id"].as_str().unwrap(),
        c["id"].as_str().unwrap(),
    );
    assert_eq!(
        ids(call(&mut d, "parts", json!({"attrs": {"role": "plate"}}))),
        [ia, ib]
    );
    assert_eq!(ids(call(&mut d, "parts", json!({"has": ["level"]}))), [ib]);
    assert_eq!(
        ids(call(&mut d, "parts", json!({"attrs": {"level": null}}))),
        [ia, ic]
    );
    assert_eq!(
        ids(call(&mut d, "parts", json!({"def": a["def"]}))),
        [ia, ib]
    );
    assert_eq!(
        ids(call(&mut d, "parts", json!({"ids": [ic, ia]}))),
        [ic, ia]
    );
    assert_eq!(
        ids(call(
            &mut d,
            "parts",
            json!({"bounds": {"min": [0, 0, 400], "max": [10, 10, 600]}})
        )),
        [ib]
    );
    // Plan bounds (2 coordinates): every height.
    assert_eq!(
        ids(call(
            &mut d,
            "parts",
            json!({"bounds": {"min": [90, 40], "max": [95, 45]}})
        )),
        [ia, ib]
    );
    // Attributes on parts, all or nothing.
    let u = call(
        &mut d,
        "setPartAttrs",
        json!({"target": {"has": ["role"]}, "attrs": {"checked": true}}),
    );
    assert_eq!(u["updated"].as_array().unwrap().len(), 3);
    call(
        &mut d,
        "removePartAttrs",
        json!({"target": ic, "keys": ["checked"]}),
    );
    assert_eq!(
        ids(call(&mut d, "parts", json!({"has": ["checked"]}))),
        [ia, ib]
    );
    let before = state(&d);
    fails(
        &mut d,
        "setPartAttrs",
        json!({"target": ia, "attrs": {"bad key!": 1}}),
    );
    assert_eq!(state(&d), before);
    // Unknown keys and ids fail.
    assert!(fails(&mut d, "parts", json!({"layer": "x"})).contains("unknown part selector key"));
    assert!(fails(&mut d, "parts", json!({"ids": ["p_999"]})).contains("not found"));
    assert!(fails(&mut d, "parts", json!({"ids": ["e_1"]})).contains("bad part id"));
    // describe() counts parts and their attribute keys.
    let desc = call(&mut d, "describe", json!({}));
    assert_eq!(desc["parts"], 3);
    assert_eq!(desc["partDefs"], 2);
    assert_eq!(desc["partAttrs"]["role"], 3);
    assert_eq!(desc["solidKernel"], "fake (closed forms)");
}

#[test]
fn degenerate_input_fails_and_changes_nothing() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap();
    let before = state(&d);
    let cases = [
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [1, 1]}, "distance": 0}),
            "non-zero",
        ),
        (
            "extrude",
            json!({"profile": [[0, 0], [1, 0]], "distance": 1}),
            "at least 3",
        ),
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [0, 1]}, "distance": 1}),
            "non-zero width",
        ),
        (
            "extrude",
            json!({"profile": {"center": [0, 0], "radius": 0}, "distance": 1}),
            "radius",
        ),
        ("extrude", json!({"distance": 1}), "profile is required"),
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [1, 1]}}),
            "distance is required",
        ),
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [1, 1]}, "distance": 1, "plane": "ab"}),
            "unknown plane",
        ),
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [1, 1]}, "distance": 1, "plane": {"x": [1, 0, 0], "normal": [1, 1, 0]}}),
            "perpendicular",
        ),
        (
            "extrude",
            json!({"profile": "e_99", "distance": 1}),
            "not found",
        ),
        (
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [1, 1]}, "distance": 1, "attrs": {"": 1}}),
            "",
        ),
        (
            "revolve",
            json!({"profile": {"min": [1, 0], "max": [2, 1]}, "axis": [[0, 0], [0, 1]], "angle": 0}),
            "(0, 360]",
        ),
        (
            "revolve",
            json!({"profile": {"min": [1, 0], "max": [2, 1]}, "axis": [[0, 0], [0, 0]]}),
            "two different",
        ),
        (
            "loft",
            json!({"sections": [{"profile": {"min": [0, 0], "max": [1, 1]}}]}),
            "at least two",
        ),
        (
            "sweep",
            json!({"profile": {"center": [0, 0], "radius": 1}, "path": []}),
            "non-empty",
        ),
        (
            "sweep",
            json!({"profile": {"center": [0, 0], "radius": 1}, "path": [{"to": [10, 0, 0]}]}),
            "normal",
        ),
        (
            "hole",
            json!({"target": id, "at": [1, 1, 18], "dia": -1}),
            "dia",
        ),
        (
            "hole",
            json!({"target": id, "at": [1, 1, 18], "dia": 2, "depth": 0}),
            "depth",
        ),
        (
            "hole",
            json!({"target": id, "at": [1, 1, 18], "dia": 2, "dir": [0, 0, 0]}),
            "non-zero",
        ),
        ("hole", json!({"target": id, "dia": 2}), "at is required"),
        (
            "hole",
            json!({"target": "p_99", "at": [1, 1, 18], "dia": 2}),
            "not found",
        ),
        (
            "hole",
            json!({"target": {"has": ["nothing"]}, "at": [1, 1, 18], "dia": 2}),
            "no part",
        ),
        ("cut", json!({"target": id}), "tool is required"),
        (
            "cut",
            json!({"target": id, "tool": {"bend": 1}}),
            "needs one of",
        ),
        (
            "cut",
            json!({"target": id, "tool": {"part": "p_99"}}),
            "not found",
        ),
        (
            "fillet3d",
            json!({"target": id, "edges": "some", "radius": 1}),
            "edges is",
        ),
        (
            "fillet3d",
            json!({"target": id, "edges": "all", "radius": 0}),
            "radius",
        ),
        (
            "chamfer3d",
            json!({"target": id, "edges": {"cap": "end", "feature": 7}, "d": 1}),
            "no feature 7",
        ),
        ("move3d", json!({"target": id}), "by"),
        (
            "rotate3d",
            json!({"target": id, "angle": 10, "axis": [0, 0, 0]}),
            "non-zero",
        ),
        ("copy3d", json!({"target": id, "count": 0}), "count"),
        ("remove3d", json!({"target": "e_2"}), "bad part id"),
        ("partDef", json!({"def": "d_99"}), "not found"),
    ];
    for (m, a, want) in cases {
        let e = fails(&mut d, m, a.clone());
        assert!(e.contains(want), "{m} {a}: {e}");
        assert_eq!(state(&d), before, "{m} {a} changed the document");
    }
}

#[test]
fn a_refused_feature_names_part_and_feature_and_changes_nothing() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap();
    call(&mut d, "copy3d", json!({"target": id, "move": [0, 0, 50]}));
    let before = state(&d);
    // The fake kernel has no counterbores.
    let e = fails(
        &mut d,
        "hole",
        json!({"target": id, "at": [10, 10, 18], "dia": 5, "depth": 10, "cbore": [9, 3]}),
    );
    assert_eq!(
        e,
        "hole: parts p_2, p_3 (\"plate\"), feature 2 (hole): a counterbore is not supported by this kernel yet"
    );
    assert_eq!(state(&d), before);
    // A refused new part allocates nothing.
    let e = fails(
        &mut d,
        "revolve",
        json!({"profile": {"min": [1, 0], "max": [2, 1]}, "axis": [[0, 0], [0, 1]]}),
    );
    assert!(
        e.starts_with("revolve: definition d_4, feature 1 (revolve): revolve is not supported"),
        "{e}"
    );
    assert_eq!(state(&d), before);
}

#[test]
fn undo_after_a_cut_restores_the_previous_definition() {
    let mut d = doc();
    let p = plate(&mut d);
    let id = p["id"].as_str().unwrap();
    let def_before = d.part_defs.clone();
    call(&mut d, "checkpoint", json!({}));
    let c = call(
        &mut d,
        "cut",
        json!({"target": id, "tool": {"extrude": {"min": [0, 0], "max": [10, 50]}, "distance": 18}}),
    );
    assert!(rel(c["volume"].as_f64().unwrap(), 81_000.0) < EPS);
    let n = d.solids.get().unwrap().evaluations();
    call(&mut d, "undo", json!({}));
    assert_eq!(d.part_defs, def_before);
    let u = call(&mut d, "part", json!({"id": id}));
    assert_eq!(u["volume"].as_f64().unwrap(), 90_000.0);
    assert_eq!(
        d.solids.get().unwrap().evaluations(),
        n,
        "undo hits the cache"
    );
    call(&mut d, "redo", json!({}));
    let r = call(&mut d, "part", json!({"id": id}));
    assert!(rel(r["volume"].as_f64().unwrap(), 81_000.0) < EPS);
    // clear removes parts too, and undo brings them back.
    call(&mut d, "clear", json!({}));
    assert!(d.parts.is_empty() && d.part_defs.is_empty());
    call(&mut d, "undo", json!({}));
    assert_eq!(d.parts.len(), 1);
}

#[test]
fn without_a_kernel_parts_store_but_geometry_says_so() {
    let mut d = Document::new();
    let p = plate(&mut d);
    assert!(
        p.get("volume").is_none(),
        "no geometry without a kernel: {p}"
    );
    // A feature a real kernel might refuse is stored: nobody evaluates it.
    call(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [10, 10, 18], "dia": 5, "cbore": [9, 3]}),
    );
    let listed = call(&mut d, "parts", json!({"geometry": false}));
    assert_eq!(listed[0]["features"].as_array().unwrap().len(), 2);
    let e = fails(&mut d, "parts", json!({}));
    assert!(e.contains(NO_KERNEL), "{e}");
    let e = fails(
        &mut d,
        "parts",
        json!({"bounds": {"min": [0, 0], "max": [1, 1]}, "geometry": false}),
    );
    assert!(e.contains(NO_KERNEL), "{e}");
    assert_eq!(
        call(&mut d, "describe", json!({}))["solidKernel"],
        Value::Null
    );
    // The scene needs one too.
    assert!(super::scene::parts_scene(&d, &json!({}), (100, 100))
        .unwrap_err()
        .contains(NO_KERNEL));
}

#[test]
fn json_round_trip_keeps_parts_and_old_files_load() {
    let mut d = doc();
    let p = plate(&mut d);
    call(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [33.3, 12.1, 18], "dia": 4.2, "depth": 7.7}),
    );
    call(
        &mut d,
        "copy3d",
        json!({"target": p["id"], "rotate": {"angle": 33, "axis": [1, 2, 3], "about": [1, 1, 1]}}),
    );
    call(&mut d, "addLine", json!({"start": [0, 0], "end": [10, 0]}));
    let text = document_to_json(&d).unwrap();
    let mut back = document_from_json(&text).unwrap();
    assert_eq!(back.part_defs, d.part_defs);
    assert_eq!(back.parts, d.parts, "placements bit for bit");
    assert_eq!(back.next_id(), d.next_id());
    assert_eq!(document_to_json(&back).unwrap(), text);
    back.solids = fake_env();
    let a = call(&mut d, "parts", json!({}));
    let b = call(&mut back, "parts", json!({}));
    assert_eq!(a, b);
    // A file from before parts.
    let mut old: Value = serde_json::from_str(&text).unwrap();
    old.as_object_mut().unwrap().remove("parts");
    old.as_object_mut().unwrap().remove("partDefs");
    let loaded = document_from_json(&old.to_string()).unwrap();
    assert!(loaded.parts.is_empty());
    assert_eq!(loaded.entities.len(), 1);
    // A part whose definition is missing is refused.
    let mut broken: Value = serde_json::from_str(&text).unwrap();
    broken["partDefs"] = json!([]);
    assert!(document_from_json(&broken.to_string())
        .unwrap_err()
        .contains("does not hold"));
}

#[test]
fn dwg_save_ignores_parts() {
    let dir = std::env::temp_dir().join(format!("codecad-3d1a-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut flat = Document::new();
    call(
        &mut flat,
        "addLine",
        json!({"start": [0, 0], "end": [100, 0], "layer": "A"}),
    );
    call(
        &mut flat,
        "addCircle",
        json!({"center": [5, 5], "radius": 3}),
    );
    let mut with = flat.clone();
    with.solids = fake_env();
    plate(&mut with);
    let (a, b) = (dir.join("flat.dwg"), dir.join("with.dwg"));
    crate::save_dwg(&flat, a.to_str().unwrap()).unwrap();
    crate::save_dwg(&with, b.to_str().unwrap()).unwrap();
    let (la, lb) = (
        crate::load_dwg(a.to_str().unwrap()).unwrap(),
        crate::load_dwg(b.to_str().unwrap()).unwrap(),
    );
    let content = |d: &Document| {
        let mut e: Vec<String> = d
            .entities
            .iter()
            .map(|e| format!("{} {:?}", e.layer, e.shape))
            .collect();
        e.sort();
        let mut l: Vec<String> = d.layers.iter().map(|l| l.name.clone()).collect();
        l.sort();
        (e, l)
    };
    assert_eq!(content(&la), content(&lb));
    assert!(lb.parts.is_empty());
    let losses = crate::dispatch_3d::dwg_part_losses(&with);
    assert_eq!(losses["dropped"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_peers_converge_on_definitions_and_parts() {
    let a = SyncDoc::new(1);
    let mut da = doc();
    a.begin_batch(&da);
    let run = |d: &mut Document, m: &str, args: Value| {
        a.apply_mutation(d, m, &args.to_string())
            .unwrap_or_else(|e| panic!("{m}: {e}"))
            .0
    };
    // A small assembly: two sides, three shelves cut into the sides.
    let side: Value = serde_json::from_str(&run(
        &mut da,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [300, 800]}, "plane": "yz", "distance": 18, "attrs": {"part": "side"}}),
    ))
    .unwrap();
    let shelf: Value = serde_json::from_str(&run(
        &mut da,
        "extrude",
        json!({"profile": {"min": [12, 0], "max": [588, 300]}, "at": [0, 0, 200], "distance": 18, "attrs": {"part": "shelf"}}),
    ))
    .unwrap();
    run(
        &mut da,
        "copy3d",
        json!({"target": shelf["id"], "move": [0, 0, 200], "count": 2}),
    );
    for s in call(&mut da, "parts", json!({"attrs": {"part": "shelf"}}))
        .as_array()
        .unwrap()
    {
        run(
            &mut da,
            "cut",
            json!({"target": side["id"], "tool": {"part": s["id"]}}),
        );
    }
    run(
        &mut da,
        "copy3d",
        json!({"target": side["id"], "move": [582, 0, 0]}),
    );
    let update = a.end_batch(&da);
    assert!(!update.is_empty());

    let b = SyncDoc::new(2);
    b.apply_update(&update).unwrap();
    assert_eq!(a.part_blobs(), b.part_blobs(), "byte-identical part maps");
    let mut db = b.to_document();
    assert_eq!(db.part_defs, da.part_defs);
    assert_eq!(db.parts, da.parts);
    assert!(db.next_id() > db.max_used_id());
    db.solids = fake_env();
    let pa = call(&mut da, "parts", json!({}));
    let pb = call(&mut db, "parts", json!({}));
    assert_eq!(pa, pb, "same boxes and volumes on both peers");
    assert_eq!(pa.as_array().unwrap().len(), 5);
    // Each side lost three 6-deep... here the shelves run 12 into each
    // side's 18: 3 x (6 x 300 x 18) from the left side's material.
    let left = &pa[0];
    assert!(
        rel(
            left["volume"].as_f64().unwrap(),
            300.0 * 800.0 * 18.0 - 3.0 * 6.0 * 300.0 * 18.0
        ) < EPS,
        "{left}"
    );

    // Peer B moves a part and removes another; A converges.
    let (_, u1) = b
        .apply_mutation(
            &mut db,
            "move3d",
            &json!({"target": shelf["id"], "by": [0, 0, 10]}).to_string(),
        )
        .unwrap();
    let (_, u2) = b
        .apply_mutation(
            &mut db,
            "remove3d",
            &json!({"target": pa[4]["id"]}).to_string(),
        )
        .unwrap();
    a.apply_update(&u1).unwrap();
    a.apply_update(&u2).unwrap();
    assert_eq!(a.part_blobs(), b.part_blobs());
    let back = a.to_document();
    assert_eq!(back.parts, db.parts);
    assert_eq!(back.part_defs, db.part_defs);
}

#[test]
fn the_render_scene_has_one_item_per_part() {
    let mut d = doc();
    let p = plate(&mut d);
    call(
        &mut d,
        "copy3d",
        json!({"target": p["id"], "move": [0, 0, 100], "count": 2, "attrs": {"color": "#ff0000"}}),
    );
    let ps = super::scene::parts_scene(&d, &json!({"view": "front", "explode": 0.5}), (960, 640))
        .unwrap();
    assert_eq!(ps.parts, 3);
    assert_eq!(ps.scene.items.len(), 3);
    assert_eq!(ps.triangles, 36);
    // The diagonal of 100 x 50 x 218 at 640 px: a quarter pixel, rounded
    // down to a power of two.
    let diag = (100f64.powi(2) + 50f64.powi(2) + 218f64.powi(2)).sqrt();
    assert_eq!(ps.chord, super::scene::auto_chord(diag, 640));
    assert!(ps.chord <= 0.25 * diag * 1.2 / 640.0 && ps.chord > 0.125 * diag * 1.2 / 640.0);
    // Shared definitions share one mesh.
    assert!(Arc::ptr_eq(
        &ps.scene.items[0].mesh,
        &ps.scene.items[2].mesh
    ));
    // Exploded along z: the parts move apart.
    let z: Vec<f64> = ps
        .scene
        .items
        .iter()
        .map(|i| i.placement.translation.z)
        .collect();
    assert!(z[0] < 0.0 && z[2] > 200.0, "{z:?}");
    // colorBy an attribute; "#rrggbb" values are colours.
    let c =
        super::scene::parts_scene(&d, &json!({"colorBy": {"attr": "color"}}), (100, 100)).unwrap();
    let red = crate::scene3d::Material::from_srgb8(255, 0, 0);
    assert_eq!(c.scene.items[1].material, red);
    assert_eq!(
        c.scene.items[0].material,
        crate::scene3d::Material::from_srgb8(200, 200, 200)
    );
    // Options are checked.
    for bad in [
        json!({"view": "sideways"}),
        json!({"clip": {"point": [0, 0, 0], "normal": [0, 0, 0]}}),
        json!({"explode": -1}),
        json!({"colorBy": 3}),
        json!({"select": {"has": ["nope"]}}),
    ] {
        assert!(
            super::scene::parts_scene(&d, &bad, (100, 100)).is_err(),
            "{bad}"
        );
    }
    assert!(super::scene::wants_3d(&d, &json!({})).unwrap());
    call(&mut d, "addLine", json!({"start": [0, 0], "end": [1, 0]}));
    assert!(!super::scene::wants_3d(&d, &json!({})).unwrap());
    assert!(super::scene::wants_3d(&d, &json!({"view": "iso"})).unwrap());
    assert!(!super::scene::wants_3d(&d, &json!({"view": "iso", "mode": "2d"})).unwrap());
}

#[test]
fn part_def_reports_the_feature_tree() {
    let mut d = doc();
    let p = plate(&mut d);
    let e = fails(
        &mut d,
        "fillet3d",
        json!({"target": p["id"], "edges": {"cap": "end", "segs": [0]}, "radius": 2}),
    );
    assert!(e.contains("feature 2 (fillet)"), "{e}");
    call(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [5, 5, 18], "dia": 2, "depth": 3}),
    );
    let def = call(&mut d, "partDef", json!({"def": p["id"]}));
    assert_eq!(def["id"], "d_1");
    assert_eq!(def["parts"], json!(["p_2"]));
    let f = def["features"].as_array().unwrap();
    assert_eq!(f.len(), 2);
    assert!(f[1]["Hole"]["dia"] == 2.0, "{def}");
    // Without a kernel the same fillet is stored (nothing evaluates it).
    let mut bare = Document::new();
    let q = plate(&mut bare);
    let r = call(
        &mut bare,
        "fillet3d",
        json!({"target": q["id"], "edges": "all", "radius": 2, "fallback": "visual"}),
    );
    assert_eq!(r["features"][1]["kind"], "fillet");
}

/// The fake kernel, except that it cannot box a part that has a hole (or is
/// named "unboxable"): like a kernel that evaluates a feature but cannot
/// bound the edges it makes.
struct NoBoxWithHoles;

impl super::SolidKernel for NoBoxWithHoles {
    type Brep = (super::fake::FakeBrep, bool);

    fn name(&self) -> String {
        "fake, no boxes with holes".into()
    }

    fn evaluate(
        &self,
        def: &super::PartDef,
        tools: &dyn super::ToolResolver,
    ) -> Result<super::Evaluated<Self::Brep>, super::EvalError> {
        let e = FakeKernel.evaluate(def, tools)?;
        let holes = def.name == "unboxable"
            || def
                .features
                .iter()
                .any(|f| matches!(f, super::Feature::Hole { .. }));
        Ok(super::Evaluated {
            brep: (e.brep, holes),
        })
    }

    fn mass(&self, b: &Self::Brep) -> Result<super::MassProps, super::EvalError> {
        FakeKernel.mass(&b.0)
    }

    fn bbox(
        &self,
        b: &Self::Brep,
        frame: &super::Placement,
    ) -> Result<super::Aabb3, super::EvalError> {
        if b.1 {
            return Err(super::EvalError::unsupported(None, "a box round a hole"));
        }
        FakeKernel.bbox(&b.0, frame)
    }

    fn check(&self, b: &Self::Brep) -> Result<super::Validity, super::EvalError> {
        FakeKernel.check(&b.0)
    }
}

/// A call whose feature the kernel accepts but whose geometry it cannot
/// report fails as a whole, names the call, and changes nothing.
#[test]
fn a_part_whose_geometry_cannot_be_reported_is_not_kept() {
    let mut d = Document::new();
    d.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(NoBoxWithHoles)));
    let p = plate(&mut d);
    let before = state(&d);
    let e = fails(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [10, 10, 18], "dia": 4, "depth": 5}),
    );
    assert!(e.starts_with("hole: part p_2"), "{e}");
    assert!(e.contains("a box round a hole"), "{e}");
    assert_eq!(state(&d), before);
    assert_eq!(
        call(&mut d, "partDef", json!({"def": p["id"]}))["features"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // The part still answers geometry queries.
    assert!(
        rel(
            call(&mut d, "part", json!({"id": p["id"]}))["volume"]
                .as_f64()
                .unwrap(),
            90_000.0
        ) < EPS
    );

    // Copies of a part that cannot be boxed are not kept either. (Here the
    // definition is stored without the kernel, then the kernel is put back.)
    let solids = std::mem::replace(&mut d.solids, SolidEnv::none());
    call(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [10, 10, 18], "dia": 4, "depth": 5}),
    );
    d.solids = solids;
    let before = state(&d);
    for unique in [false, true] {
        let e = fails(
            &mut d,
            "copy3d",
            json!({"target": p["id"], "move": [0, 0, 50], "count": 3, "unique": unique}),
        );
        assert!(e.starts_with("copy3d: "), "{e}");
        assert_eq!(state(&d), before, "unique: {unique}");
    }
    // A new part that cannot be boxed is not created.
    let e = fails(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [5, 5]}, "distance": 5, "name": "unboxable"}),
    );
    assert!(e.starts_with("extrude: "), "{e}");
    assert_eq!(state(&d), before);
}

/// A part keeps its palette colour whichever parts a render selects, and
/// colour groups are numbered in document order.
#[test]
fn render_colours_follow_the_document_not_the_selection() {
    let mut d = doc();
    plate(&mut d);
    let b = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [10, 10]}, "distance": 5, "attrs": {"kind": "x"}}),
    );
    let c = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [20, 0], "max": [30, 10]}, "distance": 5, "attrs": {"kind": "y"}}),
    );
    let all = super::scene::parts_scene(&d, &json!({}), (100, 100)).unwrap();
    let some =
        super::scene::parts_scene(&d, &json!({"select": [c["id"], b["id"]]}), (100, 100)).unwrap();
    assert_eq!(some.scene.items.len(), 2);
    // Selected in the order c, b: each keeps its colour from the full view.
    assert_eq!(some.scene.items[0].material, all.scene.items[2].material);
    assert_eq!(some.scene.items[1].material, all.scene.items[1].material);
    let pal = |i: usize| {
        let [r, g, b] = crate::scene3d::palette_color(i);
        crate::scene3d::Material::from_srgb8(r, g, b)
    };
    assert_eq!(all.scene.items[2].material, pal(2));
    // By attribute: "y" is the second group in the document even when only
    // its part is drawn; parts without the key are grey.
    let by = json!({"colorBy": {"attr": "kind"}, "select": [c["id"]]});
    let only_c = super::scene::parts_scene(&d, &by, (100, 100)).unwrap();
    assert_eq!(only_c.scene.items[0].material, pal(1));
}

/// The four isometric corners: the eye sits above the named corner.
#[test]
fn render_views_include_the_four_isometric_corners() {
    let mut d = doc();
    plate(&mut d);
    let s = 1.0 / 3f64.sqrt();
    for (name, eye) in [
        ("iso", [s, -s, s]),
        ("iso-se", [s, -s, s]),
        ("iso-sw", [-s, -s, s]),
        ("iso-ne", [s, s, s]),
        ("iso-nw", [-s, s, s]),
    ] {
        let ps = super::scene::parts_scene(&d, &json!({ "view": name }), (100, 100)).unwrap();
        let b = ps.scene.camera.back();
        assert!(
            (b.x - eye[0]).abs() < 1e-12
                && (b.y - eye[1]).abs() < 1e-12
                && (b.z - eye[2]).abs() < 1e-12,
            "{name}: {b:?}"
        );
    }
    assert!(super::scene::parts_scene(&d, &json!({ "view": "iso-up" }), (100, 100)).is_err());
}
