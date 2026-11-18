//! Prebaked 3D scenes: what a viewer needs to show a document's parts,
//! written once by a peer that has a solid kernel, read by one that has
//! none (docs/3d-design.md, "3D in the browser").
//!
//! A bake is a snapshot of the evaluation cache, not a picture. It holds the
//! document's parts and definitions and, for every distinct definition, the
//! kernel's answers a view needs: its render mesh at one chord, its exact
//! box in its own frame and at every placement a part gives it, and its
//! mass properties. Loaded, it becomes an [`Evaluator`] ([`BakedEvaluator`])
//! that answers those questions from the file, so the viewer builds its
//! scene with the same core code the server uses (`solid::scene`), and the
//! render options stored with the bake (view, colours, materials, light)
//! give the same picture. A question the file cannot answer (a new or
//! edited definition, a new placement, a distance between parts) fails with
//! [`NEEDS_KERNEL`]; that is the moment a shell loads the real kernel.
//!
//! # Format
//!
//! `MAGIC` (4 bytes), [`FORMAT_VERSION`] (u32, little endian), then the
//! bincode bytes of the file body: the kernel's name, the chord (mm), the
//! render options and the model (definitions and parts) as JSON text (self
//! describing, so a file survives a new optional field in the feature
//! tree), and one entry per distinct solid: the definition it was evaluated
//! from, mass, box, placed boxes and the mesh (positions as f32, which is
//! what a GPU draws; boxes and volumes keep f64). A reader refuses any other
//! magic or version. Shared definitions are stored once, and so are
//! definitions with equal features: entries are matched to definitions by
//! the evaluation cache's content key, computed on load.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::eval::{content_key, Evaluator, Solid, SolidEnv};
use super::kernel::{EvalError, MassProps, TightBox, ToolResolver, TriMesh, Validity};
use super::part::{def_to_bytes, part_key, tool_defs, Part};
use super::types::{DefId, Feature, PartDef, Placement, Point3, ToolRef};
use super::wire::{Wire, WireBox, WireMass};
use crate::document::Document;

/// First bytes of every bake.
pub const MAGIC: [u8; 4] = *b"CCB3";

/// The body layout. Bump it with any change to [`FileBody`] or to what its
/// fields mean; readers refuse other versions.
pub const FORMAT_VERSION: u32 = 1;

/// The image size (shorter side, device pixels) the default chord is
/// chosen for: a quarter pixel at this size, the same rule as `cad.render`
/// (`solid::scene::auto_chord`), so a view up to about twice this size still
/// shows no facets wider than half a pixel.
pub const BAKE_PIXELS: u32 = 1280;

/// Where the kernel refuses to tessellate a solid at the bake's chord, the
/// chord is doubled up to this many times (`cad.render` at 640 px uses
/// twice the bake's chord).
pub const COARSER_TRIES: u32 = 3;

/// What a prebaked scene answers when asked something it does not hold.
pub const NEEDS_KERNEL: &str = "the prebaked scene does not hold this answer (a new or edited \
     definition, a new placement or a query between parts); it needs the solid kernel, which \
     this page has not loaded";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileBody {
    kernel: String,
    chord: f64,
    /// The render options the bake was made with, JSON.
    options: String,
    /// [`Model`] as JSON.
    model: String,
    solids: Vec<SolidData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Model {
    defs: Vec<PartDef>,
    parts: Vec<Part>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SolidData {
    /// A definition (in the model) this solid was evaluated from.
    def: DefId,
    mass: Option<WireMass>,
    local_box: WireBox,
    /// Boxes at the placements parts give this solid (identity excluded).
    placed: Vec<(Placement, WireBox)>,
    /// The chord this mesh was made at: the file's, or a coarser one where
    /// the kernel refused the file's ([`COARSER_TRIES`]).
    chord: f64,
    positions: Vec<[f32; 3]>,
    triangles: Vec<[u32; 3]>,
}

/// What [`bake`] wrote.
#[derive(Debug, Clone, PartialEq)]
pub struct BakeReport {
    pub parts: usize,
    pub defs: usize,
    /// Distinct solids (definitions with equal features count once).
    pub solids: usize,
    pub triangles: usize,
    pub chord: f64,
    /// Solids whose mesh had to be made at a coarser chord.
    pub coarser: usize,
    pub kernel: String,
}

/// Bake the document's parts for a viewer: every part, its definition's
/// mesh at `options.chord` (default: [`BAKE_PIXELS`]), boxes and mass, and
/// `options` itself (the `cad.render` options the viewer starts from; `out`
/// and `size` are dropped). Needs a kernel (`doc.solids`).
pub fn bake(doc: &Document, options: &Value) -> Result<(Vec<u8>, BakeReport), String> {
    let solids = doc.solids.get()?;
    if doc.parts.is_empty() {
        return Err("the document has no parts to bake".into());
    }
    let who = |p: &Part| format!("part {}", part_key(p.id));
    // The scene's size picks the default chord, as cad.render does.
    let mut all: Option<(Point3, Point3)> = None;
    for p in &doc.parts {
        let def = doc
            .part_defs
            .get(&p.def)
            .ok_or_else(|| format!("{}: its definition is missing", who(p)))?;
        let b = solids
            .bbox(&doc.part_defs, def, &p.placement)
            .map_err(|e| format!("{}: {e}", who(p)))?;
        all = Some(match all {
            None => (b.min, b.max),
            Some((lo, hi)) => (
                Point3::new(lo.x.min(b.min.x), lo.y.min(b.min.y), lo.z.min(b.min.z)),
                Point3::new(hi.x.max(b.max.x), hi.y.max(b.max.y), hi.z.max(b.max.z)),
            ),
        });
    }
    let (lo, hi) = all.expect("at least one part");
    let chord = match options.get("chord") {
        None | Some(Value::Null) => super::scene::auto_chord(lo.distance(hi), BAKE_PIXELS),
        Some(v) => v
            .as_f64()
            .filter(|c| c.is_finite() && *c > 0.0)
            .ok_or("chord must be a positive number (mm)")?,
    };

    let mut order: Vec<Vec<u8>> = Vec::new();
    let mut by_key: HashMap<Vec<u8>, SolidData> = HashMap::new();
    let mut triangles = 0;
    let mut coarser = 0;
    for p in &doc.parts {
        let def = &doc.part_defs[&p.def];
        let key = content_key(&doc.part_defs, def);
        if !by_key.contains_key(&key) {
            let local = solids
                .local_bbox(&doc.part_defs, def)
                .map_err(|e| format!("{}: {e}", who(p)))?;
            // The kernel's own mesh, not the render mesh (whose vertices
            // are split at creases): the viewer derives the render mesh
            // with the same code.
            let solid = solids
                .solid(&doc.part_defs, def)
                .map_err(|e| format!("{}: {e}", who(p)))?;
            let mut used = chord;
            let mut tri = solid.tessellate(used);
            for _ in 0..COARSER_TRIES {
                if tri.is_ok() {
                    break;
                }
                used *= 2.0;
                tri = solid.tessellate(used);
            }
            let tri = tri.map_err(|e| format!("{}: {e}", who(p)))?;
            coarser += usize::from(used != chord);
            triangles += tri.triangles.len();
            by_key.insert(
                key.clone(),
                SolidData {
                    def: def.id,
                    mass: solids.mass(&doc.part_defs, def).ok().map(WireMass::of),
                    local_box: WireBox::of(local),
                    placed: Vec::new(),
                    chord: used,
                    positions: tri
                        .positions
                        .iter()
                        .map(|q| [q.x as f32, q.y as f32, q.z as f32])
                        .collect(),
                    triangles: tri.triangles,
                },
            );
            order.push(key.clone());
        }
        if !p.placement.is_identity() {
            let entry = by_key.get_mut(&key).expect("inserted");
            if !entry.placed.iter().any(|(q, _)| *q == p.placement) {
                let b = solids
                    .tight_bbox(&doc.part_defs, def, &p.placement)
                    .map_err(|e| format!("{}: {e}", who(p)))?;
                entry.placed.push((p.placement, WireBox::of(b)));
            }
        }
    }

    let mut opts = options.clone();
    if let Some(o) = opts.as_object_mut() {
        for k in ["out", "size", "pixelRatio", "backend"] {
            o.remove(k);
        }
        o.insert("chord".into(), chord.into());
    }
    let model = Model {
        defs: doc.part_defs.values().cloned().collect(),
        parts: doc.parts.clone(),
    };
    let body = FileBody {
        kernel: solids.kernel_name(),
        chord,
        options: opts.to_string(),
        model: serde_json::to_string(&model).map_err(|e| format!("bake: {e}"))?,
        solids: order
            .iter()
            .map(|k| by_key.remove(k).expect("listed"))
            .collect(),
    };
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend(bincode::serialize(&body).map_err(|e| format!("bake: {e}"))?);
    let report = BakeReport {
        parts: doc.parts.len(),
        defs: doc.part_defs.len(),
        solids: body.solids.len(),
        triangles,
        chord,
        coarser,
        kernel: body.kernel,
    };
    Ok((out, report))
}

/// A loaded bake.
#[derive(Clone)]
pub struct Baked {
    /// The kernel that made the answers ("arris 0.11.1").
    pub kernel: String,
    /// The chord (mm) of every mesh in the file.
    pub chord: f64,
    /// The render options the bake was made with (its `chord` included).
    pub options: Value,
    pub defs: BTreeMap<DefId, PartDef>,
    pub parts: Vec<Part>,
    evaluator: Arc<BakedEvaluator>,
}

impl std::fmt::Debug for Baked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Baked")
            .field("kernel", &self.kernel)
            .field("chord", &self.chord)
            .field("parts", &self.parts.len())
            .field("solids", &self.evaluator.by_key.len())
            .finish()
    }
}

impl Baked {
    /// Read a bake, refusing anything that is not one of this version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Baked, String> {
        if bytes.len() < 8 || bytes[..4] != MAGIC {
            return Err("not a CodeCAD prebaked 3D scene".into());
        }
        let version = u32::from_le_bytes(bytes[4..8].try_into().expect("4 bytes"));
        if version != FORMAT_VERSION {
            return Err(format!(
                "prebaked 3D scene format {version}; this build reads format {FORMAT_VERSION} \
                 (bake it again with this version)"
            ));
        }
        let body: FileBody = bincode::deserialize(&bytes[8..])
            .map_err(|e| format!("prebaked 3D scene is damaged: {e}"))?;
        let model: Model = serde_json::from_str(&body.model)
            .map_err(|e| format!("prebaked 3D scene model: {e}"))?;
        let options: Value = serde_json::from_str(&body.options)
            .map_err(|e| format!("prebaked 3D scene options: {e}"))?;
        let defs: BTreeMap<DefId, PartDef> = model.defs.into_iter().map(|d| (d.id, d)).collect();
        let mut by_key = HashMap::new();
        for s in body.solids {
            let def = defs
                .get(&s.def)
                .ok_or_else(|| format!("prebaked 3D scene: definition {} missing", s.def.0))?;
            if let Some(t) = s
                .triangles
                .iter()
                .flatten()
                .find(|&&i| i as usize >= s.positions.len())
            {
                return Err(format!("prebaked 3D scene: vertex index {t} out of range"));
            }
            by_key.insert(content_key(&defs, def), Arc::new(BakedSolid(s)));
        }
        for p in &model.parts {
            if !defs.contains_key(&p.def) {
                return Err(format!(
                    "prebaked 3D scene: part {} has no definition",
                    part_key(p.id)
                ));
            }
        }
        Ok(Baked {
            kernel: body.kernel.clone(),
            chord: body.chord,
            options,
            defs,
            parts: model.parts,
            evaluator: Arc::new(BakedEvaluator {
                kernel: body.kernel,
                by_key,
            }),
        })
    }

    /// The answers as an evaluator.
    pub fn evaluator(&self) -> Arc<BakedEvaluator> {
        self.evaluator.clone()
    }

    /// A fresh solid environment over the bake's answers (its own cache).
    pub fn env(&self) -> SolidEnv {
        SolidEnv::new(self.evaluator.clone())
    }

    /// A document holding the bake's parts and definitions, with
    /// [`Self::env`]: what a viewer renders and queries.
    pub fn document(&self) -> Document {
        let mut doc = Document::new();
        doc.part_defs = self.defs.clone();
        doc.parts = self.parts.clone();
        doc.solids = self.env();
        doc
    }

    /// Distinct solids and their triangles.
    pub fn solids(&self) -> (usize, usize) {
        let e = &self.evaluator.by_key;
        (e.len(), e.values().map(|s| s.0.triangles.len()).sum())
    }
}

/// The cache key [`content_key`] gives a definition with these features,
/// computed through a [`ToolResolver`] (what an evaluator is handed) instead
/// of the document's map.
pub fn content_key_via(tools: &dyn ToolResolver, def: &PartDef) -> Vec<u8> {
    let tool_ids = |features: &[Feature]| -> Vec<DefId> {
        features
            .iter()
            .filter_map(|f| match f {
                Feature::Boolean {
                    tool: ToolRef::Def(id, _),
                    ..
                } => Some(*id),
                _ => None,
            })
            .collect()
    };
    let mut key = bincode::serialize(&def.features).expect("bincode serialize failed");
    let mut seen: BTreeMap<DefId, ()> = BTreeMap::new();
    let mut todo = tool_ids(&def.features);
    while let Some(id) = todo.pop() {
        if seen.insert(id, ()).is_some() {
            continue;
        }
        if let Some(t) = tools.def(id) {
            todo.extend(tool_defs(t));
        }
    }
    for id in seen.keys() {
        key.extend_from_slice(&id.0.to_le_bytes());
        match tools.def(*id) {
            Some(t) => key.extend_from_slice(&def_to_bytes(&PartDef {
                id: t.id,
                name: String::new(),
                features: t.features.clone(),
            })),
            None => key.push(0xff),
        }
    }
    key
}

fn needs_kernel() -> EvalError {
    EvalError::Kernel {
        feature: None,
        message: NEEDS_KERNEL.into(),
    }
}

/// An evaluator that answers from a bake.
pub struct BakedEvaluator {
    kernel: String,
    by_key: HashMap<Vec<u8>, Arc<BakedSolid>>,
}

impl Evaluator for BakedEvaluator {
    fn name(&self) -> String {
        format!("{} (prebaked)", self.kernel)
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        match self.by_key.get(&content_key_via(tools, def)) {
            Some(s) => Ok(s.clone()),
            None => Err(needs_kernel()),
        }
    }
}

/// One solid of a bake.
pub struct BakedSolid(SolidData);

impl Solid for BakedSolid {
    fn mass(&self) -> Result<MassProps, EvalError> {
        self.0.mass.map(WireMass::get).ok_or_else(needs_kernel)
    }

    fn bbox(&self, frame: &Placement) -> Result<TightBox, EvalError> {
        if frame.is_identity() {
            return Ok(self.0.local_box.get());
        }
        self.0
            .placed
            .iter()
            .find(|(p, _)| p == frame)
            .map(|(_, b)| b.get())
            .ok_or_else(needs_kernel)
    }

    fn check(&self) -> Result<Validity, EvalError> {
        Err(needs_kernel())
    }

    /// The baked mesh, whatever chord is asked for (a viewer asks for the
    /// bake's own chord).
    fn tessellate(&self, _chord: f64) -> Result<TriMesh, EvalError> {
        Ok(TriMesh {
            positions: self
                .0
                .positions
                .iter()
                .map(|p| Point3::new(p[0] as f64, p[1] as f64, p[2] as f64))
                .collect(),
            triangles: self.0.triangles.clone(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solid::eval::{DefTools, KernelEvaluator};
    use crate::solid::fake::FakeKernel;
    use serde_json::json;

    fn fake_doc() -> Document {
        let mut d = Document::new();
        d.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(FakeKernel)));
        d
    }

    fn call(d: &mut Document, method: &str, args: Value) -> Value {
        let out = crate::cad_call(d, method, &args.to_string())
            .unwrap_or_else(|e| panic!("{method} {args}: {e}"));
        serde_json::from_str(&out).unwrap()
    }

    /// A plate with a hole cut by a pin part, the pin, a rotated copy of
    /// the plate and a second pin sharing the first one's definition.
    fn assembly() -> Document {
        let mut d = fake_doc();
        let plate = call(
            &mut d,
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [100, 50]}, "distance": 18, "attrs": {"m": "oak"}}),
        );
        let pin = call(
            &mut d,
            "extrude",
            json!({"profile": {"min": [20, 20], "max": [30, 30]}, "distance": 30, "at": [0, 0, -5]}),
        );
        call(
            &mut d,
            "cut",
            json!({"target": plate["id"], "tool": {"part": pin["id"]}}),
        );
        call(
            &mut d,
            "copy3d",
            json!({"target": pin["id"], "move": [40, 0, 0]}),
        );
        let c = call(
            &mut d,
            "copy3d",
            json!({"target": plate["id"], "move": [0, 120, 0]}),
        );
        call(
            &mut d,
            "rotate3d",
            json!({"target": c["id"], "angle": 30, "axis": [0, 0, 1], "about": [0, 120, 0]}),
        );
        d
    }

    #[test]
    fn content_key_via_a_resolver_equals_the_cache_key() {
        let d = assembly();
        for def in d.part_defs.values() {
            assert_eq!(
                content_key_via(&DefTools(&d.part_defs), def),
                content_key(&d.part_defs, def)
            );
        }
    }

    #[test]
    fn a_bake_answers_what_the_kernel_answered() {
        let d = assembly();
        let opts = json!({"view": "iso-sw", "colorBy": {"attr": "m"}, "out": "x.png"});
        let (bytes, report) = bake(&d, &opts).unwrap();
        assert_eq!(report.parts, d.parts.len());
        // The two plates share a definition, so do the two pins.
        assert_eq!(report.solids, 2);
        let b = Baked::from_bytes(&bytes).unwrap();
        assert_eq!(
            (b.parts.clone(), b.defs.clone()),
            (d.parts.clone(), d.part_defs.clone())
        );
        assert_eq!(b.options["view"], "iso-sw");
        assert!(b.options.get("out").is_none());
        assert_eq!(b.options["chord"], report.chord);
        assert!(b.kernel.contains("fake") || !b.kernel.is_empty());

        // cad.parts() gives the same numbers, bit for bit.
        let mut viewer = b.document();
        let mut src = d.clone();
        assert_eq!(
            call(&mut viewer, "parts", json!({})),
            call(&mut src, "parts", json!({}))
        );

        // The scene: same items, placements, materials, camera; meshes equal
        // to f32 rounding.
        let o = b.options.clone();
        let want = crate::solid::scene::parts_scene(&d, &o, (960, 640)).unwrap();
        let got = crate::solid::scene::parts_scene(&viewer, &o, (960, 640)).unwrap();
        assert_eq!(got.parts, want.parts);
        assert_eq!(got.triangles, want.triangles);
        assert_eq!(got.chord, want.chord);
        assert_eq!(got.scene.camera, want.scene.camera);
        assert_eq!(got.scene.environment, want.scene.environment);
        for (g, w) in got.scene.items.iter().zip(&want.scene.items) {
            assert_eq!((g.placement, g.material), (w.placement, w.material));
            assert_eq!(g.mesh.triangles, w.mesh.triangles);
            for (p, q) in g.mesh.positions.iter().zip(&w.mesh.positions) {
                assert!(p.max_abs_diff(*q) <= 1e-5 * (1.0 + q.x.abs().max(q.y.abs())));
            }
        }
        // Two items that share a definition share one mesh in the viewer too.
        let shared = got
            .scene
            .items
            .iter()
            .filter(|i| Arc::ptr_eq(&i.mesh, &got.scene.items[0].mesh))
            .count();
        assert_eq!(shared, 2);
    }

    #[test]
    fn what_a_bake_does_not_hold_needs_the_kernel_and_changes_nothing() {
        let d = assembly();
        let (bytes, _) = bake(&d, &json!({})).unwrap();
        let mut viewer = Baked::from_bytes(&bytes).unwrap().document();
        let before = (viewer.part_defs.clone(), viewer.parts.clone());
        let a = json!({"profile": {"min": [0, 0], "max": [5, 5]}, "distance": 5});
        let e = crate::cad_call(&mut viewer, "extrude", &a.to_string()).unwrap_err();
        assert!(e.contains(NEEDS_KERNEL), "extrude: {e}");
        assert_eq!((viewer.part_defs.clone(), viewer.parts.clone()), before);
        // A move needs no geometry, but the moved part's box does.
        let id = viewer.parts[0].id;
        let a = json!({"target": crate::solid::part::part_key(id), "by": [1, 0, 0]});
        crate::cad_call(&mut viewer, "move3d", &a.to_string()).unwrap();
        let e = crate::cad_call(&mut viewer, "parts", "{}").unwrap_err();
        assert!(e.contains(NEEDS_KERNEL), "parts: {e}");
        // Pair queries are the kernel's.
        let mut viewer = Baked::from_bytes(&bytes).unwrap().document();
        let r = crate::cad_call(&mut viewer, "overlapping3d", "{}");
        assert!(r.as_ref().map_or(true, |v| v.contains("skipped")), "{r:?}");
    }

    #[test]
    fn only_bakes_of_this_version_are_read() {
        let d = assembly();
        let (mut bytes, _) = bake(&d, &json!({})).unwrap();
        assert!(Baked::from_bytes(b"PNG\0\0\0\0\0").is_err());
        assert!(Baked::from_bytes(&bytes[..6]).is_err());
        let trunc = &bytes[..bytes.len() / 2];
        assert!(Baked::from_bytes(trunc).unwrap_err().contains("damaged"));
        bytes[4] = 9;
        assert!(Baked::from_bytes(&bytes).unwrap_err().contains("format 9"));
        let e = bake(&Document::new(), &json!({})).unwrap_err();
        assert!(e.contains("kernel"), "{e}");
    }
}
