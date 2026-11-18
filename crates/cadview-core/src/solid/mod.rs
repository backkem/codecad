//! Solid parts: feature trees and the kernel boundary (docs/3d-design.md
//! sections 4 and 5, ladder rung 3D0a).
//!
//! - [`types`]: the kernel-neutral feature tree (`PartDef`, `Feature`,
//!   `Profile2d`, `EdgeSel`, ...). Plain data, never flattened.
//! - [`kernel`]: the `SolidKernel` trait, errors and results.
//! - [`select`]: kernel-neutral edge-selector resolution.
//! - [`corpus`]: reference parts with closed-form volumes and boxes.
//!
//! No kernel is linked here. The kernel adapters (arris today), the
//! reference semantics and the evidence layers live in the
//! `solid-harness` crate (docs/3d-test-harness.md), a test tool that no
//! shipped crate depends on. Which kernel ships is still open.
//!
//! Not here yet: Yrs sync, `cad_call` methods, projection.

pub mod corpus;
pub mod kernel;
pub mod mesh;
pub mod select;
pub mod types;

pub use kernel::{
    Aabb3, EvalError, Evaluated, MassProps, NoTools, SolidKernel, ToolResolver, TriMesh, Validity,
};
pub use types::*;
