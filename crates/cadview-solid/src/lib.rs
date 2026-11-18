//! The solid kernel behind cadview-core's [`SolidKernel`] trait
//! (docs/3d-design.md section 5).
//!
//! - [`arris`]: [`ArrisKernel`], the adapter for the arris B-rep kernel.
//!   It is the only file in the workspace that names arris.
//!
//! cadview-core links no kernel: the feature tree, sync and the API speak
//! only the trait. This crate compiles for wasm32 and native, keeps no
//! process-wide state and starts no threads. Its evidence is the test
//! harness (`solid-harness`, docs/3d-test-harness.md), whose committed
//! scoreboard for arris must record no wrong case.
//!
//! [`SolidKernel`]: cadview_core::solid::SolidKernel

pub mod arris;

pub use self::arris::{ArrisBrep, ArrisKernel};
