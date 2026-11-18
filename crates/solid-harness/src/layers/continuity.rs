//! Layer 7: continuity sweeps. A tool moves through a flush contact or a
//! tangency in steps that shrink geometrically to 1e-9 mm on both sides
//! ([`crate::cases::sweep_params`]). Between consecutive evaluated steps
//! the volume change must be continuous and in the expected direction.
//!
//! With `L` the sweep's slope bound (the largest area the moving tool
//! sweeps per unit of travel, derived in the catalogue), `A` a bound on
//! the part's surface area, the harness band `BAND = 1e-6` mm (two faces
//! closer than this cannot be judged, design section 8) and `noise = 1e-12
//! V` (exact integration rounding):
//!
//! - `excess = |dV| - L |ds|`: at most `noise` is correct, at most
//!   `noise + A * BAND` (a face moved within the band changes the volume by
//!   at most its area times the band) is undecided, more is wrong (a jump:
//!   a topology bug).
//! - against the expected direction: at most `noise` is correct, at most
//!   `noise + A * BAND` undecided, more wrong.
//!
//! A refused step is skipped (the sweep is then refused at best); a panic
//! is wrong.

use cadview_core::solid::SolidKernel;

use crate::cases::{PartCase, Sweep};
use crate::outcome::Verdict;
use crate::run::{guard, Fail};

pub const BAND: f64 = 1e-6;

/// One evaluated step.
#[derive(Debug, Clone)]
pub struct Step {
    pub s: f64,
    pub volume: Result<f64, Fail>,
}

pub fn evaluate<K: SolidKernel>(kernel: &K, sweep: &Sweep) -> Vec<Step> {
    sweep
        .steps
        .iter()
        .map(|(s, case)| evaluate_step(kernel, *s, case))
        .collect()
}

/// One step: the recipe at `s` evaluated and measured.
pub fn evaluate_step<K: SolidKernel>(kernel: &K, s: f64, case: &PartCase) -> Step {
    let volume = guard(|| {
        let b = kernel.evaluate(&case.def, &case.tools)?.brep;
        Ok(kernel.mass(&b)?.volume)
    });
    Step { s, volume }
}

pub fn check(sweep: &Sweep, steps: &[Step]) -> Verdict {
    let mut parts = Vec::new();
    let mut refused = Vec::new();
    let mut ok: Vec<(f64, f64)> = Vec::new();
    for st in steps {
        match &st.volume {
            Ok(v) => ok.push((st.s, *v)),
            Err(Fail::Panic(m)) => {
                parts.push(Verdict::wrong(format!("s = {:e}: panic: {m}", st.s)))
            }
            Err(Fail::Refused(m) | Fail::Unsupported(m)) => {
                refused.push(format!("s = {:e}: {m}", st.s));
            }
        }
    }
    let vmax = ok.iter().map(|x| x.1.abs()).fold(0.0, f64::max);
    let noise = 1e-12 * vmax;
    let band = noise + sweep.area * BAND;
    let mut worst_excess: f64 = 0.0;
    for w in ok.windows(2) {
        let ((s0, v0), (s1, v1)) = (w[0], w[1]);
        let dv = v1 - v0;
        let ds = s1 - s0;
        let excess = dv.abs() - sweep.slope * ds.abs();
        worst_excess = worst_excess.max(excess);
        let against = -dv * sweep.direction;
        for (what, amount) in [("jump", excess), ("wrong direction", against)] {
            if amount > band {
                parts.push(Verdict::wrong(format!(
                    "{what} from s = {s0:e} to {s1:e}: V {v0} -> {v1} (dV {dv:.3e}, allowed {:.3e})",
                    sweep.slope * ds.abs()
                )));
            } else if amount > noise {
                parts.push(Verdict::undecided(format!(
                    "{what} within the band from s = {s0:e} to {s1:e}: dV {dv:.3e}"
                )));
            }
        }
    }
    if ok.len() < 2 && parts.is_empty() {
        parts.push(Verdict::refused(format!(
            "fewer than two steps evaluated; {}",
            refused.first().cloned().unwrap_or_default()
        )));
    } else if !refused.is_empty() {
        parts.push(Verdict::refused(format!(
            "{} of {} steps refused, first {}",
            refused.len(),
            steps.len(),
            refused[0]
        )));
    } else if parts.is_empty() {
        parts.push(Verdict::correct(format!(
            "{} steps continuous and monotone",
            ok.len()
        )));
    }
    Verdict::combine(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cases::catalogue;

    fn steps(vs: &[(f64, f64)]) -> Vec<Step> {
        vs.iter().map(|&(s, v)| Step { s, volume: Ok(v) }).collect()
    }

    #[test]
    fn jumps_and_reversals_are_caught() {
        let sw = catalogue().sweeps.into_iter().next().unwrap();
        // V = 1000 - 400 max(s, 0): fine.
        let good = steps(&[(-1.0, 1000.0), (0.0, 1000.0), (1e-3, 999.6), (1.0, 600.0)]);
        assert_eq!(check(&sw, &good).outcome, crate::outcome::Outcome::Correct);
        // A jump of a whole 1 mm slab at a 1e-3 step.
        let jump = steps(&[(-1.0, 1000.0), (0.0, 1000.0), (1e-3, 600.0)]);
        assert_eq!(check(&sw, &jump).outcome, crate::outcome::Outcome::Wrong);
        // Volume growing while the tool cuts deeper.
        let rev = steps(&[(0.0, 1000.0), (1.0, 1001.0)]);
        assert_eq!(check(&sw, &rev).outcome, crate::outcome::Outcome::Wrong);
    }
}
