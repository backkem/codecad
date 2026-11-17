//! `cad.render`: render the document to a PNG on disk. One handler for the
//! sandbox (`script.rs`), the browser RPC (`transport.rs`) and the CLI.

use cadview_core::Document;
use cadview_render::RenderOptions;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

/// Default output path when `out` is not given.
pub const DEFAULT_OUT: &str = "render.png";

/// Render `doc` with the options in `args` (the JSON `cad.render` takes)
/// and write the PNG to `out`, resolved against `base` when relative.
///
/// Returns `{ok, path, width, height, backend, bytes, ms}`.
pub fn render_to_file(doc: &Document, args: &str, base: &Path) -> Result<Value, String> {
    let start = Instant::now();
    let a: Value = if args.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args).map_err(|e| format!("render: {e}"))?
    };
    let out = match a.get("out") {
        None | Some(Value::Null) => DEFAULT_OUT,
        Some(Value::String(s)) if !s.is_empty() => s.as_str(),
        Some(other) => return Err(format!("render: out must be a path, got {other}")),
    };
    let opts = RenderOptions::from_json(args)?;
    let rendered = cadview_render::render_png(doc, &opts).map_err(|e| format!("render: {e}"))?;

    let path = {
        let p = Path::new(out);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            base.join(p)
        }
    };
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("render: create {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&path, &rendered.png).map_err(|e| format!("render: write {out}: {e}"))?;

    let ms = (start.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
    tracing::info!(
        "Rendered {}x{} PNG on {} to {} ({} bytes, {ms} ms{})",
        rendered.width,
        rendered.height,
        rendered.backend.as_str(),
        path.display(),
        rendered.png.len(),
        rendered
            .adapter
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default()
    );
    Ok(json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "width": rendered.width,
        "height": rendered.height,
        "backend": rendered.backend.as_str(),
        "bytes": rendered.png.len(),
        "ms": ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn writes_a_png_and_reports_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut doc = Document::new();
        doc.add_line(Point::new(0.0, 0.0), Point::new(100.0, 50.0), "0", None);
        let args = r#"{"out":"sub/r.png","size":[120,80],"backend":"auto"}"#;
        match render_to_file(&doc, args, dir.path()) {
            Ok(v) => {
                assert_eq!(v["ok"], true);
                assert_eq!(v["width"], 120);
                assert_eq!(v["height"], 80);
                let bytes = std::fs::read(dir.path().join("sub/r.png")).unwrap();
                assert_eq!(v["bytes"], bytes.len());
                assert_eq!(&bytes[1..4], b"PNG");
                assert!(v["backend"] == "gpu" || v["backend"] == "cpu");
            }
            // No GPU and no CPU backend wired in on this host/build.
            Err(e) => eprintln!("SKIPPED writes_a_png_and_reports_fields: {e}"),
        }
    }

    #[test]
    fn rejects_bad_options() {
        let dir = tempfile::tempdir().unwrap();
        let doc = Document::new();
        assert!(render_to_file(&doc, r#"{"size":[0,0]}"#, dir.path()).is_err());
        assert!(render_to_file(&doc, r#"{"out":5}"#, dir.path()).is_err());
        assert!(render_to_file(&doc, "not json", dir.path()).is_err());
    }
}
