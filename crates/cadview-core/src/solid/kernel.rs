//! The kernel boundary (docs/3d-design.md section 5).
//!
//! All solid-modelling goes through [`SolidKernel`]. Kernel types stay
//! behind `Self::Brep` and inside the kernel's own file: the feature tree,
//! sync and the API never see them, so a backend can be swapped without
//! touching documents.

use std::fmt;

use super::types::{DefId, FeatId, PartDef, Placement, Point3};

/// Resolves the tool bodies a [`PartDef`] refers to
/// ([`ToolRef`](super::ToolRef)).
pub trait ToolResolver {
    /// The definition with this id, if known.
    fn def(&self, id: DefId) -> Option<&PartDef>;
}

/// A resolver that knows no definitions.
pub struct NoTools;

impl ToolResolver for NoTools {
    fn def(&self, _id: DefId) -> Option<&PartDef> {
        None
    }
}

impl ToolResolver for [PartDef] {
    fn def(&self, id: DefId) -> Option<&PartDef> {
        self.iter().find(|d| d.id == id)
    }
}

/// The result of evaluating a [`PartDef`].
///
/// The designed `face_tags`, `edge_tags` and `degraded` fields arrive with
/// the rungs that use them (projection, selection, roundover fallback).
#[derive(Debug, Clone)]
pub struct Evaluated<B> {
    pub brep: B,
}

/// Mass properties at unit density.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProps {
    /// Enclosed volume (mm^3).
    pub volume: f64,
    /// Centre of volume.
    pub centroid: Point3,
    /// True when the kernel integrated on the exact surfaces; false when
    /// any part of it came from a tessellation.
    pub exact: bool,
}

/// An axis-aligned box (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb3 {
    pub min: Point3,
    pub max: Point3,
}

impl Aabb3 {
    pub const fn new(min: Point3, max: Point3) -> Self {
        Self { min, max }
    }

    /// The largest difference between corresponding coordinates of two
    /// boxes.
    pub fn max_abs_diff(&self, o: &Aabb3) -> f64 {
        self.min
            .max_abs_diff(o.min)
            .max(self.max.max_abs_diff(o.max))
    }
}

/// What a kernel's validity check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Validity {
    /// The kernel found nothing wrong with the B-rep.
    pub valid: bool,
    /// Number of solids (lumps) in the result.
    pub solids: usize,
    /// Number of faces, for reports.
    pub faces: usize,
    /// One line per problem found.
    pub problems: Vec<String>,
}

/// Why a part could not be evaluated. Never a panic.
#[derive(Debug, Clone, PartialEq)]
pub enum EvalError {
    /// The feature (or the variant of it) is not implemented by this kernel.
    Unsupported {
        feature: Option<FeatId>,
        what: String,
    },
    /// The input is malformed: bad dimensions, an open profile, nothing to
    /// subtract from.
    Invalid {
        feature: Option<FeatId>,
        reason: String,
    },
    /// An edge selector resolved to no edge.
    Selector { feature: FeatId, reason: String },
    /// A tool definition is not known to the resolver.
    UnknownTool { feature: FeatId, def: DefId },
    /// Tool definitions refer to each other in a cycle.
    ToolCycle { feature: FeatId, def: DefId },
    /// The kernel refused or failed an operation.
    Kernel {
        feature: Option<FeatId>,
        message: String,
    },
}

impl EvalError {
    pub fn invalid(feature: Option<FeatId>, reason: impl Into<String>) -> Self {
        EvalError::Invalid {
            feature,
            reason: reason.into(),
        }
    }

    pub fn unsupported(feature: Option<FeatId>, what: impl Into<String>) -> Self {
        EvalError::Unsupported {
            feature,
            what: what.into(),
        }
    }

    /// The same error attributed to `feature` if it has no feature yet.
    pub fn at(self, f: FeatId) -> Self {
        match self {
            EvalError::Unsupported {
                feature: None,
                what,
            } => EvalError::Unsupported {
                feature: Some(f),
                what,
            },
            EvalError::Invalid {
                feature: None,
                reason,
            } => EvalError::Invalid {
                feature: Some(f),
                reason,
            },
            EvalError::Kernel {
                feature: None,
                message,
            } => EvalError::Kernel {
                feature: Some(f),
                message,
            },
            other => other,
        }
    }
}

fn feat(f: &Option<FeatId>) -> String {
    match f {
        Some(id) => format!("feature {}: ", id.0),
        None => String::new(),
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Unsupported { feature, what } => write!(
                f,
                "{}{what} is not supported by this kernel yet",
                feat(feature)
            ),
            EvalError::Invalid { feature, reason } => write!(f, "{}{reason}", feat(feature)),
            EvalError::Selector { feature, reason } => {
                write!(f, "feature {}: edge selector: {reason}", feature.0)
            }
            EvalError::UnknownTool { feature, def } => write!(
                f,
                "feature {}: tool definition {} is not known",
                feature.0, def.0
            ),
            EvalError::ToolCycle { feature, def } => write!(
                f,
                "feature {}: tool definition {} refers back to itself",
                feature.0, def.0
            ),
            EvalError::Kernel { feature, message } => {
                write!(f, "{}kernel: {message}", feat(feature))
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// A triangle mesh of a solid's boundary (mm).
///
/// The contract a kernel's [`SolidKernel::tessellate`] promises:
///
/// - every position lies within the requested chord tolerance of the exact
///   boundary, and every point of the exact boundary lies within the chord
///   tolerance of some triangle (an inscribed mesh whose sagitta is at
///   most the chord);
/// - positions shared by neighbouring triangles are one index, so a closed
///   boundary gives a closed mesh;
/// - every triangle is counter-clockwise seen from outside the solid, so
///   `(b - a) x (c - a)` points out of the material.
///
/// Derived data, like the B-rep: never synced or stored.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriMesh {
    pub positions: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
}

/// A solid-modelling backend.
///
/// Required: evaluation, mass properties, bounding box and a validity
/// check. Optional capabilities have a default implementation that returns
/// [`EvalError::Unsupported`]; a test harness reads that as "not
/// applicable", never as a pass. Optional today: [`Self::tessellate`]. The
/// rest of the designed trait (`edges`, `project`, `section`, `distance`,
/// and the face/edge tags on [`Evaluated`]) arrives with the rungs that
/// need it (3D3a, 3D4a, 3D1b).
pub trait SolidKernel {
    /// The kernel's B-rep. Derived data: never synced or stored.
    type Brep: Clone;

    /// A short name with version, for reports ("arris 0.11.1").
    fn name(&self) -> String;

    /// Evaluates a definition's features top to bottom into one B-rep.
    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Evaluated<Self::Brep>, EvalError>;

    /// Volume and centroid.
    fn mass(&self, brep: &Self::Brep) -> Result<MassProps, EvalError>;

    /// The tight axis-aligned box of `brep` placed by `frame`.
    fn bbox(&self, brep: &Self::Brep, frame: &Placement) -> Result<Aabb3, EvalError>;

    /// The kernel's own validity check of the B-rep.
    fn check(&self, brep: &Self::Brep) -> Result<Validity, EvalError>;

    /// Optional capability: the boundary of `brep` as a [`TriMesh`] to the
    /// chord tolerance `chord` (mm, finite and positive), in the part's
    /// own frame. The default says the kernel cannot do it.
    fn tessellate(&self, brep: &Self::Brep, chord: f64) -> Result<TriMesh, EvalError> {
        let _ = (brep, chord);
        Err(EvalError::unsupported(None, "tessellation"))
    }
}
