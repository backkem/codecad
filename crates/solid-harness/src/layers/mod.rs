//! The evidence layers and the runner that applies them to the catalogue.

pub mod algebra;
pub mod closed_form;
pub mod continuity;
pub mod enclosure;
pub mod equivalence;
pub mod mesh_invariants;
pub mod shell;
pub mod validity;

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Instant;

use cadview_core::solid::{Feature, SolidKernel};

use crate::cases::{Catalogue, PartCase, Relation};
use crate::outcome::{overall, Layer, Outcome, Verdict};
use crate::reference::Reference;
use crate::run::{par_map, par_map_by_cost, run_part, PartRun};

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

/// Computes every enclosure once; they depend only on the recipes. The
/// parts are enclosed in parallel ([`par_map`]); each is a pure function
/// of its recipe.
pub fn enclosures(cat: &Catalogue) -> Enclosures {
    let t0 = Instant::now();
    let mut map = HashMap::new();
    let mut violations = Vec::new();
    let encs = par_map(&cat.parts, |p| {
        Reference::build(&p.def, &p.tools)
            .ok()
            .and_then(|r| enclosure::enclose(&r, enclosure::BUDGET, enclosure::REL_TARGET))
    });
    for (p, enc) in cat.parts.iter().zip(encs) {
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

/// One line per part with an enclosure: its volume (the enclosure's
/// middle), the certified area bound, the positional band `A * BAND`, and
/// the band and the enclosure's width relative to the volume.
pub fn band_table(cat: &Catalogue, enc: &Enclosures) -> String {
    let mut s = format!(
        "{:<64} {:>14} {:>12} {:>10} {:>9} {:>9}\n",
        "part", "volume", "area bound", "band", "band/V", "width/V"
    );
    for p in &cat.parts {
        let Some(Some(e)) = enc.map.get(&p.name) else {
            continue;
        };
        let v = 0.5 * (e.lo + e.hi);
        let rel = |x: f64| if v > 0.0 { x / v } else { f64::INFINITY };
        s.push_str(&format!(
            "{:<64} {:>14.6e} {:>12.4e} {:>10.3e} {:>9.1e} {:>9.1e}\n",
            p.name.chars().take(64).collect::<String>(),
            v,
            e.area,
            e.band(),
            rel(e.band()),
            rel(e.hi - e.lo)
        ));
    }
    s
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

/// The row of one part: the layers that judge a part on its own.
pub fn part_row<B>(r: &PartRun<B>, enc: &Enclosures) -> Row {
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
    Row {
        name: format!("part: {}", r.case.name),
        verdicts,
        overall,
    }
}

/// A rough order of a part's cost, to start the slow parts first: curved
/// features tessellate into many triangles at the finest chord.
pub fn part_cost(case: &PartCase) -> u64 {
    std::iter::once(&case.def)
        .chain(&case.tools.0)
        .flat_map(|d| &d.features)
        .map(|f| match f {
            Feature::Revolve { .. }
            | Feature::Sweep { .. }
            | Feature::Loft { .. }
            | Feature::Fillet { .. } => 100,
            Feature::Hole { .. } => 10,
            _ => 1,
        })
        .sum()
}

/// Where a part's run comes from (see [`run_parts_with`]).
pub enum Prepared<'a, B> {
    /// Run the part on the kernel.
    Run,
    /// The run and row are these, unchanged (borrowed, not copied).
    Same(&'a PartRun<B>, &'a Row),
    /// The run is this one; its row is computed from it.
    Derived(Box<PartRun<B>>),
}

/// The parts' runs (borrowed when taken over unchanged) and rows, in the
/// catalogue's order.
pub type PartRuns<'a, B> = (Vec<Cow<'a, PartRun<B>>>, Vec<Row>);

/// Every part of the catalogue on `kernel` with its row, in the
/// catalogue's order. Parts run in parallel ([`par_map_by_cost`]).
/// `prepared(i, case)` says where part `i`'s run comes from: the broken
/// adapters hand over the runs their mutation provably does not change
/// ([`crate::kernels::broken::Broken::derive`]).
pub fn run_parts_with<'a, K>(
    kernel: &K,
    cat: &Catalogue,
    enc: &Enclosures,
    prepared: impl Fn(usize, &PartCase) -> Prepared<'a, K::Brep> + Sync,
) -> PartRuns<'a, K::Brep>
where
    K: SolidKernel + Sync,
    K::Brep: Send + Sync,
{
    let indexed: Vec<(usize, &PartCase)> = cat.parts.iter().enumerate().collect();
    par_map_by_cost(
        &indexed,
        |(_, p)| part_cost(p),
        |&(i, p)| match prepared(i, p) {
            Prepared::Same(run, row) => (Cow::Borrowed(run), row.clone()),
            Prepared::Derived(run) => {
                let row = part_row(&run, enc);
                (Cow::Owned(*run), row)
            }
            Prepared::Run => {
                let run = run_part(kernel, p, true);
                let row = part_row(&run, enc);
                (Cow::Owned(run), row)
            }
        },
    )
    .into_iter()
    .unzip()
}

/// Runs the whole catalogue on `kernel`.
pub fn run_all<K>(kernel: &K, cat: &Catalogue, enc: &Enclosures) -> Report
where
    K: SolidKernel + Sync,
    K::Brep: Send + Sync,
{
    let t0 = Instant::now();
    let (runs, rows) = run_parts_with(kernel, cat, enc, |_, _| Prepared::Run);
    finish(kernel, cat, &runs, rows, t0)
}

/// The report from the parts' runs and rows: adds the relation and the
/// sweep rows (both in parallel, in the catalogue's order).
pub fn finish<K>(
    kernel: &K,
    cat: &Catalogue,
    runs: &[Cow<'_, PartRun<K::Brep>>],
    rows: Vec<Row>,
    t0: Instant,
) -> Report
where
    K: SolidKernel + Sync,
    K::Brep: Send + Sync,
{
    finish_with(kernel, cat, runs, rows, t0, |_| None, |_| None)
}

/// [`finish`], where `relation(i)` and `sweep(i)` may hand over the row of
/// relation or sweep `i` instead of computing it (the broken adapters do
/// for the ones their mutation provably does not change).
pub fn finish_with<K>(
    kernel: &K,
    cat: &Catalogue,
    runs: &[Cow<'_, PartRun<K::Brep>>],
    mut rows: Vec<Row>,
    t0: Instant,
    relation: impl Fn(usize) -> Option<Row> + Sync,
    sweep: impl Fn(usize) -> Option<Row> + Sync,
) -> Report
where
    K: SolidKernel + Sync,
    K::Brep: Send + Sync,
{
    let by_name: HashMap<&str, &PartRun<K::Brep>> = runs
        .iter()
        .map(|r| (r.case.name.as_str(), r.as_ref()))
        .collect();
    let get = |n: &str| *by_name.get(n).unwrap_or_else(|| panic!("no part {n}"));
    let relations: Vec<(usize, &Relation)> = cat.relations.iter().enumerate().collect();
    rows.extend(par_map(&relations, |&(i, rel)| {
        if let Some(row) = relation(i) {
            return row;
        }
        match rel {
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
        }
    }));
    // Every step of every sweep is its own item.
    let given: Vec<Option<Row>> = (0..cat.sweeps.len()).map(&sweep).collect();
    let steps: Vec<(usize, &(f64, PartCase))> = cat
        .sweeps
        .iter()
        .enumerate()
        .filter(|(i, _)| given[*i].is_none())
        .flat_map(|(i, sw)| sw.steps.iter().map(move |s| (i, s)))
        .collect();
    let mut evaluated: Vec<Vec<continuity::Step>> = vec![Vec::new(); cat.sweeps.len()];
    for (i, step) in steps.iter().zip(par_map(&steps, |(_, (s, case))| {
        continuity::evaluate_step(kernel, *s, case)
    })) {
        evaluated[i.0].push(step);
    }
    for ((sw, steps), given) in cat.sweeps.iter().zip(&evaluated).zip(given) {
        rows.push(given.unwrap_or_else(|| {
            na_row(
                format!("sweep: {}", sw.name),
                Layer::Continuity,
                continuity::check(sw, steps),
            )
        }));
    }
    Report {
        kernel: kernel.name(),
        rows,
        seconds: t0.elapsed().as_secs_f64(),
    }
}
