//! Attributes and set-level queries through `cad_call` (rung A8).

use cadview_core::construct::Curve;
use cadview_core::setquery::curve_distance;
use cadview_core::sync::SyncDoc;
use cadview_core::*;
use kurbo::Point;
use serde_json::{json, Value};
use std::f64::consts::{PI, TAU};

fn call(doc: &mut Document, m: &str, a: Value) -> Value {
    let r = cad_call(doc, m, &a.to_string()).unwrap_or_else(|e| panic!("{m}: {e}"));
    serde_json::from_str(&r).unwrap()
}

fn call_err(doc: &mut Document, m: &str, a: Value) -> String {
    cad_call(doc, m, &a.to_string()).expect_err("expected an error")
}

fn id(v: &Value) -> String {
    v["id"].as_str().unwrap().to_string()
}

fn ids(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

fn line(doc: &mut Document, a: [f64; 2], b: [f64; 2], extra: Value) -> String {
    let mut args = json!({"start": a, "end": b});
    for (k, v) in extra.as_object().unwrap() {
        args[k] = v.clone();
    }
    id(&call(doc, "addLine", args))
}

fn pairs(v: &Value) -> Vec<(String, String)> {
    v["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["a"].as_str().unwrap().to_string(),
                p["b"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ── Attributes ─────────────────────────────────────────────────────────

#[test]
fn creation_calls_take_attributes_and_queries_report_them() {
    let mut d = Document::new();
    let l = call(
        &mut d,
        "addLine",
        json!({"start": [0, 0], "end": [10, 0], "attrs": {"net": "GND", "width": 0.2, "plated": true}}),
    );
    assert_eq!(
        l["attrs"],
        json!({"net": "GND", "plated": true, "width": 0.2})
    );
    let c = call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 1, "attrs": {"net": "VCC"}}),
    );
    let a = call(
        &mut d,
        "addArc",
        json!({"center": [0, 0], "radius": 2, "from": 0, "to": 90, "attrs": {"net": "VCC"}}),
    );
    let p = call(
        &mut d,
        "addPolyline",
        json!({"points": [[0, 0], [1, 0], [1, 1]], "closed": true, "attrs": {"room": "kitchen"}}),
    );
    let t = call(
        &mut d,
        "addText",
        json!({"text": "U1", "at": [0, 0], "attrs": {"ref": "U1"}}),
    );
    let plain = call(&mut d, "addLine", json!({"start": [0, 0], "end": [1, 1]}));
    assert!(
        plain.get("attrs").is_none(),
        "no attrs field when there are none"
    );
    assert_eq!(
        call(&mut d, "attrs", json!({"id": id(&c)})),
        json!({"net": "VCC"})
    );
    assert_eq!(call(&mut d, "attrs", json!({"id": id(&plain)})), json!({}));
    assert_eq!(
        call(&mut d, "entity", json!({"id": id(&a)}))["attrs"]["net"],
        "VCC"
    );
    assert_eq!(
        call(&mut d, "entity", json!({"id": id(&p)}))["attrs"]["room"],
        "kitchen"
    );
    assert_eq!(
        call(&mut d, "entity", json!({"id": t["id"]}))["attrs"]["ref"],
        "U1"
    );
    let desc = call(&mut d, "describe", json!({}));
    assert_eq!(
        desc["attrs"],
        json!({"net": 3, "plated": 1, "ref": 1, "room": 1, "width": 1})
    );
    // Bad attributes are refused before anything is created.
    let n = d.entities.len();
    assert!(call_err(
        &mut d,
        "addLine",
        json!({"start": [0, 0], "end": [1, 0], "attrs": {"a b": 1}})
    )
    .contains("may only contain"));
    assert!(call_err(
        &mut d,
        "addLine",
        json!({"start": [0, 0], "end": [1, 0], "attrs": {"x": [1]}})
    )
    .contains("string, number or boolean"));
    assert_eq!(d.entities.len(), n);
}

#[test]
fn set_merge_remove_and_replace() {
    let mut d = Document::new();
    let a = line(&mut d, [0.0, 0.0], [1.0, 0.0], json!({}));
    let b = line(&mut d, [0.0, 1.0], [1.0, 1.0], json!({}));
    let r = call(
        &mut d,
        "setAttrs",
        json!({"target": [a, b], "attrs": {"net": "GND", "w": 1}}),
    );
    assert_eq!(r["updated"], json!([a, b]));
    call(
        &mut d,
        "setAttrs",
        json!({"target": a, "attrs": {"w": null, "x": "y"}}),
    );
    assert_eq!(
        call(&mut d, "attrs", json!({"id": a})),
        json!({"net": "GND", "x": "y"})
    );
    call(
        &mut d,
        "setAttrs",
        json!({"target": a, "attrs": {"only": true}, "replace": true}),
    );
    assert_eq!(
        call(&mut d, "attrs", json!({"id": a})),
        json!({"only": true})
    );
    call(
        &mut d,
        "removeAttrs",
        json!({"target": b, "keys": ["w", "absent"]}),
    );
    assert_eq!(
        call(&mut d, "attrs", json!({"id": b})),
        json!({"net": "GND"})
    );
    // A selector as the target.
    let r = call(
        &mut d,
        "setAttrs",
        json!({"target": {"attrs": {"net": "GND"}}, "attrs": {"checked": true}}),
    );
    assert_eq!(r["updated"], json!([b]));
    // All or nothing: one entity over the limit changes none.
    call(
        &mut d,
        "setAttrs",
        json!({"target": a, "attrs": {"big": "x".repeat(1000)}}),
    );
    let heavy: serde_json::Map<String, Value> = (0..8)
        .map(|i| (format!("k{i}"), json!("z".repeat(1000))))
        .collect();
    let e = call_err(
        &mut d,
        "setAttrs",
        json!({"target": [b, a], "attrs": heavy}),
    );
    assert!(
        e.contains(&format!("{a}:")) && e.contains("in total"),
        "{e}"
    );
    assert_eq!(
        call(&mut d, "attrs", json!({"id": b})),
        json!({"checked": true, "net": "GND"})
    );
    // Unknown entity and bad key.
    assert!(call_err(
        &mut d,
        "setAttrs",
        json!({"target": "e_999", "attrs": {"a": 1}})
    )
    .contains("not found"));
    assert!(
        call_err(&mut d, "setAttrs", json!({"target": a, "attrs": {"": 1}})).contains("1 to 64")
    );
}

#[test]
fn selectors_filter_entities_by_attribute_layer_type_and_bounds() {
    let mut d = Document::new();
    let g1 = line(
        &mut d,
        [0.0, 0.0],
        [10.0, 0.0],
        json!({"layer": "TOP", "attrs": {"net": "GND"}}),
    );
    let g2 = line(
        &mut d,
        [0.0, 5.0],
        [10.0, 5.0],
        json!({"layer": "BOT", "attrs": {"net": "GND"}}),
    );
    let v = line(
        &mut d,
        [0.0, 9.0],
        [10.0, 9.0],
        json!({"layer": "TOP", "attrs": {"net": "VCC", "hot": true}}),
    );
    let bare = line(&mut d, [50.0, 50.0], [60.0, 50.0], json!({"layer": "TOP"}));
    let circ = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [5, 5], "radius": 1, "attrs": {"net": "GND"}}),
    ));
    let sel = |d: &mut Document, s: Value| ids(&call(d, "entities", s));
    assert_eq!(
        sel(&mut d, json!({"attrs": {"net": "GND"}})),
        vec![g1.clone(), g2.clone(), circ.clone()]
    );
    assert_eq!(
        sel(&mut d, json!({"attrs": {"net": "GND"}, "layer": "TOP"})),
        vec![g1.clone()]
    );
    assert_eq!(
        sel(
            &mut d,
            json!({"layer": ["TOP", "BOT"], "type": "line", "has": ["net"]})
        ),
        vec![g1.clone(), g2.clone(), v.clone()]
    );
    assert_eq!(
        sel(&mut d, json!({"attrs": {"net": null}})),
        vec![bare.clone()]
    );
    assert_eq!(
        sel(&mut d, json!({"attrs": {"hot": true}})),
        vec![v.clone()]
    );
    // Values are typed: the string "true" is not the boolean.
    assert!(sel(&mut d, json!({"attrs": {"hot": "true"}})).is_empty());
    assert_eq!(
        sel(
            &mut d,
            json!({"bounds": {"min": [45, 45], "max": [70, 70]}})
        ),
        vec![bare.clone()]
    );
    assert_eq!(
        sel(
            &mut d,
            json!({"ids": [v.clone(), g1.clone()], "attrs": {"net": "GND"}})
        ),
        vec![g1.clone()]
    );
    assert!(sel(&mut d, json!({"attrs": {"net": "nope"}})).is_empty());
    assert!(
        call_err(&mut d, "entities", json!({"atrs": {}})).contains("unknown selector key 'atrs'")
    );
    // Mutations take selectors.
    let moved = call(
        &mut d,
        "move",
        json!({"target": {"attrs": {"net": "VCC"}}, "dx": 1, "dy": 0}),
    );
    assert_eq!(moved["moved"], json!([v.clone()]));
    let removed = call(&mut d, "remove", json!({"target": {"layer": "BOT"}}));
    assert_eq!(removed["removed"], json!([g2]));
    assert!(call_err(
        &mut d,
        "move",
        json!({"target": {"expand": true}, "dx": 1, "dy": 0})
    )
    .contains("expand"));
}

fn footprint_doc() -> Document {
    let mut d = Document::new();
    call(
        &mut d,
        "defineBlock",
        json!({
            "name": "PADS2",
            "attrs": {"footprint": "R0603", "side": "top"},
            "shapes": [
                {"type": "circle", "center": [-1, 0], "radius": 0.4, "layer": "CU", "attrs": {"pin": "1"}},
                {"type": "circle", "center": [1, 0], "radius": 0.4, "layer": "CU", "attrs": {"pin": "2", "side": "bottom"}},
                {"type": "line", "start": [-1.5, 0.8], "end": [1.5, 0.8], "layer": "SILK"},
            ],
        }),
    );
    d
}

#[test]
fn block_attributes_layer_definition_shape_and_insert() {
    let mut d = footprint_doc();
    let r1 = call(
        &mut d,
        "place",
        json!({"block": "PADS2", "at": [10, 0], "attrs": {"ref": "R1", "side": "insert"}}),
    );
    let r2 = call(
        &mut d,
        "place",
        json!({"block": "PADS2", "at": [20, 0], "rotation": 90, "attrs": {"ref": "R2"}}),
    );
    assert_eq!(r1["attrs"], json!({"ref": "R1", "side": "insert"}));
    let kids = call(&mut d, "children", json!({"id": id(&r2)}));
    assert_eq!(kids[0]["child"], 0);
    assert_eq!(
        kids[1]["attrs"],
        json!({"footprint": "R0603", "pin": "2", "ref": "R2", "side": "bottom"})
    );
    assert_eq!(
        kids[2]["attrs"],
        json!({"footprint": "R0603", "ref": "R2", "side": "top"})
    );
    // The insert wins over the shape and the definition.
    let ex = call(
        &mut d,
        "entities",
        json!({"expand": true, "attrs": {"ref": "R1"}}),
    );
    let ex = ex.as_array().unwrap();
    assert_eq!(ex.len(), 4, "3 children and the insert itself");
    assert!(ex[..3].iter().all(|e| e["attrs"]["side"] == "insert"));
    // Pads of pin 2 across all footprints: matched child by child.
    let pin2 = call(
        &mut d,
        "entities",
        json!({"expand": true, "attrs": {"pin": "2"}}),
    );
    let pin2 = pin2.as_array().unwrap();
    assert_eq!(pin2.len(), 2);
    assert_eq!(pin2[1]["id"], r2["id"]);
    assert_eq!(pin2[1]["child"], 1);
    assert!(
        (pin2[1]["center"][1].as_f64().unwrap() - 1.0).abs() < 1e-9,
        "rotated"
    );
    // Without expand an insert matches on its own attributes only.
    assert!(ids(&call(&mut d, "entities", json!({"attrs": {"pin": "2"}}))).is_empty());
    // Read and change block attributes.
    let ba = call(&mut d, "blockAttrs", json!({"name": "PADS2"}));
    assert_eq!(ba["attrs"], json!({"footprint": "R0603", "side": "top"}));
    assert_eq!(ba["shapes"][2], json!({}));
    call(
        &mut d,
        "setBlockAttrs",
        json!({"name": "PADS2", "shape": 2, "attrs": {"silk": true}}),
    );
    call(
        &mut d,
        "setBlockAttrs",
        json!({"name": "PADS2", "attrs": {"side": null}}),
    );
    let ba = call(&mut d, "blockAttrs", json!({"name": "PADS2"}));
    assert_eq!(ba["attrs"], json!({"footprint": "R0603"}));
    assert_eq!(ba["shapes"][2], json!({"silk": true}));
    assert!(call_err(
        &mut d,
        "setBlockAttrs",
        json!({"name": "PADS2", "shape": 9, "attrs": {}})
    )
    .contains("no shape 9"));
    // clone keeps them.
    call(
        &mut d,
        "clone",
        json!({"source": "PADS2", "name": "PADS2B"}),
    );
    assert_eq!(
        call(&mut d, "blockAttrs", json!({"name": "PADS2B"}))["shapes"][2],
        json!({"silk": true})
    );
}

#[test]
fn edits_keep_attributes() {
    let mut d = Document::new();
    let a = line(
        &mut d,
        [0.0, 0.0],
        [10.0, 0.0],
        json!({"attrs": {"net": "A", "w": 1}}),
    );
    let b = line(
        &mut d,
        [10.0, 0.0],
        [10.0, 10.0],
        json!({"attrs": {"net": "A", "w": 2}}),
    );
    let copies = call(&mut d, "copy", json!({"target": a, "dx": 0, "dy": 5}));
    assert_eq!(copies[0]["attrs"], json!({"net": "A", "w": 1.0}));
    let arr = call(
        &mut d,
        "array",
        json!({"target": a, "rect": {"rows": 1, "cols": 3, "dx": 20, "dy": 0}}),
    );
    assert!(arr
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["attrs"]["net"] == "A"));
    for (m, args) in [
        ("move", json!({"target": a, "dx": 1, "dy": 1})),
        (
            "rotate",
            json!({"target": a, "center": [0, 0], "angle": 10}),
        ),
        ("mirror", json!({"target": a, "p1": [0, 0], "p2": [0, 1]})),
    ] {
        call(&mut d, m, args);
        assert_eq!(call(&mut d, "attrs", json!({"id": a}))["net"], "A", "{m}");
    }
    let off = call(&mut d, "offset", json!({"id": b, "distance": 1}));
    assert_eq!(off["attrs"]["w"], 2.0);
    let trimmed = call(
        &mut d,
        "trim",
        json!({"id": b, "cut": [10, 5], "keep": "start"}),
    );
    assert_eq!(trimmed["attrs"]["w"], 2.0);
    // A fillet arc joins two: it gets what both share.
    let mut d = Document::new();
    let a = line(
        &mut d,
        [0.0, 0.0],
        [10.0, 0.0],
        json!({"attrs": {"net": "A", "w": 1}}),
    );
    let b = line(
        &mut d,
        [0.0, 0.0],
        [0.0, 10.0],
        json!({"attrs": {"net": "A", "w": 2}}),
    );
    let f = call(&mut d, "fillet", json!({"a": a, "b": b, "radius": 2}));
    assert_eq!(f["arc"]["attrs"], json!({"net": "A"}));
    assert_eq!(f["a"]["attrs"]["w"], 1.0);
}

#[test]
fn undo_and_redo_restore_attributes() {
    let mut d = Document::new();
    let a = line(&mut d, [0.0, 0.0], [1.0, 0.0], json!({"attrs": {"v": 1}}));
    call(&mut d, "checkpoint", json!({}));
    call(&mut d, "setAttrs", json!({"target": a, "attrs": {"v": 2}}));
    call(&mut d, "undo", json!({}));
    assert_eq!(call(&mut d, "attrs", json!({"id": a})), json!({"v": 1.0}));
    call(&mut d, "redo", json!({}));
    assert_eq!(call(&mut d, "attrs", json!({"id": a})), json!({"v": 2.0}));
}

#[test]
fn attributes_sync_between_peers() {
    let server = SyncDoc::new(1);
    let mut sdoc = footprint_doc();
    let lid = line(
        &mut sdoc,
        [0.0, 0.0],
        [1.0, 0.0],
        json!({"attrs": {"net": "GND"}}),
    );
    server.populate_from_document(&sdoc);
    let client = SyncDoc::new(2);
    client
        .apply_update(&server.encode_diff(&client.state_vector()).unwrap())
        .unwrap();
    let cdoc = client.to_document();
    assert_eq!(cdoc.entities[0].attrs, sdoc.entities[0].attrs);
    assert_eq!(
        cdoc.blocks["PADS2"].shape_attrs,
        sdoc.blocks["PADS2"].shape_attrs
    );
    // Changing only attributes produces an update, for entities and blocks.
    let sv = client.state_vector();
    let (_, u1) = server
        .apply_mutation(
            &mut sdoc,
            "setAttrs",
            &json!({"target": lid, "attrs": {"net": "VCC"}}).to_string(),
        )
        .unwrap();
    assert!(!u1.is_empty());
    let (_, u2) = server
        .apply_mutation(
            &mut sdoc,
            "setBlockAttrs",
            r#"{"name":"PADS2","attrs":{"rev":"B"}}"#,
        )
        .unwrap();
    assert!(!u2.is_empty(), "a block whose attributes change is synced");
    client
        .apply_update(&server.encode_diff(&sv).unwrap())
        .unwrap();
    let cdoc = client.to_document();
    assert_eq!(
        cdoc.entities[0].attrs["net"],
        attrs::AttrValue::Str("VCC".into())
    );
    assert_eq!(
        cdoc.blocks["PADS2"].attrs["rev"],
        attrs::AttrValue::Str("B".into())
    );
}

#[test]
fn blobs_from_before_attributes_still_decode() {
    // The layout of DrawEntity and BlockDef before this rung.
    #[derive(serde::Serialize)]
    struct OldEntity {
        id: EntityId,
        layer: String,
        color: Option<Color>,
        linetype: Option<String>,
        lineweight: Option<f64>,
        transparency: u8,
        shape: Shape,
    }
    #[derive(serde::Serialize)]
    struct OldBlock {
        name: String,
        shapes: Vec<(Shape, String, Option<Color>)>,
        insert_point: Point,
        default_layer: String,
    }
    let shape = Shape::Circle(kurbo::Circle::new((1.0, 2.0), 3.0));
    let old = bincode::serialize(&OldEntity {
        id: EntityId(7),
        layer: "L".into(),
        color: None,
        linetype: None,
        lineweight: Some(0.5),
        transparency: 3,
        shape: shape.clone(),
    })
    .unwrap();
    let e = entity_from_bytes(&old).expect("old entity blob decodes");
    assert_eq!(
        (e.id, e.layer.as_str(), e.lineweight, e.transparency),
        (EntityId(7), "L", Some(0.5), 3)
    );
    assert!(e.attrs.is_empty());
    let old = bincode::serialize(&OldBlock {
        name: "B".into(),
        shapes: vec![(shape, "0".into(), None)],
        insert_point: Point::ZERO,
        default_layer: "0".into(),
    })
    .unwrap();
    let b = block_from_bytes(&old).expect("old block blob decodes");
    assert_eq!((b.name.as_str(), b.shapes.len()), ("B", 1));
    assert!(b.attrs.is_empty() && b.shape_attrs.is_empty());
}

#[test]
fn json_document_round_trips_exactly() {
    let mut d = footprint_doc();
    call(
        &mut d,
        "addLayer",
        json!({"name": "CU", "color": 1, "lineweight": 0.35}),
    );
    call(
        &mut d,
        "addLine",
        json!({"start": [0.1, 1.0 / 3.0], "end": [PI, 1e-17], "layer": "CU", "attrs": {"net": "GND", "w": 0.1 + 0.2}}),
    );
    call(
        &mut d,
        "addPolyline",
        json!({"points": [[0, 0, 0.5], [3, 0], [3, 2]], "closed": true, "attrs": {"name": "ü€ – ok"}}),
    );
    call(
        &mut d,
        "addArc",
        json!({"center": [1, 1], "radius": 2, "from": 10, "to": 350}),
    );
    call(
        &mut d,
        "place",
        json!({"block": "PADS2", "at": [5, 5], "scale": [2, -1], "attrs": {"ref": "R9"}}),
    );
    call(
        &mut d,
        "addFill",
        json!({"boundary": {"center": [0, 0], "radius": 3}, "holes": [[[-1, -1], [1, -1], [1, 1]]], "attrs": {"zone": 1}}),
    );
    let text = document_to_json(&d).unwrap();
    let back = document_from_json(&text).unwrap();
    assert_eq!(back.entities.len(), d.entities.len());
    for (x, y) in d.entities.iter().zip(&back.entities) {
        assert_eq!(
            entity_to_bytes(x),
            entity_to_bytes(y),
            "entity {} differs",
            x.id.0
        );
    }
    for (name, b) in &d.blocks {
        assert_eq!(
            block_to_bytes(b),
            block_to_bytes(&back.blocks[name]),
            "block {name}"
        );
    }
    assert_eq!(
        layer_to_bytes(&d.layers[0]),
        layer_to_bytes(&back.layers[0])
    );
    assert_eq!(back.linetypes.len(), d.linetypes.len());
    // New ids continue after the loaded ones.
    let mut back = back;
    let n = call(
        &mut back,
        "addLine",
        json!({"start": [0, 0], "end": [1, 1]}),
    );
    assert!(d.entities.iter().all(|e| format!("e_{}", e.id.0) != id(&n)));
    // A file written before entities had attributes loads with none.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    for e in v["entities"].as_array_mut().unwrap() {
        e.as_object_mut().unwrap().remove("attrs");
    }
    for b in v["blocks"].as_object_mut().unwrap().values_mut() {
        b.as_object_mut().unwrap().remove("attrs");
        b.as_object_mut().unwrap().remove("shape_attrs");
    }
    let old = document_from_json(&v.to_string()).unwrap();
    assert!(old.entities.iter().all(|e| e.attrs.is_empty()));
    assert!(document_from_json(r#"{"format":"other","version":1}"#).is_err());
    assert!(document_from_json("[]")
        .unwrap_err()
        .contains("not a document file"));
}

#[test]
fn dwg_round_trips_attributes_as_extended_data() {
    let mut d = footprint_doc();
    let long = "x".repeat(300);
    let attrs = json!({"net": "GND", "w": 0.15, "on": false, "note": "Ø 3 – ü", "long": long});
    let l = call(
        &mut d,
        "addLine",
        json!({"start": [0, 0], "end": [10, 0], "attrs": attrs}),
    );
    call(
        &mut d,
        "addArc",
        json!({"center": [0, 0], "radius": 5, "from": 0, "to": 90, "attrs": {"k": "arc"}}),
    );
    call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 1, "attrs": {"k": "circle"}}),
    );
    call(
        &mut d,
        "addPolyline",
        json!({"points": [[0, 0], [1, 0], [1, 1]], "closed": true, "attrs": {"k": "poly"}}),
    );
    call(
        &mut d,
        "addPolyline",
        json!({"points": [[0, 0, 1], [2, 0]], "attrs": {"k": "bulge"}}),
    );
    call(&mut d, "addLine", json!({"start": [0, 0], "end": [0, 3]}));
    call(
        &mut d,
        "place",
        json!({"block": "PADS2", "at": [10, 0], "attrs": {"ref": "R1"}}),
    );
    let losses = dwg_attr_losses(&d);
    assert_eq!(losses["droppedBlocks"], json!(["PADS2"]));
    assert_eq!(losses["droppedEntities"], json!([]));
    let bytes = export_dwg_bytes(&d).unwrap();
    let back = load_dwg_bytes(&bytes).unwrap();
    let mut back = back;
    let find =
        |doc: &mut Document, k: &str| ids(&call(doc, "entities", json!({"attrs": {"k": k}}))).len();
    for k in ["arc", "circle", "poly", "bulge"] {
        assert_eq!(find(&mut back, k), 1, "{k}");
    }
    let lines = call(&mut back, "entities", json!({"attrs": {"net": "GND"}}));
    assert_eq!(lines.as_array().unwrap().len(), 1);
    let mut want = attrs.clone();
    want["w"] = json!(0.15);
    assert_eq!(
        lines[0]["attrs"], want,
        "line attrs survive (was {})",
        l["id"]
    );
    assert_eq!(
        ids(&call(&mut back, "entities", json!({"has": ["k"]}))).len(),
        4
    );
    let ins = call(&mut back, "entities", json!({"attrs": {"ref": "R1"}}));
    assert_eq!(ins[0]["type"], "block_insert");
    // Shape attributes inside the block survive; the definition's own do not.
    let ba = call(&mut back, "blockAttrs", json!({"name": "PADS2"}));
    assert_eq!(ba["shapes"][0], json!({"pin": "1"}));
    assert_eq!(ba["shapes"][1], json!({"pin": "2", "side": "bottom"}));
    assert_eq!(ba["attrs"], json!({}));
    // A document without attributes registers no application.
    let mut plain = Document::new();
    plain.add_line(Point::ZERO, Point::new(1.0, 0.0), "0", None);
    let back = load_dwg_bytes(&export_dwg_bytes(&plain).unwrap()).unwrap();
    assert!(back.entities[0].attrs.is_empty());
}

#[test]
fn extended_data_encoding_round_trips() {
    let a =
        attrs::parse(&json!({"a": "plain", "b": 1.5, "c": true, "d": "é".repeat(400), "e": ""}))
            .unwrap();
    let bytes = encode_attrs_eed(&a);
    assert_eq!(decode_attrs_eed(&bytes), Some(a));
    assert_eq!(decode_attrs_eed(&[0, 3]), None, "truncated");
    assert_eq!(decode_attrs_eed(&[99]), None, "unknown code");
}

// ── Set queries ────────────────────────────────────────────────────────

#[test]
fn pairs_within_finds_close_pairs_exactly() {
    let mut d = Document::new();
    let a = line(
        &mut d,
        [0.0, 0.0],
        [10.0, 0.0],
        json!({"attrs": {"net": "A"}}),
    );
    let b = line(
        &mut d,
        [0.0, 0.15],
        [10.0, 0.15],
        json!({"attrs": {"net": "B"}}),
    );
    let a2 = line(
        &mut d,
        [10.0, 0.0],
        [20.0, 0.0],
        json!({"attrs": {"net": "A"}}),
    );
    // An arc of radius 1 around (5, 2): its lowest point is (5, 1).
    let arc = id(&call(
        &mut d,
        "addArc",
        json!({"center": [5, 2], "radius": 1, "from": 180, "to": 360, "attrs": {"net": "C"}}),
    ));
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": {"has": ["net"]}, "distance": 0.2}),
    );
    assert_eq!(
        pairs(&r),
        vec![
            (a.clone(), b.clone()),
            (a.clone(), a2.clone()),
            (b.clone(), a2.clone())
        ]
    );
    let p = &r["pairs"][0];
    assert!(near(p["distance"].as_f64().unwrap(), 0.15));
    // Touching ends are at distance 0.
    assert!(near(r["pairs"][1]["distance"].as_f64().unwrap(), 0.0));
    // differ: only pairs of different nets.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": {"has": ["net"]}, "distance": 0.2, "differ": ["net"]}),
    );
    assert_eq!(
        pairs(&r),
        vec![(a.clone(), b.clone()), (b.clone(), a2.clone())]
    );
    // The arc is exactly 0.85 from line b (arc bottom at y = 1).
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": arc, "b": {"attrs": {"net": "B"}}, "distance": 1, "attrs": true}),
    );
    let p = &r["pairs"][0];
    assert!(near(p["distance"].as_f64().unwrap(), 0.85), "{p}");
    assert!(near(p["pa"][0].as_f64().unwrap(), 5.0) && near(p["pa"][1].as_f64().unwrap(), 1.0));
    assert!(near(p["pb"][1].as_f64().unwrap(), 0.15));
    assert_eq!(p["aAttrs"], json!({"net": "C"}));
    // Strictly closer than the distance.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": arc, "b": b, "distance": 0.85}),
    );
    assert!(pairs(&r).is_empty());
    // Empty sets, a zero distance.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": {"layer": "none"}, "distance": 5}),
    );
    assert_eq!(r, json!({"pairs": [], "skipped": []}));
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": {"has": ["net"]}, "distance": 0}),
    );
    assert!(pairs(&r).is_empty());
    assert!(call_err(&mut d, "pairsWithin", json!({"a": null, "distance": -1})).contains(">= 0"));
    // limit
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": null, "distance": 5, "limit": 1}),
    );
    assert_eq!(
        (r["pairs"].as_array().unwrap().len(), &r["truncated"]),
        (1, &json!(true))
    );
}

#[test]
fn coincident_entities_and_curve_pairs() {
    let mut d = Document::new();
    let a = line(&mut d, [0.0, 0.0], [10.0, 0.0], json!({}));
    let b = line(&mut d, [0.0, 0.0], [10.0, 0.0], json!({}));
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": [a.clone(), b.clone()], "distance": 1e-6}),
    );
    assert_eq!(pairs(&r), vec![(a.clone(), b.clone())]);
    assert_eq!(r["pairs"][0]["distance"], 0.0);
    // The same entity in both sets is never paired with itself.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": a, "b": [a.clone(), b.clone()], "distance": 1}),
    );
    assert_eq!(pairs(&r), vec![(a.clone(), b.clone())]);
    // Circle and circle, concentric and apart.
    let mut d = Document::new();
    let c1 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 2}),
    ));
    let c2 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 1.5}),
    ));
    let c3 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [10, 0], "radius": 3}),
    ));
    let r = call(&mut d, "pairsWithin", json!({"a": null, "distance": 6}));
    assert_eq!(
        pairs(&r),
        vec![
            (c1.clone(), c2.clone()),
            (c1.clone(), c3.clone()),
            (c2.clone(), c3.clone())
        ]
    );
    assert!(near(r["pairs"][0]["distance"].as_f64().unwrap(), 0.5));
    assert!(near(r["pairs"][1]["distance"].as_f64().unwrap(), 5.0));
    assert!(near(r["pairs"][2]["distance"].as_f64().unwrap(), 5.5));
}

#[test]
fn skipped_shapes_are_reported_not_ignored() {
    let mut d = Document::new();
    line(&mut d, [0.0, 0.0], [1.0, 0.0], json!({}));
    let t = call(&mut d, "addText", json!({"text": "X", "at": [0, 0.1]}));
    let s = call(
        &mut d,
        "addSpline",
        json!({"points": [[0, 0], [1, 1], [2, 0], [3, 1]]}),
    );
    let r = call(&mut d, "pairsWithin", json!({"a": null, "distance": 1}));
    assert!(pairs(&r).is_empty());
    let skipped: Vec<&str> = r["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["type"].as_str().unwrap())
        .collect();
    assert_eq!(skipped, vec!["text", "spline"]);
    assert_eq!(r["skipped"][0]["id"], t["id"]);
    assert_eq!(r["skipped"][1]["id"], s["id"]);
}

#[test]
fn queries_see_inside_blocks_with_and_without_expand() {
    let mut d = footprint_doc();
    let r1 = id(&call(
        &mut d,
        "place",
        json!({"block": "PADS2", "at": [0, 0], "attrs": {"ref": "R1"}}),
    ));
    let track = line(
        &mut d,
        [-3.0, 0.5],
        [3.0, 0.5],
        json!({"layer": "CU", "attrs": {"net": "N1"}}),
    );
    // Without expand: the insert is one item; the nearest child is named.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": track, "b": r1, "distance": 0.2}),
    );
    let p = &r["pairs"][0];
    assert_eq!(pairs(&r), vec![(track.clone(), r1.clone())]);
    assert!(
        near(p["distance"].as_f64().unwrap(), 0.1),
        "pad top at y=0.4: {p}"
    );
    assert!(p["bChild"] == 0 || p["bChild"] == 1);
    // With expand: one pair per pad, silkscreen left out by layer.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": track, "b": {"expand": true, "layer": "CU"}, "distance": 0.2, "attrs": true}),
    );
    let ps = r["pairs"].as_array().unwrap();
    assert_eq!(ps.len(), 2);
    assert_eq!(
        (ps[0]["bChild"].clone(), ps[1]["bChild"].clone()),
        (json!(0), json!(1))
    );
    assert_eq!(ps[1]["bAttrs"]["pin"], "2");
    // Pads of one footprint against each other, within one set.
    let r = call(
        &mut d,
        "pairsWithin",
        json!({"a": {"expand": true, "layer": "CU", "has": ["pin"]}, "distance": 1.5}),
    );
    let ps = r["pairs"].as_array().unwrap();
    assert_eq!(ps.len(), 1);
    assert!(near(ps[0]["distance"].as_f64().unwrap(), 1.2));
    assert_eq!(
        (ps[0]["aChild"].clone(), ps[0]["bChild"].clone()),
        (json!(0), json!(1))
    );
}

#[test]
fn overlapping_reports_points_shared_stretches_and_containment() {
    let mut d = Document::new();
    let h = line(&mut d, [0.0, 0.0], [10.0, 0.0], json!({}));
    let v = line(&mut d, [5.0, -5.0], [5.0, 5.0], json!({}));
    let col = line(&mut d, [8.0, 0.0], [12.0, 0.0], json!({}));
    let far = line(&mut d, [100.0, 100.0], [101.0, 100.0], json!({}));
    let r = call(
        &mut d,
        "overlapping",
        json!({"a": [h.clone(), v.clone(), col.clone(), far.clone()]}),
    );
    assert_eq!(
        pairs(&r),
        vec![(h.clone(), v.clone()), (h.clone(), col.clone())]
    );
    assert_eq!(r["pairs"][0]["points"], json!([[5.0, 0.0]]));
    assert_eq!(r["pairs"][0]["shared"], false);
    assert_eq!(r["pairs"][1]["shared"], true);
    assert_eq!(r["pairs"][1]["points"], json!([[8.0, 0.0], [10.0, 0.0]]));
    // Circles: crossing, tangent, equal, nested.
    let mut d = Document::new();
    let c1 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 2}),
    ));
    let c2 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [3, 0], "radius": 2}),
    ));
    let c3 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [-4, 0], "radius": 2}),
    ));
    let c4 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0], "radius": 2}),
    ));
    let c5 = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [0, 0.5], "radius": 0.5}),
    ));
    let r = call(&mut d, "overlapping", json!({"a": null}));
    let got = pairs(&r);
    assert!(got.contains(&(c1.clone(), c2.clone())));
    assert!(got.contains(&(c1.clone(), c3.clone())), "tangent counts");
    assert!(
        got.contains(&(c1.clone(), c4.clone())),
        "equal circles share their outline"
    );
    assert!(
        !got.iter().any(|p| p.1 == c5),
        "outlines of nested circles do not meet"
    );
    let tangent = &r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["b"] == c3)
        .unwrap()["points"];
    assert!(near(tangent[0][0].as_f64().unwrap(), -2.0), "{tangent}");
    let r = call(
        &mut d,
        "overlapping",
        json!({"a": c1, "b": [c5.clone()], "areas": true}),
    );
    assert_eq!(r["pairs"][0]["contains"], "a");
    let r = call(
        &mut d,
        "overlapping",
        json!({"a": c5, "b": [c1.clone()], "areas": true}),
    );
    assert_eq!(r["pairs"][0]["contains"], "b");
    // An empty set.
    let r = call(&mut d, "overlapping", json!({"a": [], "b": null}));
    assert!(pairs(&r).is_empty());
}

#[test]
fn inside_rectangles_rings_and_entities() {
    let mut d = Document::new();
    let a = line(&mut d, [1.0, 1.0], [2.0, 2.0], json!({}));
    let edge = line(&mut d, [0.0, 0.0], [5.0, 0.0], json!({}));
    let out = line(&mut d, [4.0, 4.0], [12.0, 4.0], json!({}));
    let arc = id(&call(
        &mut d,
        "addArc",
        json!({"center": [5, 5], "radius": 4.9, "from": 0, "to": 90}),
    ));
    let sel = json!([a.clone(), edge.clone(), out.clone(), arc.clone()]);
    let r = call(
        &mut d,
        "inside",
        json!({"a": sel, "region": {"min": [0, 0], "max": [10, 10]}}),
    );
    let got: Vec<String> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["a"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(got, vec![a.clone(), edge.clone(), arc.clone()]);
    assert_eq!(r["pairs"][1]["touches"], true);
    let r = call(
        &mut d,
        "inside",
        json!({"a": sel, "region": {"min": [0, 0], "max": [10, 10]}, "strict": true}),
    );
    assert_eq!(r["pairs"].as_array().unwrap().len(), 2);
    // A triangle ring.
    let r = call(
        &mut d,
        "inside",
        json!({"a": sel, "region": [[0, 0], [10, 0], [0, 10]]}),
    );
    let got: Vec<&str> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["a"].as_str().unwrap())
        .collect();
    assert_eq!(got, vec![a.as_str(), edge.as_str()]);
    // Entity regions: a closed bulge polyline (a stadium) and a fill with a hole.
    let stadium = id(&call(
        &mut d,
        "addPolyline",
        json!({"points": [[10, 20, 1], [10, 24], [0, 24, 1], [0, 20]], "closed": true}),
    ));
    let inner = line(&mut d, [-1.9, 22.0], [11.9, 22.0], json!({}));
    let outside = line(&mut d, [-2.1, 22.0], [5.0, 22.0], json!({}));
    let fill = id(&call(
        &mut d,
        "addFill",
        json!({"boundary": [[20, 20], [30, 20], [30, 30], [20, 30]], "holes": [[[24, 24], [26, 24], [26, 26], [24, 26]]]}),
    ));
    let in_ring = line(&mut d, [21.0, 21.0], [29.0, 21.0], json!({}));
    let across_hole = line(&mut d, [21.0, 25.0], [29.0, 25.0], json!({}));
    let around_hole = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [25, 25], "radius": 2}),
    ));
    let r = call(
        &mut d,
        "inside",
        json!({"a": [inner.clone(), outside.clone(), in_ring.clone(), across_hole.clone(), around_hole.clone()], "region": [stadium.clone(), fill.clone()]}),
    );
    assert_eq!(
        pairs(&r),
        vec![
            (inner.clone(), stadium.clone()),
            (in_ring.clone(), fill.clone())
        ]
    );
    // An open curve is no region: reported.
    let r = call(&mut d, "inside", json!({"a": a, "region": [edge.clone()]}));
    assert!(pairs(&r).is_empty());
    assert_eq!(r["skipped"][0]["type"], "not a closed area");
}

#[test]
fn nearest_neighbours() {
    let mut d = Document::new();
    let p: Vec<String> = (0..5)
        .map(|i| {
            id(&call(
                &mut d,
                "addCircle",
                json!({"center": [i as f64 * 10.0, 0], "radius": 1}),
            ))
        })
        .collect();
    let r = call(&mut d, "nearest", json!({"a": null}));
    let ps = r["pairs"].as_array().unwrap();
    assert_eq!(ps.len(), 5);
    assert_eq!(
        (ps[0]["a"].as_str().unwrap(), ps[0]["b"].as_str().unwrap()),
        (p[0].as_str(), p[1].as_str())
    );
    assert!(near(ps[0]["distance"].as_f64().unwrap(), 8.0));
    let r = call(&mut d, "nearest", json!({"a": p[2], "b": {}, "k": 3}));
    let got: Vec<&str> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["b"].as_str().unwrap())
        .collect();
    assert_eq!(got.len(), 3);
    assert!(got[..2].contains(&p[1].as_str()) && got[..2].contains(&p[3].as_str()));
    let r = call(
        &mut d,
        "nearest",
        json!({"a": p[0], "b": [p[4].clone()], "within": 20}),
    );
    assert!(pairs(&r).is_empty(), "the only candidate is 38 away");
    let r = call(&mut d, "nearest", json!({"a": p[0], "b": []}));
    assert!(pairs(&r).is_empty());
    assert!(call_err(&mut d, "nearest", json!({"a": null, "k": 0})).contains("k"));
}

#[test]
fn near_and_in_bounds() {
    let mut d = Document::new();
    let a = line(&mut d, [0.0, 0.0], [10.0, 0.0], json!({"attrs": {"k": 1}}));
    let c = id(&call(
        &mut d,
        "addCircle",
        json!({"center": [5, 5], "radius": 1}),
    ));
    let t = call(
        &mut d,
        "addText",
        json!({"text": "T", "at": [5, 2], "height": 1}),
    );
    let r = call(&mut d, "near", json!({"point": [5, 2.5], "radius": 3}));
    let got = ids(&r);
    assert_eq!(got[0], t["id"].as_str().unwrap());
    assert_eq!(r[0]["approx"], true);
    assert!(got.contains(&a) && got.contains(&c));
    assert!(near(
        r.as_array().unwrap().iter().find(|e| e["id"] == c).unwrap()["distance"]
            .as_f64()
            .unwrap(),
        1.5
    ));
    let r = call(
        &mut d,
        "near",
        json!({"point": [5, 2.5], "radius": 3, "select": {"attrs": {"k": 1}}}),
    );
    assert_eq!(ids(&r), vec![a.clone()]);
    let r = call(
        &mut d,
        "inBounds",
        json!({"rect": {"min": [3, 3.5], "max": [7, 7]}}),
    );
    assert_eq!(ids(&r), vec![c.clone()]);
    let r = call(
        &mut d,
        "inBounds",
        json!({"rect": {"min": [-1, -1], "max": [11, 4.5]}, "mode": "contain"}),
    );
    assert!(ids(&r).contains(&a) && !ids(&r).contains(&c));
}

// ── Exactness and speed ────────────────────────────────────────────────

/// A small deterministic generator (no dependency).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn random_curves(n: usize, size: f64, seed: u64) -> Vec<Curve> {
    let mut g = Lcg(seed);
    (0..n)
        .map(|i| {
            let x = g.next() * size;
            let y = g.next() * size;
            if i % 2 == 0 {
                let a = g.next() * TAU;
                let l = 0.5 + g.next() * 2.0;
                Curve::Seg {
                    p0: Point::new(x, y),
                    p1: Point::new(x + l * a.cos(), y + l * a.sin()),
                }
            } else {
                Curve::Arc {
                    c: Point::new(x, y),
                    r: 0.3 + g.next(),
                    start: g.next() * TAU,
                    sweep: 0.2 + g.next() * 5.0,
                }
            }
        })
        .collect()
}

fn sample(c: &Curve, n: usize) -> Vec<Point> {
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            match *c {
                Curve::Seg { p0, p1 } => p0 + (p1 - p0) * t,
                Curve::Arc { c, r, start, sweep } => {
                    let a = start + sweep * t;
                    Point::new(c.x + r * a.cos(), c.y + r * a.sin())
                }
            }
        })
        .collect()
}

/// The exact distance is never more than a dense sampling finds, and the
/// sampling comes close to it (independent check of the closed form).
#[test]
fn curve_distance_agrees_with_dense_sampling() {
    let cs = random_curves(120, 6.0, 7);
    for i in 0..cs.len() {
        for j in (i + 1)..cs.len().min(i + 12) {
            let (d, pa, pb) = curve_distance(&cs[i], &cs[j]);
            assert!(
                near((pa - pb).hypot(), d),
                "the points realise the distance"
            );
            let sa = sample(&cs[i], 400);
            let sb = sample(&cs[j], 400);
            let mut best = f64::INFINITY;
            for p in &sa {
                for q in &sb {
                    best = best.min((*p - *q).hypot());
                }
            }
            assert!(d <= best + 1e-9, "{i} {j}: exact {d} > sampled {best}");
            assert!(
                best - d < 0.03,
                "{i} {j}: exact {d} far below sampled {best}"
            );
        }
    }
}

fn naive_pairs(cs: &[Curve], dist: f64) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..cs.len() {
        for j in (i + 1)..cs.len() {
            if curve_distance(&cs[i], &cs[j]).0 < dist {
                out.push((i, j));
            }
        }
    }
    out
}

fn doc_of(cs: &[Curve]) -> Document {
    let mut d = Document::new();
    for c in cs {
        match *c {
            Curve::Seg { p0, p1 } => {
                d.add_line(p0, p1, "0", None);
            }
            Curve::Arc { c, r, start, sweep } => {
                d.add_arc(c, r, start, start + sweep, "0", None);
            }
        }
    }
    d
}

fn indexed_pairs(d: &mut Document, dist: f64) -> Vec<(usize, usize)> {
    let r = call(
        d,
        "pairsWithin",
        json!({"a": null, "distance": dist, "limit": 10_000_000}),
    );
    let mut out: Vec<(usize, usize)> = r["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            let n = |k: &str| p[k].as_str().unwrap()[2..].parse::<usize>().unwrap() - 1;
            (n("a"), n("b"))
        })
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn pairs_within_equals_the_naive_loop() {
    let cs = random_curves(1500, 60.0, 11);
    let mut d = doc_of(&cs);
    assert_eq!(indexed_pairs(&mut d, 0.3), naive_pairs(&cs, 0.3));
}

/// Timings for the report: `cargo test -p cadview-core --release --test
/// attrs_setquery -- --ignored --nocapture timing`.
#[test]
#[ignore]
fn timing_pairs_within_10000_segments() {
    let cs = random_curves(10_000, 300.0, 3);
    let mut d = doc_of(&cs);
    let t = std::time::Instant::now();
    let fast = indexed_pairs(&mut d, 0.2);
    let fast_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    let slow = naive_pairs(&cs, 0.2);
    let slow_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(fast, slow);
    println!(
        "pairsWithin, 10 000 lines and arcs, d = 0.2: {} pairs; cad_call {fast_ms:.1} ms, naive loop {slow_ms:.1} ms",
        fast.len()
    );
}

#[test]
fn methods_lists_the_new_calls() {
    let mut d = Document::new();
    let m = call(&mut d, "methods", json!({}));
    let names: Vec<&str> = m
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    for n in [
        "setAttrs",
        "removeAttrs",
        "attrs",
        "setBlockAttrs",
        "blockAttrs",
        "pairsWithin",
        "overlapping",
        "inside",
        "nearest",
        "near",
        "inBounds",
    ] {
        assert!(names.contains(&n), "{n}");
        // Every listed call is dispatched (an error about arguments, never "unknown method").
        let e = cad_call(&mut d, n, "{}").err().unwrap_or_default();
        assert!(!e.contains("unknown method"), "{n}: {e}");
    }
}
