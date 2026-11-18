//! The kernel's own validity check, on every part (the closed-form layer
//! checks it too, but only where a closed form exists).
//!
//! Correct: the kernel's check passes and the result has at least one
//! solid, or none where the reference certifies no material (its volume
//! enclosure's lower bound is 0, as for `A - A`: interval arithmetic
//! cannot certify that set empty, only that nothing in it is inside).
//! Wrong: the kernel returned a B-rep its own check rejects, or no solid
//! where the reference certifies material. This is weaker evidence than
//! the reference layers (a kernel's checker can be wrong both ways), but
//! a kernel that calls its own result invalid has returned a wrong solid.

use crate::outcome::Verdict;
use crate::run::PartRun;

use super::closed_form::fail_verdict;

/// `no_material`: the reference certifies no material (enclosure lower
/// bound 0).
pub fn check<B>(run: &PartRun<B>, no_material: bool) -> Verdict {
    if let Err(f) = &run.brep {
        return fail_verdict(f);
    }
    match &run.validity {
        Ok(v) if v.valid && (v.solids >= 1 || no_material) => {
            Verdict::correct(format!("valid, {} solids", v.solids))
        }
        Ok(v) => Verdict::wrong(format!(
            "kernel check: valid={} solids={} {:?}",
            v.valid,
            v.solids,
            v.problems.iter().take(2).collect::<Vec<_>>()
        )),
        Err(f) => fail_verdict(f),
    }
}
