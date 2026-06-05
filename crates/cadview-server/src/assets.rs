//! Asset providers: embedded (compiled-in) or disk-based file serving.
//!
//! Feature flags:
//! - `embedded-dist`:     bake dist/ (frontend + WASM) into the binary
//! - `embedded-examples`: bake examples/ (drawings, scripts) into the binary
//!
//! Without these features the server loads assets from disk at runtime.
//! CLI `--dist <path>` / `--examples <path>` override to disk regardless
//! of features.

use std::borrow::Cow;
use std::path::PathBuf;

// ── Trait ───────────────────────────────────────────────────────────

/// Resolves asset files by relative path.
pub trait AssetProvider: Send + Sync {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>>;
}

// ── Disk provider ──────────────────────────────────────────────────

/// Loads assets from a directory on disk.
pub struct DiskAssets {
    root: PathBuf,
}

impl DiskAssets {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl AssetProvider for DiskAssets {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        let file_path = self.root.join(path);
        if !file_path.starts_with(&self.root) {
            return None; // path traversal
        }
        std::fs::read(&file_path).ok().map(Cow::Owned)
    }
}

// ── Embedded dist ──────────────────────────────────────────────────

#[cfg(feature = "embedded-dist")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../dist"]
#[exclude = "*.d.ts"]
#[exclude = "screenshot*"]
#[exclude = "debug-*"]
struct EmbeddedDist;

#[cfg(feature = "embedded-dist")]
impl AssetProvider for EmbeddedDist {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        <Self as rust_embed::RustEmbed>::get(path).map(|f| f.data)
    }
}

// ── Embedded examples ──────────────────────────────────────────────

#[cfg(feature = "embedded-examples")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../examples"]
struct EmbeddedExamples;

#[cfg(feature = "embedded-examples")]
impl AssetProvider for EmbeddedExamples {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        <Self as rust_embed::RustEmbed>::get(path).map(|f| f.data)
    }
}

// ── Constructors ───────────────────────────────────────────────────

/// Default dist provider: embedded if compiled with `embedded-dist`,
/// otherwise searches for dist/ on disk.
pub fn default_dist() -> anyhow::Result<Box<dyn AssetProvider>> {
    #[cfg(feature = "embedded-dist")]
    {
        tracing::info!("Serving dist from embedded assets");
        Ok(Box::new(EmbeddedDist))
    }
    #[cfg(not(feature = "embedded-dist"))]
    {
        let dir = find_dist_dir()?;
        tracing::info!("Serving dist from disk: {}", dir.display());
        Ok(Box::new(DiskAssets::new(dir)))
    }
}

/// Default examples provider: embedded if compiled with
/// `embedded-examples`, otherwise None.
pub fn default_examples() -> Option<Box<dyn AssetProvider>> {
    #[cfg(feature = "embedded-examples")]
    {
        tracing::info!("Serving examples from embedded assets");
        Some(Box::new(EmbeddedExamples))
    }
    #[cfg(not(feature = "embedded-examples"))]
    {
        None
    }
}

/// Locate the dist/ directory on disk (legacy / non-embedded mode).
#[cfg(not(feature = "embedded-dist"))]
fn find_dist_dir() -> anyhow::Result<PathBuf> {
    let candidates = [PathBuf::from("dist"), PathBuf::from("../../dist")];
    for p in &candidates {
        if p.join("index.html").exists() {
            return Ok(std::fs::canonicalize(p)?);
        }
    }
    anyhow::bail!(
        "Cannot find dist/index.html. Build the frontend first (just build), \
         or pass --dist <path>, or compile with --features embedded-dist."
    )
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Content-Type for a file path based on extension.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}
