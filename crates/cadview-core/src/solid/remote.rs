//! A solid kernel behind a byte interface: how a page uses a kernel that
//! lives in a separately loaded WebAssembly module (docs/3d-design.md, "3D
//! in the browser").
//!
//! Both sides run this file. The module that links the kernel wraps it in a
//! [`KernelServer`] (any [`Evaluator`], usually `KernelEvaluator<ArrisKernel>`)
//! and answers one call: request bytes in, reply bytes out. The page puts a
//! [`RemoteEvaluator`] on its documents (`SolidEnv::new`), which turns every
//! [`Evaluator`] and [`Solid`] call into a [`Request`] and the [`Reply`] back
//! into the kernel's answer. Solids stay in the module, named by a
//! [`Handle`]; the page's evaluation cache (`Solids`) holds the handles, so
//! everything it caches (boxes, volumes, meshes) is cached on the page and
//! asked across the boundary once. A handle is released when the page drops
//! the last reference to its solid.
//!
//! The interface is one enum each way, serialized with bincode, so there is
//! no wrapper per method to fall out of step: a method added to [`Solid`] or
//! [`Evaluator`] without a [`Request`] variant and a proxy method here fails
//! `every_trait_method_crosses_the_boundary` (it reads eval.rs).
//! [`WIRE_VERSION`] is checked once, when the page connects ([`Request::Hello`]).
//!
//! Calls are synchronous, like the script API: in the browser the module
//! runs on the page's thread, called through JavaScript.

use std::any::Any;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::eval::{Evaluator, Solid};
use super::kernel::{
    BrepTopology, Closest, EvalError, Interference, MassProps, Ray3, RayHits, Silhouettes,
    TightBox, ToolResolver, TriMesh, Validity,
};
use super::part::tool_defs;
use super::types::{DefId, PartDef, Placement, Vec3};
use super::wire::{
    Wire, WireBox, WireClosest, WireError, WireInterference, WireMass, WireMesh, WireValidity,
};

/// The layout of [`Request`] and [`Reply`]. Bump it with any change to them;
/// a page and a module of different versions refuse to connect.
pub const WIRE_VERSION: u32 = 2;

/// A solid held by the kernel module.
pub type Handle = u64;

/// What the page asks. One variant per [`Evaluator`] and [`Solid`] method
/// (see [`COVERAGE`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Connect: the page's wire version.
    Hello { wire: u32 },
    /// [`Evaluator::evaluate`]: the definition and every tool definition it
    /// reaches (the page's resolver, flattened).
    Evaluate { def: PartDef, tools: Vec<PartDef> },
    /// [`Evaluator::evaluate_from`].
    EvaluateFrom {
        def: PartDef,
        tools: Vec<PartDef>,
        base: Handle,
        done: u64,
    },
    /// [`Evaluator::distances`].
    Distances {
        solids: Vec<(Handle, Placement)>,
        pairs: Vec<(u64, u64)>,
    },
    /// [`Evaluator::interferences`].
    Interferences {
        solids: Vec<(Handle, Placement)>,
        pairs: Vec<(u64, u64)>,
    },
    /// [`Solid::mass`].
    Mass(Handle),
    /// [`Solid::bbox`].
    Bbox(Handle, Placement),
    /// [`Solid::check`].
    Check(Handle),
    /// [`Solid::tessellate`].
    Tessellate(Handle, f64),
    /// [`Solid::topology`].
    Topology(Handle),
    /// [`Solid::silhouettes`].
    Silhouettes(Handle, Vec3),
    /// [`Solid::ray_hits`].
    RayHits(Handle, Vec<Ray3>),
    /// The page dropped these solids.
    Release(Vec<Handle>),
}

/// What the module answers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Reply {
    Hello {
        wire: u32,
        kernel: String,
    },
    Solid(Handle),
    Mass(WireMass),
    Box(WireBox),
    Validity(WireValidity),
    Mesh(WireMesh),
    Distances(Vec<Result<WireClosest, WireError>>),
    Interferences(Vec<Result<WireInterference, WireError>>),
    Topology(BrepTopology),
    Silhouettes(Silhouettes),
    RayHits(Vec<Result<RayHits, WireError>>),
    Released,
    /// The kernel's refusal.
    Failed(WireError),
    /// The request was not understood (damaged, another wire version, an
    /// unknown handle).
    Refused(String),
}

/// Which request each trait method becomes. The coverage test reads the
/// traits in eval.rs and demands an entry, and a proxy method in this file,
/// for every method but the two that never cross (`name` is asked once at
/// connect, `as_any` is local).
pub const COVERAGE: &[(&str, &str)] = &[
    ("evaluate", "Evaluate"),
    ("evaluate_from", "EvaluateFrom"),
    ("distances", "Distances"),
    ("interferences", "Interferences"),
    ("mass", "Mass"),
    ("bbox", "Bbox"),
    ("check", "Check"),
    ("tessellate", "Tessellate"),
    ("topology", "Topology"),
    ("silhouettes", "Silhouettes"),
    ("ray_hits", "RayHits"),
];

fn encode<T: Serialize>(v: &T) -> Vec<u8> {
    bincode::serialize(v).expect("bincode serialize failed")
}

// ── The module's side ──────────────────────────────────────────────────

/// Solids of a pair query with their placements.
type PlacedSolids = Vec<(Arc<dyn Solid>, Placement)>;

/// The tool definitions a request carries, as a resolver.
struct Tools<'a>(&'a [PartDef]);

impl ToolResolver for Tools<'_> {
    fn def(&self, id: DefId) -> Option<&PartDef> {
        self.0.iter().find(|d| d.id == id)
    }
}

/// Answers requests with an evaluator, keeping the solids it made.
pub struct KernelServer {
    evaluator: Arc<dyn Evaluator>,
    solids: HashMap<Handle, Arc<dyn Solid>>,
    next: Handle,
}

impl KernelServer {
    pub fn new(evaluator: Arc<dyn Evaluator>) -> Self {
        Self {
            evaluator,
            solids: HashMap::new(),
            next: 1,
        }
    }

    /// Solids held (for tests and reports).
    pub fn live(&self) -> usize {
        self.solids.len()
    }

    /// One call: request bytes in, reply bytes out. Never panics on bad
    /// input.
    pub fn handle(&mut self, request: &[u8]) -> Vec<u8> {
        let reply = match bincode::deserialize::<Request>(request) {
            Ok(r) => self.answer(r),
            Err(e) => Reply::Refused(format!("kernel module: unreadable request: {e}")),
        };
        encode(&reply)
    }

    fn keep(&mut self, s: Result<Arc<dyn Solid>, EvalError>) -> Reply {
        match s {
            Ok(s) => {
                let h = self.next;
                self.next += 1;
                self.solids.insert(h, s);
                Reply::Solid(h)
            }
            Err(e) => Reply::Failed(WireError::of(e)),
        }
    }

    fn placed(&self, solids: &[(Handle, Placement)]) -> Result<PlacedSolids, Reply> {
        solids
            .iter()
            .map(|(h, p)| match self.solids.get(h) {
                Some(s) => Ok((s.clone(), *p)),
                None => Err(Reply::Refused(format!("kernel module: no solid {h}"))),
            })
            .collect()
    }

    fn answer(&mut self, r: Request) -> Reply {
        let solid = |s: &Self, h: Handle| {
            s.solids
                .get(&h)
                .cloned()
                .ok_or_else(|| Reply::Refused(format!("kernel module: no solid {h}")))
        };
        let pairs = |p: &[(u64, u64)]| -> Vec<(usize, usize)> {
            p.iter().map(|&(a, b)| (a as usize, b as usize)).collect()
        };
        match r {
            Request::Hello { wire } if wire == WIRE_VERSION => Reply::Hello {
                wire: WIRE_VERSION,
                kernel: self.evaluator.name(),
            },
            Request::Hello { wire } => Reply::Refused(format!(
                "kernel module speaks wire version {WIRE_VERSION}, the page {wire}"
            )),
            Request::Evaluate { def, tools } => {
                let s = self.evaluator.evaluate(&def, &Tools(&tools));
                self.keep(s)
            }
            Request::EvaluateFrom {
                def,
                tools,
                base,
                done,
            } => match solid(self, base) {
                Ok(b) => {
                    let s = self
                        .evaluator
                        .evaluate_from(&def, &Tools(&tools), &*b, done as usize);
                    self.keep(s)
                }
                Err(e) => e,
            },
            Request::Distances { solids, pairs: p } => match self.placed(&solids) {
                Ok(v) => {
                    let refs: Vec<(&dyn Solid, Placement)> =
                        v.iter().map(|(s, p)| (&**s, *p)).collect();
                    match self.evaluator.distances(&refs, &pairs(&p)) {
                        Ok(all) => Reply::Distances(
                            all.into_iter()
                                .map(|r| r.map(WireClosest::of).map_err(WireError::of))
                                .collect(),
                        ),
                        Err(e) => Reply::Failed(WireError::of(e)),
                    }
                }
                Err(e) => e,
            },
            Request::Interferences { solids, pairs: p } => match self.placed(&solids) {
                Ok(v) => {
                    let refs: Vec<(&dyn Solid, Placement)> =
                        v.iter().map(|(s, p)| (&**s, *p)).collect();
                    match self.evaluator.interferences(&refs, &pairs(&p)) {
                        Ok(all) => Reply::Interferences(
                            all.into_iter()
                                .map(|r| r.map(WireInterference::of).map_err(WireError::of))
                                .collect(),
                        ),
                        Err(e) => Reply::Failed(WireError::of(e)),
                    }
                }
                Err(e) => e,
            },
            Request::Mass(h) => match solid(self, h) {
                Ok(s) => s.mass().map_or_else(
                    |e| Reply::Failed(WireError::of(e)),
                    |m| Reply::Mass(WireMass::of(m)),
                ),
                Err(e) => e,
            },
            Request::Bbox(h, p) => match solid(self, h) {
                Ok(s) => s.bbox(&p).map_or_else(
                    |e| Reply::Failed(WireError::of(e)),
                    |b| Reply::Box(WireBox::of(b)),
                ),
                Err(e) => e,
            },
            Request::Check(h) => match solid(self, h) {
                Ok(s) => s.check().map_or_else(
                    |e| Reply::Failed(WireError::of(e)),
                    |v| Reply::Validity(WireValidity::of(v)),
                ),
                Err(e) => e,
            },
            Request::Tessellate(h, chord) => match solid(self, h) {
                Ok(s) => s.tessellate(chord).map_or_else(
                    |e| Reply::Failed(WireError::of(e)),
                    |m| Reply::Mesh(WireMesh::of(m)),
                ),
                Err(e) => e,
            },
            Request::Topology(h) => match solid(self, h) {
                Ok(s) => s
                    .topology()
                    .map_or_else(|e| Reply::Failed(WireError::of(e)), Reply::Topology),
                Err(e) => e,
            },
            Request::Silhouettes(h, view) => match solid(self, h) {
                Ok(s) => s
                    .silhouettes(view)
                    .map_or_else(|e| Reply::Failed(WireError::of(e)), Reply::Silhouettes),
                Err(e) => e,
            },
            Request::RayHits(h, rays) => match solid(self, h) {
                Ok(s) => s.ray_hits(&rays).map_or_else(
                    |e| Reply::Failed(WireError::of(e)),
                    |all| {
                        Reply::RayHits(all.into_iter().map(|r| r.map_err(WireError::of)).collect())
                    },
                ),
                Err(e) => e,
            },
            Request::Release(hs) => {
                for h in hs {
                    self.solids.remove(&h);
                }
                Reply::Released
            }
        }
    }
}

// ── The page's side ────────────────────────────────────────────────────

/// How the page reaches the module: one synchronous call.
pub trait Transport: Send + Sync {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, String>;
}

struct Link {
    transport: Box<dyn Transport>,
    kernel: String,
    /// Handles dropped on the page, released with the next request (a
    /// release never makes a call of its own from inside a drop).
    released: Mutex<Vec<Handle>>,
}

impl Link {
    fn ask(&self, r: &Request) -> Result<Reply, EvalError> {
        let pending: Vec<Handle> =
            std::mem::take(&mut *self.released.lock().unwrap_or_else(|p| p.into_inner()));
        if !pending.is_empty() {
            self.raw(&Request::Release(pending))?;
        }
        self.raw(r)
    }

    fn raw(&self, r: &Request) -> Result<Reply, EvalError> {
        let bytes = self
            .transport
            .call(&encode(r))
            .map_err(|e| EvalError::Kernel {
                feature: None,
                message: format!("kernel module: {e}"),
            })?;
        match bincode::deserialize::<Reply>(&bytes) {
            Ok(Reply::Failed(e)) => Err(e.get()),
            Ok(Reply::Refused(m)) => Err(EvalError::Kernel {
                feature: None,
                message: m,
            }),
            Ok(reply) => Ok(reply),
            Err(e) => Err(EvalError::Kernel {
                feature: None,
                message: format!("kernel module: unreadable reply: {e}"),
            }),
        }
    }
}

impl Drop for Link {
    /// The last solids of this link were dropped with it: release them.
    fn drop(&mut self) {
        let pending = std::mem::take(self.released.get_mut().unwrap_or_else(|p| p.into_inner()));
        if !pending.is_empty() {
            let _ = self.raw(&Request::Release(pending));
        }
    }
}

fn unexpected(what: &str, r: Reply) -> EvalError {
    EvalError::Kernel {
        feature: None,
        message: format!("kernel module: expected {what}, got {r:?}"),
    }
}

/// An [`Evaluator`] whose kernel is behind a [`Transport`].
pub struct RemoteEvaluator {
    link: Arc<Link>,
}

impl RemoteEvaluator {
    /// Connect: check the module's wire version and learn its kernel.
    pub fn connect(transport: Box<dyn Transport>) -> Result<Self, String> {
        let mut link = Link {
            transport,
            kernel: String::new(),
            released: Mutex::new(Vec::new()),
        };
        match link.raw(&Request::Hello { wire: WIRE_VERSION }) {
            Ok(Reply::Hello { wire, kernel }) if wire == WIRE_VERSION => {
                link.kernel = kernel;
                Ok(Self {
                    link: Arc::new(link),
                })
            }
            Ok(other) => Err(format!("kernel module: unexpected hello {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    }

    fn solid(&self, r: Reply) -> Result<Arc<dyn Solid>, EvalError> {
        match r {
            Reply::Solid(handle) => Ok(Arc::new(RemoteSolid {
                link: self.link.clone(),
                handle,
            })),
            other => Err(unexpected("a solid", other)),
        }
    }

    fn handles(
        &self,
        solids: &[(&dyn Solid, Placement)],
    ) -> Result<Vec<(Handle, Placement)>, EvalError> {
        solids
            .iter()
            .map(|(s, p)| {
                s.as_any()
                    .downcast_ref::<RemoteSolid>()
                    .map(|r| (r.handle, *p))
                    .ok_or_else(|| {
                        EvalError::invalid(None, "a solid of another kernel cannot be compared")
                    })
            })
            .collect()
    }
}

/// The tool definitions `def` reaches, through the page's resolver.
fn tool_closure(def: &PartDef, tools: &dyn ToolResolver) -> Vec<PartDef> {
    let mut seen: BTreeSet<DefId> = BTreeSet::new();
    let mut todo: Vec<DefId> = tool_defs(def).collect();
    let mut out = Vec::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(t) = tools.def(id) {
            todo.extend(tool_defs(t));
            out.push(t.clone());
        }
    }
    out
}

fn pairs(p: &[(usize, usize)]) -> Vec<(u64, u64)> {
    p.iter().map(|&(a, b)| (a as u64, b as u64)).collect()
}

impl Evaluator for RemoteEvaluator {
    fn name(&self) -> String {
        self.link.kernel.clone()
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        let r = self.link.ask(&Request::Evaluate {
            def: def.clone(),
            tools: tool_closure(def, tools),
        })?;
        self.solid(r)
    }

    fn evaluate_from(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
        base: &dyn Solid,
        done: usize,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        let Some(b) = base.as_any().downcast_ref::<RemoteSolid>() else {
            return self.evaluate(def, tools);
        };
        let r = self.link.ask(&Request::EvaluateFrom {
            def: def.clone(),
            tools: tool_closure(def, tools),
            base: b.handle,
            done: done as u64,
        })?;
        self.solid(r)
    }

    fn distances(
        &self,
        solids: &[(&dyn Solid, Placement)],
        pairs_: &[(usize, usize)],
    ) -> Result<Vec<Result<Closest, EvalError>>, EvalError> {
        match self.link.ask(&Request::Distances {
            solids: self.handles(solids)?,
            pairs: pairs(pairs_),
        })? {
            Reply::Distances(v) => Ok(v
                .into_iter()
                .map(|r| r.map(WireClosest::get).map_err(WireError::get))
                .collect()),
            other => Err(unexpected("distances", other)),
        }
    }

    fn interferences(
        &self,
        solids: &[(&dyn Solid, Placement)],
        pairs_: &[(usize, usize)],
    ) -> Result<Vec<Result<Interference, EvalError>>, EvalError> {
        match self.link.ask(&Request::Interferences {
            solids: self.handles(solids)?,
            pairs: pairs(pairs_),
        })? {
            Reply::Interferences(v) => Ok(v
                .into_iter()
                .map(|r| r.map(WireInterference::get).map_err(WireError::get))
                .collect()),
            other => Err(unexpected("interferences", other)),
        }
    }
}

/// The name of [`NotLoaded`].
pub const NOT_LOADED: &str = "none (the solid kernel is not loaded yet)";

/// What a page puts on its documents while it can load a kernel module but
/// has not: every call that needs geometry fails with `hint` (which names
/// the call that loads the kernel), the way the server's kernel would have
/// answered it, instead of storing parts no kernel checked.
pub struct NotLoaded {
    pub hint: String,
}

impl Evaluator for NotLoaded {
    fn name(&self) -> String {
        NOT_LOADED.to_string()
    }

    fn evaluate(
        &self,
        _def: &PartDef,
        _tools: &dyn ToolResolver,
    ) -> Result<Arc<dyn Solid>, EvalError> {
        Err(EvalError::Kernel {
            feature: None,
            message: self.hint.clone(),
        })
    }
}

/// A solid in the module.
pub struct RemoteSolid {
    link: Arc<Link>,
    handle: Handle,
}

impl Drop for RemoteSolid {
    fn drop(&mut self) {
        if let Ok(mut r) = self.link.released.lock() {
            r.push(self.handle);
        }
    }
}

impl Solid for RemoteSolid {
    fn mass(&self) -> Result<MassProps, EvalError> {
        match self.link.ask(&Request::Mass(self.handle))? {
            Reply::Mass(m) => Ok(m.get()),
            other => Err(unexpected("mass", other)),
        }
    }

    fn bbox(&self, frame: &Placement) -> Result<TightBox, EvalError> {
        match self.link.ask(&Request::Bbox(self.handle, *frame))? {
            Reply::Box(b) => Ok(b.get()),
            other => Err(unexpected("a box", other)),
        }
    }

    fn check(&self) -> Result<Validity, EvalError> {
        match self.link.ask(&Request::Check(self.handle))? {
            Reply::Validity(v) => Ok(v.get()),
            other => Err(unexpected("validity", other)),
        }
    }

    fn tessellate(&self, chord: f64) -> Result<TriMesh, EvalError> {
        match self.link.ask(&Request::Tessellate(self.handle, chord))? {
            Reply::Mesh(m) => Ok(m.get()),
            other => Err(unexpected("a mesh", other)),
        }
    }

    fn topology(&self) -> Result<BrepTopology, EvalError> {
        match self.link.ask(&Request::Topology(self.handle))? {
            Reply::Topology(t) => Ok(t),
            other => Err(unexpected("edges and faces", other)),
        }
    }

    fn silhouettes(&self, view: Vec3) -> Result<Silhouettes, EvalError> {
        match self.link.ask(&Request::Silhouettes(self.handle, view))? {
            Reply::Silhouettes(s) => Ok(s),
            other => Err(unexpected("silhouettes", other)),
        }
    }

    fn ray_hits(&self, rays: &[Ray3]) -> Result<Vec<Result<RayHits, EvalError>>, EvalError> {
        match self
            .link
            .ask(&Request::RayHits(self.handle, rays.to_vec()))?
        {
            Reply::RayHits(all) => Ok(all.into_iter().map(|r| r.map_err(|e| e.get())).collect()),
            other => Err(unexpected("ray hits", other)),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::solid::eval::{KernelEvaluator, SolidEnv};
    use crate::solid::fake::FakeKernel;
    use serde_json::{json, Value};

    /// The module, in-process: what a test or a native host would use.
    struct Loopback(Arc<Mutex<KernelServer>>);

    impl Transport for Loopback {
        fn call(&self, request: &[u8]) -> Result<Vec<u8>, String> {
            Ok(self.0.lock().unwrap().handle(request))
        }
    }

    fn server() -> Arc<Mutex<KernelServer>> {
        Arc::new(Mutex::new(KernelServer::new(Arc::new(
            KernelEvaluator::new(FakeKernel),
        ))))
    }

    fn call(d: &mut Document, method: &str, args: Value) -> Value {
        let out = crate::cad_call(d, method, &args.to_string())
            .unwrap_or_else(|e| panic!("{method} {args}: {e}"));
        serde_json::from_str(&out).unwrap()
    }

    fn build(d: &mut Document) {
        let plate = call(
            d,
            "extrude",
            json!({"profile": {"min": [0, 0], "max": [100, 50]}, "distance": 18}),
        );
        let pin = call(
            d,
            "extrude",
            json!({"profile": {"min": [20, 20], "max": [30, 30]}, "distance": 30, "at": [0, 0, -5]}),
        );
        call(
            d,
            "cut",
            json!({"target": plate["id"], "tool": {"part": pin["id"]}}),
        );
        call(
            d,
            "hole",
            json!({"target": plate["id"], "at": [80, 40, 18], "dia": 6, "depth": "through"}),
        );
        let c = call(
            d,
            "copy3d",
            json!({"target": plate["id"], "move": [0, 120, 0]}),
        );
        call(
            d,
            "rotate3d",
            json!({"target": c["id"], "angle": 30, "axis": [0, 0, 1], "about": [0, 120, 0]}),
        );
    }

    #[test]
    fn a_kernel_behind_bytes_answers_like_the_kernel() {
        let mut direct = Document::new();
        direct.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(FakeKernel)));
        build(&mut direct);

        let srv = server();
        let remote = RemoteEvaluator::connect(Box::new(Loopback(srv.clone()))).unwrap();
        assert_eq!(remote.name(), KernelEvaluator::new(FakeKernel).name());
        let mut page = Document::new();
        page.solids = SolidEnv::new(Arc::new(remote));
        build(&mut page);

        for m in ["parts", "describe"] {
            assert_eq!(
                call(&mut page, m, json!({})),
                call(&mut direct, m, json!({})),
                "{m}"
            );
        }
        let o = json!({"view": "iso", "ground": true});
        let a = crate::solid::scene::parts_scene(&page, &o, (480, 320)).unwrap();
        let b = crate::solid::scene::parts_scene(&direct, &o, (480, 320)).unwrap();
        assert_eq!(a.scene.camera, b.scene.camera);
        for (x, y) in a.scene.items.iter().zip(&b.scene.items) {
            assert_eq!(*x.mesh, *y.mesh);
            assert_eq!(x.placement, y.placement);
        }
        // The page continued definitions from cached prefixes (the hole on
        // top of the cut), through the module.
        assert!(page.solids.get().unwrap().continued() >= 1);
        // A refusal crosses as the same error.
        let bad = json!({"profile": {"min": [0, 0], "max": [0, 5]}, "distance": 5});
        assert_eq!(
            crate::cad_call(&mut page, "extrude", &bad.to_string()),
            crate::cad_call(&mut direct, "extrude", &bad.to_string())
        );
        // Every solid is released once the page lets go of it.
        assert!(srv.lock().unwrap().live() > 0);
        drop(page);
        assert_eq!(srv.lock().unwrap().live(), 0);
    }

    #[test]
    fn mismatched_or_damaged_requests_are_refused() {
        let srv = server();
        let mut s = srv.lock().unwrap();
        let r: Reply =
            bincode::deserialize(&s.handle(&encode(&Request::Hello { wire: 99 }))).unwrap();
        assert!(matches!(r, Reply::Refused(m) if m.contains("99")));
        let r: Reply = bincode::deserialize(&s.handle(b"\xff\xff\xff")).unwrap();
        assert!(matches!(r, Reply::Refused(_)));
        let r: Reply = bincode::deserialize(&s.handle(&encode(&Request::Mass(42)))).unwrap();
        assert!(matches!(r, Reply::Refused(m) if m.contains("42")));
    }

    /// The tripwire: every method of `Solid` and `Evaluator` (eval.rs) has a
    /// request ([`COVERAGE`]) and a proxy method in this file's impls, so a
    /// method added to a trait cannot fall back to its default silently in
    /// the browser.
    #[test]
    fn every_trait_method_crosses_the_boundary() {
        fn methods(src: &str, header: &str) -> Vec<String> {
            let start = src
                .find(header)
                .unwrap_or_else(|| panic!("{header} not found"));
            let body = &src[start..];
            let end = body.find("\n}\n").expect("end of the trait");
            body[..end]
                .lines()
                .filter_map(|l| l.trim_start().strip_prefix("fn "))
                .map(|l| l.split(['(', '<']).next().unwrap().trim().to_string())
                .collect()
        }
        let eval = include_str!("eval.rs");
        let this = include_str!("remote.rs");
        let skip = ["name", "as_any"];
        for (header, proxy) in [
            ("pub trait Solid", "impl Solid for RemoteSolid"),
            ("pub trait Evaluator", "impl Evaluator for RemoteEvaluator"),
        ] {
            let ours = methods(this, proxy);
            for m in methods(eval, header) {
                if skip.contains(&m.as_str()) {
                    continue;
                }
                let variant = COVERAGE
                    .iter()
                    .find(|(f, _)| *f == m)
                    .unwrap_or_else(|| panic!("{header}::{m} has no Request variant in COVERAGE"))
                    .1;
                assert!(
                    this.contains(&format!("Request::{variant}")),
                    "COVERAGE names Request::{variant}, which is never sent"
                );
                assert!(
                    ours.contains(&m),
                    "{proxy} does not implement {m}: the browser would use the trait's default"
                );
            }
        }
    }
}
