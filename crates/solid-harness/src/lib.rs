//! A kernel-independent test harness for CodeCAD's solid kernels
//! (docs/3d-test-harness.md).
//!
//! The kernel's code is not what we trust; the evidence produced here is.
//! The harness holds:
//!
//! - [`reference`]: the certified reference meaning of the features the
//!   kernels evaluate (extrude, hole, boolean with an inline tool,
//!   equal-distance chamfer, revolve, sweep along lines and arcs, and the
//!   loft and fillet cases it can define exactly), as a sign-exact
//!   distance bound evaluated in outward-rounded interval arithmetic
//!   ([`interval`]). It calls no kernel and shares no code with any
//!   adapter.
//! - [`layers`]: the evidence layers (closed forms, certified volume
//!   enclosure, boundary shell test, mesh invariants, equivalent
//!   constructions, algebra and motion, continuity sweeps) and the runner.
//! - [`cases`]: the catalogue of solids, relations and sweeps.
//! - [`fuzz`]: the fuzzer's generator, judge, shrinker and case printer
//!   (the driver is `examples/fuzz.rs`, a development-time tool).
//! - [`kernels`]: the adapters under test (arris, from the shipped crate
//!   cadview-solid, and cad-kernel) and the broken wrappers that test the
//!   harness itself.
//! - [`scoreboard`]: the committed per-kernel file and its ratchet.
//!
//! This crate is a test tool. No shipped crate may depend on it.

pub mod cases;
pub mod fuzz;
pub mod interval;
pub mod kernels;
pub mod layers;
pub mod mesh;
pub mod outcome;
pub mod reference;
pub mod rng;
pub mod run;
pub mod scoreboard;
