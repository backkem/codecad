//! Evaluation of part definitions without a kernel in core.
//!
//! cadview-core links no solid kernel. A shell that has one (the server,
//! with the arris adapter in `cadview-solid`) wraps it in a
//! [`KernelEvaluator`] and hands it to each document as a [`SolidEnv`]
//! (`Document::solids`). The environment is shell state: it is never
//! synced, serialized or part of undo, and a document without one still
//! stores and syncs parts. Only geometry answers (box, volume, centroid,
//! meshes) need it, and they fail with [`NO_KERNEL`] when it is missing.
//!
//! The environment owns the evaluation cache. Entries are keyed by content:
//! the bincode bytes of the definition's features plus those of every tool
//! definition it reaches (with their ids). The same features evaluate once,
//! whichever definition, part or document holds them; an edit changes the
//! key, so a stale result is never returned; undo brings back a key that is
//! usually still cached. Least recently used entries are dropped past
//! [`CACHE_ENTRIES`]. A shell that shares one environment between its
//! documents (the server does) shares the cache too.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use super::kernel::{Aabb3, EvalError, MassProps, SolidKernel, ToolResolver, TriMesh, Validity};
use super::part::{def_to_bytes, tool_defs};
use super::types::{DefId, PartDef, Placement};
use crate::scene3d::{RenderMesh, DEFAULT_CREASE_DEG};

/// The message of every geometry query in a build without a kernel.
pub const NO_KERNEL: &str = "this build has no solid kernel: parts are stored and synced, but \
     their geometry (box, volume, centroid, meshes) is only available where a kernel runs \
     (the server's scripts)";

/// Most cached evaluations per environment.
pub const CACHE_ENTRIES: usize = 4096;

/// One evaluated part, behind a kernel-neutral interface.
pub trait Solid: Send + Sync {
    fn mass(&self) -> Result<MassProps, EvalError>;
    /// The tight box of the solid placed by `frame`.
    fn bbox(&self, frame: &Placement) -> Result<Aabb3, EvalError>;
    fn check(&self) -> Result<Validity, EvalError>;
    /// The boundary as a closed triangle mesh in the part's own frame.
    fn tessellate(&self, chord: f64) -> Result<TriMesh, EvalError>;
}

/// What a shell supplies: something that turns a definition into a
/// [`Solid`]. Object safe, so core never names the kernel type.
pub trait Evaluator: Send + Sync {
    /// Kernel name and version, for reports ("arris 0.11.1").
    fn name(&self) -> String;
    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Arc<dyn Solid>, EvalError>;
}

/// Any [`SolidKernel`] as an [`Evaluator`].
pub struct KernelEvaluator<K> {
    kernel: Arc<K>,
}

impl<K> KernelEvaluator<K> {
    pub fn new(kernel: K) -> Self {
        Self {
            kernel: Arc::new(kernel),
        }
    }
}

struct KernelSolid<K: SolidKernel> {
    kernel: Arc<K>,
    brep: K::Brep,
}

impl<K> Solid for KernelSolid<K>
where
    K: SolidKernel + Send + Sync,
    K::Brep: Send + Sync,
{
    fn mass(&self) -> Result<MassProps, EvalError> {
        self.kernel.mass(&self.brep)
    }
    fn bbox(&self, frame: &Placement) -> Result<Aabb3, EvalError> {
        self.kernel.bbox(&self.brep, frame)
    }
    fn check(&self) -> Result<Validity, EvalError> {
        self.kernel.check(&self.brep)
    }
    fn tessellate(&self, chord: f64) -> Result<TriMesh, EvalError> {
        self.kernel.tessellate(&self.brep, chord)
    }
}

impl<K> Evaluator for KernelEvaluator<K>
where
    K: SolidKernel + Send + Sync + 'static,
    K::Brep: Send + Sync + 'static,
{
    fn name(&self) -> String {
        self.kernel.name()
    }
    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        let brep = self.kernel.evaluate(def, tools)?.brep;
        Ok(Arc::new(KernelSolid {
            kernel: self.kernel.clone(),
            brep,
        }))
    }
}

/// The document's definitions as a [`ToolResolver`].
pub struct DefTools<'a>(pub &'a BTreeMap<DefId, PartDef>);

impl ToolResolver for DefTools<'_> {
    fn def(&self, id: DefId) -> Option<&PartDef> {
        self.0.get(&id)
    }
}

/// The cache key of `def`: its features, then every tool definition it
/// reaches (id and features), in id order. Its own id and name are left
/// out, so copies with the same features share one evaluation.
pub fn content_key(defs: &BTreeMap<DefId, PartDef>, def: &PartDef) -> Vec<u8> {
    let mut key = bincode::serialize(&def.features).expect("bincode serialize failed");
    let mut seen: BTreeMap<DefId, ()> = BTreeMap::new();
    let mut todo: Vec<DefId> = tool_defs(def).collect();
    while let Some(id) = todo.pop() {
        if seen.insert(id, ()).is_some() {
            continue;
        }
        if let Some(t) = defs.get(&id) {
            todo.extend(tool_defs(t));
        }
    }
    for id in seen.keys() {
        key.extend_from_slice(&id.0.to_le_bytes());
        match defs.get(id) {
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

/// One cached evaluation.
struct Entry {
    solid: Result<Arc<dyn Solid>, EvalError>,
    mass: Option<Result<MassProps, EvalError>>,
    /// Meshes by chord (f64 bits).
    meshes: HashMap<u64, Result<Arc<RenderMesh>, EvalError>>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<Vec<u8>, Entry>,
    tick: u64,
    evaluations: u64,
}

/// An evaluator and its cache.
pub struct Solids {
    evaluator: Arc<dyn Evaluator>,
    cache: Mutex<Cache>,
}

impl Solids {
    pub fn new(evaluator: Arc<dyn Evaluator>) -> Self {
        Self {
            evaluator,
            cache: Mutex::new(Cache::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn kernel_name(&self) -> String {
        self.evaluator.name()
    }

    /// How many times the evaluator actually ran (cache misses).
    pub fn evaluations(&self) -> u64 {
        self.lock().evaluations
    }

    /// The evaluated solid of `def`, from the cache when its content was
    /// evaluated before. The cache lock is not held while the kernel runs.
    pub fn solid(
        &self,
        defs: &BTreeMap<DefId, PartDef>,
        def: &PartDef,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        let key = content_key(defs, def);
        {
            let mut c = self.lock();
            c.tick += 1;
            let tick = c.tick;
            if let Some(e) = c.entries.get_mut(&key) {
                e.used = tick;
                return e.solid.clone();
            }
        }
        let solid = self.evaluator.evaluate(def, &DefTools(defs));
        let mut c = self.lock();
        c.evaluations += 1;
        if c.entries.len() >= CACHE_ENTRIES {
            if let Some(oldest) = c
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            {
                c.entries.remove(&oldest);
            }
        }
        let used = c.tick;
        c.entries.insert(
            key,
            Entry {
                solid: solid.clone(),
                mass: None,
                meshes: HashMap::new(),
                used,
            },
        );
        solid
    }

    /// Volume and centroid in the definition's frame (cached).
    pub fn mass(
        &self,
        defs: &BTreeMap<DefId, PartDef>,
        def: &PartDef,
    ) -> Result<MassProps, EvalError> {
        let solid = self.solid(defs, def)?;
        let key = content_key(defs, def);
        if let Some(Some(m)) = self.lock().entries.get(&key).map(|e| e.mass.clone()) {
            return m;
        }
        let m = solid.mass();
        if let Some(e) = self.lock().entries.get_mut(&key) {
            e.mass = Some(m.clone());
        }
        m
    }

    /// The tight box of `def` placed by `frame`.
    pub fn bbox(
        &self,
        defs: &BTreeMap<DefId, PartDef>,
        def: &PartDef,
        frame: &Placement,
    ) -> Result<Aabb3, EvalError> {
        self.solid(defs, def)?.bbox(frame)
    }

    /// The render mesh of `def` at `chord` (cached per chord).
    pub fn render_mesh(
        &self,
        defs: &BTreeMap<DefId, PartDef>,
        def: &PartDef,
        chord: f64,
    ) -> Result<Arc<RenderMesh>, EvalError> {
        let solid = self.solid(defs, def)?;
        let key = content_key(defs, def);
        let bits = chord.to_bits();
        if let Some(Some(m)) = self
            .lock()
            .entries
            .get(&key)
            .map(|e| e.meshes.get(&bits).cloned())
        {
            return m;
        }
        let m = solid
            .tessellate(chord)
            .map(|t| Arc::new(RenderMesh::from_trimesh(&t, DEFAULT_CREASE_DEG)));
        if let Some(e) = self.lock().entries.get_mut(&key) {
            e.meshes.insert(bits, m.clone());
        }
        m
    }
}

/// The solid environment a shell gives a document: an evaluator with its
/// cache, or none. Cloning shares it. Never synced, serialized or undone.
#[derive(Clone, Default)]
pub struct SolidEnv(Option<Arc<Solids>>);

impl SolidEnv {
    /// No kernel: parts are stored and synced, geometry queries fail.
    pub fn none() -> Self {
        SolidEnv(None)
    }

    pub fn new(evaluator: Arc<dyn Evaluator>) -> Self {
        SolidEnv(Some(Arc::new(Solids::new(evaluator))))
    }

    /// The evaluator and cache, or the [`NO_KERNEL`] error.
    pub fn get(&self) -> Result<&Solids, String> {
        self.0.as_deref().ok_or_else(|| NO_KERNEL.to_string())
    }

    pub fn is_some(&self) -> bool {
        self.0.is_some()
    }

    pub fn kernel_name(&self) -> Option<String> {
        self.0.as_ref().map(|s| s.kernel_name())
    }
}

impl fmt::Debug for SolidEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(s) => write!(f, "SolidEnv({})", s.kernel_name()),
            None => write!(f, "SolidEnv(none)"),
        }
    }
}
