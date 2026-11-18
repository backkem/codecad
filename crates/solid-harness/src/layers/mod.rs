//! The evidence layers and the runner that applies them to the catalogue.

pub mod algebra;
pub mod closed_form;
pub mod continuity;
pub mod enclosure;
pub mod equivalence;
pub mod mesh_invariants;
pub mod shell;
pub mod validity;

use std::collections::HashMap;
use std::time::Instant;

use cadview_core::solid::SolidKernel;

use crate::cases::{Catalogue, Relation};
use crate::outcome::{overall, Layer, Outcome, Verdict};
use crate::reference::Reference;
use crate::run::{run_part, PartRun};

use enclosure::Enclosure;

/// One scoreboard row.
#[derive(Debug, Clone)]
pub struct Row {
    pub name: String,
    /// One verdict per [`Layer::ALL`] entry.
    pub verdicts: Vec<Verdict>,
    pub overall: Outcome,
}

impl Row {
    pub fn verdict(&self, l: Layer) -> &Verdict {
        &self.verdicts[Layer::ALL.iter().position(|x| *x == l).unwrap()]
    }
}

/// A whole run of the catalogue on one kernel.
#[derive(Debug, Clone)]
pub struct Report {
    pub kernel: String,
    pub rows: Vec<Row>,
    pub seconds: f64,
}

impl Report {
    pub fn row(&self, name: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.name == name)
    }

    /// Counts per layer and outcome.
    pub fn counts(&self) -> Vec<(Layer, Vec<(Outcome, usize)>)> {
        Layer::ALL
            .iter()
            .map(|&l| {
                let mut c: Vec<(Outcome, usize)> = [
                    Outcome::Correct,
                    Outcome::Refused,
                    Outcome::Undecided,
                    Outcome::Wrong,
                    Outcome::NotApplicable,
                ]
                .iter()
                .map(|&o| (o, 0))
                .collect();
                for r in &self.rows {
                    let o = r.verdict(l).outcome;
                    c.iter_mut().find(|x| x.0 == o).unwrap().1 += 1;
                }
                (l, c)
            })
            .collect()
    }
}

/// The certified enclosures of every part the reference can bound, and
/// the closed forms that fall outside their enclosure (a reference or
/// closed-form bug, never a kernel one).
pub struct Enclosures {
    pub map: HashMap<String, Option<Enclosure>>,
    pub closed_form_violations: Vec<String>,
    pub seconds: f64,
}

/// Computes every enclosure once; they depend only on the recipes.
pub fn enclosures(cat: &Catalogue) -> Enclosures {
    let t0 = Instant::now();
    let mut map = HashMap::new();
    let mut violations = Vec::new();
    for p in &cat.parts {
        let enc = Reference::build(&p.def, &p.tools)
            .ok()
            .and_then(|r| enclosure::enclose(&r, enclosure::BUDGET, enclosure::REL_TARGET));
        if let (Some(e), Some(v)) = (&enc, p.volume) {
            if !(e.lo <= v && v <= e.hi) {
                violations.push(format!(
                    "{}: closed form {v} outside the certified [{}, {}]",
                    p.name, e.lo, e.hi
                ));
            }
        }
        map.insert(p.name.clone(), enc);
    }
    Enclosures {
        map,
        closed_form_violations: violations,
        seconds: t0.elapsed().as_secs_f64(),
    }
}

fn na_row(name: String, layer: Layer, v: Verdict) -> Row {
    let verdicts: Vec<Verdict> = Layer::ALL
        .iter()
        .map(|&l| {
            if l == layer {
                v.clone()
            } else {
                Verdict::na("")
            }
        })
        .collect();
    let overall = overall(&verdicts, false);
    Row {
        name,
        verdicts,
        overall,
    }
}

/// Runs the whole catalogue on `kernel`.
pub fn run_all<K: SolidKernel>(kernel: &K, cat: &Catalogue, enc: &Enclosures) -> Report {
    let t0 = Instant::now();
    let runs: Vec<PartRun<K::Brep>> = cat
        .parts
        .iter()
        .map(|p| run_part(kernel, p, true))
        .collect();
    let by_name: HashMap<&str, &PartRun<K::Brep>> =
        runs.iter().map(|r| (r.case.name.as_str(), r)).collect();
    let mut rows = Vec::new();
    for r in &runs {
        let e = enc.map.get(&r.case.name).and_then(|e| e.as_ref());
        let verdicts: Vec<Verdict> = Layer::ALL
            .iter()
            .map(|l| match l {
                Layer::ClosedForm => closed_form::check(r),
                Layer::Enclosure => enclosure::check(r, e),
                Layer::Shell => shell::check(r),
                Layer::Mesh => mesh_invariants::check(r),
                Layer::Validity => validity::check(r, e.is_some_and(|e| e.lo == 0.0)),
                _ => Verdict::na(""),
            })
            .collect();
        let mut overall = overall(&verdicts, r.refused());
        if r.panicked().is_some() {
            overall = Outcome::Wrong;
        }
        rows.push(Row {
            name: format!("part: {}", r.case.name),
            verdicts,
            overall,
        });
    }
    let get = |n: &str| *by_name.get(n).unwrap_or_else(|| panic!("no part {n}"));
    for rel in &cat.relations {
        let row = match rel {
            Relation::Equivalent { name, a, b } => na_row(
                format!("equiv: {name}"),
                Layer::Equivalence,
                equivalence::check(kernel, name, get(a), get(b)),
            ),
            Relation::Same { name, a, b } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::same(kernel, name, get(a), get(b)),
            ),
            Relation::UnionIdentity {
                name,
                a,
                b,
                union,
                inter,
            } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::union_identity(get(a), get(b), get(union), get(inter)),
            ),
            Relation::Empty { name, part, of } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::empty(get(part), get(of)),
            ),
            Relation::Motion {
                name,
                base,
                moved,
                motion,
            } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::moved(
                    kernel,
                    name,
                    get(base),
                    get(moved),
                    equivalence::Map::Motion(*motion),
                ),
            ),
            Relation::Mirror {
                name,
                base,
                mirrored,
                x0,
            } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::moved(
                    kernel,
                    name,
                    get(base),
                    get(mirrored),
                    equivalence::Map::Mirror(*x0),
                ),
            ),
            Relation::Split {
                name,
                whole,
                pieces,
            } => na_row(
                format!("algebra: {name}"),
                Layer::Algebra,
                algebra::split(get(whole), get(&pieces[0]), get(&pieces[1])),
            ),
        };
        rows.push(row);
    }
    for sw in &cat.sweeps {
        let steps = continuity::evaluate(kernel, sw);
        rows.push(na_row(
            format!("sweep: {}", sw.name),
            Layer::Continuity,
            continuity::check(sw, &steps),
        ));
    }
    Report {
        kernel: kernel.name(),
        rows,
        seconds: t0.elapsed().as_secs_f64(),
    }
}
