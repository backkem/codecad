//! Running a kernel on a part, with every kernel call guarded: a typed
//! error is a refusal, a panic is caught and recorded (and counts as
//! wrong), and an unsupported optional capability is "not applicable".

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use cadview_core::solid::{Aabb3, EvalError, MassProps, Placement, SolidKernel, TriMesh, Validity};

use crate::cases::PartCase;
use crate::reference::{NotDefined, Reference};

/// Chord tolerances (mm) the mesh layers ask for, coarse to fine. The
/// finest is the one the shell test samples.
pub const CHORDS: [f64; 4] = [1e-1, 1e-2, 1e-3, 1e-4];
/// The chord of the mesh the membership comparisons (layers 5 and 6) use:
/// fine enough that a point 2e-3 mm off the surface is classified right,
/// coarse enough that a winding number over all triangles is cheap.
pub const MEMBERSHIP_CHORD: f64 = 1e-3;

/// Why a kernel call gave no value.
#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    /// A typed error.
    Refused(String),
    /// `EvalError::Unsupported`: the capability or feature is missing.
    Unsupported(String),
    /// The kernel panicked.
    Panic(String),
}

impl Fail {
    pub fn text(&self) -> String {
        match self {
            Fail::Refused(s) => format!("refused: {s}"),
            Fail::Unsupported(s) => format!("unsupported: {s}"),
            Fail::Panic(s) => format!("PANIC: {s}"),
        }
    }
}

/// Runs a kernel call, catching panics.
pub fn guard<T>(f: impl FnOnce() -> Result<T, EvalError>) -> Result<T, Fail> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e @ EvalError::Unsupported { .. })) => Err(Fail::Unsupported(e.to_string())),
        Ok(Err(e)) => Err(Fail::Refused(e.to_string())),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "a panic without a message".into());
            Err(Fail::Panic(msg))
        }
    }
}

/// Everything the layers need about one part on one kernel.
pub struct PartRun<B> {
    pub case: PartCase,
    pub reference: Result<Reference, NotDefined>,
    /// The evaluated B-rep. An `Unsupported` evaluation is a refusal.
    pub brep: Result<B, Fail>,
    pub mass: Result<MassProps, Fail>,
    pub bbox: Result<Aabb3, Fail>,
    pub validity: Result<Validity, Fail>,
    /// One mesh per entry of [`CHORDS`].
    pub meshes: Vec<(f64, Result<TriMesh, Fail>)>,
    pub eval_ms: f64,
}

impl<B> PartRun<B> {
    /// Whether the kernel refused the recipe itself (not a panic).
    pub fn refused(&self) -> bool {
        matches!(self.brep, Err(Fail::Refused(_) | Fail::Unsupported(_)))
    }

    pub fn panicked(&self) -> Option<&str> {
        match &self.brep {
            Err(Fail::Panic(s)) => Some(s),
            _ => None,
        }
    }

    pub fn mesh(&self, chord: f64) -> Option<&Result<TriMesh, Fail>> {
        self.meshes.iter().find(|m| m.0 == chord).map(|m| &m.1)
    }
}

/// Evaluates a case on a kernel. `with_meshes` asks for the tessellations.
pub fn run_part<K: SolidKernel>(
    kernel: &K,
    case: &PartCase,
    with_meshes: bool,
) -> PartRun<K::Brep> {
    let reference = Reference::build(&case.def, case.tools.as_slice());
    let t0 = Instant::now();
    let brep = guard(|| {
        kernel
            .evaluate(&case.def, case.tools.as_slice())
            .map(|e| e.brep)
    });
    let eval_ms = t0.elapsed().as_secs_f64() * 1e3;
    let not_built = || Fail::Refused("not evaluated".into());
    let (mass, bbox, validity, meshes) = match &brep {
        Ok(b) => {
            let mass = guard(|| kernel.mass(b));
            let bbox = guard(|| kernel.bbox(b, &Placement::IDENTITY));
            let validity = guard(|| kernel.check(b));
            let meshes = if with_meshes {
                CHORDS
                    .iter()
                    .map(|&c| (c, guard(|| kernel.tessellate(b, c))))
                    .collect()
            } else {
                Vec::new()
            };
            (mass, bbox, validity, meshes)
        }
        Err(_) => (
            Err(not_built()),
            Err(not_built()),
            Err(not_built()),
            Vec::new(),
        ),
    };
    PartRun {
        case: case.clone(),
        reference,
        brep,
        mass,
        bbox,
        validity,
        meshes,
        eval_ms,
    }
}
