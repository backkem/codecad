//! HTTP server: serves dist/, document list API, cert hash injection, auth.

use crate::assets::{self, AssetProvider};
use crate::registry::DocumentRegistry;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct HttpState {
    pub dist: Box<dyn AssetProvider>,
    pub cert_hash: Vec<u8>,
    pub wt_port: u16,
    pub registry: Arc<Mutex<DocumentRegistry>>,
    pub token: String,
    pub sandbox: crate::script::ScriptSandbox,
}

pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/api/documents", get(documents_handler))
        .route("/api/run", post(run_handler))
        .route("/api/bake3d", get(bake3d_handler))
        .route("/", get(index_handler))
        .route("/{*path}", get(static_handler))
        .with_state(state)
}

// ── API handlers ────────────────────────────────────────────────────

async fn documents_handler(
    State(state): State<Arc<HttpState>>,
    req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    // Auth check
    if let Err(resp) = check_bearer(&state.token, &req) {
        return resp.into_response();
    }
    let reg = state.registry.lock().await;
    let docs = reg.list_available();
    Json(docs).into_response()
}

fn check_bearer(
    expected: &str,
    req: &axum::http::Request<axum::body::Body>,
) -> Result<(), (StatusCode, String)> {
    let Some(auth) = req.headers().get(header::AUTHORIZATION) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Missing Authorization header".to_string(),
        ));
    };
    let Ok(val) = auth.to_str() else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Invalid Authorization header".to_string(),
        ));
    };
    let Some(token) = val.strip_prefix("Bearer ") else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Expected Bearer token".to_string(),
        ));
    };
    if token != expected {
        return Err((StatusCode::FORBIDDEN, "Invalid token".to_string()));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct BakeQuery {
    /// Document key (default: the first available).
    doc: Option<String>,
    /// `cad.render`-style options as JSON (view, colours, materials, ...).
    options: Option<String>,
}

/// `GET /api/bake3d?doc=<key>[&options=<json>]`: the document's parts as
/// a prebaked 3D scene (the bytes `cad.bake3d` writes). The browser's 3D
/// view shows it until the page evaluates parts itself.
async fn bake3d_handler(
    State(state): State<Arc<HttpState>>,
    axum::extract::Query(q): axum::extract::Query<BakeQuery>,
    req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    if let Err(resp) = check_bearer(&state.token, &req) {
        return resp.into_response();
    }
    let opts: serde_json::Value = match q.options.as_deref() {
        None | Some("") => serde_json::json!({}),
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, format!("options: {e}")).into_response(),
        },
    };
    let mut reg = state.registry.lock().await;
    let key = match q.doc {
        Some(k) => k,
        None => reg
            .list_available()
            .first()
            .map(|d| d.id.clone())
            .unwrap_or_default(),
    };
    let slot = match reg.get_or_load(&key) {
        Ok(s) => s,
        Err(e) => return (StatusCode::NOT_FOUND, format!("{key}: {e}")).into_response(),
    };
    match crate::bake::bake_bytes(&slot.document, &opts) {
        Ok((bytes, _)) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, e).into_response(),
    }
}

// ── Static file handlers ────────────────────────────────────────────

async fn index_handler(State(state): State<Arc<HttpState>>) -> impl IntoResponse {
    let bytes = match state.dist.get("index.html") {
        Some(b) => b,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Cannot read index.html").into_response();
        }
    };
    let html = String::from_utf8_lossy(&bytes).into_owned();

    // Inject cert hash, WT port, and auth token
    let hash_hex: Vec<String> = state
        .cert_hash
        .iter()
        .map(|b| format!("0x{b:02x}"))
        .collect();
    let inject = format!(
        r#"<script>window.__CADVIEW_WT_PORT={};window.__CADVIEW_CERT_HASH=new Uint8Array([{}]);window.__CADVIEW_TOKEN="{}";</script>"#,
        state.wt_port,
        hash_hex.join(","),
        state.token,
    );
    let html = html.replace("</head>", &format!("{inject}\n</head>"));

    Html(html).into_response()
}

async fn static_handler(
    State(state): State<Arc<HttpState>>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    match state.dist.get(&path) {
        Some(bytes) => {
            let ct = assets::content_type(&path);
            ([(header::CONTENT_TYPE, ct)], bytes.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

// ── Script execution ────────────────────────────────────────────────

async fn run_handler(
    State(state): State<Arc<HttpState>>,
    req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    if let Err(resp) = check_bearer(&state.token, &req) {
        return resp.into_response();
    }

    let body = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("body: {e}")).into_response(),
    };
    let params: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("json: {e}")).into_response(),
    };

    // Resolve doc_id (default to first available).
    let doc_id = match params["doc_id"].as_str() {
        Some(id) => id.to_string(),
        None => {
            let reg = state.registry.lock().await;
            reg.list_available()
                .first()
                .map(|d| d.id.clone())
                .unwrap_or_default()
        }
    };

    let timeout = params["timeout"]
        .as_f64()
        .map(std::time::Duration::from_secs_f64);

    // exec mode: load .js file from disk
    if let Some(path) = params["exec"].as_str() {
        return match crate::script::exec_file(
            &state.sandbox,
            &state.registry,
            &doc_id,
            path,
            timeout,
        )
        .await
        {
            Ok(output) => Json(serde_json::json!({
                "ok": true,
                "value": output.value,
                "stdout": output.stdout,
                "stderr": output.stderr,
            }))
            .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response(),
        };
    }

    // program mode: run inline script
    let program = params["program"].as_str().unwrap_or("");
    let base_dir = std::env::current_dir().unwrap_or_default();

    match crate::script::run_script(
        &state.sandbox,
        &state.registry,
        &doc_id,
        program,
        &base_dir,
        timeout,
    )
    .await
    {
        Ok(output) => Json(serde_json::json!({
            "ok": true,
            "value": output.value,
            "stdout": output.stdout,
            "stderr": output.stderr,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}
