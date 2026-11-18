//! Kernel adapters and broken wrappers.
//!
//! - [`arris`]: the arris adapter, the only file that names arris.
//! - [`cad_kernel`]: the cad-kernel adapter, the only file that names
//!   cad-kernel.
//! - [`broken`]: wrappers that make a real kernel subtly wrong while the
//!   harness still sees the original recipe (design section 7).

pub mod arris;
pub mod broken;
pub mod cad_kernel;

pub use arris::ArrisKernel;
pub use cad_kernel::CadKernel;
