//! The outcome model (docs/3d-test-harness.md section 5).

use std::fmt;

/// What one layer concluded about one case, or what a case concluded
/// overall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Outcome {
    /// The kernel returned a solid that fails the layer, or panicked.
    Wrong,
    /// The evidence fell inside the declared tolerance band.
    Undecided,
    /// The kernel returned a typed error. Safe.
    Refused,
    /// Every check of the layer passed.
    Correct,
    /// The layer does not apply: the kernel lacks a capability, the
    /// reference has no meaning for the recipe, or the case has nothing
    /// for this layer. Never a pass.
    NotApplicable,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Wrong => "WRONG",
            Outcome::Undecided => "undecided",
            Outcome::Refused => "refused",
            Outcome::Correct => "correct",
            Outcome::NotApplicable => "n/a",
        }
    }

    pub fn parse(s: &str) -> Option<Outcome> {
        Some(match s {
            "WRONG" => Outcome::Wrong,
            "undecided" => Outcome::Undecided,
            "refused" => Outcome::Refused,
            "correct" => Outcome::Correct,
            "n/a" => Outcome::NotApplicable,
            _ => return None,
        })
    }

    /// Rank for the ratchet: wrong < undecided < refused < correct.
    /// `NotApplicable` has no rank: see [`Outcome::regressed_from`].
    fn rank(self) -> Option<u8> {
        match self {
            Outcome::Wrong => Some(0),
            Outcome::Undecided => Some(1),
            Outcome::Refused => Some(2),
            Outcome::Correct => Some(3),
            Outcome::NotApplicable => None,
        }
    }

    /// Whether moving from `old` to `self` is a regression. Losing
    /// evidence (anything to n/a) is one; gaining it (n/a to anything but
    /// wrong) is not.
    pub fn regressed_from(self, old: Outcome) -> bool {
        match (old.rank(), self.rank()) {
            (Some(o), Some(n)) => n < o,
            (Some(_), None) => true,
            (None, Some(n)) => n == 0,
            (None, None) => false,
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// One layer's outcome on one case, with a short reason.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub outcome: Outcome,
    pub detail: String,
}

impl Verdict {
    pub fn new(outcome: Outcome, detail: impl Into<String>) -> Verdict {
        Verdict {
            outcome,
            detail: detail.into(),
        }
    }
    pub fn na(detail: impl Into<String>) -> Verdict {
        Verdict::new(Outcome::NotApplicable, detail)
    }
    pub fn correct(detail: impl Into<String>) -> Verdict {
        Verdict::new(Outcome::Correct, detail)
    }
    pub fn wrong(detail: impl Into<String>) -> Verdict {
        Verdict::new(Outcome::Wrong, detail)
    }
    pub fn undecided(detail: impl Into<String>) -> Verdict {
        Verdict::new(Outcome::Undecided, detail)
    }
    pub fn refused(detail: impl Into<String>) -> Verdict {
        Verdict::new(Outcome::Refused, detail)
    }

    /// Combines sub-checks of one layer: the worst wins, wrong first, then
    /// undecided, refused; correct only if at least one sub-check is
    /// correct and none is worse.
    pub fn combine(parts: Vec<Verdict>) -> Verdict {
        let order = [
            Outcome::Wrong,
            Outcome::Undecided,
            Outcome::Refused,
            Outcome::Correct,
        ];
        for o in order {
            let hits: Vec<&Verdict> = parts.iter().filter(|v| v.outcome == o).collect();
            if !hits.is_empty() {
                let detail = hits
                    .iter()
                    .map(|v| v.detail.as_str())
                    .filter(|d| !d.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Verdict::new(o, detail);
            }
        }
        let detail = parts
            .iter()
            .map(|v| v.detail.as_str())
            .filter(|d| !d.is_empty())
            .collect::<Vec<_>>()
            .join("; ");
        Verdict::na(detail)
    }
}

/// The evidence layers this slice builds, in scoreboard column order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// Layer 1: hand-derived volume and bounding box.
    ClosedForm,
    /// Layer 1b: certified volume enclosure from the reference.
    Enclosure,
    /// Layer 2: boundary shell test on the mesh.
    Shell,
    /// Layer 4: mesh invariants and volume convergence.
    Mesh,
    /// The kernel's own validity check on every part (one or more valid
    /// solids, or nothing where the reference is empty). Weaker evidence
    /// than ours, but a kernel that calls its own result invalid has
    /// returned a wrong solid.
    Validity,
    /// Layer 5: equivalent constructions.
    Equivalence,
    /// Layer 6: algebra and motion.
    Algebra,
    /// Layer 7: continuity sweeps.
    Continuity,
}

impl Layer {
    pub const ALL: [Layer; 8] = [
        Layer::ClosedForm,
        Layer::Enclosure,
        Layer::Shell,
        Layer::Mesh,
        Layer::Validity,
        Layer::Equivalence,
        Layer::Algebra,
        Layer::Continuity,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Layer::ClosedForm => "closed",
            Layer::Enclosure => "enclosure",
            Layer::Shell => "shell",
            Layer::Mesh => "mesh",
            Layer::Validity => "valid",
            Layer::Equivalence => "equiv",
            Layer::Algebra => "algebra",
            Layer::Continuity => "continuity",
        }
    }
}

/// The overall outcome of a case from its layers: wrong if any layer is
/// wrong; refused if the kernel refused the case itself; otherwise
/// undecided if any layer is; refused if any layer is; correct if at least
/// one layer is correct; undecided (no evidence) if every layer is n/a.
pub fn overall(layers: &[Verdict], kernel_refused: bool) -> Outcome {
    let has = |o: Outcome| layers.iter().any(|v| v.outcome == o);
    if has(Outcome::Wrong) {
        Outcome::Wrong
    } else if kernel_refused {
        Outcome::Refused
    } else if has(Outcome::Undecided) {
        Outcome::Undecided
    } else if has(Outcome::Refused) {
        Outcome::Refused
    } else if has(Outcome::Correct) {
        Outcome::Correct
    } else {
        Outcome::Undecided
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratchet_order() {
        use Outcome::*;
        assert!(Wrong.regressed_from(Correct));
        assert!(Refused.regressed_from(Correct));
        assert!(Undecided.regressed_from(Refused));
        assert!(!Correct.regressed_from(Refused));
        assert!(NotApplicable.regressed_from(Correct));
        assert!(NotApplicable.regressed_from(Wrong));
        assert!(!Correct.regressed_from(NotApplicable));
        assert!(Wrong.regressed_from(NotApplicable));
        assert!(!NotApplicable.regressed_from(NotApplicable));
        for o in [Wrong, Undecided, Refused, Correct, NotApplicable] {
            assert_eq!(Outcome::parse(o.word()), Some(o));
        }
    }
}
