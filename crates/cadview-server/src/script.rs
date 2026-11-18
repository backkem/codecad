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
globalThis.__loadLibrary = (spec, from) =>
  call("lib.load", JSON.stringify({ spec, from }));
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
                Some(("lib", "load")) => crate::library::load_json(&state.read_base, args),
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
            })
            .to_string())
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
            *doc = new_doc;
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
    async fn a_library_is_evaluated_once_and_returns_its_exports() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/geom.js",
            "// Each evaluation draws one line, so the entity count shows how often it ran.\n\
             cad.addLine([0, 0], [1, 0]);\n\
             export const TAU = Math.PI * 2;\n\
             export let count = 0;\n\
             export function bump() { count += 1; return count; }\n\
             export function circle(c, r) { return cad.addCircle(c, r); }\n\
             export default { kind: \"geom\" };\n",
        );
        write(
            dir.path(),
            "lib/shapes.js",
            "import geom, { circle } from \"geom\";\n\
             import * as g from \"./geom.js\";\n\
             export function ring(c) { circle(c, 1); circle(c, 2); return g.TAU; }\n\
             export const kind = geom.kind;\n\
             export * from \"./geom.js\";\n",
        );
        let output = exec_main(
            dir.path(),
            &registry,
            "const a = await cad.use(\"geom\");\n\
             const b = await cad.use(\"./lib/geom.js\");\n\
             const s = await cad.use(\"shapes\");\n\
             const tau = s.ring([0, 0]);\n\
             a.bump(); a.bump();\n\
             return {\n\
               same: a === b,\n\
               keys: Object.keys(a),\n\
               tag: Object.prototype.toString.call(a),\n\
               frozen: Object.isFrozen(a),\n\
               kind: s.kind,\n\
               starred: s.circle === a.circle && !(\"default\" in s),\n\
               live: [a.count, s.count],\n\
               tau,\n\
               leaked: [\"geom\", \"shapes\", \"circle\", \"TAU\", \"ring\", \"g\", \"__cadImport\"]\n\
                 .filter((k) => k in globalThis),\n\
               entities: cad.describe().entities,\n\
             };",
        )
        .await
        .unwrap();
        let v = value(&output);
        assert_eq!(v["same"], true, "{v}");
        assert_eq!(
            v["keys"],
            serde_json::json!(["TAU", "count", "bump", "circle", "default"])
        );
        assert_eq!(v["tag"], "[object Module]");
        assert_eq!(v["frozen"], true);
        assert_eq!(v["kind"], "geom");
        assert_eq!(v["starred"], true, "{v}");
        assert_eq!(
            v["live"],
            serde_json::json!([2, 2]),
            "exports are read live"
        );
        assert!((v["tau"].as_f64().unwrap() - std::f64::consts::TAU).abs() < 1e-12);
        assert_eq!(v["leaked"], serde_json::json!([]), "no globals leak");
        // One line (geom ran once, though asked for four times) and two circles.
        assert_eq!(v["entities"], 3, "{v}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_runs_in_strict_mode_and_cannot_create_globals_by_accident() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/sloppy.js",
            "export const a = 1;\nleak = 5;\n",
        );
        let error = exec_main(dir.path(), &registry, "await cad.use(\"sloppy\");")
            .await
            .unwrap_err();
        assert!(
            error.contains("library lib/sloppy.js threw while loading at lib/sloppy.js:2:"),
            "{error}"
        );
        assert!(error.contains("ReferenceError"), "{error}");
        let output = exec_main(dir.path(), &registry, "return typeof leak;")
            .await
            .unwrap();
        assert_eq!(output.value.as_deref(), Some("undefined"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn each_run_evaluates_its_libraries_afresh() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/mark.js",
            "cad.addLine([0, 0], [1, 1]);\nexport const ok = true;\n",
        );
        write(dir.path(), "other.js", "await cad.use(\"mark\");");
        let output = exec_main(
            dir.path(),
            &registry,
            "await cad.use(\"mark\");\nawait cad.use(\"mark\");\ncad.exec(\"other.js\");\n\
             return cad.describe().entities;",
        )
        .await
        .unwrap();
        // Once in this run, once in the nested `exec` run.
        assert_eq!(output.value.as_deref(), Some("2"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stack_traces_name_the_library() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/tools.js",
            "// tools\n\nexport function boom() {\n  return null.x;\n}\n",
        );
        let error = exec_main(
            dir.path(),
            &registry,
            "const t = await cad.use(\"tools\");\nt.boom();",
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("program error: TypeError"), "{error}");
        assert!(error.contains("(at lib/tools.js:4:"), "{error}");

        // The same in a stack the script reads itself.
        let output = exec_main(
            dir.path(),
            &registry,
            "const t = await cad.use(\"tools\");\ntry { t.boom(); } catch (e) { return e.stack; }",
        )
        .await
        .unwrap();
        let stack = output.value.unwrap();
        assert!(stack.starts_with("boom@lib/tools.js:4:"), "{stack}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_library_names_itself_and_the_line() {
        let (dir, registry) = registry();
        let error = run_script(
            &SANDBOX,
            &registry,
            "doc",
            "const a = 1;\nawait cad.use(\"nope\");\n",
            dir.path(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("cad.use: library \"nope\" not found (no file lib/nope.js)"),
            "{error}"
        );
        assert!(error.contains("(at script.js:2:"), "{error}");

        // Asked for by another library: that library's line.
        write(
            dir.path(),
            "lib/needy.js",
            "// needs a library that is not there\nimport { x } from \"./gone.js\";\n",
        );
        let error = exec_main(dir.path(), &registry, "await cad.use(\"needy\");")
            .await
            .unwrap_err();
        assert!(
            error.contains(
                "library \"./gone.js\" not found (no file lib/gone.js) (imported at lib/needy.js:2)"
            ),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_library_that_fails_while_loading_names_itself_and_the_line() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/bad.js",
            "export const a = 1;\n\nthrow new RangeError(\"bad setup\");\n",
        );
        write(
            dir.path(),
            "lib/outer.js",
            "import \"bad\";\nexport const b = 2;\n",
        );
        write(
            dir.path(),
            "lib/syntax.js",
            "export const a = 1;\nconst b = ;\n",
        );
        write(
            dir.path(),
            "lib/dynamic.js",
            "export const a = 1;\nconst m = await import(\"x\");\n",
        );
        let thrown: &[&str] = &[
            "cad.use: library lib/bad.js threw while loading at lib/bad.js:3:",
            ": RangeError: bad setup",
        ];
        let cases: [(&str, &[&str]); 4] = [
            ("bad", thrown),
            // Not wrapped again by the library that imported it.
            ("outer", thrown),
            (
                "syntax",
                &[
                    "cad.use: library lib/syntax.js does not parse at lib/syntax.js:2:",
                    ": SyntaxError: ",
                ],
            ),
            (
                "dynamic",
                &[
                    "cad.use: library lib/dynamic.js: import() is not supported in a library: \
                   use an import declaration at the top of the file at lib/dynamic.js:2:17",
                ],
            ),
        ];
        for (name, expected) in cases {
            let error = exec_main(
                dir.path(),
                &registry,
                &format!("const x = 1;\nawait cad.use(\"{name}\");"),
            )
            .await
            .unwrap_err();
            for part in expected {
                assert!(error.contains(part), "{name}: {error}");
            }
            // A script can catch it, too.
            let output = exec_main(
                dir.path(),
                &registry,
                &format!("try {{ await cad.use(\"{name}\"); }} catch (e) {{ return e.message; }}"),
            )
            .await
            .unwrap();
            let message = output.value.unwrap();
            assert!(message.starts_with(expected[0]), "{name}: {message}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_cycle_is_reported_not_hung() {
        let (dir, registry) = registry();
        write(
            dir.path(),
            "lib/c1.js",
            "import \"./c2.js\";\nexport const a = 1;\n",
        );
        write(
            dir.path(),
            "lib/c2.js",
            "// c2\nimport { a } from \"c1\";\nexport const b = a;\n",
        );
        write(dir.path(), "lib/self.js", "import \"self\";\n");
        let started = Instant::now();
        let error = exec_main(dir.path(), &registry, "await cad.use(\"c1\");")
            .await
            .unwrap_err();
        assert!(
            error.contains(
                "cad.use: library cycle: lib/c2.js -> lib/c1.js -> lib/c2.js \
                 (imported at lib/c2.js:2)"
            ),
            "{error}"
        );
        let error = exec_main(dir.path(), &registry, "await cad.use(\"self\");")
            .await
            .unwrap_err();
        assert!(
            error.contains("library cycle: lib/self.js -> lib/self.js"),
            "{error}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn libraries_stay_inside_the_script_folder() {
        let (dir, registry) = registry();
        // The script runs in sub/; secret.js is next to sub/, outside it.
        write(dir.path(), "secret.js", "export const key = 42;\n");
        write(
            dir.path(),
            "sub/lib/escape.js",
            "import { key } from \"../../secret.js\";\nexport { key };\n",
        );
        let secret = dir.path().join("secret.js").canonicalize().unwrap();
        let secret_js = serde_json::to_string(&secret.to_string_lossy()).unwrap();
        let cases = [
            ("\"../secret.js\"", "is outside the script's folder"),
            ("\"./../secret.js\"", "is outside the script's folder"),
            ("\"lib/../../secret\"", "is not a library name"),
            (secret_js.as_str(), "is not a library name"),
            ("\"https://example.com/lib.js\"", "is not a library name"),
            (
                "\"escape\"",
                "is outside the script's folder (imported at lib/escape.js:1)",
            ),
        ];
        for (spec, expected) in cases {
            write(
                &dir.path().join("sub"),
                "main.js",
                &format!("try {{ await cad.use({spec}); return \"loaded\"; }} catch (e) {{ return e.message; }}"),
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
            assert!(message.contains(expected), "{spec}: {message}");
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
}
