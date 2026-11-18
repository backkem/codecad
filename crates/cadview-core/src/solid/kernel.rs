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
    /// Centre of volume, or `None` when the kernel computes no first
    /// moments (a harness reads that as "not applicable", never a pass).
    pub centroid: Option<Point3>,
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

    /// The box grown by `d` on every side.
    pub fn inflate(&self, d: f64) -> Aabb3 {
        Aabb3::new(
            Point3::new(self.min.x - d, self.min.y - d, self.min.z - d),
            Point3::new(self.max.x + d, self.max.y + d, self.max.z + d),
        )
    }

    /// Whether the two boxes share a point (closed boxes).
    pub fn meets(&self, o: &Aabb3) -> bool {
        self.min.x <= o.max.x
            && o.min.x <= self.max.x
            && self.min.y <= o.max.y
            && o.min.y <= self.max.y
            && self.min.z <= o.max.z
            && o.min.z <= self.max.z
    }

    /// Whether the two boxes share volume (their interiors meet).
    pub fn overlaps(&self, o: &Aabb3) -> bool {
        self.min.x < o.max.x
            && o.min.x < self.max.x
            && self.min.y < o.max.y
            && o.min.y < self.max.y
            && self.min.z < o.max.z
            && o.min.z < self.max.z
    }

    /// Whether `o` lies in this box (boundary included).
    pub fn contains(&self, o: &Aabb3) -> bool {
        self.min.x <= o.min.x
            && self.min.y <= o.min.y
            && self.min.z <= o.min.z
            && o.max.x <= self.max.x
            && o.max.y <= self.max.y
            && o.max.z <= self.max.z
    }
}

/// A bounding box with a statement of how tight it is.
///
/// `tol == 0` means the box is exact: each side touches the solid. A
/// positive `tol` means the box still contains the solid, and each side
/// lies within `tol` (mm) outside the solid's extreme in its direction.
/// What [`SolidKernel::bbox_within`] returns; [`SolidKernel::bbox`] is
/// always exact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TightBox {
    pub aabb: Aabb3,
    pub tol: f64,
}

impl TightBox {
    /// An exact box.
    pub const fn exact(aabb: Aabb3) -> Self {
        TightBox { aabb, tol: 0.0 }
    }
}

/// The closest points of two placed solids (world coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Closest {
    /// The distance between the solids as point sets: `0.0` when they
    /// meet or one contains the other.
    pub distance: f64,
    /// The closest point on the first solid.
    pub a: Point3,
    /// The closest point on the second.
    pub b: Point3,
}

/// How two placed solids relate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interference {
    /// Farther apart than the kernel's linear tolerance.
    Apart(Closest),
    /// They meet and share no volume; `at` is a point of the contact.
    Touching { at: Point3 },
    /// They share volume: the shared volume, and whether either lies
    /// wholly in the other.
    Overlapping {
        volume: f64,
        a_in_b: bool,
        b_in_a: bool,
    },
}

/// A solid of a pair query: its B-rep and where it is placed.
pub type Placed<'a, B> = (&'a B, Placement);

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
/// [`EvalError::Unsupported`] (or, where a slower answer exists, falls
/// back to it); a test harness reads "unsupported" as "not applicable",
/// never as a pass. Optional today: [`Self::tessellate`],
/// [`Self::evaluate_from`] (falls back to [`Self::evaluate`]),
/// [`Self::bbox_within`] (falls back to [`Self::bbox`]),
/// [`Self::distances`] and [`Self::interferences`]. The rest of the
/// designed trait (`edges`, `project`, `section`, and the face/edge tags
/// on [`Evaluated`]) arrives with the rungs that need it (3D3a, 3D4a).
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

    /// Optional capability: evaluate `def` continuing from `base`, the
    /// B-rep this kernel made of the first `done` features of `def` with the
    /// same tools. The result must be the one [`Self::evaluate`] gives for
    /// the whole definition (same volume, box and mesh), so a cache may
    /// reuse an evaluated prefix and a modifying call costs about one
    /// feature. The default evaluates from scratch.
    fn evaluate_from(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
        base: &Self::Brep,
        done: usize,
    ) -> Result<Evaluated<Self::Brep>, EvalError> {
        let _ = (base, done);
        self.evaluate(def, tools)
    }

    /// The tight axis-aligned box of `brep` placed by `frame`, exact: each
    /// side touches the solid. A kernel that cannot make it exact refuses
    /// ([`EvalError::Unsupported`]); [`Self::bbox_within`] may then still
    /// give a box tight to a tolerance.
    fn bbox(&self, brep: &Self::Brep, frame: &Placement) -> Result<Aabb3, EvalError>;

    /// Optional capability: a box of `brep` placed by `frame` that contains
    /// the solid and is tight to `tol` (mm, finite and positive) where it
    /// cannot be exact; [`TightBox::tol`] says which. The default is
    /// [`Self::bbox`], exact.
    fn bbox_within(
        &self,
        brep: &Self::Brep,
        frame: &Placement,
        tol: f64,
    ) -> Result<TightBox, EvalError> {
        let _ = tol;
        self.bbox(brep, frame).map(TightBox::exact)
    }

    /// Optional capability: the closest points of each pair of placed
    /// solids (indices into `solids`). One result per pair, in order; a
    /// pair the kernel cannot answer is an error in its slot. The default
    /// says the kernel cannot do it.
    fn distances(
        &self,
        solids: &[Placed<'_, Self::Brep>],
        pairs: &[(usize, usize)],
    ) -> Result<Vec<Result<Closest, EvalError>>, EvalError> {
        let _ = (solids, pairs);
        Err(EvalError::unsupported(None, "distance between solids"))
    }

    /// Optional capability: how each pair of placed solids interferes
    /// (apart, touching, or overlapping with the shared volume), as
    /// [`Self::distances`]. The default says the kernel cannot do it.
    fn interferences(
        &self,
        solids: &[Placed<'_, Self::Brep>],
        pairs: &[(usize, usize)],
    ) -> Result<Vec<Result<Interference, EvalError>>, EvalError> {
        let _ = (solids, pairs);
        Err(EvalError::unsupported(None, "interference between solids"))
    }

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
