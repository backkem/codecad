//! Running a kernel on a part, with every kernel call guarded: a typed
//! error is a refusal, a panic is caught and recorded (and counts as
//! wrong), and an unsupported optional capability is "not applicable".

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
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

/// The environment variable that sets how many threads [`par_map`] uses
/// (`1` runs everything on the calling thread).
pub const THREADS_ENV: &str = "SOLID_HARNESS_THREADS";

/// How many threads [`par_map`] uses: [`THREADS_ENV`] if set, else the
/// machine's parallelism.
pub fn threads() -> usize {
    std::env::var(THREADS_ENV)
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// `items.iter().map(f).collect()`, the items taken one at a time by up to
/// [`threads`] threads (`std::thread::scope`), the results in the items'
/// order whatever the threads did. Items are started in the order given,
/// so put the slow ones first.
///
/// Why this keeps every result the same: each item is a pure function of
/// its input. A part's run builds its own kernel model (arris keeps no
/// process-wide state and starts no threads; cad-kernel's caches change
/// how fast it answers, not what), its sampling is seeded from the case
/// name, and nothing is shared between items but read-only inputs.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let threads = threads().min(n);
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let r = f(&items[i]);
                *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    });
    slots
        .into_iter()
        .map(|s| {
            s.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .expect("every item ran")
        })
        .collect()
}

/// [`par_map`] over `items` started in the order of `cost`, highest first
/// (ties in the items' order), so that the slowest items do not start
/// last; the results are still in the items' order.
pub fn par_map_by_cost<T: Sync, R: Send>(
    items: &[T],
    cost: impl Fn(&T) -> u64,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(cost(&items[i])));
    let done = par_map(&order, |&i| (i, f(&items[i])));
    let mut out: Vec<Option<R>> = (0..items.len()).map(|_| None).collect();
    for (i, r) in done {
        out[i] = Some(r);
    }
    out.into_iter()
        .map(|r| r.expect("every item ran"))
        .collect()
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
#[derive(Clone)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn par_map_keeps_the_items_order() {
        let items: Vec<u64> = (0..1000).collect();
        let want: Vec<u64> = items.iter().map(|x| x * x).collect();
        assert_eq!(par_map(&items, |x| x * x), want);
        // Started by cost (the reverse here), still answered in order.
        assert_eq!(par_map_by_cost(&items, |x| *x, |x| x * x), want);
        assert!(par_map(&[] as &[u64], |x| *x).is_empty());
    }
}
