//! The server's solid kernel: arris, through the `cadview-solid` adapter.
//!
//! cadview-core links no kernel; it evaluates parts through whatever
//! evaluator the shell gives each document (`Document::solids`). The server
//! gives every document the same one, so all documents share one
//! evaluation cache (keyed by content, so sharing is safe).

use cadview_core::solid::{KernelEvaluator, SolidEnv};
use cadview_solid::ArrisKernel;
use std::sync::{Arc, OnceLock};

/// The process-wide solid environment: the arris kernel and its cache.
pub fn env() -> SolidEnv {
    static ENV: OnceLock<SolidEnv> = OnceLock::new();
    ENV.get_or_init(|| SolidEnv::new(Arc::new(KernelEvaluator::new(ArrisKernel))))
        .clone()
}
