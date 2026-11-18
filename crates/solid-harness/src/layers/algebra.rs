//! Layer 6: algebra and motion.
//!
//! - `A + A = A`, `A n A = A`, operand order: [`super::equivalence::compare`]
//!   with no map.
//! - Rigid motion of the whole recipe: volume equal, the moved solid's box
//!   equal to the kernel's box of the original under the same placement,
//!   centroid moved, membership moved.
//! - Mirroring in `x = x0`: the same with the mirror.
//! - Union identity `V(A u B) + V(A n B) = V(A) + V(B)`, to `1e-9` of
//!   `V(A) + V(B)`.
//! - `A - A` is empty: the kernel refuses (safe) or returns a volume at
//!   most `1e-9 V(A)`; a larger volume is wrong.
//! - Splitting by a plane preserves volume: `V(A n H) + V(A - H) = V(A)`
//!   to `1e-9 V(A)`.
//!
//! The `1e-9` relative tolerance is the closed-form layer's, for the same
//! reason: exact integration leaves only rounding.

use cadview_core::solid::SolidKernel;

use crate::outcome::Verdict;
use crate::run::PartRun;

use super::closed_form::fail_verdict;
use super::equivalence::{compare, eval_failures, tidy, Map};

pub const REL_TOL: f64 = 1e-9;

pub fn same<K: SolidKernel>(
    kernel: &K,
    name: &str,
    a: &PartRun<K::Brep>,
    b: &PartRun<K::Brep>,
) -> Verdict {
    tidy(compare(kernel, a, b, Map::Same, name))
}

pub fn moved<K: SolidKernel>(
    kernel: &K,
    name: &str,
    base: &PartRun<K::Brep>,
    other: &PartRun<K::Brep>,
    map: Map,
) -> Verdict {
    tidy(compare(kernel, base, other, map, name))
}

fn volumes<B>(runs: &[&PartRun<B>]) -> Result<Vec<f64>, Verdict> {
    if let Some(v) = eval_failures(runs) {
        return Err(v);
    }
    runs.iter()
        .map(|r| r.mass.as_ref().map(|m| m.volume).map_err(fail_verdict))
        .collect()
}

pub fn union_identity<B>(
    a: &PartRun<B>,
    b: &PartRun<B>,
    union: &PartRun<B>,
    inter: &PartRun<B>,
) -> Verdict {
    let v = match volumes(&[a, b, union, inter]) {
        Ok(v) => v,
        Err(v) => return v,
    };
    let (va, vb, vu, vi) = (v[0], v[1], v[2], v[3]);
    let err = (vu + vi - va - vb).abs();
    let scale = va.abs() + vb.abs();
    if err <= REL_TOL * scale {
        Verdict::correct(format!("V(AuB) {vu:.6} + V(AnB) {vi:.6} = V(A) + V(B)"))
    } else {
        Verdict::wrong(format!(
            "V(AuB) {vu} + V(AnB) {vi} != V(A) {va} + V(B) {vb}: off by {:.1e} relative",
            err / scale
        ))
    }
}

pub fn empty<B>(part: &PartRun<B>, of: &PartRun<B>) -> Verdict {
    let base = match volumes(&[of]) {
        Ok(v) => v[0],
        Err(v) => return v,
    };
    match &part.brep {
        Err(f) => fail_verdict(f),
        Ok(_) => match &part.mass {
            Ok(m) if m.volume.abs() <= REL_TOL * base.abs() => {
                Verdict::correct(format!("A - A has volume {:.1e}", m.volume))
            }
            Ok(m) => Verdict::wrong(format!("A - A has volume {} (A has {base})", m.volume)),
            Err(f) => fail_verdict(f),
        },
    }
}

pub fn split<B>(whole: &PartRun<B>, p0: &PartRun<B>, p1: &PartRun<B>) -> Verdict {
    let v = match volumes(&[whole, p0, p1]) {
        Ok(v) => v,
        Err(v) => return v,
    };
    let err = (v[1] + v[2] - v[0]).abs();
    if err <= REL_TOL * v[0].abs() {
        Verdict::correct(format!("{:.6} + {:.6} = {:.6}", v[1], v[2], v[0]))
    } else {
        Verdict::wrong(format!(
            "pieces {} + {} != whole {}: off by {:.1e} relative",
            v[1],
            v[2],
            v[0],
            err / v[0].abs()
        ))
    }
}
