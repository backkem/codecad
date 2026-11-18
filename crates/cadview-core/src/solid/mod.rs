//! Solid parts: feature trees and the kernel boundary (docs/3d-design.md
//! sections 4 and 5, ladder rung 3D0a).
//!
//! - [`types`]: the kernel-neutral feature tree (`PartDef`, `Feature`,
//!   `Profile2d`, `EdgeSel`, ...). Plain data, never flattened.
//! - [`kernel`]: the `SolidKernel` trait, errors and results.
//! - [`select`]: kernel-neutral edge-selector resolution.
//! - [`mesh`]: kernel-neutral checks on a `TriMesh` (closed, Euler counts).
//! - [`corpus`]: reference parts with closed-form volumes and boxes.
//!
//! No kernel is linked here. The arris adapter lives in the `cadview-solid`
//! crate; the reference semantics and the evidence layers live in the
//! `solid-harness` crate (docs/3d-test-harness.md), a test tool that no
//! shipped crate depends on.
//!
//! - [`part`]: placed parts (`Part`), ids, placement maths (rung 3D1a).
//! - [`eval`]: the evaluator a shell supplies, and the content-keyed
//!   evaluation cache.
//! - [`scene`]: the document's parts as a `Scene3d` for `cad.render`.
//! - [`baked`]: prebaked scenes (the answers a viewer needs, as a file)
//!   and the evaluator that answers from one; [`wire`]: kernel answers as
//!   serializable data.
//!
//! - Projection (rung 3D3a): [`curves`] (evaluating exact curves),
//!   [`silhouette`] (closed-form silhouettes), [`hlr`] (hidden lines on
//!   exact curves), [`oracle`] (the visibility oracle), [`project`]
//!   (`cad.project` and `cad.viewPoints`).
//!
//! The `cad_call` methods for parts are in `dispatch_3d.rs`, their Yrs maps
//! in `sync.rs`.

pub mod baked;
pub mod corpus;
pub mod curves;
pub mod eval;
pub mod hlr;
pub mod kernel;
pub mod mesh;
pub mod oracle;
pub mod part;
pub mod project;
pub mod remote;
pub mod scene;
pub mod select;
pub mod silhouette;
pub mod types;
pub mod wire;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_checks;

pub use eval::{Evaluator, KernelEvaluator, SolidEnv};
pub use kernel::{
    Aabb3, EvalError, Evaluated, MassProps, NoTools, SolidKernel, ToolResolver, TriMesh, Validity,
};
pub use part::Part;
pub use types::*;
