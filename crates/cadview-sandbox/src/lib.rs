mod error;
mod host;

pub use error::SandboxError;
pub use host::{CallHandler, ReadHandler};

use std::path::PathBuf;
use std::time::Duration;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi_http::WasiHttpCtx;

use host::Host;

// Generate typed bindings from the WIT world.
// This creates:
//   - CadviewRuntime struct (from world name) with call_run() and add_to_linker()
//   - cadview::sandbox::cad::Host trait for the custom cad interface
wasmtime::component::bindgen!(in "../../wit");

/// Output from a sandbox run.
#[derive(Debug, Clone)]
pub struct RunOutput {
    /// Return value from the JS program (Ok variant of the WIT result).
    pub value: Option<String>,
    /// Captured stdout (console.log output).
    pub stdout: String,
    /// Captured stderr (console.error output).
    pub stderr: String,
}

/// Reusable sandbox environment. Create once, call run() many times.
pub struct Sandbox {
    engine: Engine,
    component: Component,
    linker: Linker<Host>,
}

impl Sandbox {
    /// Create a sandbox from component bytes.
    /// Pre-compiled native code is cached to disk for fast subsequent startups.
    pub fn new(component_bytes: &[u8]) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine =
            Engine::new(&config).map_err(|e| SandboxError::ComponentLoad(e.into()))?;

        let component = load_or_compile(&engine, component_bytes)?;

        let mut linker = Linker::<Host>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| SandboxError::ComponentLoad(e.into()))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_sync(&mut linker)
            .map_err(|e| SandboxError::ComponentLoad(e.into()))?;
        // Link the custom cadview:sandbox/cad interface.
        cadview::sandbox::cad::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
            &mut linker,
            |host| host,
        )
        .map_err(|e| SandboxError::ComponentLoad(e.into()))?;

        // Background thread increments epoch every 100ms for timeout support.
        let engine_clone = engine.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(100));
            engine_clone.increment_epoch();
        });

        Ok(Self {
            engine,
            component,
            linker,
        })
    }

    /// Run a JS program with custom handlers for cad operations and file I/O.
    pub fn run(
        &self,
        program: &str,
        timeout: Duration,
        cad_handler: crate::host::CallHandler,
        rpc_handler: crate::host::CallHandler,
        read_file_handler: crate::host::ReadHandler,
    ) -> Result<RunOutput, SandboxError> {
        let stdout_pipe = MemoryOutputPipe::new(64 * 1024 * 1024);
        let stderr_pipe = MemoryOutputPipe::new(64 * 1024 * 1024);

        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.stdout(stdout_pipe.clone());
        wasi_builder.stderr(stderr_pipe.clone());

        let host = Host {
            wasi_ctx: wasi_builder.build(),
            table: ResourceTable::new(),
            http_ctx: WasiHttpCtx::new(),
            no_hooks: [],
            cad_handler,
            rpc_handler,
            read_file_handler,
        };

        let mut store = Store::new(&self.engine, host);

        // Epoch deadline: each tick is 100ms.
        let ticks = (timeout.as_millis() / 100).max(1) as u64;
        store.set_epoch_deadline(ticks);

        let bindings =
            CadviewRuntime::instantiate(&mut store, &self.component, &self.linker)
                .map_err(|e| SandboxError::Instantiation(e.into()))?;

        let result = bindings.call_run(&mut store, program);

        let stdout = String::from_utf8_lossy(&stdout_pipe.contents()).to_string();
        let stderr = String::from_utf8_lossy(&stderr_pipe.contents()).to_string();

        match result {
            Ok(Ok(value)) => Ok(RunOutput {
                value: if value.is_empty() { None } else { Some(value) },
                stdout,
                stderr,
            }),
            Ok(Err(error_msg)) => Err(SandboxError::ProgramError(error_msg)),
            Err(trap) => {
                let msg = trap.to_string();

                // StarlingMonkey's post_run cleanup can trap if HTTP response
                // streams were not fully consumed.
                if msg.contains("post_run") && (!stdout.is_empty() || !stderr.is_empty()) {
                    let mut combined_stderr = stderr;
                    combined_stderr
                        .push_str(&format!("\n[sandbox] post_run cleanup error: {msg}\n"));
                    return Ok(RunOutput {
                        value: None,
                        stdout,
                        stderr: combined_stderr,
                    });
                }

                if msg.contains("epoch") || msg.contains("interrupt") {
                    Err(SandboxError::Timeout(timeout))
                } else {
                    Err(SandboxError::Execution(trap.into()))
                }
            }
        }
    }
}

/// Cache directory for pre-compiled components.
fn cache_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cadview").join("cache"))
}

/// Cache key: hash of component bytes. Changes when the component is rebuilt.
fn cache_key(component_bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    component_bytes.len().hash(&mut hasher);
    // Hash first/last 4KB + length for speed (12MB component).
    let head = &component_bytes[..component_bytes.len().min(4096)];
    let tail_start = component_bytes.len().saturating_sub(4096);
    let tail = &component_bytes[tail_start..];
    head.hash(&mut hasher);
    tail.hash(&mut hasher);
    format!("sandbox-{:016x}.cwasm", hasher.finish())
}

/// Load a pre-compiled component from cache, or compile and cache it.
fn load_or_compile(engine: &Engine, component_bytes: &[u8]) -> Result<Component, SandboxError> {
    let cache_path = cache_dir().map(|d| d.join(cache_key(component_bytes)));

    // Try loading from cache.
    if let Some(ref path) = cache_path {
        if path.exists() {
            // SAFETY: we trust our own cache files. The serialized module is tied to the
            // engine configuration (wasmtime version, flags). A mismatched cache file
            // will fail to deserialize rather than cause UB.
            match unsafe { Component::deserialize_file(engine, path) } {
                Ok(component) => return Ok(component),
                Err(_) => {
                    // Stale or incompatible cache, remove and recompile.
                    let _ = std::fs::remove_file(path);
                }
            }
        }
    }

    // Compile from source.
    let component = Component::from_binary(engine, component_bytes)
        .map_err(|e| SandboxError::ComponentLoad(e.into()))?;

    // Write to cache.
    if let Some(ref path) = cache_path {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = engine.precompile_component(component_bytes) {
            let _ = std::fs::write(path, bytes);
        }
    }

    Ok(component)
}
