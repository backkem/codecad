//! Layer 1b: the certified volume enclosure. No closed form and no kernel:
//! the reference alone bounds the volume, `V_lo <= V <= V_hi`.
//!
//! The solid is first expressed in the frame of its first sketch plane
//! ([`Reference::aligned`]: a rigid motion, so the volume is unchanged).
//! The reference's bounding box there is cut into cells by repeated bisection
//! of the longest side, largest cell first. A cell is certified
//!
//! - inside when the reference's interval over the whole cell is `<= 0`
//!   (every point of the closed cell lies in the closed solid; the
//!   boundary has no volume), or when the value at the cell's centre is
//!   below minus the cell's half-diagonal: `|f(c)|` is a lower bound on
//!   the distance from `c` to the boundary, so no boundary point lies in
//!   the ball around `c` that holds the cell, and the cell is on `c`'s
//!   side;
//! - outside by the mirror conditions.
//!
//! A cell whose face coincides with a face of the solid is tried once more
//! shrunk by 1e-6 of its size; if the shrunk box certifies, its volume
//! counts and the thin rest stays undecided.
//!
//! Otherwise it is split. Volumes are summed in interval arithmetic, so
//! `V_lo` (the certified inside cells) and `V_hi` (plus every undecided
//! cell) are guaranteed. Refinement stops at a relative width of
//! [`REL_TARGET`] or after [`BUDGET`] cell evaluations, whichever comes
//! first; the widths reached are reported.
//!
//! The layer, with `slack = 1e-9 |V|` (a kernel reports a rounded float
//! even of an exact volume) and the positional band `A * BAND`:
//!
//! - correct: the stated volume `V` lies in `[V_lo - slack, V_hi + slack]`;
//! - undecided: outside that, but within `A * BAND` of it;
//! - wrong: further out.
//!
//! `BAND = 1e-6` mm is the harness band of design section 8 (two faces
//! closer than that cannot be judged), the same one the continuity layer
//! uses. `A` is a certified upper bound on the area of the solid's
//! boundary, from the recipe alone ([`Reference::area_bound`]), never from
//! anything a kernel reports. A boundary moved by at most `BAND` along
//! its normal changes the volume by at most `A * BAND` (to first order;
//! the second-order term, about `BAND^2` times the edge length, is below
//! the `1e-9` slack on every catalogue part). So a volume outside the
//! enclosure by less than that could come from faces placed within the
//! band, which the harness does not judge; one further out cannot.
//!
//! The enclosure's width is set by the boundary area times the smallest
//! cell the budget reaches, so the layer catches a missing or extra
//! feature (a dropped hole, a swapped op, a lost cavity) but not a
//! 0.01 mm shift; that is the shell test's job.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use crate::interval::{Iv, Iv3};
use crate::layers::closed_form::VOLUME_REL_TOL;
use crate::layers::continuity::BAND;
use crate::outcome::Verdict;
use crate::reference::Reference;
use crate::run::PartRun;

/// Cell evaluations per part (each cell costs one interval evaluation
/// over the cell and at most one at its centre).
pub const BUDGET: usize = 30_000;
/// Stop when `(V_hi - V_lo) <= REL_TARGET * V_lo`.
pub const REL_TARGET: f64 = 1e-4;

/// A certified volume enclosure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Enclosure {
    pub lo: f64,
    pub hi: f64,
    pub evals: usize,
    /// A certified upper bound on the boundary's area (mm^2).
    pub area: f64,
}

impl Enclosure {
    /// The positional band in volume: `area * BAND` (mm^3), rounded up.
    pub fn band(&self) -> f64 {
        (Iv::point(self.area) * BAND).hi
    }

    pub fn rel_width(&self) -> f64 {
        if self.lo > 0.0 {
            (self.hi - self.lo) / self.lo
        } else {
            f64::INFINITY
        }
    }
}

struct Cell {
    lo: [f64; 3],
    hi: [f64; 3],
    vol: f64,
}

impl PartialEq for Cell {
    fn eq(&self, o: &Self) -> bool {
        self.vol == o.vol
    }
}
impl Eq for Cell {}
impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cell {
    fn cmp(&self, o: &Self) -> Ordering {
        self.vol.total_cmp(&o.vol)
    }
}

fn cell_volume(lo: [f64; 3], hi: [f64; 3]) -> Iv {
    let w = |i: usize| Iv::point(hi[i]) - Iv::point(lo[i]);
    w(0) * w(1) * w(2)
}

/// Encloses the reference's volume, or `None` if it cannot bound the
/// solid.
pub fn enclose(r: &Reference, budget: usize, rel_target: f64) -> Option<Enclosure> {
    let r = &r.aligned();
    let b = r.bounds();
    if b.is_empty() {
        return Some(Enclosure {
            lo: 0.0,
            hi: 0.0,
            evals: 0,
            area: 0.0,
        });
    }
    if !b.is_bounded() {
        return None;
    }
    let area = r.area_bound()?;
    let mut heap = BinaryHeap::new();
    let root_vol = cell_volume(b.min, b.max);
    heap.push(Cell {
        lo: b.min,
        hi: b.max,
        vol: root_vol.mid(),
    });
    let mut inside = Iv::ZERO;
    let mut residue = Iv::ZERO;
    let mut inside_f = 0.0;
    let mut unknown_f = root_vol.mid();
    let mut evals = 0;
    while let Some(c) = heap.pop() {
        if evals >= budget || (inside_f > 0.0 && unknown_f <= rel_target * inside_f) {
            heap.push(c);
            break;
        }
        let bx = Iv3::new(
            Iv::new(c.lo[0], c.hi[0]),
            Iv::new(c.lo[1], c.hi[1]),
            Iv::new(c.lo[2], c.hi[2]),
        );
        let vol = cell_volume(c.lo, c.hi);
        evals += 1;
        let f = r.eval(bx);
        let mut verdict = if f.hi <= 0.0 {
            Some(true)
        } else if f.lo >= 0.0 {
            Some(false)
        } else {
            None
        };
        if verdict.is_none() {
            evals += 1;
            let centre = Iv3::point(
                0.5 * (c.lo[0] + c.hi[0]),
                0.5 * (c.lo[1] + c.hi[1]),
                0.5 * (c.lo[2] + c.hi[2]),
            );
            // Half-diagonal from the float centre to the farthest corner,
            // rounded up.
            let mut r2 = Iv::ZERO;
            for i in 0..3 {
                let m = [centre.x, centre.y, centre.z][i];
                let a = (Iv::point(c.hi[i]) - m)
                    .abs()
                    .max((m - Iv::point(c.lo[i])).abs());
                r2 = r2 + a.sqr();
            }
            let rad = r2.sqrt().hi;
            let fc = r.eval(centre);
            if fc.hi < -rad {
                verdict = Some(true);
            } else if fc.lo > rad {
                verdict = Some(false);
            }
        }
        // A cell with a face of the solid on (or a hair inside) its own
        // face never certifies: its interval touches zero. Shrink it by
        // 1e-6 of its extent on every side; if the shrunk box certifies,
        // its volume counts and the thin shell left over stays undecided
        // for good (`residue`), which costs at most 6e-6 of the cell.
        if verdict.is_none() {
            let s = [0, 1, 2].map(|i| 1e-6 * (c.hi[i] - c.lo[i]));
            let lo = [0, 1, 2].map(|i| c.lo[i] + s[i]);
            let hi = [0, 1, 2].map(|i| c.hi[i] - s[i]);
            if (0..3).all(|i| lo[i] >= c.lo[i] && hi[i] <= c.hi[i] && lo[i] < hi[i]) {
                evals += 1;
                let fs = r.eval(Iv3::new(
                    Iv::new(lo[0], hi[0]),
                    Iv::new(lo[1], hi[1]),
                    Iv::new(lo[2], hi[2]),
                ));
                if fs.hi < 0.0 || fs.lo > 0.0 {
                    let inner = cell_volume(lo, hi);
                    let shell = vol - inner;
                    residue = residue + Iv::new(0.0, shell.hi.max(0.0));
                    unknown_f += shell.mid() - vol.mid();
                    if fs.hi < 0.0 {
                        inside = inside + Iv::new(inner.lo, inner.lo);
                        inside_f += inner.mid();
                    }
                    continue;
                }
            }
        }
        unknown_f -= vol.mid();
        match verdict {
            Some(true) => {
                inside = inside + vol;
                inside_f += vol.mid();
            }
            Some(false) => {}
            None => {
                // Split the longest side at its float midpoint: the two
                // children cover the parent exactly.
                let ext = [0, 1, 2].map(|i| c.hi[i] - c.lo[i]);
                let k = (0..3).max_by(|&a, &b| ext[a].total_cmp(&ext[b])).unwrap();
                let m = 0.5 * (c.lo[k] + c.hi[k]);
                if !(m > c.lo[k] && m < c.hi[k]) {
                    // Cannot split further in floats: leave it undecided.
                    heap.push(c);
                    break;
                }
                let mut hi1 = c.hi;
                hi1[k] = m;
                let mut lo2 = c.lo;
                lo2[k] = m;
                for (lo, hi) in [(c.lo, hi1), (lo2, c.hi)] {
                    let v = cell_volume(lo, hi).mid();
                    unknown_f += v;
                    heap.push(Cell { lo, hi, vol: v });
                }
            }
        }
    }
    let mut unknown = Iv::ZERO;
    for c in heap.iter() {
        unknown = unknown + cell_volume(c.lo, c.hi);
    }
    Some(Enclosure {
        lo: inside.lo.max(0.0),
        hi: (inside + unknown + residue).hi,
        evals,
        area,
    })
}

/// The layer verdict for a part, given its enclosure.
pub fn check<B>(run: &PartRun<B>, enc: Option<&Enclosure>) -> Verdict {
    let Some(enc) = enc else {
        return match &run.reference {
            Err(e) => Verdict::na(format!("reference {e}")),
            Ok(_) => Verdict::na("the reference cannot bound this solid"),
        };
    };
    if let Err(f) = &run.brep {
        return super::closed_form::fail_verdict(f);
    }
    match &run.mass {
        Ok(m) => {
            let width = format!(
                "[{:.10e}, {:.10e}], rel width {:.1e}",
                enc.lo,
                enc.hi,
                enc.rel_width()
            );
            // The kernel reports a rounded float of its volume: allow the
            // harness-wide relative volume tolerance on either side.
            let slack = VOLUME_REL_TOL * m.volume.abs();
            let out = (enc.lo - slack - m.volume).max(m.volume - enc.hi - slack);
            if out <= 0.0 {
                Verdict::correct(format!("volume {:.10e} in {width}", m.volume))
            } else if out <= enc.band() {
                Verdict::undecided(format!(
                    "volume {} outside {width} by {out:.3e}, within the positional band {:.3e} \
                     (area bound {:.4e} mm^2 times {BAND:e} mm)",
                    m.volume,
                    enc.band(),
                    enc.area
                ))
            } else {
                Verdict::wrong(format!(
                    "volume {} outside {width} by {out:.3e}, beyond the positional band {:.3e}",
                    m.volume,
                    enc.band()
                ))
            }
        }
        Err(f) => super::closed_form::fail_verdict(f),
    }
}
