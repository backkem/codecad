//! Server-side script execution via Wasmtime sandbox.
//!
//! Scripts run in a WASM sandbox with access to cad_call, rpc_call, and
//! read_file host functions. All document mutations are batched into a
//! single Yrs update that gets broadcast to connected browsers.

use crate::registry::DocumentRegistry;
use cadview_sandbox::{RunOutput, Sandbox, SandboxError};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const COMPONENT: &[u8] = include_bytes!("../../cadview-sandbox/cadview-sandbox.wasm");
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The cad API setup code prepended to every user script.
/// This reads from the bridge functions set up by runtime-wrapper.js
/// and builds the full `cad` object on globalThis.
const CAD_API_SETUP: &str = include_str!("../../../cad-client/cad-api-setup.js");

/// Create a reusable sandbox. Call once at server startup.
pub fn create_sandbox() -> Result<Sandbox, SandboxError> {
    Sandbox::new(COMPONENT)
}

/// Run a script string against a document.
pub async fn run_script(
    sandbox: &Arc<Sandbox>,
    registry: &Arc<Mutex<DocumentRegistry>>,
    doc_key: &str,
    program: &str,
    base_dir: &Path,
    timeout: Option<Duration>,
) -> Result<RunOutput, String> {
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);

    // Lock registry for the duration of the script (mutations must be exclusive).
    let mut reg = registry.lock().await;
    let slot = reg
        .get_or_load(doc_key)
        .map_err(|e| format!("document '{}': {e}", doc_key))?;

    // Begin batch: all mutations produce a single Yrs update.
    slot.sync_doc.begin_batch(&slot.document);

    // Move document into Arc<Mutex<>> for the closure (needs Send + 'static).
    let doc = std::mem::take(&mut slot.document);
    let doc = Arc::new(std::sync::Mutex::new(doc));

    // Build cad_call handler.
    let doc_c = doc.clone();
    let cad_handler = Box::new(move |method: &str, args: &str| -> Result<String, String> {
        let mut doc = doc_c.lock().unwrap();
        cadview_core::cad_call(&mut doc, method, args)
    });

    // Build rpc_call handler.
    // Reads resolve relative to base_dir (script directory when run via
    // exec_file, CWD for inline scripts). Writes go to CWD.
    let doc_r = doc.clone();
    let read_base = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf());
    let write_base = std::env::current_dir().unwrap_or_else(|_| base_dir.to_path_buf());
    let rpc_sandbox = sandbox.clone();
    let rpc_handler = Box::new(move |method: &str, args: &str| -> Result<String, String> {
        // Helper: resolve a read path relative to base_dir (script directory).
        let resolve = |p: &str| -> Result<PathBuf, String> {
            let candidate = PathBuf::from(p);
            let joined = if candidate.is_absolute() {
                candidate
            } else {
                read_base.join(p)
            };
            let resolved = joined
                .canonicalize()
                .map_err(|e| format!("resolve '{p}': {e}"))?;
            Ok(resolved)
        };
        // Helper: resolve a write path relative to CWD.
        let resolve_write = |p: &str| -> PathBuf {
            let candidate = PathBuf::from(p);
            if candidate.is_absolute() {
                candidate
            } else {
                write_base.join(p)
            }
        };

        // Path-bearing methods: resolve path then delegate to cad_call.
        // Only file I/O (save, saveDwg, loadDwg, exec) has server-specific logic.
        // Everything else goes straight to cadview_core::cad_call.
        let path_methods = ["loadElmt", "loadElmtDir", "loadDwgAsBlock"];
        if path_methods.contains(&method) {
            let mut a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path_str = a["path"]
                .as_str()
                .ok_or(format!("{method}: path required"))?;
            let resolved = resolve(path_str)?;
            a["path"] = serde_json::Value::String(resolved.to_string_lossy().to_string());
            let mut doc = doc_r.lock().unwrap();
            return cadview_core::cad_call(&mut doc, method, &a.to_string());
        }

        match method {
            "save" => {
                let a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let path = a["path"].as_str().unwrap_or("cadview-output.json");
                let resolved = resolve_write(path);
                let doc = doc_r.lock().unwrap();
                let entities: Vec<cadview_core::EntityJson> =
                    doc.entities.iter().map(|e| e.to_json()).collect();
                let json = serde_json::to_string_pretty(&entities).map_err(|e| e.to_string())?;
                std::fs::write(&resolved, &json).map_err(|e| format!("write {path}: {e}"))?;
                Ok(serde_json::json!({
                    "ok": true,
                    "path": resolved.to_string_lossy(),
                    "entities": entities.len(),
                })
                .to_string())
            }
            "saveDwg" => {
                let a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let path = a["path"].as_str().unwrap_or("output.dwg");
                let resolved = resolve_write(path);
                let doc = doc_r.lock().unwrap();
                cadview_core::save_dwg(&doc, &resolved.to_string_lossy())
                    .map_err(|e| format!("dwg: {e}"))?;
                Ok(serde_json::json!({
                    "ok": true,
                    "path": resolved.to_string_lossy(),
                    "entities": doc.entities.len(),
                })
                .to_string())
            }
            "savePdf" => {
                let a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let path = a["path"].as_str().unwrap_or("output.pdf");
                let resolved = resolve_write(path);
                let doc = doc_r.lock().unwrap();
                let opts = cadview_core::pdf::PdfOptions::default();
                let bytes = cadview_core::export_pdf_bytes(&doc, &opts);
                std::fs::write(&resolved, &bytes).map_err(|e| format!("pdf: {e}"))?;
                Ok(serde_json::json!({
                    "ok": true,
                    "path": resolved.to_string_lossy(),
                    "bytes": bytes.len(),
                })
                .to_string())
            }
            "loadDwg" => {
                let a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let path_str = a["path"].as_str().ok_or("loadDwg: path required")?;
                let resolved = resolve(path_str)?;
                let new_doc = cadview_core::load_dwg(&resolved.to_string_lossy())
                    .map_err(|e| format!("loadDwg: {e}"))?;
                let count = new_doc.entities.len();
                let mut doc = doc_r.lock().unwrap();
                *doc = new_doc;
                Ok(format!(r#"{{"ok":true,"entities":{count}}}"#))
            }
            "exec" => {
                let a: serde_json::Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
                let path_str = a["path"].as_str().ok_or("exec: path required")?;
                let resolved = resolve(path_str)?;
                let program = std::fs::read_to_string(&resolved)
                    .map_err(|e| format!("exec read '{}': {e}", resolved.display()))?;
                let exec_base = resolved
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                let full_program = format!("{CAD_API_SETUP}\n{program}");

                // Build fresh handlers sharing the same doc for the nested run.
                let nested_doc_c = doc_r.clone();
                let nested_cad = Box::new(move |m: &str, a: &str| -> Result<String, String> {
                    let mut doc = nested_doc_c.lock().unwrap();
                    cadview_core::cad_call(&mut doc, m, a)
                });
                // Nested exec gets a minimal rpc_handler (no further nesting).
                let nested_doc_r = doc_r.clone();
                let nested_rpc = Box::new(move |m: &str, _a: &str| -> Result<String, String> {
                    // Only allow cad_call passthrough in nested exec.
                    // save/saveDwg/loadElmt from nested scripts would need
                    // the full handler; keep it simple for now.
                    let _ = nested_doc_r; // keep alive
                    Err(format!("RPC '{m}' not available in nested exec"))
                });
                let nested_root = exec_base
                    .canonicalize()
                    .unwrap_or_else(|_| exec_base.clone());
                let nested_read = Box::new(move |p: &str| -> Result<String, String> {
                    let r = nested_root
                        .join(p)
                        .canonicalize()
                        .map_err(|e| format!("resolve '{p}': {e}"))?;
                    if !r.starts_with(&nested_root) {
                        return Err(format!("path escapes project root: {p}"));
                    }
                    std::fs::read_to_string(&r).map_err(|e| format!("read '{p}': {e}"))
                });

                let output = rpc_sandbox
                    .run(
                        &full_program,
                        DEFAULT_TIMEOUT,
                        nested_cad,
                        nested_rpc,
                        nested_read,
                    )
                    .map_err(|e| format!("exec: {e}"))?;

                Ok(serde_json::json!({
                    "ok": true,
                    "value": output.value,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                })
                .to_string())
            }
            _ => {
                // Everything else delegates to cadview_core::cad_call
                let mut doc = doc_r.lock().unwrap();
                cadview_core::cad_call(&mut doc, method, args)
            }
        }
    });

    // Build read_file handler with path sandboxing.
    let project_root = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf());
    let read_file_handler = Box::new(move |path: &str| -> Result<String, String> {
        let resolved = project_root
            .join(path)
            .canonicalize()
            .map_err(|e| format!("resolve '{path}': {e}"))?;
        // Allow reads from project root and its children.
        if !resolved.starts_with(&project_root) {
            return Err(format!("path escapes project root: {path}"));
        }
        std::fs::read_to_string(&resolved).map_err(|e| format!("read '{path}': {e}"))
    });

    // Prepend cad API setup code to the user program.
    let full_program = format!("{CAD_API_SETUP}\n{program}");

    // Run sandbox on a blocking thread. Wasmtime's WASI sync implementation
    // uses block_on internally, which panics inside a tokio async context.
    let result = {
        let sb = sandbox.clone();
        tokio::task::spawn_blocking(move || {
            sb.run(
                &full_program,
                timeout,
                cad_handler,
                rpc_handler,
                read_file_handler,
            )
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    };

    // Move document back.
    slot.document = Arc::try_unwrap(doc)
        .expect("sandbox should be the only holder")
        .into_inner()
        .unwrap();

    // End batch, broadcast single Yrs update.
    let update = slot.sync_doc.end_batch(&slot.document);
    if !update.is_empty() {
        let _ = slot.update_tx.send(update);
    }

    result.map_err(|e| e.to_string())
}

/// Run a .js file from disk against a document.
pub async fn exec_file(
    sandbox: &Arc<Sandbox>,
    registry: &Arc<Mutex<DocumentRegistry>>,
    doc_key: &str,
    script_path: &str,
    timeout: Option<Duration>,
) -> Result<RunOutput, String> {
    let path = PathBuf::from(script_path);
    let abs_path = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("cwd: {e}"))?
            .join(&path)
    };

    let program = std::fs::read_to_string(&abs_path)
        .map_err(|e| format!("read '{}': {e}", abs_path.display()))?;

    let base_dir = abs_path.parent().unwrap_or(Path::new("."));

    run_script(sandbox, registry, doc_key, &program, base_dir, timeout).await
}
