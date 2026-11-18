//! Kernel adapters and broken wrappers.
//!
//! - [`arris`]: the arris adapter, from the shipped crate `cadview-solid`
//!   (the only crate that names arris), re-exported here.
//! - [`cad_kernel`]: the cad-kernel adapter, the only file that names
//!   cad-kernel. A second opinion, test-only.
//! - [`broken`]: wrappers that make a real kernel subtly wrong while the
//!   harness still sees the original recipe (design section 7).

pub mod broken;
pub mod cad_kernel;

pub use cadview_solid::arris;

pub use cad_kernel::CadKernel;
pub use cadview_solid::ArrisKernel;
