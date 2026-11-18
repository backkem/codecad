//! Reference parts with closed-form answers, kernel-independent.
//!
//! Each [`CorpusEntry`] is one or more [`PartDef`]s plus the tool
//! definitions they cut with, and the exact volume and bounding box of each
//! part. The entries are plain data and [`measure`] is generic over
//! [`SolidKernel`], so the same corpus runs on every backend and on every
//! target (the harness crate runs it on two kernels and in wasm). Timing is
//! left to the caller, since `std::time::Instant` does not exist on
//! `wasm32-unknown-unknown`.
//!
//! - [`prismatic`]: extrudes, holes, booleans and chamfers (rung 3D0a).
//! - [`curved`]: a round, a loft, a revolve and a sweep (rung 3D0b).

pub mod curved;
pub mod prismatic;

use super::kernel::{Aabb3, EvalError, SolidKernel, ToolResolver};
use super::types::{DefId, PartDef, Placement};

/// A part and its closed-form answers.
#[derive(Debug, Clone)]
pub struct CorpusPart {
    pub def: PartDef,
    /// Exact volume (mm^3).
    pub volume: f64,
    /// Exact axis-aligned bounding box in the part's own frame (mm).
    pub bbox: Aabb3,
}

/// One corpus entry: its parts and the tool definitions they use.
#[derive(Debug, Clone)]
pub struct CorpusEntry {
    pub name: &'static str,
    pub parts: Vec<CorpusPart>,
    pub tools: Vec<PartDef>,
}

impl ToolResolver for CorpusEntry {
    fn def(&self, id: DefId) -> Option<&PartDef> {
        self.tools.iter().find(|d| d.id == id)
    }
}

/// What a kernel produced for one corpus part, against the closed form.
#[derive(Debug, Clone)]
pub struct PartResult {
    pub part: String,
    pub expected_volume: f64,
    pub volume: f64,
    /// `|volume - expected| / expected`.
    pub volume_rel_err: f64,
    /// Largest coordinate difference between the boxes (mm).
    pub bbox_err: f64,
    pub bbox: Aabb3,
    pub valid: bool,
    pub solids: usize,
    pub faces: usize,
    /// The kernel integrated the volume on exact surfaces.
    pub exact_volume: bool,
    pub problems: Vec<String>,
}

impl PartResult {
    /// The gate: a valid single solid, volume and box within tolerance.
    pub fn passes(&self, volume_rel_tol: f64, bbox_tol: f64) -> bool {
        self.valid
            && self.solids == 1
            && self.volume_rel_err <= volume_rel_tol
            && self.bbox_err <= bbox_tol
    }
}

/// Checks an evaluated part against its closed form: validity, volume and
/// bounding box, all through the kernel trait.
pub fn measure<K: SolidKernel>(
    kernel: &K,
    part: &CorpusPart,
    brep: &K::Brep,
) -> Result<PartResult, EvalError> {
    let validity = kernel.check(brep)?;
    let mass = kernel.mass(brep)?;
    let bbox = kernel.bbox(brep, &Placement::IDENTITY)?;
    Ok(PartResult {
        part: part.def.name.clone(),
        expected_volume: part.volume,
        volume: mass.volume,
        volume_rel_err: (mass.volume - part.volume).abs() / part.volume.abs(),
        bbox_err: bbox.max_abs_diff(&part.bbox),
        bbox,
        valid: validity.valid,
        solids: validity.solids,
        faces: validity.faces,
        exact_volume: mass.exact,
        problems: validity.problems,
    })
}
