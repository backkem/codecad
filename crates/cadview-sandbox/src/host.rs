use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};
use wasmtime_wasi_http::WasiHttpCtx;

pub type CallHandler = Box<dyn FnMut(&str, &str) -> Result<String, String> + Send>;
pub type ReadHandler = Box<dyn FnMut(&str) -> Result<String, String> + Send>;

pub(crate) struct Host {
    pub wasi_ctx: WasiCtx,
    pub table: ResourceTable,
    pub http_ctx: WasiHttpCtx,
    /// No-op HTTP hooks (scripts don't use outbound HTTP).
    pub no_hooks: [(); 0],
    /// Handler for cad-call WIT import: (method, args_json) -> result_json
    pub cad_handler: CallHandler,
    /// Handler for rpc-call WIT import: (method, args_json) -> result_json
    pub rpc_handler: CallHandler,
    /// Handler for read-file WIT import: (path) -> contents
    pub read_file_handler: ReadHandler,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for Host {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http_ctx,
            table: &mut self.table,
            hooks: &mut self.no_hooks,
        }
    }
}

// Implement the cadview:sandbox/cad WIT interface.
// wasmtime bindgen maps `result<string, string>` directly to `Result<String, String>`.
impl crate::cadview::sandbox::cad::Host for Host {
    fn cad_call(&mut self, method: String, args: String) -> Result<String, String> {
        (self.cad_handler)(&method, &args)
    }

    fn rpc_call(&mut self, method: String, args: String) -> Result<String, String> {
        (self.rpc_handler)(&method, &args)
    }

    fn read_file(&mut self, path: String) -> Result<String, String> {
        (self.read_file_handler)(&path)
    }
}
