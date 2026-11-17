//! cadview-server: Rust server for runtime DWG loading + Yrs sync.
//!
//! Serves the cadview WASM app over HTTP and syncs document state
//! with the browser via WebTransport + Yrs CRDT.
//!
//! Multi-document: uses a DocumentRegistry backed by a DocumentStore
//! (single file or folder). Documents are lazy-loaded on first access.

pub mod assets;
mod http;
pub mod registry;
mod render;
pub mod script;
pub mod store;
mod sync;
mod transport;

use registry::DocumentRegistry;
use store::{FolderStore, SingleFileStore};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

const USAGE: &str = "\
codecad - AI-native 2D CAD server

Usage: codecad [OPTIONS] [PATH]

PATH is a .dwg file or a directory of .dwg files. With no PATH, the
current directory is scanned. Serves http://localhost:8765.

Options:
  --exec <SCRIPT>     Run a JS script headless and exit (no server)
  --render <PNG>      Render the drawing to a PNG and exit (no server).
                      Works on a .dwg PATH alone, or after --exec
  --size <WxH>        Render size in pixels (default 960x640)
  --render-backend <auto|gpu|cpu>
                      Render backend (default auto: GPU, else CPU)
  --dist <DIR>        Serve frontend assets from disk instead of embedded
  --examples <DIR>    Read example drawings from disk instead of embedded
  -V, --version       Print version
  -h, --help          Print this help

Environment:
  CADVIEW_TOKEN       Bearer token for the HTTP API and WebTransport
                      (default: cadview-local-dev)
  RUST_LOG            Log filter, e.g. RUST_LOG=info
";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Parse CLI flags
    let mut dist_override: Option<PathBuf> = None;
    let mut examples_override: Option<PathBuf> = None;
    let mut exec_script: Option<String> = None;
    let mut render_out: Option<String> = None;
    let mut render_size: Option<[u32; 2]> = None;
    let mut render_backend: Option<cadview_render::Backend> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut args_iter = std::env::args().skip(1);
    while let Some(arg) = args_iter.next() {
        match arg.as_str() {
            "--dist" => {
                dist_override = Some(PathBuf::from(
                    args_iter.next().expect("--dist requires a path argument"),
                ));
            }
            "--examples" => {
                examples_override = Some(PathBuf::from(
                    args_iter
                        .next()
                        .expect("--examples requires a path argument"),
                ));
            }
            "--exec" => {
                exec_script = Some(args_iter.next().expect("--exec requires a script path"));
            }
            "--render" => {
                render_out = Some(args_iter.next().expect("--render requires a PNG path"));
            }
            "--size" => {
                let s = args_iter.next().expect("--size requires WxH, e.g. 960x640");
                render_size = Some(parse_size(&s)?);
            }
            "--render-backend" => {
                let s = args_iter
                    .next()
                    .expect("--render-backend requires auto, gpu or cpu");
                render_backend = Some(s.parse().map_err(|e: String| anyhow::anyhow!(e))?);
            }
            "--version" | "-V" => {
                println!("codecad {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            _ => positional.push(arg),
        }
    }

    if render_out.is_none() && (render_size.is_some() || render_backend.is_some()) {
        anyhow::bail!("--size and --render-backend need --render <PNG>");
    }
    let render = render_out.map(|out| {
        let size = render_size.unwrap_or([960, 640]);
        let backend = render_backend.unwrap_or_default();
        serde_json::json!({ "out": out, "size": size, "backend": backend }).to_string()
    });

    // Headless mode: run a script and/or render, then exit (no HTTP/WebTransport server).
    if exec_script.is_some() || render.is_some() {
        return run_headless(&positional, exec_script.as_deref(), render.as_deref()).await;
    }

    // Resolve asset providers
    let dist: Box<dyn assets::AssetProvider> = match dist_override {
        Some(path) => {
            tracing::info!("Dist from disk: {}", path.display());
            Box::new(assets::DiskAssets::new(path))
        }
        None => assets::default_dist()?,
    };

    let _examples: Option<Box<dyn assets::AssetProvider>> = match examples_override {
        Some(path) => {
            tracing::info!("Examples from disk: {}", path.display());
            Some(Box::new(assets::DiskAssets::new(path)))
        }
        None => assets::default_examples(),
    };

    let token = std::env::var("CADVIEW_TOKEN").unwrap_or_else(|_| "cadview-local-dev".to_string());

    let registry = if positional.is_empty() {
        // No args: FolderStore on cwd
        tracing::info!("No path specified, scanning cwd for .dwg files");
        DocumentRegistry::new(Box::new(FolderStore::new(std::env::current_dir()?)))
    } else {
        let path = PathBuf::from(&positional[0]);
        if path.is_dir() {
            tracing::info!("Folder mode: scanning {} for .dwg files", path.display());
            DocumentRegistry::new(Box::new(FolderStore::new(path)))
        } else if path.is_file() {
            tracing::info!("Single file mode: {}", path.display());
            let abs = std::fs::canonicalize(&path)?;
            let abs_str = abs.to_string_lossy().to_string();
            let store = SingleFileStore::new(abs.clone());
            let key = store.key().to_string();
            let mut reg = DocumentRegistry::new(Box::new(store));
            // Pre-load the single file
            let doc = cadview_core::load_dwg(&positional[0])?;
            let entity_count = doc.entities.len();
            reg.insert(key.clone(), doc, Some(abs_str));
            tracing::info!("Loaded {entity_count} entities from {key}");
            reg
        } else {
            anyhow::bail!(
                "Path '{}' is neither a file nor a directory",
                path.display()
            );
        }
    };

    let available = registry.list_available();
    tracing::info!("{} document(s) available", available.len());

    let registry = Arc::new(Mutex::new(registry));

    // Initialize the sandbox for server-side script execution.
    tracing::info!("Initializing script sandbox (the first start compiles the JS engine)");
    let sandbox = script::create_sandbox()
        .await
        .expect("Failed to initialize script sandbox");
    tracing::info!("Script sandbox initialized");

    // TLS for WebTransport
    let identity = wtransport::Identity::self_signed(["localhost", "127.0.0.1", "::1"])?;
    let cert_hash = identity.certificate_chain().as_slice()[0]
        .hash()
        .as_ref()
        .to_vec();

    let wt_config = wtransport::ServerConfig::builder()
        .with_bind_default(0)
        .with_identity(identity)
        .build();
    let wt_endpoint = wtransport::Endpoint::server(wt_config)?;
    let wt_port = wt_endpoint.local_addr()?.port();
    tracing::info!("WebTransport listening on port {wt_port}");

    // HTTP server
    let http_port = 8765;
    let http_state = Arc::new(http::HttpState {
        dist,
        cert_hash,
        wt_port,
        registry: registry.clone(),
        token: token.clone(),
        sandbox: sandbox.clone(),
    });
    let http_router = http::router(http_state);
    let http_addr = SocketAddr::from(([127, 0, 0, 1], http_port));
    let listener = match tokio::net::TcpListener::bind(http_addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind port {http_port}: {e}");
            tracing::error!("Is another instance already running?");
            std::process::exit(1);
        }
    };
    tracing::info!("HTTP server: http://localhost:{http_port}");
    tracing::info!("Ready. Open http://localhost:{http_port} in browser.");

    tokio::select! {
        r = axum::serve(listener, http_router) => {
            tracing::error!("HTTP server exited: {r:?}");
        }
        r = accept_wt_sessions(wt_endpoint, registry, sandbox) => {
            tracing::error!("WebTransport server exited: {r:?}");
        }
    }

    Ok(())
}

/// Parse `WxH` (e.g. `960x640`).
fn parse_size(s: &str) -> anyhow::Result<[u32; 2]> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| anyhow::anyhow!("--size expects WxH, e.g. 960x640, got '{s}'"))?;
    let w: u32 = w.trim().parse()?;
    let h: u32 = h.trim().parse()?;
    if w == 0 || h == 0 {
        anyhow::bail!("--size must be positive, got '{s}'");
    }
    Ok([w, h])
}

/// Headless mode: load an optional DWG, run an optional script, render an
/// optional PNG (args are `cad.render` JSON), print output, exit.
async fn run_headless(
    positional: &[String],
    script_path: Option<&str>,
    render: Option<&str>,
) -> anyhow::Result<()> {
    let Some(script_path) = script_path else {
        // Render only: no sandbox needed.
        let path = positional
            .first()
            .ok_or_else(|| anyhow::anyhow!("--render needs a .dwg PATH or --exec <SCRIPT>"))?;
        let doc = cadview_core::load_dwg(path)?;
        tracing::info!("Loaded {} entities from {path}", doc.entities.len());
        let cwd = std::env::current_dir()?;
        let result = render::render_to_file(&doc, render.unwrap_or("{}"), &cwd)
            .map_err(|e| anyhow::anyhow!(e))?;
        println!("{result}");
        return Ok(());
    };

    let sandbox = script::create_sandbox()
        .await
        .expect("Failed to initialize script sandbox");

    // Create registry with optional input DWG
    let mut registry =
        DocumentRegistry::new(Box::new(store::FolderStore::new(std::env::current_dir()?)));
    let doc_key = if let Some(path_str) = positional.first() {
        let path = PathBuf::from(path_str);
        if path.is_file() {
            let doc = cadview_core::load_dwg(path_str)?;
            let entity_count = doc.entities.len();
            let key = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            registry.insert(key.clone(), doc, Some(path_str.clone()));
            tracing::info!("Loaded {entity_count} entities from {key}");
            key
        } else {
            "default".to_string()
        }
    } else {
        // Create empty document
        let key = "default".to_string();
        registry.insert(key.clone(), cadview_core::Document::default(), None);
        key
    };

    let registry = Arc::new(Mutex::new(registry));

    let output = script::exec_file(&sandbox, &registry, &doc_key, script_path, None)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    if let Some(val) = &output.value {
        println!("{val}");
    }

    if let Some(args) = render {
        let reg = registry.lock().await;
        let slot = reg
            .get(&doc_key)
            .ok_or_else(|| anyhow::anyhow!("document '{doc_key}' missing after the script"))?;
        let cwd = std::env::current_dir()?;
        let result =
            render::render_to_file(&slot.document, args, &cwd).map_err(|e| anyhow::anyhow!(e))?;
        println!("{result}");
    }

    Ok(())
}

async fn accept_wt_sessions(
    endpoint: wtransport::Endpoint<wtransport::endpoint::endpoint_side::Server>,
    registry: Arc<Mutex<DocumentRegistry>>,
    sandbox: script::ScriptSandbox,
) -> anyhow::Result<()> {
    loop {
        let incoming = endpoint.accept().await;
        let session_request = incoming.await?;
        let conn = session_request.accept().await?;

        let reg = registry.clone();
        let sb = sandbox.clone();
        tokio::spawn(async move {
            transport::handle_session_v2(conn, reg, sb).await;
        });
    }
}
