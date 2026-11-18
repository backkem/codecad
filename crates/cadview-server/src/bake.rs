//! `cad.bake3d`: write the document's parts as a prebaked 3D scene, the
//! file a viewer without a kernel shows (cadview-core `solid::baked`). One
//! handler for the sandbox (`script.rs`) and the browser RPC
//! (`transport.rs`); `GET /api/bake3d` (`http.rs`) serves the same bytes.

use cadview_core::Document;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

/// Default output path when `out` is not given.
pub const DEFAULT_OUT: &str = "scene.bake3d";

/// The bake of `doc` for the `cad.render`-style options in `args`, with the
/// summary `cad.bake3d` reports (no path).
pub fn bake_bytes(doc: &Document, args: &Value) -> Result<(Vec<u8>, Value), String> {
    let start = Instant::now();
    let (bytes, r) =
        cadview_core::solid::baked::bake(doc, args).map_err(|e| format!("bake3d: {e}"))?;
    let ms = (start.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
    let n = bytes.len();
    Ok((
        bytes,
        json!({
            "ok": true,
            "bytes": n,
            "parts": r.parts,
            "defs": r.defs,
            "solids": r.solids,
            "triangles": r.triangles,
            "chord": r.chord,
            "coarser": r.coarser,
            "kernel": r.kernel,
            "format": cadview_core::solid::baked::FORMAT_VERSION,
            "ms": ms,
        }),
    ))
}

/// Bake `doc` with the options in `args` (JSON) and write the file to
/// `out`, resolved against `base` when relative. Returns `{ok, path,
/// bytes, parts, defs, solids, triangles, chord, kernel, format, ms}`.
pub fn bake_to_file(doc: &Document, args: &str, base: &Path) -> Result<Value, String> {
    let a: Value = if args.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args).map_err(|e| format!("bake3d: {e}"))?
    };
    let out = match a.get("out") {
        None | Some(Value::Null) => DEFAULT_OUT,
        Some(Value::String(s)) if !s.is_empty() => s.as_str(),
        Some(other) => return Err(format!("bake3d: out must be a path, got {other}")),
    };
    let (bytes, mut v) = bake_bytes(doc, &a)?;
    let path = if Path::new(out).is_absolute() {
        Path::new(out).to_path_buf()
    } else {
        base.join(out)
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("bake3d: create {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&path, &bytes).map_err(|e| format!("bake3d: write {out}: {e}"))?;
    tracing::info!(
        "Baked {} parts ({} solids, {} triangles) to {} ({} bytes)",
        v["parts"],
        v["solids"],
        v["triangles"],
        path.display(),
        bytes.len()
    );
    v["path"] = json!(path.to_string_lossy());
    Ok(v)
}
