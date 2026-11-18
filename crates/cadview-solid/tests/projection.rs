//! Rung 3D3a on arris, through `cad_call`: projected views with hidden
//! lines as exact 2D entities, against coordinates computed by hand.
//!
//! Run with `-- --nocapture` to see the numbers.

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;
use std::time::Instant;

use cadview_core::document::Document;
use cadview_core::solid::eval::{KernelEvaluator, SolidEnv};
use cadview_core::types::Shape;
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

/// A 100 x 60 x 10 plate (corner at the origin) with a through hole of
/// diameter 10 at (25, 30) and a through slot of width 8 from x = 56 to
/// x = 84 at y = 30 (half circles of radius 4 about (60, 30) and (80, 30)).
fn plate(d: &mut Document) -> String {
    let p = call(
        d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [100, 60]}, "distance": 10}),
    );
    let id = p["id"].as_str().unwrap().to_string();
    call(
        d,
        "hole",
        json!({"target": id, "at": [25, 30, 10], "dia": 10}),
    );
    // The slot: two lines and two half circles (bulge 1).
    call(
        d,
        "cut",
        json!({"target": id, "tool": {"extrude": [[60, 26], [80, 26, 1], [80, 34], [60, 34, 1]],
               "at": [0, 0, -1], "distance": 12}}),
    );
    id
}

/// One shape of a view block, with its attributes.
#[derive(Debug, Clone)]
struct Got {
    shape: Shape,
    layer: String,
    edge: String,
    curve: String,
}

fn block(d: &Document, name: &str) -> Vec<Got> {
    let b = &d.blocks[name];
    b.shapes
        .iter()
        .enumerate()
        .map(|(i, (s, layer, _))| {
            let a = b.shape_attrs(i);
            let get = |k: &str| match a.get(k) {
                Some(cadview_core::attrs::AttrValue::Str(s)) => s.clone(),
                _ => String::new(),
            };
            Got {
                shape: s.clone(),
                layer: layer.clone(),
                edge: get("edge"),
                curve: get("curve"),
            }
        })
        .collect()
}

/// Lines as sorted endpoint pairs (rounded for sorting, compared exactly).
fn lines(got: &[Got], edge: &str) -> Vec<[f64; 4]> {
    let mut v: Vec<[f64; 4]> = got
        .iter()
        .filter(|g| g.edge == edge)
        .filter_map(|g| match &g.shape {
            Shape::Line(l) => {
                let (a, b) = if (l.p0.x, l.p0.y) <= (l.p1.x, l.p1.y) {
                    (l.p0, l.p1)
                } else {
                    (l.p1, l.p0)
                };
                Some([a.x, a.y, b.x, b.y])
            }
            _ => None,
        })
        .collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn assert_lines(what: &str, got: &[[f64; 4]], want: &[[f64; 4]]) -> f64 {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    let mut worst: f64 = 0.0;
    for (g, w) in got.iter().zip(want) {
        for k in 0..4 {
            worst = worst.max((g[k] - w[k]).abs());
        }
    }
    assert!(
        worst <= 1e-9,
        "{what}: off by {worst:e}: {got:?} vs {want:?}"
    );
    worst
}

fn count(got: &[Got], f: impl Fn(&Got) -> bool) -> usize {
    got.iter().filter(|g| f(g)).count()
}

#[test]
fn plate_with_hole_and_slot_in_three_views() {
    let mut d = doc();
    plate(&mut d);
    let origin = json!([0, 0, 0]);
    let mut worst: f64 = 0.0;

    // Top: x right, y up. Everything is visible: the outline, the hole's
    // circle, the slot's two lines and two half circles (the bottom rims
    // lie under the top ones and are merged into them).
    let t = Instant::now();
    let r = call(&mut d, "project", json!({"view": "top", "origin": origin}));
    let top_ms = t.elapsed().as_secs_f64() * 1e3;
    let top = block(&d, "view-top");
    worst = worst.max(assert_lines(
        "top visible",
        &lines(&top, "visible"),
        &[
            [0.0, 0.0, 0.0, 60.0],
            [0.0, 0.0, 100.0, 0.0],
            [0.0, 60.0, 100.0, 60.0],
            [60.0, 26.0, 80.0, 26.0],
            [60.0, 34.0, 80.0, 34.0],
            [100.0, 0.0, 100.0, 60.0],
        ],
    ));
    assert_eq!(count(&top, |g| g.edge == "hidden"), 0, "{top:?}");
    let circles: Vec<_> = top
        .iter()
        .filter_map(|g| match g.shape {
            Shape::Circle(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(circles.len(), 1);
    worst = worst
        .max((circles[0].center.x - 25.0).abs())
        .max((circles[0].center.y - 30.0).abs())
        .max((circles[0].radius - 5.0).abs());
    let mut arcs: Vec<(f64, f64, f64, f64, f64)> = top
        .iter()
        .filter_map(|g| match g.shape {
            Shape::Arc {
                center,
                radius,
                start_angle,
                end_angle,
            } => Some((center.x, center.y, radius, start_angle, end_angle)),
            _ => None,
        })
        .collect();
    arcs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(arcs.len(), 2, "{arcs:?}");
    // The left end runs from 90 to 270 degrees, the right from 270 to 90.
    let ang = |a: f64| a.rem_euclid(2.0 * PI);
    for (got, want) in arcs.iter().zip([
        (60.0, 30.0, 4.0, FRAC_PI_2, 3.0 * FRAC_PI_2),
        (80.0, 30.0, 4.0, 3.0 * FRAC_PI_2, FRAC_PI_2),
    ]) {
        worst = worst
            .max((got.0 - want.0).abs())
            .max((got.1 - want.1).abs())
            .max((got.2 - want.2).abs())
            .max(4.0 * (ang(got.3) - ang(want.3)).abs())
            .max(4.0 * (ang(got.4) - ang(want.4)).abs());
    }
    assert!(worst <= 1e-9, "top: {worst:e} {arcs:?}");
    assert_eq!(top.len(), 9, "{top:?}");
    assert_eq!(r["entities"], 9);
    assert!(top.iter().all(|g| g.layer == "visible"));

    // Front: x right, z up. The outline is visible; the hole's two
    // silhouette lines (x = 20, 30) and the slot's ends (x = 56, 84) are
    // hidden, from z = 0 to 10.
    let t = Instant::now();
    call(
        &mut d,
        "project",
        json!({"view": "front", "origin": origin}),
    );
    let front_ms = t.elapsed().as_secs_f64() * 1e3;
    let front = block(&d, "view-front");
    worst = worst.max(assert_lines(
        "front visible",
        &lines(&front, "visible"),
        &[
            [0.0, 0.0, 0.0, 10.0],
            [0.0, 0.0, 100.0, 0.0],
            [0.0, 10.0, 100.0, 10.0],
            [100.0, 0.0, 100.0, 10.0],
        ],
    ));
    worst = worst.max(assert_lines(
        "front hidden",
        &lines(&front, "hidden"),
        &[
            [20.0, 0.0, 20.0, 10.0],
            [30.0, 0.0, 30.0, 10.0],
            [56.0, 0.0, 56.0, 10.0],
            [84.0, 0.0, 84.0, 10.0],
        ],
    ));
    assert_eq!(front.len(), 8, "{front:?}");
    assert!(front
        .iter()
        .filter(|g| g.edge == "hidden")
        .all(|g| g.layer == "hidden" && g.curve == "silhouette"));

    // Right side: y right, z up. Hidden: the hole (y = 25, 35) and the
    // slot (y = 26, 34).
    let t = Instant::now();
    call(
        &mut d,
        "project",
        json!({"view": "right", "origin": origin}),
    );
    let side_ms = t.elapsed().as_secs_f64() * 1e3;
    let side = block(&d, "view-right");
    worst = worst.max(assert_lines(
        "side visible",
        &lines(&side, "visible"),
        &[
            [0.0, 0.0, 0.0, 10.0],
            [0.0, 0.0, 60.0, 0.0],
            [0.0, 10.0, 60.0, 10.0],
            [60.0, 0.0, 60.0, 10.0],
        ],
    ));
    worst = worst.max(assert_lines(
        "side hidden",
        &lines(&side, "hidden"),
        &[
            [25.0, 0.0, 25.0, 10.0],
            [26.0, 0.0, 26.0, 10.0],
            [34.0, 0.0, 34.0, 10.0],
            [35.0, 0.0, 35.0, 10.0],
        ],
    ));
    assert_eq!(side.len(), 8, "{side:?}");

    // Hidden lines off: the side view keeps only its outline.
    call(
        &mut d,
        "project",
        json!({"view": "right", "origin": origin, "hidden": false, "name": "side-plain"}),
    );
    let plain = block(&d, "side-plain");
    assert_eq!(plain.len(), 4, "{plain:?}");
    assert_eq!(count(&plain, |g| g.edge == "hidden"), 0);
    println!(
        "plate views: worst coordinate error {worst:e} mm; top {top_ms:.1} ms, front \
         {front_ms:.1} ms, side {side_ms:.1} ms"
    );
}

/// An isometric view of a 10 mm hole in a 40 x 40 x 10 plate: the top
/// rim is one ellipse. Its major semi-axis is the radius, 5; its minor
/// semi-axis is 5 |n . v| for the rim's normal n = z and the iso view
/// direction v = (1, -1, 1) / sqrt 3, so 5 / sqrt 3 = 5 sin(35.264 deg) =
/// 2.886751... (35.264 degrees is the angle between the line of sight and
/// the plate).
#[test]
fn iso_view_of_a_hole_is_one_exact_ellipse() {
    let mut d = doc();
    let p = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [0, 0], "max": [40, 40]}, "distance": 10}),
    );
    call(
        &mut d,
        "hole",
        json!({"target": p["id"], "at": [20, 20, 10], "dia": 10}),
    );
    let r = call(
        &mut d,
        "project",
        json!({"view": "iso", "origin": [20, 20, 10], "hidden": false}),
    );
    let got = block(&d, "view-iso");
    let ellipses: Vec<_> = got
        .iter()
        .filter_map(|g| match g.shape {
            Shape::Ellipse {
                center,
                major_axis,
                minor_ratio,
                start_param,
                end_param,
            } => Some((center, major_axis, minor_ratio, start_param, end_param)),
            _ => None,
        })
        .collect();
    assert_eq!(ellipses.len(), 1, "{got:?}");
    let (c, m, ratio, s, e) = ellipses[0];
    let a = m.0.hypot(m.1);
    let b = a * ratio;
    let want_b = 5.0 / 3f64.sqrt();
    println!(
        "iso hole: one ellipse, centre ({:e}, {:e}), semi-axes {a} and {b} (closed forms 5 and \
         {want_b}), params {s}..{e}; {} entities: {}",
        c.x, c.y, r["entities"], r["counts"]
    );
    assert!(c.x.abs() <= 1e-9 && c.y.abs() <= 1e-9);
    assert!((a - 5.0).abs() <= 1e-9, "{a}");
    assert!((b - want_b).abs() <= 1e-9, "{b}");
    assert!((e - s - 2.0 * PI).abs() <= 1e-12, "{s} {e}");
    // The major axis is horizontal on the sheet, as for any horizontal
    // circle in an isometric view.
    assert!(m.1.abs() <= 1e-9, "{m:?}");
    // The outline: four top edges, three vertical edges, two bottom edges.
    assert_eq!(count(&got, |g| matches!(g.shape, Shape::Line(_))), 9);
    assert_eq!(got.len(), 10);
}

/// A cylinder of radius 7 and height 30 standing on the origin, seen from
/// the front: its two silhouettes are the exact tangent lines x = -7 and
/// x = 7 from z = 0 to 30, and its rims are seen edge on as the lines
/// z = 0 and z = 30. Lying along x and seen from above, the same.
#[test]
fn cylinder_silhouettes_are_the_tangent_lines() {
    let mut d = doc();
    let p = call(
        &mut d,
        "extrude",
        json!({"profile": {"center": [0, 0], "radius": 7}, "distance": 30}),
    );
    call(
        &mut d,
        "project",
        json!({"view": "front", "origin": [0, 0, 0]}),
    );
    let got = block(&d, "view-front");
    let sil: Vec<_> = got
        .iter()
        .filter(|g| g.curve == "silhouette")
        .cloned()
        .collect();
    assert_lines(
        "silhouettes",
        &lines(&sil, "visible"),
        &[[-7.0, 0.0, -7.0, 30.0], [7.0, 0.0, 7.0, 30.0]],
    );
    assert_lines(
        "front view",
        &lines(&got, "visible"),
        &[
            [-7.0, 0.0, -7.0, 30.0],
            [-7.0, 0.0, 7.0, 0.0],
            [-7.0, 30.0, 7.0, 30.0],
            [7.0, 0.0, 7.0, 30.0],
        ],
    );
    assert_eq!(got.len(), 4, "{got:?}");
    // Turned to lie along +x and seen from above: the tangent lines
    // y = -7 and y = 7 from x = 0 to 30.
    call(
        &mut d,
        "rotate3d",
        json!({"target": p["id"], "angle": 90, "axis": [0, 1, 0], "about": [0, 0, 0]}),
    );
    call(
        &mut d,
        "project",
        json!({"view": "top", "origin": [0, 0, 0]}),
    );
    let got = block(&d, "view-top");
    let worst = assert_lines(
        "lying cylinder from above",
        &lines(&got, "visible"),
        &[
            [0.0, -7.0, 0.0, 7.0],
            [0.0, -7.0, 30.0, -7.0],
            [0.0, 7.0, 30.0, 7.0],
            [30.0, -7.0, 30.0, 7.0],
        ],
    );
    println!("cylinder silhouettes: exact in the front view; rotated part off by {worst:e} mm");
}

/// The visibility oracle against exact ray hits on every corpus part: in
/// six views, every classified piece is sampled at 5 %, 50 % and 95 % of
/// its parameter range (next to its split points), and the exact ray from
/// each sample toward the eye must agree with the class the piece got.
#[test]
fn the_oracle_agrees_with_exact_rays_on_the_corpus() {
    use cadview_core::solid::corpus::{curved, prismatic};
    use cadview_core::solid::Part;
    let entries: Vec<_> = prismatic::entries()
        .into_iter()
        .chain(curved::entries())
        .collect();
    let views = ["front", "top", "right", "iso", "iso-nw", "bottom"];
    let (mut samples, mut disagree, mut parts, mut pieces, mut unanswered) = (0, 0, 0, 0, 0);
    let mut groups = std::collections::BTreeMap::<String, usize>::new();
    // Pieces shorter than the merge tolerance take a neighbour's class
    // without a question; they are counted, not checked.
    let mut slivers = 0;
    let t = Instant::now();
    for entry in &entries {
        for cp in &entry.parts {
            let mut d = doc();
            for tool in &entry.tools {
                d.part_defs.insert(tool.id, tool.clone());
            }
            d.part_defs.insert(cp.def.id, cp.def.clone());
            d.parts.push(Part {
                id: cadview_core::types::EntityId(10_000),
                def: cp.def.id,
                placement: cadview_core::solid::Placement::IDENTITY,
                attrs: Default::default(),
            });
            d.set_next_id(10_001);
            parts += 1;
            for v in views {
                let mut pr =
                    cadview_core::solid::project::compute(&d, &json!({"view": v, "trace": true}))
                        .unwrap_or_else(|e| panic!("{} {v}: {e}", entry.name));
                for tr in &pr.traced {
                    if !tr.asked {
                        slivers += 1;
                        continue;
                    }
                    pieces += 1;
                    for f in [0.05, 0.5, 0.95] {
                        let u = tr.t[0] + (tr.t[1] - tr.t[0]) * f;
                        let p = tr.curve.point(u);
                        samples += 1;
                        match pr.oracle.exact_answer(p) {
                            Some(h) if h != tr.hidden => {
                                disagree += 1;
                                if std::env::var("PROJECTION_DEBUG").is_ok() {
                                    let m = pr.oracle.mesh_answer(p);
                                    println!(
                                        "    {} {v} {:?} t {:?} at {f}: p {p:?} mesh {m:?}",
                                        entry.name, tr.curve, tr.t
                                    );
                                }
                                *groups
                                    .entry(format!(
                                        "{} / {v} / {} {:?} exact hidden={h}",
                                        entry.name,
                                        tr.curve.kind(),
                                        tr.kind
                                    ))
                                    .or_insert(0) += 1;
                            }
                            Some(_) => {}
                            None => unanswered += 1,
                        }
                    }
                }
            }
        }
    }
    for (g, n) in &groups {
        println!("  disagree: {g}: {n} samples");
    }
    println!(
        "oracle vs exact rays: {parts} corpus parts, {} views, {pieces} pieces ({slivers} \
         slivers not asked), {samples} samples, {disagree} disagree, {unanswered} the kernel \
         could not answer ({:.1} s)",
        views.len(),
        t.elapsed().as_secs_f64()
    );
    assert!(samples > 1000, "{samples}");
    assert_eq!(disagree, 0);
    assert_eq!(unanswered, 0);
}

/// The same projection twice, and in a fresh document, gives the same
/// block bytes; re-projecting keeps the one insert and moves it; and
/// `viewPoints` maps 3D points as the view draws them.
#[test]
fn projection_is_deterministic() {
    let bytes = |d: &Document, name: &str| cadview_core::document::block_to_bytes(&d.blocks[name]);
    let mut a = doc();
    plate(&mut a);
    call(&mut a, "project", json!({"view": "iso-sw"}));
    let first = bytes(&a, "view-iso-sw");
    call(&mut a, "project", json!({"view": "iso-sw"}));
    assert_eq!(first, bytes(&a, "view-iso-sw"));
    let mut b = doc();
    plate(&mut b);
    call(&mut b, "project", json!({"view": "iso-sw"}));
    assert_eq!(first, bytes(&b, "view-iso-sw"));
    let inserts = |d: &Document| {
        d.entities
            .iter()
            .filter(|e| {
                matches!(&e.shape, Shape::BlockInsert { block_name, .. }
                    if block_name == "view-iso-sw")
            })
            .count()
    };
    assert_eq!(inserts(&a), 1);
    let r = call(
        &mut a,
        "project",
        json!({"view": "iso-sw", "at": [500, 0], "scale": 0.5}),
    );
    assert_eq!(inserts(&a), 1);
    let ins = a
        .entities
        .iter()
        .find(|e| format!("e_{}", e.id.0) == r["insert"].as_str().unwrap())
        .unwrap();
    assert!(
        matches!(ins.shape, Shape::BlockInsert { position, x_scale, .. }
        if position.x == 500.0 && x_scale == 0.5)
    );
    let pts = call(
        &mut a,
        "viewPoints",
        json!({"view": "front", "origin": [0, 0, 0], "at": [10, 20], "scale": 0.5,
               "points": [[100, 60, 10], [25, 0, 5]]}),
    );
    assert_eq!(pts, json!([[60.0, 25.0], [22.5, 22.5]]));
}

/// The pillow block of examples/3d/12-projected-views.js, its four views
/// timed (release: `cargo test -p cadview-solid --release --test
/// projection pillow -- --nocapture`).
#[test]
fn pillow_block_views() {
    let mut d = doc();
    let p = call(
        &mut d,
        "extrude",
        json!({"profile": {"min": [-70, -30], "max": [70, 30]}, "distance": 14}),
    );
    let id = p["id"].clone();
    call(
        &mut d,
        "fillet3d",
        json!({"target": id, "edges": {"near": [[-70, -30, 7], [70, -30, 7], [70, 30, 7], [-70, 30, 7]]}, "radius": 8}),
    );
    call(
        &mut d,
        "union",
        json!({"target": id, "tool": {"extrude": [[-35, 14], [35, 14], [35, 50, 1], [-35, 50]],
               "plane": "xz", "at": [0, 20, 0], "distance": 40}}),
    );
    for x in [-50, 50] {
        call(
            &mut d,
            "hole",
            json!({"target": id, "at": [x, 0, 14], "dia": 11, "cbore": [18, 6]}),
        );
    }
    call(
        &mut d,
        "hole",
        json!({"target": id, "at": [0, -20, 50], "dir": [0, 1, 0], "dia": 32}),
    );
    call(
        &mut d,
        "hole",
        json!({"target": id, "at": [8, 0, 90], "dia": 6, "depth": 30}),
    );
    call(
        &mut d,
        "chamfer3d",
        json!({"target": id, "edges": {"near": [[0, -20, 66], [0, 20, 66]]}, "d": 2}),
    );
    {
        let mut pr = cadview_core::solid::project::compute(
            &d,
            &json!({"view": "front", "silhouettes": false, "hidden": false, "exact": false}),
        )
        .unwrap();
        // One exact ray each (what the oracle's last resort costs): the
        // front of the upright is seen, its back and the far bolt hole's
        // wall are not.
        for (p, want) in [
            ([0.0, -30.0, 30.0], false),
            ([0.0, 30.0, 30.0], true),
            ([60.0, 0.0, 7.0], true),
        ] {
            let t = Instant::now();
            let h = pr
                .oracle
                .exact_answer(cadview_core::solid::Point3::new(p[0], p[1], p[2]));
            println!(
                "exact ray at {p:?}: {h:?} in {:.1} ms",
                t.elapsed().as_secs_f64() * 1e3
            );
            assert_eq!(h, Some(want), "{p:?}");
        }
    }
    for v in ["front", "top", "left", "iso"] {
        let t = Instant::now();
        let r = call(&mut d, "project", json!({"view": v}));
        println!(
            "pillow block {v}: {:.1} ms, {} entities {} stats {} oracle {} ms {} open {}",
            t.elapsed().as_secs_f64() * 1e3,
            r["entities"],
            r["counts"],
            r["stats"],
            r["oracle"],
            r["ms"],
            r["open"]
        );
        assert_eq!(r["open"], json!([]), "{v}");
        assert_eq!(r["oracle"]["guessed"], 0, "{v}");
    }
}
