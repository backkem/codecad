//! Layer 1: closed forms. The kernel's volume and bounding box against the
//! hand-derived ones, and the kernel's own validity check (one valid
//! solid), as the 3D0a gate had it.
//!
//! Tolerances: volume `1e-9` relative and box `1e-9` mm, the corpus gate's
//! ([`cadview_core::solid::corpus::prismatic::VOLUME_REL_TOL`]). A kernel
//! that integrates on exact surfaces is off by rounding only (arris: about
//! 1e-16 relative), so 1e-9 leaves seven orders of magnitude of slack for
//! rounding while still catching a hole 0.01 mm too wide in any corpus
//! part (that changes the volume by more than 1e-7 relative).

pub use cadview_core::solid::corpus::prismatic::{BBOX_TOL, VOLUME_REL_TOL};

use crate::outcome::Verdict;
use crate::run::{Fail, PartRun};

pub fn check<B>(run: &PartRun<B>) -> Verdict {
    let (Some(want_v), Some(want_b)) = (run.case.volume, run.case.bbox) else {
        return Verdict::na("no closed form");
    };
    if let Err(f) = &run.brep {
        return fail_verdict(f);
    }
    let mut parts = Vec::new();
    match &run.validity {
        Ok(v) if v.valid && v.solids == 1 => {}
        Ok(v) => parts.push(Verdict::wrong(format!(
            "kernel check: valid={} solids={} {:?}",
            v.valid,
            v.solids,
            v.problems.iter().take(3).collect::<Vec<_>>()
        ))),
        Err(f) => parts.push(fail_verdict(f)),
    }
    match &run.mass {
        Ok(m) => {
            let rel = (m.volume - want_v).abs() / want_v.abs();
            if rel <= VOLUME_REL_TOL {
                parts.push(Verdict::correct(""));
            } else {
                parts.push(Verdict::wrong(format!(
                    "volume {} vs closed form {want_v} (rel err {rel:.1e})",
                    m.volume
                )));
            }
        }
        Err(f) => parts.push(fail_verdict(f)),
    }
    match &run.bbox {
        Ok(b) => {
            let e = b.max_abs_diff(&want_b);
            if e > BBOX_TOL {
                parts.push(Verdict::wrong(format!(
                    "bounding box off by {e:.1e} mm: {b:?}"
                )));
            }
        }
        Err(f) => parts.push(fail_verdict(f)),
    }
    Verdict::combine(parts)
}

/// The verdict for a kernel call that gave no value.
pub fn fail_verdict(f: &Fail) -> Verdict {
    match f {
        Fail::Panic(s) => Verdict::wrong(format!("kernel panicked: {s}")),
        Fail::Refused(s) | Fail::Unsupported(s) => Verdict::refused(s.clone()),
    }
}
