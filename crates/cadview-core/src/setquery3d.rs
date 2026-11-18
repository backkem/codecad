//! Set queries between solid parts (ladder rung 3D1b): the 3D twins of the
//! 2D set queries in `setquery.rs`, with the same names plus the `3d` that
//! every part call with a 2D twin carries (`move3d`, `copy3d`, ...).
//!
//! - `overlapping3d(a, b?)`: every pair of parts that share volume, with
//!   the shared volume (interference).
//! - `pairsWithin3d(a, b?, distance)`: every pair closer than a distance,
//!   with the distance and the closest points (clearance).
//! - `inside3d(a, region)`: the parts of `a` inside a box, or inside the
//!   solid of each part of a selection.
//! - `distance3d(a, b)`: the distance and closest points of two parts.
//!
//! One call answers for whole selections (the part selector) and returns
//! every pair at once. A broad phase on the parts' world boxes picks the
//! candidate pairs (sweep and prune along x); only those reach the kernel
//! (`solid::eval::Evaluator::distances` and `interferences`), which answers
//! on the exact solids. A pair the kernel cannot answer is listed in
//! `skipped` with the reason, so a rule never passes silently on a pair it
//! did not see. Nothing here knows what a part is for.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::attrs::{self, Attrs};
use crate::dispatch_3d::{num, p3_json, parse, part_id_of, refusal, select_parts_value};
use crate::document::Document;
use crate::solid::eval::{Solid, Solids};
use crate::solid::kernel::{Aabb3, Closest, EvalError, Interference};
use crate::solid::part::part_key;
use crate::solid::types::Placement;
use crate::types::EntityId;

type R = Result<String, String>;

/// Most pairs returned unless `limit` says otherwise (as in 2D).
const DEFAULT_LIMIT: usize = 100_000;

/// Shared solids thinner than this (mm) count as touching, not overlap,
/// unless `tolerance` says otherwise.
pub const DEFAULT_OVERLAP_TOL: f64 = 1e-6;

/// The set-query methods for parts. `None` for any other method.
pub fn dispatch(doc: &mut Document, method: &str, args: &str) -> Option<R> {
    let r = match method {
        "overlapping3d" => overlapping3d(doc, args),
        "pairsWithin3d" => pairs_within3d(doc, args),
        "inside3d" => inside3d(doc, args),
        "distance3d" => distance3d(doc, args),
        _ => return None,
    };
    Some(r.map_err(|e| format!("{method}: {e}")))
}

/// One part of a query: its solid, placement and world box.
struct Item<'a> {
    id: EntityId,
    attrs: &'a Attrs,
    solid: Arc<dyn Solid>,
    placement: Placement,
    bbox: Aabb3,
}

fn items<'a>(doc: &'a Document, solids: &Solids, sel: &Value) -> Result<Vec<Item<'a>>, String> {
    let ids = select_parts_value(doc, sel)?;
    ids.into_iter()
        .map(|id| {
            let p = doc.part(id).expect("selected");
            let def = doc
                .part_defs
                .get(&p.def)
                .ok_or_else(|| format!("part {}: its definition is missing", part_key(id)))?;
            let who = format!("part {}", part_key(id));
            let solid = solids
                .solid(&doc.part_defs, def)
                .map_err(|e| refusal(def, &who, &e))?;
            let bbox = solids
                .tight_bbox(&doc.part_defs, def, &p.placement)
                .map_err(|e| refusal(def, &who, &e))?
                .aabb;
            Ok(Item {
                id,
                attrs: &p.attrs,
                solid,
                placement: p.placement,
                bbox,
            })
        })
        .collect()
}

/// Options every pair query takes, as in 2D.
struct PairOpts {
    differ: Vec<String>,
    attrs: bool,
    limit: usize,
}

fn pair_opts(o: &Value) -> Result<PairOpts, String> {
    let differ = match o.get("differ") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|k| {
                k.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "differ is an array of attribute keys".to_string())
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("differ is an array of attribute keys".into()),
    };
    let limit = match o.get("limit") {
        None | Some(Value::Null) => DEFAULT_LIMIT,
        Some(v) => v.as_u64().ok_or("limit is a whole number")? as usize,
    };
    Ok(PairOpts {
        differ,
        attrs: o.get("attrs").and_then(Value::as_bool).unwrap_or(false),
        limit,
    })
}

/// The values of all `keys` are equal on both (a missing key counts as a
/// value), so `differ` leaves the pair out.
fn same_values(a: &Attrs, b: &Attrs, keys: &[String]) -> bool {
    keys.iter().all(|k| a.get(k) == b.get(k))
}

/// The two sets of a pair query: `b` absent or null means pairs within
/// `a`. Returns the items and, for two sets, the second.
fn two_sets<'a>(
    doc: &'a Document,
    solids: &Solids,
    o: &Value,
) -> Result<(Vec<Item<'a>>, Option<Vec<Item<'a>>>), String> {
    let a = items(doc, solids, o.get("a").unwrap_or(&Value::Null))?;
    let b = match o.get("b") {
        None | Some(Value::Null) => None,
        Some(v) => Some(items(doc, solids, v)?),
    };
    Ok((a, b))
}

/// Candidate pairs `(i in a, j in b)` whose boxes, grown by `reach`, pass
/// `test`; within one set each pair once (`i < j`); never a part with
/// itself. Sweep and prune: `b` sorted by the low x of its boxes. Sorted by
/// `(i, j)`.
fn candidates(
    ga: &[Item],
    gb: &[Item],
    same_set: bool,
    reach: f64,
    test: impl Fn(&Aabb3, &Aabb3) -> bool,
) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..gb.len()).collect();
    order.sort_by(|&x, &y| gb[x].bbox.min.x.total_cmp(&gb[y].bbox.min.x));
    let mut out = Vec::new();
    for (i, a) in ga.iter().enumerate() {
        let grown = a.bbox.inflate(reach);
        // The first box of b that could reach: its low x at most our high x.
        let end = order.partition_point(|&j| gb[j].bbox.min.x <= grown.max.x);
        for &j in &order[..end] {
            if gb[j].bbox.max.x < grown.min.x {
                continue;
            }
            if same_set && j <= i {
                continue;
            }
            if gb[j].id == a.id {
                continue;
            }
            if test(&grown, &gb[j].bbox) {
                out.push((i, j));
            }
        }
    }
    out.sort_unstable();
    out
}

fn head(a: &Item, b: &Item, opts: &PairOpts) -> Value {
    let mut v = json!({ "a": part_key(a.id), "b": part_key(b.id) });
    if opts.attrs {
        v["aAttrs"] = attrs::to_json(a.attrs);
        v["bAttrs"] = attrs::to_json(b.attrs);
    }
    v
}

fn skipped_pair(a: &Item, b: &Item, e: &EvalError) -> Value {
    json!({ "a": part_key(a.id), "b": part_key(b.id), "reason": e.to_string() })
}

/// Placed solids for the kernel, and pairs of indices into them.
type KernelPairs<'s> = (Vec<(&'s dyn Solid, Placement)>, Vec<(usize, usize)>);

/// The solids of `pairs`, for the kernel: the items of both sets in one
/// list, and the pairs as indices into it.
fn kernel_pairs<'s>(ga: &'s [Item], gb: &'s [Item], pairs: &[(usize, usize)]) -> KernelPairs<'s> {
    let mut solids: Vec<(&dyn Solid, Placement)> = Vec::new();
    let mut index_a = BTreeMap::new();
    let mut index_b = BTreeMap::new();
    let mut out = Vec::with_capacity(pairs.len());
    for &(i, j) in pairs {
        let ka = *index_a.entry(i).or_insert_with(|| {
            solids.push((&*ga[i].solid, ga[i].placement));
            solids.len() - 1
        });
        let kb = *index_b.entry(j).or_insert_with(|| {
            solids.push((&*gb[j].solid, gb[j].placement));
            solids.len() - 1
        });
        out.push((ka, kb));
    }
    (solids, out)
}

fn finish(pairs: Vec<Value>, skipped: Vec<Value>, limit: usize) -> R {
    let truncated = pairs.len() > limit;
    let pairs: Vec<Value> = pairs.into_iter().take(limit).collect();
    let mut out = json!({ "pairs": pairs, "skipped": skipped });
    if truncated {
        out["truncated"] = json!(true);
    }
    Ok(out.to_string())
}

// ── overlapping3d ──────────────────────────────────────────────────────

/// `{a, b?, tolerance?, touching?, differ?, attrs?, limit?}`: pairs that
/// share volume. The overlap's depth is its volume over the largest face of
/// the box both parts' boxes share ([`depth`]); one at most `tolerance`
/// (mm, default [`DEFAULT_OVERLAP_TOL`]) is a touch. `touching: true` also
/// lists pairs that touch (volume 0).
fn overlapping3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let solids = doc.solids.get()?;
    let opts = pair_opts(&o)?;
    let tolerance = num(&o, "tolerance")?.unwrap_or(DEFAULT_OVERLAP_TOL);
    if tolerance < 0.0 {
        return Err("tolerance must be >= 0".into());
    }
    let touching = o.get("touching").and_then(Value::as_bool).unwrap_or(false);
    let (ga, gb) = two_sets(doc, solids, &o)?;
    let same_set = gb.is_none();
    let gb: &[Item] = gb.as_deref().unwrap_or(&ga);
    // Parts that share volume have boxes whose interiors meet; touching
    // parts have boxes that meet (grown by the kernel's tolerance).
    let mut pairs = if touching {
        candidates(&ga, gb, same_set, 1e-6, |x, y| x.meets(y))
    } else {
        candidates(&ga, gb, same_set, 0.0, |x, y| x.overlaps(y))
    };
    if !opts.differ.is_empty() {
        pairs.retain(|&(i, j)| !same_values(ga[i].attrs, gb[j].attrs, &opts.differ));
    }
    let (list, kp) = kernel_pairs(&ga, gb, &pairs);
    let verdicts = solids
        .evaluator()
        .interferences(&list, &kp)
        .map_err(|e| e.to_string())?;
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for (&(i, j), verdict) in pairs.iter().zip(verdicts) {
        let (a, b) = (&ga[i], &gb[j]);
        match verdict {
            Ok(Interference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
            }) => {
                let depth = depth(volume, &a.bbox, &b.bbox);
                if depth > tolerance {
                    let mut v = head(a, b, &opts);
                    v["volume"] = json!(volume);
                    v["depth"] = json!(depth);
                    v["aInB"] = json!(a_in_b);
                    v["bInA"] = json!(b_in_a);
                    out.push(v);
                } else if touching {
                    let mut v = head(a, b, &opts);
                    v["volume"] = json!(0.0);
                    v["touching"] = json!(true);
                    out.push(v);
                }
            }
            Ok(Interference::Touching { at }) => {
                if touching {
                    let mut v = head(a, b, &opts);
                    v["volume"] = json!(0.0);
                    v["touching"] = json!(true);
                    v["at"] = p3_json(at);
                    out.push(v);
                }
            }
            Ok(Interference::Apart(_)) => {}
            Err(e) => skipped.push(skipped_pair(a, b, &e)),
        }
    }
    finish(out, skipped, opts.limit)
}

/// How deep an overlap of `volume` reaches: the volume over the largest
/// face of the box both parts' boxes share. The shared solid lies in that
/// box, so this never exceeds the box's smallest side; for two box-shaped
/// parts it is their penetration depth exactly. A sliver left by rounding
/// in a rotated placement comes out about as thin as it is.
fn depth(volume: f64, a: &Aabb3, b: &Aabb3) -> f64 {
    let e = [
        a.max.x.min(b.max.x) - a.min.x.max(b.min.x),
        a.max.y.min(b.max.y) - a.min.y.max(b.min.y),
        a.max.z.min(b.max.z) - a.min.z.max(b.min.z),
    ]
    .map(|v| v.max(0.0));
    let face = (e[0] * e[1]).max(e[1] * e[2]).max(e[0] * e[2]);
    if face > 0.0 {
        volume / face
    } else {
        0.0
    }
}

// ── pairsWithin3d ──────────────────────────────────────────────────────

/// `{a, b?, distance, differ?, attrs?, limit?}`: pairs strictly closer than
/// `distance`, with the distance and the closest points. Parts that touch
/// or overlap are at distance 0.
fn pairs_within3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let solids = doc.solids.get()?;
    let opts = pair_opts(&o)?;
    let d = num(&o, "distance")?.ok_or("distance is required")?;
    if d < 0.0 {
        return Err("distance must be a finite number >= 0".into());
    }
    let (ga, gb) = two_sets(doc, solids, &o)?;
    let same_set = gb.is_none();
    let gb: &[Item] = gb.as_deref().unwrap_or(&ga);
    let mut pairs = candidates(&ga, gb, same_set, d, |x, y| x.meets(y));
    if !opts.differ.is_empty() {
        pairs.retain(|&(i, j)| !same_values(ga[i].attrs, gb[j].attrs, &opts.differ));
    }
    let (list, kp) = kernel_pairs(&ga, gb, &pairs);
    let found = solids
        .evaluator()
        .distances(&list, &kp)
        .map_err(|e| e.to_string())?;
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for (&(i, j), c) in pairs.iter().zip(found) {
        let (a, b) = (&ga[i], &gb[j]);
        match c {
            Ok(c) if c.distance < d => {
                let mut v = head(a, b, &opts);
                closest_json(&mut v, &c);
                out.push(v);
            }
            Ok(_) => {}
            Err(e) => skipped.push(skipped_pair(a, b, &e)),
        }
    }
    finish(out, skipped, opts.limit)
}

fn closest_json(v: &mut Value, c: &Closest) {
    v["distance"] = json!(c.distance);
    v["pa"] = p3_json(c.a);
    v["pb"] = p3_json(c.b);
}

// ── inside3d ───────────────────────────────────────────────────────────

/// `{a, region, strict?, attrs?}`: the parts of `a` inside `region`, a box
/// `{min: [x, y, z], max: [x, y, z]}` (touching its boundary counts unless
/// `strict`), or a part selection: then each part of `a` that lies wholly
/// in the solid of a part of the region, as pairs.
fn inside3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let solids = doc.solids.get()?;
    let opts = pair_opts(&o)?;
    let strict = o.get("strict").and_then(Value::as_bool).unwrap_or(false);
    let region = o.get("region").ok_or("region is required")?;
    let ga = items(doc, solids, o.get("a").unwrap_or(&Value::Null))?;
    let is_box = region.get("min").is_some() && region.get("max").is_some();
    if is_box {
        let (lo, hi) = crate::dispatch_3d::bounds_val(region)?;
        let r = Aabb3::new(lo, hi);
        let mut out = Vec::new();
        for g in &ga {
            // A solid lies in a box exactly when its tight box does.
            let b = &g.bbox;
            if !r.contains(b) {
                continue;
            }
            let touches = b.min.x == r.min.x
                || b.min.y == r.min.y
                || b.min.z == r.min.z
                || b.max.x == r.max.x
                || b.max.y == r.max.y
                || b.max.z == r.max.z;
            if strict && touches {
                continue;
            }
            let mut v = json!({ "a": part_key(g.id), "b": Value::Null, "touches": touches });
            if opts.attrs {
                v["aAttrs"] = attrs::to_json(g.attrs);
            }
            out.push(v);
        }
        return finish(out, Vec::new(), opts.limit);
    }
    let gb = items(doc, solids, region)?;
    let mut pairs = candidates(&ga, &gb, false, 0.0, |x, y| y.contains(x));
    if !opts.differ.is_empty() {
        pairs.retain(|&(i, j)| !same_values(ga[i].attrs, gb[j].attrs, &opts.differ));
    }
    let (list, kp) = kernel_pairs(&ga, &gb, &pairs);
    let verdicts = solids
        .evaluator()
        .interferences(&list, &kp)
        .map_err(|e| e.to_string())?;
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for (&(i, j), verdict) in pairs.iter().zip(verdicts) {
        let (a, b) = (&ga[i], &gb[j]);
        match verdict {
            Ok(Interference::Overlapping { a_in_b: true, .. }) => out.push(head(a, b, &opts)),
            Ok(_) => {}
            Err(e) => skipped.push(skipped_pair(a, b, &e)),
        }
    }
    finish(out, skipped, opts.limit)
}

// ── distance3d ─────────────────────────────────────────────────────────

/// `{a, b}`: two parts (ids or part objects): their distance and closest
/// points, 0 when they touch or overlap.
fn distance3d(doc: &mut Document, args: &str) -> R {
    let o = parse(args)?;
    let solids = doc.solids.get()?;
    let one = |k: &str| -> Result<Value, String> {
        let v = o
            .get(k)
            .ok_or_else(|| format!("{k} is required (a part)"))?;
        Ok(Value::String(part_key(part_id_of(v)?)))
    };
    let ga = items(doc, solids, &one("a")?)?;
    let gb = items(doc, solids, &one("b")?)?;
    if ga[0].id == gb[0].id {
        return Err("a and b are the same part".into());
    }
    let (list, kp) = kernel_pairs(&ga, &gb, &[(0, 0)]);
    let c = solids
        .evaluator()
        .distances(&list, &kp)
        .map_err(|e| e.to_string())?
        .pop()
        .expect("one pair")
        .map_err(|e| format!("{} and {}: {e}", part_key(ga[0].id), part_key(gb[0].id)))?;
    let mut v = json!({ "a": part_key(ga[0].id), "b": part_key(gb[0].id) });
    closest_json(&mut v, &c);
    Ok(v.to_string())
}
