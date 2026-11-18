//! The solid kernel module, seen from the page's main module.
//!
//! The main module links no kernel. The kernel is a second WebAssembly
//! module (crate `cadview-kernel-wasm`) that JavaScript fetches on demand
//! and exposes as `globalThis.__codecadKernel.call(bytes) -> bytes`. Here a
//! [`RemoteEvaluator`] (cadview-core `solid::remote`) speaks to it through
//! that one function, and every session's document gets it as its solid
//! environment, so scripts, queries and the 3D view evaluate parts on the
//! page with the same core code the server runs. The page's environment is
//! shared by all sessions (one evaluation cache, as on the server).
//!
//! Before the module is loaded, a page that can load it puts a
//! [`NotLoaded`] environment on its documents (`kernel_expect`): a call that
//! needs geometry then fails with a message naming the call to await,
//! rather than storing parts no kernel checked.

use crate::session::SESSIONS;
use cadview_core::solid::remote::{
    NotLoaded, RemoteEvaluator, Transport, NOT_LOADED, WIRE_VERSION,
};
use cadview_core::solid::SolidEnv;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    /// The kernel module's one call (set up by the page's kernel loader).
    #[wasm_bindgen(catch, js_namespace = __codecadKernel, js_name = call)]
    fn js_kernel_call(request: &[u8]) -> Result<Vec<u8>, JsValue>;
}

/// The kernel module, through JavaScript. Synchronous: the module runs on
/// the page's thread.
struct JsTransport;

impl Transport for JsTransport {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        js_kernel_call(request).map_err(|e| {
            e.as_string()
                .or_else(|| {
                    js_sys::Reflect::get(&e, &"message".into())
                        .ok()
                        .and_then(|m| m.as_string())
                })
                .unwrap_or_else(|| format!("{e:?}"))
        })
    }
}

/// The environment the page gives its documents: none, not loaded yet, or
/// the kernel.
static PAGE_ENV: Mutex<Option<SolidEnv>> = Mutex::new(None);

/// The page's solid environment, for new sessions.
pub(crate) fn page_env() -> Option<SolidEnv> {
    PAGE_ENV.lock().unwrap().clone()
}

/// Does this environment hold a real kernel (not a placeholder)?
pub(crate) fn is_kernel(env: &SolidEnv) -> bool {
    env.kernel_name().is_some_and(|n| n != NOT_LOADED)
}

/// Put `env` on every session (and on new ones), waking their views.
fn install(env: SolidEnv, replace_kernel: bool) {
    *PAGE_ENV.lock().unwrap() = Some(env.clone());
    let mut reg = SESSIONS.lock().unwrap();
    let mut wake = Vec::new();
    for (id, s) in reg.sessions.iter_mut() {
        if replace_kernel || !is_kernel(&s.doc.solids) {
            s.doc.solids = env.clone();
            s.doc_rev += 1;
            s.render_dirty = true;
            wake.push(id.clone());
        }
    }
    for (key, sid) in &reg.renderers {
        if wake.contains(sid) {
            if let Some(f) = reg.repaint_fns.get(key) {
                f.call();
            }
        }
    }
}

/// The wire version the main module speaks (the loader checks the kernel
/// module's manifest against it before fetching the module).
#[wasm_bindgen]
pub fn kernel_wire_version() -> u32 {
    WIRE_VERSION
}

/// This page can load a kernel module but has not: calls that need
/// geometry fail with `hint` until `kernel_connect`.
#[wasm_bindgen]
pub fn kernel_expect(hint: &str) {
    if page_env().is_some_and(|e| is_kernel(&e)) {
        return;
    }
    install(
        SolidEnv::new(Arc::new(NotLoaded {
            hint: hint.to_string(),
        })),
        false,
    );
}

/// Connect to the loaded kernel module (`__codecadKernel.call`), check its
/// wire version and give every session its geometry. Returns `{ok,
/// kernel}` or `{error}`.
#[wasm_bindgen]
pub fn kernel_connect() -> String {
    match RemoteEvaluator::connect(Box::new(JsTransport)) {
        Ok(remote) => {
            let env = SolidEnv::new(Arc::new(remote));
            let name = env.kernel_name().unwrap_or_default();
            install(env, true);
            serde_json::json!({ "ok": true, "kernel": name }).to_string()
        }
        Err(e) => serde_json::json!({ "error": e }).to_string(),
    }
}

/// `{state: "none" | "not-loaded" | "loaded", kernel, evaluations}`.
#[wasm_bindgen]
pub fn kernel_status() -> String {
    let env = page_env();
    let state = match &env {
        None => "none",
        Some(e) if is_kernel(e) => "loaded",
        Some(_) => "not-loaded",
    };
    let (kernel, evaluations) = match env.as_ref().and_then(|e| e.get().ok()) {
        Some(s) => (s.kernel_name(), s.evaluations()),
        None => (String::new(), 0),
    };
    serde_json::json!({ "state": state, "kernel": kernel, "evaluations": evaluations }).to_string()
}
