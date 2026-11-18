//! Kernel adapters and broken wrappers.
//!
//! - [`arris`]: the arris adapter, the only file that names arris.
//! - [`broken`]: wrappers that make a real kernel subtly wrong while the
//!   harness still sees the original recipe (design section 7).

pub mod arris;
pub mod broken;

pub use arris::ArrisKernel;
