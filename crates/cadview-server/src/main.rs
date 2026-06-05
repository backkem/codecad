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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // Parse CLI flags
    let mut dist_override: Option<PathBuf> = None;
    let mut examples_override: Option<PathBuf> = None;
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
                    args_iter.next().expect("--examples requires a path argument"),
                ));
            }
            _ => positional.push(arg),
        }
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
            anyhow::bail!("Path '{}' is neither a file nor a directory", path.display());
        }
    };

    let available = registry.list_available();
    tracing::info!("{} document(s) available", available.len());

    let registry = Arc::new(Mutex::new(registry));

    // Initialize WASM sandbox for server-side script execution.
    let sandbox = Arc::new(
        script::create_sandbox().expect("Failed to initialize WASM sandbox"),
    );
    tracing::info!("WASM sandbox initialized");

    // TLS for WebTransport
    let identity = wtransport::Identity::self_signed(["localhost", "127.0.0.1", "::1"])?;
    let cert_hash = identity.certificate_chain().as_slice()[0].hash().as_ref().to_vec();

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

async fn accept_wt_sessions(
    endpoint: wtransport::Endpoint<wtransport::endpoint::endpoint_side::Server>,
    registry: Arc<Mutex<DocumentRegistry>>,
    sandbox: Arc<cadview_sandbox::Sandbox>,
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

