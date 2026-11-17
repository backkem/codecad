//! Solid parts: feature trees and the kernel boundary (docs/3d-design.md
//! sections 4 and 5, ladder rung 3D0a).
//!
//! - [`types`]: the kernel-neutral feature tree (`PartDef`, `Feature`,
//!   `Profile2d`, `EdgeSel`, ...). Plain data, never flattened.
//! - [`kernel`]: the `SolidKernel` trait, errors and results.
//! - [`select`]: kernel-neutral edge-selector resolution.
//! - [`corpus`]: reference parts with closed-form volumes and boxes.
//! - `arris_kernel` (tests only): `ArrisKernel`, the reference backend the
//!   corpus runs on and the only place that names arris. No kernel is
//!   linked into shipped code yet: which one ships is still open.
//!
//! Not here yet: Yrs sync, `cad_call` methods, tessellation, projection.

#[cfg(test)]
mod arris_kernel;
pub mod corpus;
pub mod kernel;
pub mod select;
pub mod types;

pub use kernel::{
    Aabb3, EvalError, Evaluated, MassProps, NoTools, SolidKernel, ToolResolver, Validity,
};
pub use types::*;
