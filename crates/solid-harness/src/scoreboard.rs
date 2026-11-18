//! The scoreboard file: one line per case with its overall outcome and its
//! outcome per layer, plus comment lines with the reason for every
//! outcome that is not correct. It is a ratchet (design section 5): wrong
//! must be zero, and no case or layer may move away from correct.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::layers::Report;
use crate::outcome::{Layer, Outcome};

/// The environment variable that rewrites the committed scoreboard.
pub const BLESS_ENV: &str = "SOLID_HARNESS_BLESS";

/// Renders a report as the committed file.
pub fn render(report: &Report) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# Solid harness scoreboard (docs/3d-test-harness.md section 5)."
    );
    let _ = writeln!(s, "# kernel: {}", report.kernel);
    let _ = writeln!(
        s,
        "# Regenerate: {BLESS_ENV}=1 cargo test -p solid-harness --release --test scoreboard"
    );
    let _ = writeln!(
        s,
        "# Outcomes: correct, refused (typed error), undecided (inside the band), WRONG, n/a."
    );
    let _ = writeln!(s, "#");
    let _ = writeln!(s, "# Counts per layer:");
    for (l, counts) in report.counts() {
        let c: Vec<String> = counts
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(o, n)| format!("{n} {o}"))
            .collect();
        let _ = writeln!(s, "#   {:<10} {}", l.name(), c.join(", "));
    }
    let mut overall: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &report.rows {
        *overall.entry(r.overall.word()).or_insert(0) += 1;
    }
    let c: Vec<String> = overall.iter().map(|(o, n)| format!("{n} {o}")).collect();
    let _ = writeln!(s, "#   {:<10} {}", "overall", c.join(", "));
    let _ = writeln!(s, "#");
    let header: Vec<&str> = std::iter::once("case")
        .chain(std::iter::once("overall"))
        .chain(Layer::ALL.iter().map(|l| l.name()))
        .collect();
    let _ = writeln!(s, "# {}", header.join(" | "));
    for r in &report.rows {
        let cols: Vec<&str> = std::iter::once(r.name.as_str())
            .chain(std::iter::once(r.overall.word()))
            .chain(r.verdicts.iter().map(|v| v.outcome.word()))
            .collect();
        let _ = writeln!(s, "{}", cols.join(" | "));
        for (l, v) in Layer::ALL.iter().zip(&r.verdicts) {
            let show = match v.outcome {
                Outcome::Correct => false,
                Outcome::NotApplicable => !v.detail.is_empty(),
                _ => true,
            };
            if show {
                let detail: String = v.detail.chars().take(400).collect();
                let _ = writeln!(s, "#   {}: {}: {}", l.name(), v.outcome, detail);
            }
        }
    }
    s
}

/// One parsed scoreboard line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub overall: Outcome,
    pub layers: Vec<Outcome>,
}

/// Parses a committed scoreboard.
pub fn parse(text: &str) -> Result<BTreeMap<String, Line>, String> {
    let mut out = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split(" | ").collect();
        if cols.len() != 2 + Layer::ALL.len() {
            return Err(format!("line {}: {} columns", i + 1, cols.len()));
        }
        let o =
            |s: &str| Outcome::parse(s).ok_or_else(|| format!("line {}: bad outcome {s}", i + 1));
        let overall = o(cols[1])?;
        let layers = cols[2..]
            .iter()
            .map(|c| o(c))
            .collect::<Result<Vec<_>, _>>()?;
        out.insert(cols[0].to_string(), Line { overall, layers });
    }
    Ok(out)
}

/// What the ratchet found.
#[derive(Debug, Default)]
pub struct Diff {
    /// Failures: wrong cases, regressions, missing or new cases.
    pub problems: Vec<String>,
    /// Improvements over the committed file (regenerate to keep them).
    pub improvements: Vec<String>,
}

pub fn compare(committed: &BTreeMap<String, Line>, report: &Report) -> Diff {
    let mut d = Diff::default();
    for r in &report.rows {
        for (l, v) in Layer::ALL.iter().zip(&r.verdicts) {
            if v.outcome == Outcome::Wrong {
                d.problems
                    .push(format!("{} is WRONG on {}: {}", r.name, l.name(), v.detail));
            }
        }
        if r.overall == Outcome::Wrong && !r.verdicts.iter().any(|v| v.outcome == Outcome::Wrong) {
            d.problems
                .push(format!("{} is WRONG (a kernel panic)", r.name));
        }
        let Some(old) = committed.get(&r.name) else {
            d.problems.push(format!(
                "{} is not in the committed scoreboard: regenerate it ({}=1)",
                r.name, BLESS_ENV
            ));
            continue;
        };
        let pairs = std::iter::once(("overall", old.overall, r.overall)).chain(
            Layer::ALL
                .iter()
                .zip(old.layers.iter().zip(&r.verdicts))
                .map(|(l, (o, n))| (l.name(), *o, n.outcome)),
        );
        for (what, o, n) in pairs {
            if n.regressed_from(o) {
                d.problems
                    .push(format!("{} got worse on {what}: {o} -> {n}", r.name));
            } else if n != o {
                d.improvements
                    .push(format!("{} improved on {what}: {o} -> {n}", r.name));
            }
        }
    }
    for name in committed.keys() {
        if report.row(name).is_none() {
            d.problems.push(format!(
                "{name} is in the committed scoreboard but no longer runs"
            ));
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::Row;
    use crate::outcome::Verdict;

    fn report(outcomes: [Outcome; 7]) -> Report {
        let verdicts: Vec<Verdict> = outcomes.iter().map(|&o| Verdict::new(o, "why")).collect();
        let overall = crate::outcome::overall(&verdicts, false);
        Report {
            kernel: "test 1".into(),
            rows: vec![Row {
                name: "part: x".into(),
                verdicts,
                overall,
            }],
            seconds: 0.0,
        }
    }

    #[test]
    fn round_trip_and_ratchet() {
        use Outcome::*;
        let a = report([
            Correct,
            Correct,
            Undecided,
            Correct,
            NotApplicable,
            NotApplicable,
            NotApplicable,
        ]);
        let text = render(&a);
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed["part: x"].layers[2], Undecided);
        assert!(compare(&parsed, &a).problems.is_empty());
        // Correct -> refused on a layer is a regression.
        let b = report([
            Refused,
            Correct,
            Undecided,
            Correct,
            NotApplicable,
            NotApplicable,
            NotApplicable,
        ]);
        assert!(!compare(&parsed, &b).problems.is_empty());
        // Undecided -> correct is an improvement.
        let c = report([
            Correct,
            Correct,
            Correct,
            Correct,
            NotApplicable,
            NotApplicable,
            NotApplicable,
        ]);
        let d = compare(&parsed, &c);
        assert!(d.problems.is_empty() && !d.improvements.is_empty());
        // Losing evidence is a regression; a wrong is always a problem.
        let e = report([
            Correct,
            NotApplicable,
            Undecided,
            Correct,
            NotApplicable,
            NotApplicable,
            NotApplicable,
        ]);
        assert!(!compare(&parsed, &e).problems.is_empty());
        let f = report([
            Correct,
            Correct,
            Wrong,
            Correct,
            NotApplicable,
            NotApplicable,
            NotApplicable,
        ]);
        assert!(!compare(&parse(&render(&f)).unwrap(), &f)
            .problems
            .is_empty());
    }
}
