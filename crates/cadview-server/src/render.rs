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
    if cadview_core::solid::scene::wants_3d(doc, &a).map_err(|e| format!("render: {e}"))? {
        return render_3d(doc, &a, &opts, out, base, start);
    }
    let rendered = cadview_render::render_png(doc, &opts).map_err(|e| format!("render: {e}"))?;

    let path = write_png(out, base, &rendered.png)?;

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

/// Write `png` to `out`, resolved against `base` when relative, creating
/// its folder. Returns the path written.
fn write_png(out: &str, base: &Path, png: &[u8]) -> Result<std::path::PathBuf, String> {
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
    std::fs::write(&path, png).map_err(|e| format!("render: write {out}: {e}"))?;
    Ok(path)
}

/// The 3D branch of `cad.render`: the document's parts, shaded. Core builds
/// the scene (`cadview_core::solid::scene`); `cadview-render3d` draws it on
/// the process-wide GPU device.
#[cfg(feature = "gpu")]
fn render_3d(
    doc: &Document,
    a: &Value,
    opts: &RenderOptions,
    out: &str,
    base: &Path,
    start: Instant,
) -> Result<Value, String> {
    use cadview_render3d::{Quality, RenderOptions3d};
    if opts.backend == cadview_render::Backend::Cpu {
        return Err("render: 3D renders need the GPU (backend \"auto\" or \"gpu\")".into());
    }
    let quality = match a.get("quality") {
        None | Some(Value::Null) => Quality::Working,
        Some(Value::String(q)) => q.parse::<Quality>().map_err(|e| format!("render: {e}"))?,
        Some(_) => return Err("render: quality is \"working\" or \"presentation\"".into()),
    };
    let count = |key: &str, default: u32| -> Result<u32, String> {
        match a.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("render: {key} must be a whole number >= 0")),
        }
    };
    // The denoiser is on for presentation renders unless the script says
    // `denoise: false` (to see the raw estimate).
    let denoise = match a.get("denoise") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("render: denoise is true or false".into()),
    };
    let opts3 = RenderOptions3d {
        size: opts.size,
        pixel_ratio: opts.pixel_ratio,
        background: None,
        quality,
        fit: true,
        spp: count("spp", cadview_render3d::trace::DEFAULT_SPP)?,
        seed: count("seed", 0)?,
        denoise,
    };
    opts3.validate().map_err(|e| format!("render: {e}"))?;
    let built = cadview_core::solid::scene::parts_scene(doc, a, opts3.pixel_size())
        .map_err(|e| format!("render: {e}"))?;
    // A camera placed by the options is used as given.
    let opts3 = RenderOptions3d {
        fit: built.fit,
        ..opts3
    };
    let rendered =
        cadview_render3d::render_png(&built.scene, &opts3).map_err(|e| format!("render: {e}"))?;
    let path = write_png(out, base, &rendered.png)?;
    let ms = (start.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
    let r1 = |v: f64| (v * 10.0).round() / 10.0;
    let trace = rendered.trace.map(|t| {
        json!({
            "samples": t.samples,
            "passes": t.passes,
            "instances": t.instances,
            "triangles": t.triangles,
            "meshesBuilt": t.meshes_built,
            "bvhMs": r1(t.bvh_ms),
            "uploadMs": r1(t.upload_ms),
            "traceMs": r1(t.trace_ms),
            "pngMs": r1(rendered.png_ms),
        })
    });
    tracing::info!(
        "Rendered {} parts in 3D ({}{}), {}x{} PNG on {} to {} ({} bytes, {ms} ms)",
        built.parts,
        quality.as_str(),
        rendered
            .trace
            .map(|t| format!(", {} spp in {:.0} ms", t.samples, t.trace_ms))
            .unwrap_or_default(),
        rendered.width,
        rendered.height,
        rendered.adapter,
        path.display(),
        rendered.png.len(),
    );
    let mut v = json!({
        "ok": true,
        "path": path.to_string_lossy(),
        "width": rendered.width,
        "height": rendered.height,
        "backend": "gpu",
        "bytes": rendered.png.len(),
        "ms": ms,
        "mode": "3d",
        "quality": quality.as_str(),
        "parts": built.parts,
        "triangles": built.triangles,
        "chord": built.chord,
        "kernel": built.kernel,
    });
    if let Some(t) = trace {
        v["samples"] = t["samples"].clone();
        v["denoised"] = json!(denoise);
        v["trace"] = t;
    }
    Ok(v)
}

#[cfg(not(feature = "gpu"))]
fn render_3d(
    _doc: &Document,
    _a: &Value,
    _opts: &RenderOptions,
    _out: &str,
    _base: &Path,
    _start: Instant,
) -> Result<Value, String> {
    Err("render: this server was built without the gpu feature; 3D renders need it".into())
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
        // 3D options are checked before anything is drawn.
        for bad in [
            r#"{"mode":"3d","quality":"best"}"#,
            r#"{"mode":"3d","quality":"presentation","spp":0}"#,
            r#"{"mode":"3d","quality":"presentation","spp":-4}"#,
            r#"{"mode":"3d","quality":"presentation","seed":1.5}"#,
            r#"{"mode":"3d","quality":"presentation","denoise":"yes"}"#,
        ] {
            let e = render_to_file(&doc, bad, dir.path()).unwrap_err();
            assert!(e.starts_with("render: "), "{bad}: {e}");
        }
    }
}
