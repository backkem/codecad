//! Server-side script execution in a codelib sandbox.
//!
//! Scripts run in codelib's embedded JavaScript engine. Two preludes give
//! them the `cad` global: [`BRIDGE`] defines the `__cadCall` / `__rpcCall` /
//! `__readFile` functions over codelib's `host.call`, then `cad-api-setup.js`
//! (built from `web/src/cad-api.ts`) builds `cad` on top of them.
//!
//! The document is moved into the run's state and codelib hands it back when
//! the run ends, whatever the script did. All mutations are batched into a
//! single Yrs update that gets broadcast to connected browsers.

use crate::registry::DocumentRegistry;
use cadview_core::Document;
use codelib::{BoxFuture, Console, Failure, HostCalls, Limits, RunError, Sandbox};
use serde_json::Value;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// The sandbox scripts run in. Cheap to clone.
pub type ScriptSandbox = Sandbox<RunState>;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on the JS engine's memory. The document itself lives on the host.
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;
/// Cap on captured stdout and stderr, each (what the old sandbox allowed).
const CONSOLE_LIMIT: usize = 64 * 1024 * 1024;
/// How deep `cad.exec` may nest.
const MAX_EXEC_DEPTH: u32 = 8;
/// A nested `exec` must end this long before its parent's deadline, so the
/// parent is never interrupted while the nested run holds the document.
const EXEC_MARGIN: Duration = Duration::from_millis(250);

/// Builds `globalThis.cad` from the bridge functions below.
const CAD_API_SETUP: &str = include_str!("../../../cad-client/cad-api-setup.js");

/// The globals `cad-api-setup.js` expects, on top of codelib's `host.call`.
///
/// They use `host.callRaw`, which passes strings through untouched: the JSON
/// text built by `cad-api-setup.js` reaches the host as-is and the host's
/// JSON text comes back as-is.
const BRIDGE: &str = r#"
const call = (method, text) => {
  try {
    return host.callRaw(method, text);
  } catch (e) {
    // Uncaught, this reads "Error: <message>", as it always has.
    if (e instanceof host.HostError) throw new Error(e.message);
    throw e;
  }
};
globalThis.__cadCall = (method, argsJson) => call("cad." + method, argsJson);
globalThis.__rpcCall = (method, argsJson) => call("rpc." + method, argsJson);
globalThis.__readFile = (path) => call("readFile", path);
globalThis.__loadLibrary = (name) => call("lib.load", String(name));
"#;

/// Output from a successful run.
#[derive(Debug, Clone)]
pub struct RunOutput {
    /// The program's return value: strings as-is, anything else as JSON.
    /// `None` when it returned nothing (or null, or "").
    pub value: Option<String>,
    /// Captured stdout (console.log output).
    pub stdout: String,
    /// Captured stderr (console.error output).
    pub stderr: String,
}

/// What a run works on: the document, and where its file access is rooted.
pub struct RunState {
    pub document: Document,
    /// Reads (`readFile`, `loadDwg`, `exec`, ...) resolve against this: the
    /// script's directory, or the working directory for inline scripts.
    read_base: PathBuf,
    /// Writes (`save`, `saveDwg`, `savePdf`, `render`) resolve against this.
    write_base: PathBuf,
    /// When this run will be interrupted.
    deadline: Instant,
    /// How many `exec`s deep this run is.
    depth: u32,
    /// The sandbox this run is on, for nested `exec`.
    sandbox: ScriptSandbox,
}

/// Build the sandbox. Call once at startup: the first start on a machine
/// compiles the JS engine (tens of seconds; cached on disk afterwards).
pub async fn create_sandbox() -> anyhow::Result<ScriptSandbox> {
    Ok(tokio::task::spawn_blocking(build_sandbox).await??)
}

fn build_sandbox() -> Result<ScriptSandbox, codelib::Error> {
    Sandbox::<RunState>::builder()
        .prelude("cad-bridge.js", BRIDGE)
        .prelude("cad-api-setup.js", CAD_API_SETUP)
        .host_calls(CadHost)
        .limits(
            Limits::default()
                .timeout(DEFAULT_TIMEOUT)
                .memory(MEMORY_LIMIT)
                .console(CONSOLE_LIMIT),
        )
        .build()
}

/// Run a script string against a document.
pub async fn run_script(
    sandbox: &ScriptSandbox,
    registry: &Arc<Mutex<DocumentRegistry>>,
    doc_key: &str,
    program: &str,
    base_dir: &Path,
    timeout: Option<Duration>,
) -> Result<RunOutput, String> {
    run_on_document(
        sandbox,
        registry,
        doc_key,
        program.to_string(),
        "script.js".to_string(),
        base_dir,
        timeout,
    )
    .await
}

/// Run a .js file from disk against a document.
pub async fn exec_file(
    sandbox: &ScriptSandbox,
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

    run_on_document(
        sandbox,
        registry,
        doc_key,
        program,
        script_path.to_string(),
        base_dir,
        timeout,
    )
    .await
}

async fn run_on_document(
    sandbox: &ScriptSandbox,
    registry: &Arc<Mutex<DocumentRegistry>>,
    doc_key: &str,
    program: String,
    name: String,
    base_dir: &Path,
    timeout: Option<Duration>,
) -> Result<RunOutput, String> {
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let sandbox = sandbox.clone();
    let registry = registry.clone();
    let doc_key = doc_key.to_string();
    let read_base = base_dir
        .canonicalize()
        .unwrap_or_else(|_| base_dir.to_path_buf());

    // On a task of its own, so a caller that goes away (an HTTP client
    // disconnecting) cannot drop the run while it holds the document.
    tokio::spawn(async move {
        // Lock the registry for the whole run: mutations must be exclusive.
        let mut reg = registry.lock().await;
        let slot = reg
            .get_or_load(&doc_key)
            .map_err(|e| format!("document '{}': {e}", doc_key))?;

        // Begin batch: all mutations produce a single Yrs update.
        slot.sync_doc.begin_batch(&slot.document);

        let state = RunState {
            document: std::mem::take(&mut slot.document),
            read_base,
            write_base: std::env::current_dir().unwrap_or_default(),
            deadline: Instant::now() + timeout,
            depth: 0,
            sandbox: sandbox.clone(),
        };
        let outcome = sandbox
            .script(program)
            .name(name)
            .timeout(timeout)
            .run_with(state)
            .await;
        // The state always comes back, error or not.
        slot.document = outcome.state.document;

        // End batch, broadcast single Yrs update.
        let update = slot.sync_doc.end_batch(&slot.document);
        if !update.is_empty() {
            let _ = slot.update_tx.send(update);
        }

        to_output(outcome.result, outcome.console)
    })
    .await
    .map_err(|e| format!("script task: {e}"))?
}

/// Turn a run's result into what callers have always received.
fn to_output(result: Result<Value, RunError>, console: Console) -> Result<RunOutput, String> {
    let value = match result {
        Ok(Value::Null) => None,
        Ok(Value::String(s)) if s.is_empty() => None,
        Ok(Value::String(s)) => Some(s),
        Ok(other) => Some(other.to_string()),
        Err(RunError::Script(error)) => {
            let mut message = format!("program error: {}: {}", error.name, error.message);
            // Where it happened, in the user's program (not the preludes).
            if let Some(at) = error.stack.as_deref().and_then(program_frame) {
                message.push_str(&format!(" (at {at})"));
            }
            return Err(message);
        }
        Err(RunError::Timeout(timeout)) => {
            return Err(format!("execution timed out after {timeout:?}"))
        }
        Err(error) => return Err(format!("execution failed: {error}")),
    };
    Ok(RunOutput {
        value,
        stdout: console.stdout,
        stderr: console.stderr,
    })
}

/// The innermost stack frame that is not in a prelude, as `file:line:col`.
/// (`ScriptError::location` only reports the very first frame, which for an
/// error thrown from a `cad.*` call is inside a prelude.)
fn program_frame(stack: &str) -> Option<&str> {
    stack
        .lines()
        .filter_map(|frame| frame.split_once('@').map(|(_, at)| at.trim()))
        .find(|at| {
            !at.is_empty()
                && !at.starts_with("cad-bridge.js")
                && !at.starts_with("cad-api-setup.js")
        })
}

// ── Host side of `host.call` ────────────────────────────────────────

/// Answers the bridge's `cad.*`, `rpc.*` and `readFile` calls.
struct CadHost;

impl HostCalls<RunState> for CadHost {
    fn call<'a>(
        &'a self,
        state: &'a mut RunState,
        method: &'a str,
        args: &'a str,
    ) -> BoxFuture<'a, Result<String, Failure>> {
        Box::pin(async move {
            // `args` and the result are the raw strings of `host.callRaw`.
            let result = match method.split_once('.') {
                Some(("cad", m)) => {
                    guarded(|| cadview_core::cad_call(&mut state.document, m, args))
                }
                Some(("rpc", "exec")) => exec(state, args).await,
                Some(("rpc", m)) => guarded(|| rpc_call(state, m, args)),
                Some(("lib", "load")) => Ok(load_library(&state.read_base, args)),
                None if method == "readFile" => read_file(&state.read_base, args),
                _ => return Err(Failure::unknown_method(method)),
            };
            result.map_err(|message| Failure::new("cad", message))
        })
    }
}

/// Run a synchronous handler, turning a panic into an error so the
/// document stays in the run's state.
fn guarded(f: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let what = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        Err(format!("internal error: {what}"))
    })
}

/// Resolve a read path relative to the script's directory.
fn resolve_read(base: &Path, p: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(p);
    let joined = if candidate.is_absolute() {
        candidate
    } else {
        base.join(p)
    };
    joined
        .canonicalize()
        .map_err(|e| format!("resolve '{p}': {e}"))
}

/// Resolve a write path relative to the working directory.
fn resolve_write(base: &Path, p: &str) -> PathBuf {
    let candidate = PathBuf::from(p);
    if candidate.is_absolute() {
        candidate
    } else {
        base.join(p)
    }
}

/// `cad.readFile`: reads stay inside the script's directory.
fn read_file(root: &Path, path: &str) -> Result<String, String> {
    let resolved = root
        .join(path)
        .canonicalize()
        .map_err(|e| format!("resolve '{path}': {e}"))?;
    if !resolved.starts_with(root) {
        return Err(format!("path escapes project root: {path}"));
    }
    std::fs::read_to_string(&resolved).map_err(|e| format!("read '{path}': {e}"))
}

/// `cad.use(name)`: the text of the script `lib/<name>.js` under `root`,
/// as the JSON `{id, code}`, or `{error}`. A name is a plain word (letters,
/// digits, `-`, `_`), so it is never a path; the file is read under the
/// same rule as `cad.readFile`.
pub fn load_library(root: &Path, name: &str) -> String {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
        let error = format!("a library name is letters, digits, '-' and '_', got {name:?}");
        return serde_json::json!({ "error": error }).to_string();
    }
    let id = format!("lib/{name}.js");
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    match read_file(&root, &id) {
        Ok(code) => serde_json::json!({ "id": id, "code": code }),
        Err(_) if !root.join(&id).exists() => {
            serde_json::json!({ "error": format!("library \"{name}\" not found (no file {id})") })
        }
        Err(e) => serde_json::json!({ "error": format!("library \"{name}\": {e}") }),
    }
    .to_string()
}

/// The synchronous rpc methods. File I/O has server-specific logic;
/// everything else goes straight to `cadview_core::cad_call`.
fn rpc_call(state: &mut RunState, method: &str, args: &str) -> Result<String, String> {
    let doc = &mut state.document;
    match method {
        // Path-bearing methods: resolve the path, then delegate to cad_call.
        "loadElmt" | "loadElmtDir" | "loadDwgAsBlock" => {
            let mut a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path_str = a["path"]
                .as_str()
                .ok_or(format!("{method}: path required"))?;
            let resolved = resolve_read(&state.read_base, path_str)?;
            a["path"] = Value::String(resolved.to_string_lossy().to_string());
            cadview_core::cad_call(doc, method, &a.to_string())
        }
        "save" => {
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path = a["path"].as_str().unwrap_or("cadview-output.json");
            let resolved = resolve_write(&state.write_base, path);
            let entities: Vec<cadview_core::EntityJson> =
                doc.entities.iter().map(|e| e.to_json(doc)).collect();
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
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path = a["path"].as_str().unwrap_or("output.dwg");
            let resolved = resolve_write(&state.write_base, path);
            cadview_core::save_dwg(doc, &resolved.to_string_lossy())
                .map_err(|e| format!("dwg: {e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "path": resolved.to_string_lossy(),
                "entities": doc.entities.len(),
                // Which attributes the DWG keeps and which it cannot hold.
                "attrs": cadview_core::dwg_attr_losses(doc),
                // DWG holds 2D only; parts are left out (saveJson keeps them).
                "parts": cadview_core::dispatch_3d::dwg_part_losses(doc),
            })
            .to_string())
        }
        // The lossless document file (attributes, blocks, ids, styles).
        "saveJson" => {
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path = a["path"].as_str().unwrap_or("document.json");
            let resolved = resolve_write(&state.write_base, path);
            let text = cadview_core::document_to_json(doc)?;
            std::fs::write(&resolved, &text).map_err(|e| format!("write {path}: {e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "path": resolved.to_string_lossy(),
                "entities": doc.entities.len(),
                "bytes": text.len(),
                "parts": doc.parts.len(),
            })
            .to_string())
        }
        "loadJson" => {
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path_str = a["path"].as_str().ok_or("loadJson: path required")?;
            let resolved = resolve_read(&state.read_base, path_str)?;
            let text = std::fs::read_to_string(&resolved)
                .map_err(|e| format!("loadJson: read '{path_str}': {e}"))?;
            let new_doc = cadview_core::document_from_json(&text)?;
            let (count, parts) = (new_doc.entities.len(), new_doc.parts.len());
            doc.replace_keeping_solids(new_doc);
            Ok(format!(
                r#"{{"ok":true,"entities":{count},"parts":{parts}}}"#
            ))
        }
        "savePdf" => {
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path = a["path"].as_str().unwrap_or("output.pdf");
            let opts = cadview_core::pdf::PdfOptions::from_args(&a)?;
            let resolved = resolve_write(&state.write_base, path);
            let bytes = cadview_core::export_pdf_bytes(doc, &opts);
            std::fs::write(&resolved, &bytes).map_err(|e| format!("pdf: {e}"))?;
            Ok(serde_json::json!({
                "ok": true,
                "path": resolved.to_string_lossy(),
                "bytes": bytes.len(),
            })
            .to_string())
        }
        "render" => {
            crate::render::render_to_file(doc, args, &state.write_base).map(|v| v.to_string())
        }
        "loadDwg" => {
            let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
            let path_str = a["path"].as_str().ok_or("loadDwg: path required")?;
            let resolved = resolve_read(&state.read_base, path_str)?;
            let new_doc = cadview_core::load_dwg(&resolved.to_string_lossy())
                .map_err(|e| format!("loadDwg: {e}"))?;
            let count = new_doc.entities.len();
            doc.replace_keeping_solids(new_doc);
            Ok(format!(r#"{{"ok":true,"entities":{count}}}"#))
        }
        _ => cadview_core::cad_call(doc, method, args),
    }
}

/// `cad.exec(path)`: run another script file against the same document.
///
/// The nested run gets the document by value and hands it back through its
/// own outcome, exactly like a top-level run. Its time budget ends before
/// this run's deadline, so the parent is never interrupted while the nested
/// run holds the document.
async fn exec(state: &mut RunState, args: &str) -> Result<String, String> {
    let a: Value = serde_json::from_str(args).map_err(|e| e.to_string())?;
    let path_str = a["path"].as_str().ok_or("exec: path required")?;
    let resolved = resolve_read(&state.read_base, path_str)?;
    let program = std::fs::read_to_string(&resolved)
        .map_err(|e| format!("exec read '{}': {e}", resolved.display()))?;
    if state.depth >= MAX_EXEC_DEPTH {
        return Err(format!("exec: nested more than {MAX_EXEC_DEPTH} deep"));
    }
    let left = state
        .deadline
        .saturating_duration_since(Instant::now())
        .saturating_sub(EXEC_MARGIN);
    if left.is_zero() {
        return Err("exec: no time left".to_string());
    }
    let timeout = left.min(DEFAULT_TIMEOUT);

    let nested = RunState {
        document: std::mem::take(&mut state.document),
        read_base: resolved
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        write_base: state.write_base.clone(),
        deadline: Instant::now() + timeout,
        depth: state.depth + 1,
        sandbox: state.sandbox.clone(),
    };
    let outcome = state
        .sandbox
        .script(program)
        .name(path_str)
        .timeout(timeout)
        .run_with(nested)
        .await;
    state.document = outcome.state.document;

    let output = to_output(outcome.result, outcome.console).map_err(|e| format!("exec: {e}"))?;
    Ok(serde_json::json!({
        "ok": true,
        "value": output.value,
        "stdout": output.stdout,
        "stderr": output.stderr,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    /// One compiled engine for all tests (the first ever build is slow).
    static SANDBOX: LazyLock<ScriptSandbox> =
        LazyLock::new(|| build_sandbox().expect("the sandbox builds"));

    fn registry() -> (tempfile::TempDir, Arc<Mutex<DocumentRegistry>>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Box::new(crate::store::FolderStore::new(dir.path().to_path_buf()));
        let mut reg = DocumentRegistry::new(store);
        reg.insert("doc".to_string(), Document::new(), None);
        (dir, Arc::new(Mutex::new(reg)))
    }

    async fn entity_count(registry: &Arc<Mutex<DocumentRegistry>>) -> usize {
        registry
            .lock()
            .await
            .get("doc")
            .unwrap()
            .document
            .entities
            .len()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_script_draws_on_the_document_and_broadcasts_one_update() {
        let (dir, registry) = registry();
        let mut updates = registry
            .lock()
            .await
            .get("doc")
            .unwrap()
            .update_tx
            .subscribe();

        let output = run_script(
            &SANDBOX,
            &registry,
            "doc",
            r#"
            cad.addLine([0, 0], [100, 0]);
            cad.addCircle([50, 50], 25);
            console.log("drawn");
            return cad.describe();
            "#,
            dir.path(),
            None,
        )
        .await
        .unwrap();

        let described: Value = serde_json::from_str(output.value.as_deref().unwrap()).unwrap();
        assert_eq!(described["entities"], 2, "{described}");
        assert_eq!(output.stdout, "drawn\n");
        assert_eq!(entity_count(&registry).await, 2);
        assert!(!updates.try_recv().unwrap().is_empty());
        assert!(updates.try_recv().is_err(), "one batched update");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn returned_objects_keep_their_key_order() {
        let (dir, registry) = registry();
        let output = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "return { zed: 1, alpha: 2, mid: 3 };",
            dir.path(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            output.value.as_deref(),
            Some(r#"{"zed":1,"alpha":2,"mid":3}"#)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_keep_the_document_and_name_the_line() {
        let (dir, registry) = registry();
        let error = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "cad.addLine([0, 0], [1, 1]);\ncad.entity('no-such-id');\n",
            dir.path(),
            None,
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("program error: Error: "), "{error}");
        assert!(error.contains("script.js:2:"), "{error}");
        assert_eq!(entity_count(&registry).await, 1);

        let error = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "cad.addLine([0, 0], [2, 2]); while (true) {}",
            dir.path(),
            Some(Duration::from_millis(500)),
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("execution timed out"), "{error}");
        assert_eq!(entity_count(&registry).await, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn render_rpc_writes_a_png() {
        let (dir, registry) = registry();
        let out = dir.path().join("renders").join("r.png");
        let out_js = serde_json::to_string(&out.to_string_lossy()).unwrap();
        // Through the raw bridge, so this works whether or not the committed
        // cad-api-setup.js already has `cad.render`.
        let program = format!(
            "cad.addLine([0, 0], [100, 50]);\n\
             return JSON.parse(__rpcCall('render', JSON.stringify({{ out: {out_js}, size: [96, 64] }})));"
        );
        match run_script(&SANDBOX, &registry, "doc", &program, dir.path(), None).await {
            Ok(output) => {
                let v: Value = serde_json::from_str(output.value.as_deref().unwrap()).unwrap();
                assert_eq!(v["ok"], true, "{v}");
                assert_eq!(
                    (v["width"].as_u64(), v["height"].as_u64()),
                    (Some(96), Some(64))
                );
                let png = std::fs::read(&out).unwrap();
                assert_eq!(v["bytes"].as_u64(), Some(png.len() as u64));
            }
            Err(e) if e.contains("render:") => eprintln!("SKIPPED render_rpc_writes_a_png: {e}"),
            Err(e) => panic!("{e}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exec_runs_a_file_against_the_same_document() {
        let (dir, registry) = registry();
        std::fs::write(dir.path().join("part.txt"), "7").unwrap();
        std::fs::write(
            dir.path().join("part.js"),
            "const n = Number(cad.readFile('part.txt'));\n\
             for (let i = 0; i < n; i++) cad.addLine([i, 0], [i, 10]);\n\
             console.log('nested');\n\
             return cad.describe().entities;",
        )
        .unwrap();

        let output = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "cad.addCircle([0, 0], 5); const r = cad.exec('part.js'); return [r.value, r.stdout, cad.describe().entities];",
            dir.path(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(output.value.as_deref(), Some(r#"["8","nested\n",8]"#));
        assert_eq!(entity_count(&registry).await, 8);

        // A failing nested run is an error in the parent; the document stays.
        std::fs::write(
            dir.path().join("bad.js"),
            "cad.addLine([0,0],[1,0]); null.x;",
        )
        .unwrap();
        let output = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "try { cad.exec('bad.js'); } catch (e) { return e.message; }",
            dir.path(),
            None,
        )
        .await
        .unwrap();
        assert!(
            output
                .value
                .as_deref()
                .unwrap()
                .starts_with("exec: program error: TypeError"),
            "{output:?}"
        );
        assert_eq!(entity_count(&registry).await, 9);
    }

    // ── cad.use: script libraries ───────────────────────────────────

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// Run `program` as the file `main.js` in `dir`, the way `/api/run`
    /// runs `{"exec": ...}`.
    async fn exec_main(
        dir: &Path,
        registry: &Arc<Mutex<DocumentRegistry>>,
        program: &str,
    ) -> Result<RunOutput, String> {
        write(dir, "main.js", program);
        let path = dir.join("main.js");
        exec_file(
            &SANDBOX,
            registry,
            "doc",
            path.to_str().unwrap(),
            Some(Duration::from_secs(10)),
        )
        .await
    }

    fn value(output: &RunOutput) -> Value {
        serde_json::from_str(output.value.as_deref().unwrap()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_runs_once_and_returns_what_it_returns() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/geom.js",
            "globalThis.geomRuns = (globalThis.geomRuns ?? 0) + 1;\n\
             const double = (x) => 2 * x;\n\
             return { double };\n",
        );
        // A library loads another library the same way a script does.
        write(
            dir.path(),
            "lib/shapes.js",
            "const geom = await cad.use(\"geom\");\n\
             return { quad: (x) => geom.double(geom.double(x)) };\n",
        );
        let output = exec_main(
            dir.path(),
            &registry,
            "const shapes = await cad.use(\"shapes\");\n\
             const geom = await cad.use(\"geom\");\n\
             const again = await cad.use(\"geom\");\n\
             return { quad: shapes.quad(3), same: geom === again, runs: globalThis.geomRuns };",
        )
        .await
        .unwrap();
        assert_eq!(
            value(&output),
            serde_json::json!({ "quad": 12, "same": true, "runs": 1 })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_cannot_create_globals_by_accident() {
        let (dir, registry) = registry();
        write(dir.path(), "lib/leaky.js", "leak = 1;\nreturn {};\n");
        let output = exec_main(
            dir.path(),
            &registry,
            "try { await cad.use(\"leaky\"); return \"loaded\"; }\n\
             catch (e) { return [e.name, typeof globalThis.leak]; }",
        )
        .await
        .unwrap();
        assert_eq!(
            value(&output),
            serde_json::json!(["ReferenceError", "undefined"])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_run_evaluates_its_libraries_afresh() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/count.js",
            "globalThis.n = (globalThis.n ?? 0) + 1;\nreturn { n: globalThis.n };\n",
        );
        for _ in 0..2 {
            let output = exec_main(
                dir.path(),
                &registry,
                "return (await cad.use(\"count\")).n;",
            )
            .await
            .unwrap();
            assert_eq!(value(&output), serde_json::json!(1));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_name_the_library_file_and_line() {
        let (dir, registry) = registry();
        // Thrown while the library loads: line 3 of the file.
        write(
            dir.path(),
            "lib/bad.js",
            "const a = 1;\n\nthrow new RangeError(\"bad setup\");\n",
        );
        // Thrown later, from a function the library returned: line 2.
        write(
            dir.path(),
            "lib/late.js",
            "function boom() {\n  throw new Error(\"later\");\n}\nreturn { boom };\n",
        );
        let output = exec_main(
            dir.path(),
            &registry,
            "const first = (e) => e.stack.split(\"\\n\")[0];\n\
             const out = [];\n\
             try { await cad.use(\"bad\"); } catch (e) { out.push(e.name, e.message, first(e)); }\n\
             const late = await cad.use(\"late\");\n\
             try { late.boom(); } catch (e) { out.push(first(e)); }\n\
             return out;",
        )
        .await
        .unwrap();
        let v = value(&output);
        assert_eq!(v[0], "RangeError");
        assert_eq!(v[1], "bad setup");
        assert!(v[2].as_str().unwrap().contains("lib/bad.js:3"), "{v}");
        assert!(v[3].as_str().unwrap().contains("lib/late.js:2"), "{v}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_library_names_itself() {
        let (dir, registry) = registry();
        let output = exec_main(
            dir.path(),
            &registry,
            "try { await cad.use(\"nope\"); return \"loaded\"; } catch (e) { return e.message; }",
        )
        .await
        .unwrap();
        assert_eq!(
            output.value.as_deref(),
            Some("cad.use: library \"nope\" not found (no file lib/nope.js)")
        );
        // Uncaught, it fails the run with the same message.
        let failed = exec_main(dir.path(), &registry, "await cad.use(\"nope\");")
            .await
            .unwrap_err();
        assert!(failed.contains("library \"nope\" not found"), "{failed}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_libraries_that_wait_for_each_other_end_the_run() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/a.js",
            "const b = await cad.use(\"b\");\nreturn {};\n",
        );
        write(
            dir.path(),
            "lib/b.js",
            "const a = await cad.use(\"a\");\nreturn {};\n",
        );
        // Not a hang: the engine sees that nothing can make progress.
        let failed = exec_main(dir.path(), &registry, "await cad.use(\"a\");")
            .await
            .unwrap_err();
        assert!(
            failed.contains("waiting for something that can never happen"),
            "{failed}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_name_is_never_a_path() {
        let (dir, registry) = registry();
        // The script runs in sub/; secret.js is next to sub/, outside it.
        write(dir.path(), "secret.js", "return { key: 42 };\n");
        write(dir.path(), "sub/lib/ok.js", "return { ok: true };\n");
        let secret = dir.path().join("secret.js").canonicalize().unwrap();
        let secret_js = serde_json::to_string(&secret.to_string_lossy()).unwrap();
        let cases = [
            ("\"ok\"", "loaded"),
            ("\"../secret\"", "a library name is"),
            ("\"./ok\"", "a library name is"),
            ("\"lib/ok\"", "a library name is"),
            ("\"ok.js\"", "a library name is"),
            ("\"\"", "a library name is"),
            ("42", "not found"),
            (secret_js.as_str(), "a library name is"),
            ("\"https://example.com/lib.js\"", "a library name is"),
        ];
        for (name, expected) in cases {
            write(
                &dir.path().join("sub"),
                "main.js",
                &format!("try {{ await cad.use({name}); return \"loaded\"; }} catch (e) {{ return e.message; }}"),
            );
            let output = exec_file(
                &SANDBOX,
                &registry,
                "doc",
                dir.path().join("sub/main.js").to_str().unwrap(),
                None,
            )
            .await
            .unwrap();
            let message = output.value.unwrap();
            assert!(message.contains(expected), "{name}: {message}");
        }
    }

    // ── Attributes and set queries (rung A8) ─────────────────────────

    /// Run one of the scripts in `tests/a8/` the way `/api/run` runs
    /// `{"exec": ...}`, and parse its return value.
    async fn exec_a8(registry: &Arc<Mutex<DocumentRegistry>>, name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/a8")
            .join(name);
        let output = exec_file(&SANDBOX, registry, "doc", path.to_str().unwrap(), None)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        value(&output)
    }

    /// Run an inline program the way `/api/run` runs `{"program": ...}`.
    async fn program(dir: &Path, registry: &Arc<Mutex<DocumentRegistry>>, js: &str) -> Value {
        let output = run_script(&SANDBOX, registry, "doc", js, dir, None)
            .await
            .unwrap_or_else(|e| panic!("{e}\n{js}"));
        value(&output)
    }

    /// Save the document as JSON, clear it, load it back; then the same with
    /// DWG. Returns what each step reported.
    async fn json_and_dwg_round_trip(
        dir: &Path,
        registry: &Arc<Mutex<DocumentRegistry>>,
    ) -> (Value, Value) {
        let json_path = serde_json::to_string(&dir.join("doc.json").to_string_lossy()).unwrap();
        let dwg_path = serde_json::to_string(&dir.join("doc.dwg").to_string_lossy()).unwrap();
        let json = program(
            dir,
            registry,
            &format!(
                 "const saved = cad.saveJson({json_path});\n\
                 cad.clear({{ all: true }});\n\
                 const cleared = cad.describe().entities;\n\
                 const loaded = cad.loadJson({json_path});\n\
                 return {{ saved: saved.entities, cleared, loaded: loaded.entities, attrs: cad.describe().attrs }};"
            ),
        )
        .await;
        let dwg = program(
            dir,
            registry,
            &format!(
                "const saved = cad.saveDwg({dwg_path});\n\
                 cad.clear({{ all: true }});\n\
                 const loaded = cad.loadDwg({dwg_path});\n\
                 return {{ saved: saved.attrs, loaded: loaded.entities, attrs: cad.describe().attrs }};"
            ),
        )
        .await;
        (json, dwg)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_clearance_rule_runs_on_attributes_that_survive_json_and_dwg() {
        let (dir, registry) = registry();
        assert_eq!(exec_a8(&registry, "pcb-board.js").await, 7);
        // Attributes from a script: set on a selection, read back typed.
        let v = program(
            dir.path(),
            &registry,
             "const r = cad.setAttrs({ layer: 'CU', has: ['net'] }, { checked: true });\n\
             return {\n\
               updated: r.updated.length,\n\
               gnd: cad.entities({ attrs: { net: 'GND' } }).map((e) => e.attrs.name),\n\
               pad1: cad.entities({ expand: true, attrs: { pin: '1' } }).map((e) => [e.attrs.ref, e.attrs['net.1'], e.child]),\n\
             };",
        )
        .await;
        assert_eq!(
            v,
            serde_json::json!({"updated": 5, "gnd": ["g1", "g2"], "pad1": [["R1", "GND", 0]]})
        );

        let expected = serde_json::json!({
            "violations": [
                {"between": ["R1.1", "v2"], "gap": 0.15},
                {"between": ["g1", "v1"], "gap": 0.15},
                {"between": ["g2", "s1"], "gap": 0},
            ],
            "shorts": [{"nets": ["GND", "SIG"], "at": [[10, 3]]}],
        });
        assert_eq!(exec_a8(&registry, "pcb-clearance.js").await, expected);

        let (json, dwg) = json_and_dwg_round_trip(dir.path(), &registry).await;
        let attrs = serde_json::json!({"checked": 5, "name": 5, "net": 5, "net.1": 1, "net.2": 1, "ref": 1});
        assert_eq!(
            json,
            serde_json::json!({"saved": 7, "cleared": 0, "loaded": 7, "attrs": attrs})
        );
        // DWG keeps entity and pad attributes; the footprint's own name is
        // the one thing it cannot hold, and the save says so.
        assert_eq!(dwg["saved"]["kept"], 6, "{dwg}");
        assert_eq!(dwg["saved"]["droppedBlocks"], serde_json::json!(["R0603"]));
        assert_eq!(dwg["attrs"], attrs, "{dwg}");
        assert_eq!(exec_a8(&registry, "pcb-clearance.js").await, expected);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_free_space_rule_runs_on_attributes_that_survive_json_and_dwg() {
        let (dir, registry) = registry();
        assert_eq!(exec_a8(&registry, "play-area.js").await, 6);
        let expected = serde_json::json!([
            "slide is not inside the site",
            "swing and bench overlap",
            "tower and sandbox are 100 apart, need 1500",
            "sandbox and swing are 1200 apart, need 2000",
        ]);
        assert_eq!(exec_a8(&registry, "play-free-space.js").await, expected);
        let (json, dwg) = json_and_dwg_round_trip(dir.path(), &registry).await;
        let attrs = serde_json::json!({"free": 5, "item": 5, "role": 1});
        assert_eq!(json["attrs"], attrs, "{json}");
        assert_eq!(dwg["attrs"], attrs, "{dwg}");
        assert_eq!(dwg["saved"]["droppedEntities"], serde_json::json!([]));
        assert_eq!(exec_a8(&registry, "play-free-space.js").await, expected);
    }

    // ── Solid parts (rung 3D1a) ──────────────────────────────────────

    /// Build the twelve-part assembly in `tests/3d1a/assembly.js` the way
    /// `/api/run` runs `{"exec": ...}`.
    async fn assembly(registry: &Arc<Mutex<DocumentRegistry>>) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/3d1a/assembly.js");
        let output = exec_file(&SANDBOX, registry, "doc", path.to_str().unwrap(), None)
            .await
            .unwrap_or_else(|e| panic!("assembly.js: {e}"));
        value(&output)
    }

    fn rel_err(got: f64, want: f64) -> f64 {
        (got - want).abs() / want.abs()
    }

    fn assert_box(got: &Value, min: [f64; 3], max: [f64; 3], what: &str) {
        for i in 0..3 {
            for (k, want) in [("min", min[i]), ("max", max[i])] {
                let g = got[k][i].as_f64().unwrap();
                assert!(
                    (g - want).abs() <= 1e-9 * want.abs().max(1.0),
                    "{what} bbox.{k}[{i}] = {g}, want {want}"
                );
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_assembly_matches_its_closed_forms_and_survives_json() {
        use std::f64::consts::PI;
        let (dir, registry) = registry();
        let v = assembly(&registry).await;
        let parts = v["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 12, "{v}");
        let d = &v["describe"];
        assert_eq!(
            (d["parts"].as_u64(), d["partDefs"].as_u64()),
            (Some(12), Some(5)),
            "{d}"
        );
        assert_eq!(d["partAttrs"]["group"], 12);
        assert!(
            d["solidKernel"].as_str().unwrap().starts_with("arris"),
            "{d}"
        );

        // Hand-computed closed forms (mm, mm^3).
        let base_v =
            400.0 * 300.0 * 12.0 - 4.0 * PI * 25.0 * 12.0 - (100.0 * 20.0 + PI * 100.0) * 12.0;
        let upright_v = 300.0 * 200.0 * 10.0 - 6.0 * 300.0 * 10.0 - PI * 16.0 * 5.0;
        let shelf_v = 352.0 * 300.0 * 10.0 - 2.0 * PI * 9.0 * 10.0 + 352.0 * 10.0 * 5.0;
        let pin_v = PI * 25.0 * 20.0;
        let foot_v = 40.0 * 40.0 * 8.0;
        let ids = &v["ids"];
        let mut seen = std::collections::BTreeMap::new();
        for p in parts {
            let group = p["group"].as_str().unwrap();
            *seen.entry(group.to_string()).or_insert(0) += 1;
            let want = match group {
                "base" => base_v,
                "upright" => upright_v,
                "shelf" => shelf_v,
                "pin" => pin_v,
                "foot" => foot_v,
                g => panic!("unexpected group {g}"),
            };
            let got = p["volume"].as_f64().unwrap();
            assert!(
                rel_err(got, want) <= 1e-9,
                "{group} volume {got}, want {want}"
            );
        }
        assert_eq!(
            seen,
            [
                ("base", 1),
                ("foot", 4),
                ("pin", 4),
                ("shelf", 1),
                ("upright", 2)
            ]
            .into_iter()
            .map(|(k, n)| (k.to_string(), n))
            .collect()
        );
        let by_id = |id: &Value| parts.iter().find(|p| p["id"] == *id).unwrap();
        assert_box(
            &by_id(&ids["base"])["bbox"],
            [0.0, 0.0, 0.0],
            [400.0, 300.0, 12.0],
            "base",
        );
        assert_box(
            &by_id(&ids["left"])["bbox"],
            [20.0, 0.0, 12.0],
            [30.0, 300.0, 212.0],
            "left",
        );
        // Half a turn about the vertical axis through (200, 150).
        assert_box(
            &by_id(&ids["right"])["bbox"],
            [370.0, 0.0, 12.0],
            [380.0, 300.0, 212.0],
            "right",
        );
        assert_eq!(by_id(&ids["right"])["def"], by_id(&ids["left"])["def"]);
        assert_eq!(by_id(&ids["left"])["instances"], 2);
        assert_box(
            &by_id(&ids["shelf"])["bbox"],
            [24.0, 0.0, 100.0],
            [376.0, 300.0, 115.0],
            "shelf",
        );
        assert_box(
            &by_id(&ids["pin"])["bbox"],
            [25.0, 25.0, -4.0],
            [35.0, 35.0, 16.0],
            "pin",
        );
        assert_box(
            &by_id(&ids["foot"])["bbox"],
            [50.0, 50.0, -8.0],
            [90.0, 90.0, 0.0],
            "foot",
        );
        let pins: Vec<&Value> = parts.iter().filter(|p| p["group"] == "pin").collect();
        assert!(pins.iter().all(|p| p["instances"] == 4));
        assert_box(
            &pins[3]["bbox"],
            [365.0, 265.0, -4.0],
            [375.0, 275.0, 16.0],
            "last pin",
        );

        // Centroids: a pin's is its axis midpoint; the left upright's is the
        // volume-weighted mean of the board, the groove and the blind hole.
        let c = &by_id(&ids["pin"])["centroid"];
        for (i, want) in [30.0, 30.0, 6.0].into_iter().enumerate() {
            assert!(
                (c[i].as_f64().unwrap() - want).abs() <= 1e-9,
                "pin centroid {c}"
            );
        }
        let (vb, vg, vh) = (600_000.0, 18_000.0, PI * 16.0 * 5.0);
        let want = [
            (vb * 25.0 - vg * 27.0 - vh * 22.5) / upright_v,
            150.0,
            (vb * 112.0 - vg * 105.0 - vh * 180.0) / upright_v,
        ];
        let c = &by_id(&ids["left"])["centroid"];
        for i in 0..3 {
            assert!(
                rel_err(c[i].as_f64().unwrap(), want[i]) <= 1e-9,
                "left centroid {c}, want {want:?}"
            );
        }

        // JSON keeps every part; DWG says it left them out.
        let json_path =
            serde_json::to_string(&dir.path().join("a.json").to_string_lossy()).unwrap();
        let dwg_path = serde_json::to_string(&dir.path().join("a.dwg").to_string_lossy()).unwrap();
        let after = program(
            dir.path(),
            &registry,
            &format!(
                "const before = cad.parts();\n\
                 const saved = cad.saveJson({json_path});\n\
                 const dwg = cad.saveDwg({dwg_path});\n\
                 cad.clear({{ all: true }});\n\
                 const cleared = cad.describe().parts;\n\
                 const loaded = cad.loadJson({json_path});\n\
                 const again = cad.parts();\n\
                 return {{ saved: saved.parts, dwg: dwg.parts, cleared, loaded: loaded.parts,\n\
                           same: JSON.stringify(before) === JSON.stringify(again) }};"
            ),
        )
        .await;
        assert_eq!(after["saved"], 12, "{after}");
        assert_eq!(after["dwg"]["dropped"], 12, "{after}");
        assert_eq!(after["cleared"], 0);
        assert_eq!(after["loaded"], 12);
        assert_eq!(after["same"], true, "{after}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_part_call_runs_on_the_kernel_to_its_closed_form() {
        use std::f64::consts::PI;
        let (_dir, registry) = registry();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/3d1a/calls.js");
        let output = exec_file(&SANDBOX, &registry, "doc", path.to_str().unwrap(), None)
            .await
            .unwrap_or_else(|e| panic!("calls.js: {e}"));
        let v = value(&output);
        let plate = 100.0 * 50.0 * 18.0;
        let want = [
            ("bulge", (4000.0 + PI * 2500.0 / 2.0) * 6.0),
            ("clockwise", 24_000.0),
            ("tube", PI * (400.0 - 100.0) * 50.0),
            // Pappus: a disc of radius 6 along 100 of line and half a turn
            // of radius 50.
            ("sweep", PI * 36.0 * (100.0 + 50.0 * PI)),
            // Prismatoid: h / 3 (A1 + A2 + sqrt(A1 A2)).
            ("loft", 100.0 * (1600.0 + 576.0 + 960.0)),
            ("fromEntity", (1200.0 + 50.0 * PI) * 10.0),
            ("intersect", 50.0 * 25.0 * 18.0),
            ("union", plate + 100.0 * 10.0 * 5.0),
            (
                "cbore",
                plate - PI * 6.25 * 18.0 - PI * (20.25 - 6.25) * 3.0,
            ),
            ("fillet", plate - (1.0 - PI / 4.0) * 9.0 * 100.0),
            ("filletNear", plate - (1.0 - PI / 4.0) * 9.0 * 100.0),
            ("chamfer", plate - 0.5 * 18.0),
            ("localCut", plate - 10.0 * 50.0 * 18.0),
        ];
        for (k, w) in want {
            let got = v[k].as_f64().unwrap_or_else(|| panic!("{k}: {}", v[k]));
            assert!(rel_err(got, w) <= 1e-9, "{k}: {got}, want {w}");
        }
        assert_box(
            &v["localCutBox"],
            [1010.0, 0.0, 0.0],
            [1100.0, 50.0, 18.0],
            "local cut",
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_feature_leaves_the_document_unchanged() {
        let (dir, registry) = registry();
        let v = assembly(&registry).await;
        let base = v["ids"]["base"].as_str().unwrap().to_string();
        let out = program(
            dir.path(),
            &registry,
            &format!(
                "const snap = () => JSON.stringify([cad.partDef(cad.part('{base}')), cad.parts(), cad.describe()]);\n\
                 const before = snap();\n\
                 const errors = [];\n\
                 for (const call of [\n\
                   () => cad.cut('{base}', {{ extrude: {{ min: [10, 10], max: [20, 20] }}, distance: 5, taper: 3 }}),\n\
                   () => cad.fillet3d('{base}', {{ edges: 'all', radius: 50 }}),\n\
                   () => cad.revolve({{ min: [0, 0], max: [10, 10] }}, {{ axis: [[5, 0], [5, 1]] }}),\n\
                 ]) {{\n\
                   try {{ call(); errors.push(null); }} catch (e) {{ errors.push(e.message); }}\n\
                 }}\n\
                 return {{ errors, same: snap() === before }};"
            ),
        )
        .await;
        let errors = out["errors"].as_array().unwrap();
        assert_eq!(
            errors[0],
            format!("cut: part {base} (\"base\"), feature 7 (extrude): a tapered extrude is not supported by this kernel yet").as_str(),
            "{out}"
        );
        // A round larger than the plate is thick: the kernel refuses it.
        let fillet = errors[1]
            .as_str()
            .unwrap_or_else(|| panic!("fillet accepted: {out}"));
        assert!(
            fillet.starts_with(&format!(
                "fillet3d: part {base} (\"base\"), feature 7 (fillet): "
            )),
            "{fillet}"
        );
        // A profile that crosses its own axis.
        let revolve = errors[2]
            .as_str()
            .unwrap_or_else(|| panic!("revolve accepted: {out}"));
        assert!(revolve.starts_with("revolve: definition d_"), "{revolve}");
        assert!(revolve.contains("feature 1 (revolve)"), "{revolve}");
        assert_eq!(out["same"], true, "{out}");
    }

    /// The PNG's width and height from its IHDR chunk.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[1..4], b"PNG");
        let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
        (be(16), be(20))
    }

    #[cfg(feature = "gpu")]
    #[tokio::test(flavor = "multi_thread")]
    async fn an_assembly_renders_in_3d_with_a_cutaway_and_exploded() {
        let (dir, registry) = registry();
        assembly(&registry).await;
        let variants = [
            ("iso", serde_json::json!({ "view": "iso", "ground": true })),
            (
                "cutaway",
                serde_json::json!({ "view": "iso", "clip": { "point": [200, 150, 0], "normal": [0, -1, 0] } }),
            ),
            (
                "exploded",
                serde_json::json!({ "view": "iso", "explode": 0.8, "colorBy": { "attr": "group" } }),
            ),
        ];
        for (name, opts) in variants {
            let out = dir.path().join(format!("{name}.png"));
            let mut args = opts.clone();
            args["out"] = serde_json::json!(out.to_string_lossy());
            args["size"] = serde_json::json!([480, 320]);
            let program_text = format!("return cad.render({args});");
            let result =
                run_script(&SANDBOX, &registry, "doc", &program_text, dir.path(), None).await;
            let r = match result {
                Ok(o) => value(&o),
                Err(e) if e.contains("GPU") || e.contains("adapter") => {
                    eprintln!("SKIPPED 3D render ({name}): {e}");
                    return;
                }
                Err(e) => panic!("{name}: {e}"),
            };
            assert_eq!(r["mode"], "3d", "{r}");
            assert_eq!(r["parts"], 12, "{r}");
            let png = std::fs::read(&out).unwrap();
            assert_eq!(png_size(&png), (480, 320), "{name}");
            assert_eq!(r["bytes"].as_u64(), Some(png.len() as u64));

            // The same scene drawn to pixels: how much of the image is not
            // background.
            let reg = registry.lock().await;
            let doc = &reg.get("doc").unwrap().document;
            let opts3 = cadview_render3d::RenderOptions3d {
                size: [480, 320],
                ..Default::default()
            };
            let scene =
                cadview_core::solid::scene::parts_scene(doc, &opts, opts3.pixel_size()).unwrap();
            let px = cadview_render3d::render_rgba(&scene.scene, &opts3).unwrap();
            let bg = cadview_core::scene3d::LIGHT_BACKGROUND;
            let ink = px
                .rgba
                .chunks(4)
                .filter(|p| (0..3).any(|i| p[i].abs_diff(bg[i]) > 8))
                .count();
            let share = ink as f64 / (480.0 * 320.0);
            eprintln!(
                "3D render {name}: {:.1} % of pixels drawn, {} triangles, chord {}",
                share * 100.0,
                r["triangles"],
                r["chord"]
            );
            assert!((0.15..0.85).contains(&share), "{name}: {share}");
        }
    }

    /// Every example loads `lib/drafting.js` through `cad.use` and runs.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_examples_run_with_their_library() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut names: Vec<PathBuf> = std::fs::read_dir(&examples)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "js")
                    && p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(|c: char| c.is_ascii_digit())
            })
            .collect();
        names.sort();
        assert_eq!(names.len(), 12, "{names:?}");
        for path in names {
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(!source.contains("new Function("), "{}", path.display());
            assert!(
                source.contains("await cad.use(\"drafting\")"),
                "{}",
                path.display()
            );
            let (_dir, registry) = registry();
            exec_file(&SANDBOX, &registry, "doc", path.to_str().unwrap(), None)
                .await
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(entity_count(&registry).await > 50, "{}", path.display());
        }
    }

    /// Can this machine draw a 3D render? (No GPU: the examples run their
    /// checks without rendering.)
    fn renders_3d() -> bool {
        #[cfg(feature = "gpu")]
        {
            let mut scene = cadview_core::scene3d::Scene3d::default();
            scene.add_mesh(
                &cadview_core::scene3d::shapes::cuboid(
                    cadview_core::solid::Point3::ORIGIN,
                    cadview_core::solid::Point3::new(1.0, 1.0, 1.0),
                ),
                cadview_core::solid::Placement::IDENTITY,
                cadview_core::scene3d::Material::default(),
            );
            let opts = cadview_render3d::RenderOptions3d {
                size: [8, 8],
                ..Default::default()
            };
            match cadview_render3d::render_rgba(&scene, &opts) {
                Ok(_) => true,
                Err(e) => {
                    eprintln!("SKIPPED 3D example renders: {e}");
                    false
                }
            }
        }
        #[cfg(not(feature = "gpu"))]
        false
    }

    /// Every script in examples/3d runs through the script entry (the one
    /// `/api/run` uses), passes the checks it makes on itself and returns
    /// its summary. Its renders go to a temporary folder, never over the
    /// committed pictures: the test sets `EXAMPLE_RENDER_DIR` (and
    /// `EXAMPLE_RENDER = false` without a GPU) before the script's text.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_3d_examples_run_and_check_themselves() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/3d");
        let mut scripts: Vec<PathBuf> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "js")
                    && p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(|c: char| c.is_ascii_digit())
            })
            .collect();
        scripts.sort();
        assert!(scripts.len() >= 9, "{scripts:?}");
        let draw = renders_3d();
        for path in scripts {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let (dir, registry) = registry();
            let out = dir.path().join("renders");
            let prelude = format!(
                "globalThis.EXAMPLE_RENDER_DIR = {}; globalThis.EXAMPLE_RENDER = {draw};\n",
                serde_json::to_string(&out.to_string_lossy()).unwrap()
            );
            let program = prelude + &std::fs::read_to_string(&path).unwrap();
            let started = Instant::now();
            let output = run_script(
                &SANDBOX,
                &registry,
                "doc",
                &program,
                &folder,
                Some(Duration::from_secs(600)),
            )
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let v = value(&output);
            eprintln!(
                "3D example {name}: {} parts, {} checks, {:.1} s ({})",
                v["parts"],
                v["checks"],
                started.elapsed().as_secs_f64(),
                v["ms"]
            );
            assert_eq!(v["example"], name.as_str(), "{v}");
            assert!(v["parts"].as_u64().unwrap() >= 1, "{v}");
            assert!(v["checks"].as_u64().unwrap() >= 3, "{v}");
            assert!(v["volume"].as_f64().unwrap() > 0.0, "{v}");
            let files = v["files"].as_array().unwrap();
            if draw {
                assert!(!files.is_empty(), "{v}");
                for f in files {
                    let f = Path::new(f.as_str().unwrap());
                    assert!(
                        f.starts_with(&out),
                        "{name}: {} outside the temporary folder",
                        f.display()
                    );
                    let png = std::fs::read(f).unwrap();
                    assert_eq!(png_size(&png), (960, 640), "{}", f.display());
                }
                assert!(
                    out.join(format!("{name}.png")).exists(),
                    "{name}: no isometric render"
                );
            } else {
                assert!(files.is_empty(), "{v}");
            }
        }
    }
}
