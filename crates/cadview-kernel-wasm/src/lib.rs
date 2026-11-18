//! The solid kernel as a separately loaded WebAssembly module
//! (docs/3d-design.md, "3D in the browser").
//!
//! The page's main module (cadview-web) links no kernel. When a page needs
//! geometry it fetches this module, instantiates it next to the main one and
//! talks to it through cadview-core's byte interface
//! (`solid::remote`): this module runs a [`KernelServer`] over the arris
//! adapter; the page runs a `RemoteEvaluator`. Both are the same core code.
//!
//! The exports are plain functions on the module's own memory, no
//! wasm-bindgen (the few imports the dependency tree brings are never
//! called; the loader stubs them):
//!
//! - `ck_alloc(len) -> ptr` and `ck_free(ptr, len)`: a buffer for a request;
//! - `ck_call(ptr, len) -> n`: answer the request in that buffer; the reply
//!   is `n` bytes at `ck_reply()`, valid until the next call;
//! - `ck_wire() -> u32`: the wire version (`solid::remote::WIRE_VERSION`),
//!   readable before the first call.
//!
//! One server per instance; the module keeps the solids the page holds
//! handles to, and frees them when the page releases them.

use std::cell::RefCell;
use std::sync::Arc;

use cadview_core::solid::remote::{KernelServer, WIRE_VERSION};
use cadview_core::solid::KernelEvaluator;
use cadview_solid::ArrisKernel;

thread_local! {
    static SERVER: RefCell<KernelServer> =
        RefCell::new(KernelServer::new(Arc::new(KernelEvaluator::new(ArrisKernel))));
    static REPLY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Answer one request (the same function the exports call).
pub fn call(request: &[u8]) -> Vec<u8> {
    SERVER.with(|s| s.borrow_mut().handle(request))
}

/// A buffer of `len` bytes for the page to write a request into.
#[no_mangle]
pub extern "C" fn ck_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Free a buffer from [`ck_alloc`].
///
/// # Safety
/// `ptr` and `len` must come from one `ck_alloc` call, freed once.
#[no_mangle]
pub unsafe extern "C" fn ck_free(ptr: *mut u8, len: usize) {
    drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
}

/// Answer the request of `len` bytes at `ptr`; returns the reply's length.
///
/// # Safety
/// `ptr` must point to `len` initialised bytes from [`ck_alloc`].
#[no_mangle]
pub unsafe extern "C" fn ck_call(ptr: *const u8, len: usize) -> usize {
    let request = std::slice::from_raw_parts(ptr, len);
    let reply = call(request);
    REPLY.with(|r| {
        let mut r = r.borrow_mut();
        *r = reply;
        r.len()
    })
}

/// Where the last reply is.
#[no_mangle]
pub extern "C" fn ck_reply() -> *const u8 {
    REPLY.with(|r| r.borrow().as_ptr())
}

/// The wire version this module speaks.
#[no_mangle]
pub extern "C" fn ck_wire() -> u32 {
    WIRE_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadview_core::solid::remote::{RemoteEvaluator, Transport};
    use cadview_core::solid::SolidEnv;
    use cadview_core::Document;

    /// The exports, called as the page calls them (through the module's
    /// memory functions), natively.
    struct Exports;

    impl Transport for Exports {
        fn call(&self, request: &[u8]) -> Result<Vec<u8>, String> {
            unsafe {
                let p = ck_alloc(request.len());
                std::ptr::copy_nonoverlapping(request.as_ptr(), p, request.len());
                let n = ck_call(p, request.len());
                let out = std::slice::from_raw_parts(ck_reply(), n).to_vec();
                ck_free(p, request.len());
                Ok(out)
            }
        }
    }

    #[test]
    fn the_module_answers_like_the_kernel_it_wraps() {
        let script = [
            (
                "extrude",
                r#"{"profile": {"min": [0, 0], "max": [100, 50]}, "distance": 18}"#,
            ),
            (
                "hole",
                r#"{"target": "p_2", "at": [50, 25, 18], "dia": 10, "depth": "through"}"#,
            ),
            ("copy3d", r#"{"target": "p_2", "move": [0, 0, 40]}"#),
            (
                "rotate3d",
                r#"{"target": "p_3", "angle": 30, "about": [50, 25, 40]}"#,
            ),
        ];
        let run = |doc: &mut Document| -> Vec<String> {
            let mut out: Vec<String> = script
                .iter()
                .map(|(m, a)| cadview_core::cad_call(doc, m, a).unwrap())
                .collect();
            out.push(cadview_core::cad_call(doc, "parts", "{}").unwrap());
            out
        };
        let mut native = Document::new();
        native.solids = SolidEnv::new(Arc::new(KernelEvaluator::new(ArrisKernel)));
        let mut module = Document::new();
        module.solids = SolidEnv::new(Arc::new(
            RemoteEvaluator::connect(Box::new(Exports)).unwrap(),
        ));
        assert_eq!(run(&mut module), run(&mut native));
        assert_eq!(ck_wire(), WIRE_VERSION);
    }
}
