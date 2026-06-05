use cadview_core::sync::SyncDoc;
use cadview_core::{
    flatten_bezpath_adaptive, tessellate_ellipse, tessellate_lwpolyline, tessellate_spline,
    triangulate_fill, Document, DrawEntity, EntityId, Shape,
};
use eframe::egui;
use egui::{Color32, Pos2, Stroke};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(all(target_arch = "wasm32", feature = "vello-renderer"))]
mod vello_render;

// ── Types ─────────────────────────────────────────────────────────────

type SessionId = String;
type RendererKey = String;
// CanvasId removed: renderers now receive HtmlCanvasElement directly.

#[cfg(target_arch = "wasm32")]
static NEXT_RENDERER_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Debug)]
#[allow(dead_code)]
enum CameraCmd {
    FitAll,
    FitBounds(f64, f64, f64, f64),
    SetView(f64, f64, f64),
}

struct Camera {
    center_x: f64,
    center_y: f64,
    zoom: f64,
}

impl Camera {
    fn new() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
        }
    }

    fn fit(bounds: (f64, f64, f64, f64), screen_w: f32, screen_h: f32) -> Self {
        let (x0, y0, x1, y1) = bounds;
        let dw = x1 - x0;
        let dh = y1 - y0;
        let margin = 1.1;
        let zoom_x = screen_w as f64 / (dw * margin);
        let zoom_y = screen_h as f64 / (dh * margin);
        Camera {
            center_x: (x0 + x1) / 2.0,
            center_y: (y0 + y1) / 2.0,
            zoom: zoom_x.min(zoom_y),
        }
    }
}

type CachedFill = (f64, Vec<[f32; 2]>, Vec<u32>);
type CachedCurve = (f64, Vec<Vec<(f64, f64)>>);

/// Render cache for tessellated geometry. Invalidated on document or zoom changes.
struct RenderCache {
    fills: HashMap<EntityId, CachedFill>,
    arcs: HashMap<EntityId, (usize, Vec<(f64, f64)>)>,
    curves: HashMap<EntityId, CachedCurve>,
    expanded: Vec<DrawEntity>,
    entity_count: usize,
}

impl RenderCache {
    fn new() -> Self {
        Self {
            fills: HashMap::new(),
            arcs: HashMap::new(),
            curves: HashMap::new(),
            expanded: Vec::new(),
            entity_count: 0,
        }
    }
    fn invalidate_if_changed(&mut self, entity_count: usize) {
        if entity_count != self.entity_count {
            self.fills.clear();
            self.arcs.clear();
            self.curves.clear();
            self.expanded.clear();
            self.entity_count = entity_count;
        }
    }
}

/// Per-document session. Bundles all state for one open drawing.
#[allow(dead_code)]
struct DocumentSession {
    doc: Document,
    sync: SyncDoc,
    sync_sv_before: Option<Vec<u8>>,
    hidden_layers: Vec<String>,
    camera_cmd: Option<CameraCmd>,
    camera: Camera,
    screen_size: (f32, f32),
    initialized: bool,
    last_entity_count: usize,
    cache: RenderCache,
    /// Set by client-side commands (setLayerVisible, etc.) to signal
    /// renderers that a repaint is needed even without entity changes.
    render_dirty: bool,
}

impl DocumentSession {
    fn new(client_id: u64) -> Self {
        Self {
            doc: Document::new(),
            sync: SyncDoc::new(client_id),
            sync_sv_before: None,
            hidden_layers: Vec::new(),
            camera_cmd: None,
            camera: Camera::new(),
            screen_size: (1280.0, 800.0),
            initialized: false,
            last_entity_count: 0,
            render_dirty: false,
            cache: RenderCache::new(),
        }
    }
}

/// Callback to schedule a render frame for a specific canvas.
/// SAFETY: WASM is single-threaded, so Send is always safe.
#[allow(dead_code)]
struct RepaintFn(Box<dyn Fn() + 'static>);
unsafe impl Send for RepaintFn {}
#[allow(dead_code)]
impl RepaintFn {
    fn call(&self) {
        (self.0)();
    }
}

/// Global session registry. Single static, replaces all per-document statics.
struct SessionRegistry {
    sessions: HashMap<SessionId, DocumentSession>,
    /// Which session `cad_call` targets (set by `session_use`).
    js_target: Option<SessionId>,
    /// Map of renderer_key -> session_id for active renderers.
    renderers: HashMap<RendererKey, SessionId>,
    /// Repaint callbacks per renderer_key. Renderers register on start.
    #[allow(dead_code)]
    repaint_fns: HashMap<RendererKey, RepaintFn>,
    /// Monotonic counter for Yrs client IDs (each session needs a unique one).
    next_client_id: u64,
}

impl SessionRegistry {
    fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            js_target: None,
            renderers: HashMap::new(),
            repaint_fns: HashMap::new(),
            next_client_id: 100, // start at 100 to avoid conflict with server (1)
        }
    }
}

static SESSIONS: LazyLock<Mutex<SessionRegistry>> =
    LazyLock::new(|| Mutex::new(SessionRegistry::new()));

// ── Session management exports ────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_create(session_id: &str) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    if reg.sessions.contains_key(session_id) {
        return format!(r#"{{"error":"session '{}' already exists"}}"#, session_id);
    }
    let client_id = reg.next_client_id;
    reg.next_client_id += 1;
    reg.sessions
        .insert(session_id.to_string(), DocumentSession::new(client_id));
    // Auto-set js_target if this is the first session
    if reg.js_target.is_none() {
        reg.js_target = Some(session_id.to_string());
    }
    format!(
        r#"{{"ok":true,"session":"{}","client_id":{}}}"#,
        session_id, client_id
    )
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_destroy(session_id: &str) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    if reg.sessions.remove(session_id).is_none() {
        return format!(r#"{{"error":"session '{}' not found"}}"#, session_id);
    }
    // Clear js_target if it pointed at the destroyed session
    if reg.js_target.as_deref() == Some(session_id) {
        reg.js_target = None;
    }
    // Remove any renderers pointing at this session
    reg.renderers.retain(|_, sid| sid != session_id);
    r#"{"ok":true}"#.to_string()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_use(session_id: &str) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    if !reg.sessions.contains_key(session_id) {
        return format!(r#"{{"error":"session '{}' not found"}}"#, session_id);
    }
    reg.js_target = Some(session_id.to_string());
    format!(r#"{{"ok":true,"session":"{}"}}"#, session_id)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_current() -> String {
    let reg = SESSIONS.lock().unwrap();
    match &reg.js_target {
        Some(id) => format!(r#""{}""#, id),
        None => "null".to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_list() -> String {
    let reg = SESSIONS.lock().unwrap();
    let items: Vec<String> = reg
        .sessions
        .iter()
        .map(|(id, s)| {
            let is_target = reg.js_target.as_deref() == Some(id.as_str());
            let has_renderer = reg.renderers.values().any(|sid| sid == id);
            format!(
                r#"{{"id":"{}","entity_count":{},"js_target":{},"visible":{}}}"#,
                id,
                s.doc.entities.len(),
                is_target,
                has_renderer
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Load DWG bytes into a session (replaces current document content).
/// Used for drag-and-drop file loading in the browser.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn session_load_dwg(session_id: &str, data: &[u8]) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get_mut(session_id) else {
        return format!(r#"{{"error":"session '{}' not found"}}"#, session_id);
    };
    match cadview_core::load_dwg_bytes(data) {
        Ok(doc) => {
            let count = doc.entities.len();
            session.sync.populate_from_document(&doc);
            session.doc = doc;
            session.cache = RenderCache::new();
            session.initialized = false; // trigger auto-fit
            session.camera_cmd = Some(CameraCmd::FitAll);
            format!(
                r#"{{"ok":true,"entities":{count},"_session":"{}"}}"#,
                session_id
            )
        }
        Err(e) => format!(r#"{{"error":"DWG parse failed: {}"}}"#, e),
    }
}

// ── Yrs sync exports (per-session) ───────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn yrs_state_vector(session_id: &str) -> Vec<u8> {
    let reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get(session_id) else {
        return Vec::new();
    };
    session.sync.state_vector()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn yrs_apply_update(session_id: &str, update: &[u8]) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get_mut(session_id) else {
        return format!(r#"{{"error":"session '{}' not found"}}"#, session_id);
    };
    if let Err(e) = session.sync.apply_update(update) {
        return format!(r#"{{"error":"{e}"}}"#);
    }

    let was_empty = session.doc.entities.is_empty();
    let new_doc = session.sync.to_document();
    let count = new_doc.entities.len();
    session.doc = new_doc;

    if was_empty && count > 0 {
        session.camera_cmd = Some(CameraCmd::FitAll);
    }

    // Wake renderer after sync update
    session.render_dirty = true;
    for (cid, sid) in &reg.renderers {
        if sid.as_str() == session_id {
            if let Some(repaint) = reg.repaint_fns.get(cid) {
                repaint.call();
            }
            break;
        }
    }

    format!(
        r#"{{"ok":true,"entities":{count},"_session":"{}"}}"#,
        session_id
    )
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn yrs_encode_update(session_id: &str, remote_sv: &[u8]) -> Vec<u8> {
    let reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get(session_id) else {
        return Vec::new();
    };
    session.sync.encode_diff(remote_sv).unwrap_or_default()
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn yrs_pending_update(session_id: &str) -> Vec<u8> {
    let mut reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get_mut(session_id) else {
        return Vec::new();
    };
    let Some(sv) = session.sync_sv_before.take() else {
        return Vec::new();
    };
    session.sync.encode_diff(&sv).unwrap_or_default()
}

// ── cad_call (targets js_target session) ──────────────────────────────

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn cad_call(method: &str, args_json: &str) -> String {
    let mut reg = SESSIONS.lock().unwrap();

    let target_id = match &reg.js_target {
        Some(id) => id.clone(),
        None => return r#"{"error":"no session selected (call session_use first)"}"#.to_string(),
    };

    let Some(session) = reg.sessions.get_mut(&target_id) else {
        return format!(r#"{{"error":"session '{}' was destroyed"}}"#, target_id);
    };

    // Viewport commands operate on the targeted session
    match method {
        "fitView" => {
            session.camera_cmd = Some(CameraCmd::FitAll);
            session.render_dirty = true;
            // Wake renderer
            for (cid, sid) in &reg.renderers {
                if sid.as_str() == target_id {
                    if let Some(repaint) = reg.repaint_fns.get(cid) {
                        repaint.call();
                    }
                    break;
                }
            }
            return format!(r#"{{"ok":true,"_session":"{}"}}"#, target_id);
        }
        "zoomTo" => {
            let v: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
            let (sw, sh) = session.screen_size;

            let result;

            if let Some(bounds) = v.get("bounds") {
                let min = bounds.get("min").and_then(|m| m.as_array());
                let max = bounds.get("max").and_then(|m| m.as_array());
                if let (Some(min), Some(max)) = (min, max) {
                    let x0 = min[0].as_f64().unwrap_or(0.0);
                    let y0 = min[1].as_f64().unwrap_or(0.0);
                    let x1 = max[0].as_f64().unwrap_or(0.0);
                    let y1 = max[1].as_f64().unwrap_or(0.0);
                    let cam = Camera::fit((x0, y0, x1, y1), sw, sh);
                    session.camera_cmd = Some(CameraCmd::FitBounds(x0, y0, x1, y1));
                    result = format!(
                        r#"{{"center":[{},{}],"zoom":{},"_session":"{}"}}"#,
                        cam.center_x, cam.center_y, cam.zoom, target_id
                    );
                } else {
                    result = format!(r#"{{"error":"invalid bounds","_session":"{}"}}"#, target_id);
                }
            } else if let Some(id_str) = v.get("id").and_then(|i| i.as_str()) {
                let num = id_str.strip_prefix("e_").unwrap_or(id_str);
                if let Ok(n) = num.parse::<u64>() {
                    if let Some(ent) = session.doc.entity(cadview_core::EntityId(n)) {
                        let (x0, y0, x1, y1) = ent.shape.bbox();
                        let pad = ((x1 - x0).max(y1 - y0) * 0.5).max(10.0);
                        let (bx0, by0, bx1, by1) = (x0 - pad, y0 - pad, x1 + pad, y1 + pad);
                        let cam = Camera::fit((bx0, by0, bx1, by1), sw, sh);
                        session.camera_cmd = Some(CameraCmd::FitBounds(bx0, by0, bx1, by1));
                        result = format!(
                            r#"{{"center":[{},{}],"zoom":{},"_session":"{}"}}"#,
                            cam.center_x, cam.center_y, cam.zoom, target_id
                        );
                    } else {
                        result = format!(
                            r#"{{"error":"entity not found","_session":"{}"}}"#,
                            target_id
                        );
                    }
                } else {
                    result = format!(
                        r#"{{"error":"entity not found","_session":"{}"}}"#,
                        target_id
                    );
                }
            } else if let (Some(center), Some(zoom)) = (
                v.get("center").and_then(|c| c.as_array()),
                v.get("zoom").and_then(|z| z.as_f64()),
            ) {
                let cx = center[0].as_f64().unwrap_or(0.0);
                let cy = center[1].as_f64().unwrap_or(0.0);
                session.camera_cmd = Some(CameraCmd::SetView(cx, cy, zoom));
                result = format!(
                    r#"{{"center":[{},{}],"zoom":{},"_session":"{}"}}"#,
                    cx, cy, zoom, target_id
                );
            } else {
                result = format!(
                    r#"{{"error":"zoomTo needs {{bounds}}, {{id}}, or {{center, zoom}}","_session":"{}"}}"#,
                    target_id
                );
            }

            // Wake renderer if camera changed
            if session.camera_cmd.is_some() {
                session.render_dirty = true;
                for (cid, sid) in &reg.renderers {
                    if sid.as_str() == target_id {
                        if let Some(repaint) = reg.repaint_fns.get(cid) {
                            repaint.call();
                        }
                        break;
                    }
                }
            }
            return result;
        }
        "getView" => {
            return format!(
                r#"{{"center":[{},{}],"zoom":{},"_session":"{}"}}"#,
                session.camera.center_x, session.camera.center_y, session.camera.zoom, target_id
            );
        }
        "setLayerVisible" => {
            let v: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
            let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let visible = v.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
            if visible {
                session.hidden_layers.retain(|n| n != name);
            } else if !session.hidden_layers.iter().any(|n| n == name) {
                session.hidden_layers.push(name.to_string());
            }
            session.render_dirty = true;
            // Wake up the renderer for this session
            for (cid, sid) in &reg.renderers {
                if sid.as_str() == target_id {
                    if let Some(repaint) = reg.repaint_fns.get(cid) {
                        repaint.call();
                    }
                    break;
                }
            }
            return format!(r#"{{"ok":true,"_session":"{}"}}"#, target_id);
        }
        _ => {}
    }

    // Document mutations via SyncDoc
    // Capture SV before first un-flushed mutation
    if session.sync_sv_before.is_none() {
        session.sync_sv_before = Some(session.sync.state_vector());
    }
    let result = match session
        .sync
        .apply_mutation(&mut session.doc, method, args_json)
    {
        Ok((result, _update)) => result, // update is coalesced at flush
        Err(e) => format!(r#"{{"error":{}}}"#, serde_json::json!(e)),
    };

    // Inject _session into the result JSON
    if result.starts_with('{') && result.ends_with('}') {
        format!(
            "{},\"_session\":\"{}\"}}",
            &result[..result.len() - 1],
            target_id
        )
    } else {
        result
    }
}

// ── Renderer lifecycle ────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_renderer(
    canvas: web_sys::HtmlCanvasElement,
    session_id: &str,
    renderer_type: &str,
) -> String {
    let key = format!(
        "rv_{}",
        NEXT_RENDERER_KEY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    {
        let mut reg = SESSIONS.lock().unwrap();
        if !reg.sessions.contains_key(session_id) {
            return format!(r#"{{"error":"session '{}' not found"}}"#, session_id);
        }
        reg.renderers.insert(key.clone(), session_id.to_string());
    }

    #[cfg(feature = "vello-renderer")]
    if renderer_type == "vello" {
        vello_render::start(canvas, session_id, &key);
        return format!(
            r#"{{"ok":true,"key":"{}","session":"{}","renderer":"vello"}}"#,
            key, session_id
        );
    }

    let _ = renderer_type; // suppress unused warning when vello feature is off
    start_egui_renderer(canvas, session_id, &key)
}

#[cfg(target_arch = "wasm32")]
fn start_egui_renderer(
    canvas: web_sys::HtmlCanvasElement,
    session_id: &str,
    renderer_key: &str,
) -> String {
    let session_id_owned = session_id.to_string();
    let key_owned = renderer_key.to_string();

    let app = CadViewApp {
        session_id: session_id_owned.clone(),
        renderer_key: key_owned.clone(),
    };

    let key_for_repaint = key_owned.clone();
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    // Register repaint callback so setLayerVisible can wake egui
                    let ctx = cc.egui_ctx.clone();
                    let mut reg = SESSIONS.lock().unwrap();
                    reg.repaint_fns.insert(
                        key_for_repaint,
                        RepaintFn(Box::new(move || {
                            ctx.request_repaint();
                        })),
                    );
                    Ok(Box::new(app))
                }),
            )
            .await
            .expect("failed to start eframe renderer");
    });

    format!(
        r#"{{"ok":true,"key":"{}","session":"{}","renderer":"egui"}}"#,
        renderer_key, session_id
    )
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn stop_renderer(renderer_key: &str) -> String {
    let mut reg = SESSIONS.lock().unwrap();
    if reg.renderers.remove(renderer_key).is_some() {
        reg.repaint_fns.remove(renderer_key);
        // The CadViewApp/Vello will see it's no longer in the renderers map
        // and stop requesting repaints. The caller should remove the canvas element.
        r#"{"ok":true}"#.to_string()
    } else {
        format!(r#"{{"error":"no renderer for key '{}'"}}"#, renderer_key)
    }
}

// ── Binary export (DWG / PDF bytes for browser download) ────────────

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn export_dwg_bytes(session_id: &str) -> Result<Vec<u8>, JsValue> {
    let reg = SESSIONS.lock().unwrap();
    let session = reg
        .sessions
        .get(session_id)
        .ok_or_else(|| JsValue::from_str(&format!("session '{}' not found", session_id)))?;
    cadview_core::export_dwg_bytes(&session.doc)
        .map_err(|e| JsValue::from_str(&format!("DWG export failed: {e}")))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn export_pdf_bytes(session_id: &str) -> Vec<u8> {
    let reg = SESSIONS.lock().unwrap();
    let session = match reg.sessions.get(session_id) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let opts = cadview_core::pdf::PdfOptions::default();
    cadview_core::export_pdf_bytes(&session.doc, &opts)
}

// ── Entry points ──────────────────────────────────────────────────────

#[cfg(feature = "embed-dwg")]
const DWG_BYTES: &[u8] = include_bytes!(concat!(env!("CODECAD_SAMPLE_DWG")));

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // Create a default session for native mode
    let mut reg = SESSIONS.lock().unwrap();
    let client_id = reg.next_client_id;
    reg.next_client_id += 1;
    #[allow(unused_mut)]
    let mut session = DocumentSession::new(client_id);

    #[cfg(feature = "embed-dwg")]
    {
        session.doc =
            cadview_core::load_dwg_bytes(DWG_BYTES).expect("failed to parse embedded DWG");
        session.sync.populate_from_document(&session.doc);
    }

    reg.sessions.insert("default".to_string(), session);
    reg.js_target = Some("default".to_string());
    reg.renderers
        .insert("native".to_string(), "default".to_string());
    drop(reg);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("cadview"),
        ..Default::default()
    };

    eframe::run_native(
        "cadview",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(CadViewApp {
                session_id: "default".to_string(),
                renderer_key: "native".to_string(),
            }))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // In WASM mode, don't start a renderer or create sessions automatically.
    // JS will call session_create() and start_renderer() as needed.
    // But if embed-dwg is on, create a default session for convenience.
    #[cfg(feature = "embed-dwg")]
    {
        let mut reg = SESSIONS.lock().unwrap();
        let client_id = reg.next_client_id;
        reg.next_client_id += 1;
        let mut session = DocumentSession::new(client_id);
        session.doc =
            cadview_core::load_dwg_bytes(DWG_BYTES).expect("failed to parse embedded DWG");
        session.sync.populate_from_document(&session.doc);
        reg.sessions.insert("default".to_string(), session);
        reg.js_target = Some("default".to_string());
    }
}

// ── Renderer ──────────────────────────────────────────────────────────

struct CadViewApp {
    session_id: SessionId,
    renderer_key: RendererKey,
}

impl CadViewApp {
    fn to_color32(c: &cadview_core::Color) -> Color32 {
        Color32::from_rgb(c.r, c.g, c.b)
    }
}

impl eframe::App for CadViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let sw = rect.width();
        let sh = rect.height();

        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(30, 30, 30));

        // Check if this renderer is still registered
        let mut reg = SESSIONS.lock().unwrap();
        let still_active = reg.renderers.get(&self.renderer_key) == Some(&self.session_id);
        if !still_active {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Session closed",
                egui::FontId::proportional(16.0),
                Color32::from_rgb(100, 100, 100),
            );
            // Don't request_repaint -- let this renderer go idle
            return;
        }

        let Some(session) = reg.sessions.get_mut(&self.session_id) else {
            return;
        };

        // Update screen size for this session (used by cad_call zoomTo)
        session.screen_size = (sw, sh);

        // Process camera commands
        let entity_count = session.doc.entities.len();
        let camera_cmd = session.camera_cmd.take();
        let has_camera_cmd = camera_cmd.is_some();
        let auto_fit = !session.initialized || (session.last_entity_count == 0 && entity_count > 0);

        if let Some(cmd) = camera_cmd {
            match cmd {
                CameraCmd::FitAll => {
                    if let Some(bounds) = session.doc.bounds() {
                        session.camera = Camera::fit(bounds, sw, sh);
                    }
                }
                CameraCmd::FitBounds(x0, y0, x1, y1) => {
                    session.camera = Camera::fit((x0, y0, x1, y1), sw, sh);
                }
                CameraCmd::SetView(cx, cy, zoom) => {
                    session.camera.center_x = cx;
                    session.camera.center_y = cy;
                    session.camera.zoom = zoom;
                }
            }
            session.initialized = true;
        } else if auto_fit {
            if let Some(bounds) = session.doc.bounds() {
                session.camera = Camera::fit(bounds, sw, sh);
            }
            session.initialized = true;
        }
        session.last_entity_count = entity_count;

        // Drop lock for input handling
        drop(reg);

        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Re-lock for camera mutation
        let mut reg = SESSIONS.lock().unwrap();
        let Some(session) = reg.sessions.get_mut(&self.session_id) else {
            return;
        };

        let mut needs_repaint = auto_fit || has_camera_cmd || session.render_dirty;
        session.render_dirty = false;

        if response.dragged() {
            let delta = response.drag_delta();
            session.camera.center_x -= delta.x as f64 / session.camera.zoom;
            session.camera.center_y += delta.y as f64 / session.camera.zoom;
            needs_repaint = true;
        }

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        let pinch_zoom = ui.input(|i| i.zoom_delta());
        let zoom_factor = if pinch_zoom != 1.0 {
            pinch_zoom as f64
        } else if scroll_delta.abs() > 0.5 {
            if scroll_delta > 0.0 {
                1.08
            } else {
                1.0 / 1.08
            }
        } else {
            1.0
        };

        if (zoom_factor - 1.0).abs() > 1e-6 {
            if let Some(mouse) = ui.input(|i| i.pointer.hover_pos()) {
                let mx = (mouse.x as f64 - sw as f64 / 2.0) / session.camera.zoom
                    + session.camera.center_x;
                let my = -(mouse.y as f64 - sh as f64 / 2.0) / session.camera.zoom
                    + session.camera.center_y;
                session.camera.zoom *= zoom_factor;
                session.camera.center_x =
                    mx - (mouse.x as f64 - sw as f64 / 2.0) / session.camera.zoom;
                session.camera.center_y =
                    my + (mouse.y as f64 - sh as f64 / 2.0) / session.camera.zoom;
            } else {
                session.camera.zoom *= zoom_factor;
            }
            needs_repaint = true;
        }

        // Render
        let doc = &session.doc;
        let painter = ui.painter_at(rect);

        let cam_cx = session.camera.center_x;
        let cam_cy = session.camera.center_y;
        let cam_zoom = session.camera.zoom;

        let half_w = (sw as f64 / 2.0) / cam_zoom;
        let half_h = (sh as f64 / 2.0) / cam_zoom;
        let view_x0 = cam_cx - half_w;
        let view_x1 = cam_cx + half_w;
        let view_y0 = cam_cy - half_h;
        let view_y1 = cam_cy + half_h;

        const CULL_THRESHOLD: f64 = 0.5;
        const FADE_THRESHOLD: f64 = 8.0;
        const MAX_ERROR_PX: f64 = 0.5;
        let world_tolerance = MAX_ERROR_PX / cam_zoom;

        let mut rendered = 0usize;

        session.cache.invalidate_if_changed(entity_count);

        // Expand block inserts + text into cached flat shape list
        if session.cache.expanded.is_empty()
            && doc.entities.iter().any(|e| {
                matches!(
                    &e.shape,
                    Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. }
                )
            })
        {
            session.cache.expanded = cadview_core::expand_for_render(doc);
        }

        // All renderable entities: direct + expanded
        let all_entities: Vec<&DrawEntity> = doc
            .entities
            .iter()
            .filter(|e| {
                !matches!(
                    &e.shape,
                    Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. }
                )
            })
            .chain(session.cache.expanded.iter())
            .collect();

        // Helper closure to convert world -> screen using session camera
        let to_screen = |x: f64, y: f64| -> Pos2 {
            let sx = (x - cam_cx) * cam_zoom + sw as f64 / 2.0;
            let sy = -(y - cam_cy) * cam_zoom + sh as f64 / 2.0;
            Pos2::new(sx as f32, sy as f32)
        };

        // Pass 1: SolidFill
        for ent in &all_entities {
            let Shape::SolidFill { boundary, holes } = &ent.shape else {
                continue;
            };
            if boundary.is_empty() {
                continue;
            }
            if session.hidden_layers.iter().any(|n| n == &ent.layer) {
                continue;
            }

            let (bx0, by0, bx1, by1) = ent.shape.bbox();
            if bx1 < view_x0 || bx0 > view_x1 || by1 < view_y0 || by0 > view_y1 {
                continue;
            }

            let needs_update = match session.cache.fills.get(&ent.id) {
                Some((cached_tol, _, _)) => world_tolerance < *cached_tol * 0.5,
                None => true,
            };
            if needs_update {
                let (t, i) = triangulate_fill(boundary, holes, world_tolerance);
                session.cache.fills.insert(ent.id, (world_tolerance, t, i));
            }
            let (_, triangles, tri_indices) = session.cache.fills.get(&ent.id).unwrap();
            if tri_indices.is_empty() {
                continue;
            }

            let base_color = Self::to_color32(&ent.color);
            let fill_color =
                Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), 35);

            let mut mesh = egui::Mesh::default();
            for &[x, y] in triangles.iter() {
                let screen = to_screen(x as f64, y as f64);
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: screen,
                    uv: Pos2::ZERO,
                    color: fill_color,
                });
            }
            mesh.indices = tri_indices.clone();
            painter.add(egui::Shape::mesh(mesh));
            rendered += 1;
        }

        // Pass 2: strokes
        for ent in &all_entities {
            if matches!(&ent.shape, Shape::SolidFill { .. }) {
                continue;
            }
            if session.hidden_layers.iter().any(|n| n == &ent.layer) {
                continue;
            }

            let (bx0, by0, bx1, by1) = ent.shape.bbox();
            if bx1 < view_x0 || bx0 > view_x1 || by1 < view_y0 || by0 > view_y1 {
                continue;
            }

            let dw = bx1 - bx0;
            let dh = by1 - by0;
            let diag_screen = (dw * dw + dh * dh).sqrt() * cam_zoom;
            if diag_screen < CULL_THRESHOLD {
                continue;
            }

            let base_color = Self::to_color32(&ent.color);
            let color = if diag_screen < FADE_THRESHOLD {
                let t = ((diag_screen - CULL_THRESHOLD) / (FADE_THRESHOLD - CULL_THRESHOLD))
                    .clamp(0.0, 1.0) as f32;
                let alpha = t * t * (3.0 - 2.0 * t);
                let a = (alpha * base_color.a() as f32) as u8;
                Color32::from_rgba_unmultiplied(base_color.r(), base_color.g(), base_color.b(), a)
            } else {
                base_color
            };
            let stroke = Stroke::new(1.0_f32, color);
            rendered += 1;

            match &ent.shape {
                Shape::Line(line) => {
                    let p0 = to_screen(line.p0.x, line.p0.y);
                    let p1 = to_screen(line.p1.x, line.p1.y);
                    painter.line_segment([p0, p1], stroke);
                }
                Shape::Circle(circle) => {
                    let c = to_screen(circle.center.x, circle.center.y);
                    let r = (circle.radius * cam_zoom) as f32;
                    painter.circle_stroke(c, r, stroke);
                }
                Shape::Arc {
                    center,
                    radius,
                    start_angle,
                    end_angle,
                    ..
                } => {
                    let r_screen = *radius * cam_zoom;
                    let mut sweep = end_angle - start_angle;
                    if sweep < 0.0 {
                        sweep += 2.0 * std::f64::consts::PI;
                    }
                    let steps = if r_screen < MAX_ERROR_PX {
                        4usize
                    } else {
                        let theta = 2.0 * (1.0 - MAX_ERROR_PX / r_screen).acos();
                        (sweep / theta).ceil().clamp(4.0, 4096.0) as usize
                    };

                    let cached = session.cache.arcs.get(&ent.id);
                    let world_pts = if cached.is_none_or(|(n, _)| *n != steps) {
                        let pts: Vec<(f64, f64)> = (0..=steps)
                            .map(|i| {
                                let t = i as f64 / steps as f64;
                                let angle = start_angle + t * sweep;
                                (
                                    center.x + radius * angle.cos(),
                                    center.y + radius * angle.sin(),
                                )
                            })
                            .collect();
                        session.cache.arcs.insert(ent.id, (steps, pts));
                        &session.cache.arcs.get(&ent.id).unwrap().1
                    } else {
                        &cached.unwrap().1
                    };
                    let points: Vec<Pos2> = world_pts
                        .iter()
                        .map(|&(wx, wy)| to_screen(wx, wy))
                        .collect();
                    painter.add(egui::epaint::PathShape::line(points, stroke));
                }
                Shape::Polyline {
                    points: pts,
                    closed,
                } => {
                    if pts.len() >= 2 {
                        let mut screen_pts: Vec<Pos2> =
                            pts.iter().map(|p| to_screen(p.x, p.y)).collect();
                        if *closed {
                            screen_pts.push(screen_pts[0]);
                        }
                        painter.add(egui::epaint::PathShape::line(screen_pts, stroke));
                    }
                }
                Shape::CurvePath { path, closed } => {
                    let needs_update = match session.cache.curves.get(&ent.id) {
                        Some((cached_tol, _)) => world_tolerance < *cached_tol * 0.5,
                        None => true,
                    };
                    if needs_update {
                        let contours = flatten_bezpath_adaptive(path, world_tolerance);
                        let world_contours: Vec<Vec<(f64, f64)>> = contours
                            .iter()
                            .map(|c| c.iter().map(|p| (p.x, p.y)).collect())
                            .collect();
                        session
                            .cache
                            .curves
                            .insert(ent.id, (world_tolerance, world_contours));
                    }
                    let (_, contours) = session.cache.curves.get(&ent.id).unwrap();
                    for contour in contours {
                        if contour.len() < 2 {
                            continue;
                        }
                        let mut screen_pts: Vec<Pos2> =
                            contour.iter().map(|&(x, y)| to_screen(x, y)).collect();
                        if *closed && screen_pts.len() >= 2 {
                            screen_pts.push(screen_pts[0]);
                        }
                        painter.add(egui::epaint::PathShape::line(screen_pts, stroke));
                    }
                }
                Shape::Ellipse {
                    center,
                    major_axis,
                    minor_ratio,
                    start_param,
                    end_param,
                } => {
                    let needs_update = match session.cache.curves.get(&ent.id) {
                        Some((cached_tol, _)) => world_tolerance < *cached_tol * 0.5,
                        None => true,
                    };
                    if needs_update {
                        let pts = tessellate_ellipse(
                            *center,
                            *major_axis,
                            *minor_ratio,
                            *start_param,
                            *end_param,
                            world_tolerance,
                        );
                        let world_pts: Vec<Vec<(f64, f64)>> =
                            vec![pts.iter().map(|p| (p.x, p.y)).collect()];
                        session
                            .cache
                            .curves
                            .insert(ent.id, (world_tolerance, world_pts));
                    }
                    let (_, contours) = session.cache.curves.get(&ent.id).unwrap();
                    for contour in contours {
                        if contour.len() < 2 {
                            continue;
                        }
                        let screen_pts: Vec<Pos2> =
                            contour.iter().map(|&(x, y)| to_screen(x, y)).collect();
                        painter.add(egui::epaint::PathShape::line(screen_pts, stroke));
                    }
                }
                Shape::Spline {
                    degree,
                    knots,
                    control_points,
                    closed,
                } => {
                    let needs_update = match session.cache.curves.get(&ent.id) {
                        Some((cached_tol, _)) => world_tolerance < *cached_tol * 0.5,
                        None => true,
                    };
                    if needs_update {
                        let pts =
                            tessellate_spline(*degree, knots, control_points, world_tolerance);
                        let world_pts: Vec<Vec<(f64, f64)>> =
                            vec![pts.iter().map(|p| (p.x, p.y)).collect()];
                        session
                            .cache
                            .curves
                            .insert(ent.id, (world_tolerance, world_pts));
                    }
                    let (_, contours) = session.cache.curves.get(&ent.id).unwrap();
                    for contour in contours {
                        if contour.len() < 2 {
                            continue;
                        }
                        let mut screen_pts: Vec<Pos2> =
                            contour.iter().map(|&(x, y)| to_screen(x, y)).collect();
                        if *closed && screen_pts.len() >= 2 {
                            screen_pts.push(screen_pts[0]);
                        }
                        painter.add(egui::epaint::PathShape::line(screen_pts, stroke));
                    }
                }
                Shape::LwPolyline { vertices, closed } => {
                    let needs_update = match session.cache.curves.get(&ent.id) {
                        Some((cached_tol, _)) => world_tolerance < *cached_tol * 0.5,
                        None => true,
                    };
                    if needs_update {
                        let pts = tessellate_lwpolyline(vertices, *closed, world_tolerance);
                        let world_pts: Vec<Vec<(f64, f64)>> =
                            vec![pts.iter().map(|p| (p.x, p.y)).collect()];
                        session
                            .cache
                            .curves
                            .insert(ent.id, (world_tolerance, world_pts));
                    }
                    let (_, contours) = session.cache.curves.get(&ent.id).unwrap();
                    for contour in contours {
                        if contour.len() < 2 {
                            continue;
                        }
                        let mut screen_pts: Vec<Pos2> =
                            contour.iter().map(|&(x, y)| to_screen(x, y)).collect();
                        if *closed && screen_pts.len() >= 2 {
                            screen_pts.push(screen_pts[0]);
                        }
                        painter.add(egui::epaint::PathShape::line(screen_pts, stroke));
                    }
                }
                Shape::Text { .. } | Shape::MText { .. } | Shape::BlockInsert { .. } => {}
                Shape::SolidFill { .. } => {}
            }
        }

        painter.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{}/{} entities | zoom {:.2}x | {}",
                rendered,
                doc.entities.len(),
                cam_zoom,
                self.session_id
            ),
            egui::FontId::proportional(14.0),
            Color32::from_rgb(180, 180, 180),
        );

        if needs_repaint {
            ui.ctx().request_repaint();
        }
    }
}

// ── Tests (native target) ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Fresh registry for each test (don't touch the global static).
    fn fresh_registry() -> SessionRegistry {
        SessionRegistry::new()
    }

    // Helper: create a session in a registry, returns the client_id assigned.
    fn create_session(reg: &mut SessionRegistry, id: &str) -> u64 {
        let client_id = reg.next_client_id;
        reg.next_client_id += 1;
        reg.sessions
            .insert(id.to_string(), DocumentSession::new(client_id));
        if reg.js_target.is_none() {
            reg.js_target = Some(id.to_string());
        }
        client_id
    }

    #[test]
    fn create_and_list_sessions() {
        let mut reg = fresh_registry();
        assert!(reg.sessions.is_empty());

        create_session(&mut reg, "doc-a");
        create_session(&mut reg, "doc-b");

        assert_eq!(reg.sessions.len(), 2);
        assert!(reg.sessions.contains_key("doc-a"));
        assert!(reg.sessions.contains_key("doc-b"));
    }

    #[test]
    fn first_session_becomes_js_target() {
        let mut reg = fresh_registry();
        assert!(reg.js_target.is_none());

        create_session(&mut reg, "first");
        assert_eq!(reg.js_target.as_deref(), Some("first"));

        // Second session doesn't change the target
        create_session(&mut reg, "second");
        assert_eq!(reg.js_target.as_deref(), Some("first"));
    }

    #[test]
    fn destroy_clears_js_target() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "x");
        assert_eq!(reg.js_target.as_deref(), Some("x"));

        reg.sessions.remove("x");
        // Simulate session_destroy: clear js_target if it was the destroyed one
        if reg.js_target.as_deref() == Some("x") {
            reg.js_target = None;
        }
        assert!(reg.js_target.is_none());
    }

    #[test]
    fn destroy_cleans_up_renderers() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        reg.renderers.insert("canvas1".to_string(), "a".to_string());
        reg.renderers.insert("canvas2".to_string(), "a".to_string());

        reg.sessions.remove("a");
        reg.renderers.retain(|_, sid| sid != "a");

        assert!(reg.renderers.is_empty());
    }

    #[test]
    fn session_use_switches_target() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");
        assert_eq!(reg.js_target.as_deref(), Some("a"));

        reg.js_target = Some("b".to_string());
        assert_eq!(reg.js_target.as_deref(), Some("b"));
    }

    #[test]
    fn unique_client_ids() {
        let mut reg = fresh_registry();
        let id1 = create_session(&mut reg, "s1");
        let id2 = create_session(&mut reg, "s2");
        let id3 = create_session(&mut reg, "s3");
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn sessions_are_isolated() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");

        // Add entity to session "a"
        let sa = reg.sessions.get_mut("a").unwrap();
        cadview_core::cad_call(&mut sa.doc, "addLine", r#"{"start":[0,0],"end":[10,0]}"#).unwrap();
        assert_eq!(sa.doc.entities.len(), 1);

        // Session "b" should still be empty
        let sb = reg.sessions.get("b").unwrap();
        assert_eq!(sb.doc.entities.len(), 0);
    }

    #[test]
    fn cad_call_targets_js_target() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");

        // js_target is "a" (first created)
        let target = reg.js_target.as_ref().unwrap().clone();
        assert_eq!(target, "a");

        let s = reg.sessions.get_mut(&target).unwrap();
        cadview_core::cad_call(&mut s.doc, "addCircle", r#"{"center":[5,5],"radius":3}"#).unwrap();

        assert_eq!(reg.sessions["a"].doc.entities.len(), 1);
        assert_eq!(reg.sessions["b"].doc.entities.len(), 0);

        // Switch target to "b" and add there
        reg.js_target = Some("b".to_string());
        let target = reg.js_target.as_ref().unwrap().clone();
        let s = reg.sessions.get_mut(&target).unwrap();
        cadview_core::cad_call(&mut s.doc, "addLine", r#"{"start":[0,0],"end":[1,1]}"#).unwrap();

        assert_eq!(reg.sessions["a"].doc.entities.len(), 1);
        assert_eq!(reg.sessions["b"].doc.entities.len(), 1);
    }

    #[test]
    fn renderer_registration() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "doc1");
        create_session(&mut reg, "doc2");

        reg.renderers
            .insert("canvas-left".to_string(), "doc1".to_string());
        reg.renderers
            .insert("canvas-right".to_string(), "doc2".to_string());

        assert_eq!(reg.renderers.len(), 2);
        assert_eq!(reg.renderers["canvas-left"], "doc1");
        assert_eq!(reg.renderers["canvas-right"], "doc2");

        // Stop one renderer
        reg.renderers.remove("canvas-left");
        assert_eq!(reg.renderers.len(), 1);
    }

    #[test]
    fn sync_doc_per_session() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");

        // Mutate session "a" via sync
        let sa = reg.sessions.get_mut("a").unwrap();
        let (_, update) = sa
            .sync
            .apply_mutation(&mut sa.doc, "addLine", r#"{"start":[0,0],"end":[50,0]}"#)
            .unwrap();
        assert!(!update.is_empty());
        assert_eq!(sa.doc.entities.len(), 1);

        // Session "b"'s sync should be independent
        let sb = reg.sessions.get("b").unwrap();
        let sv = sb.sync.state_vector();
        assert!(!sv.is_empty());
        assert_eq!(sb.doc.entities.len(), 0);
    }

    #[test]
    fn camera_per_session() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");

        reg.sessions.get_mut("a").unwrap().camera =
            Camera::fit((0.0, 0.0, 100.0, 100.0), 800.0, 600.0);
        reg.sessions.get_mut("b").unwrap().camera =
            Camera::fit((0.0, 0.0, 1000.0, 1000.0), 800.0, 600.0);

        let zoom_a = reg.sessions["a"].camera.zoom;
        let zoom_b = reg.sessions["b"].camera.zoom;
        // Zoom should differ: "a" is 10x smaller world extent -> 10x more zoom
        assert!(
            zoom_a > zoom_b * 5.0,
            "zoom_a={zoom_a} should be much larger than zoom_b={zoom_b}"
        );
    }

    #[test]
    fn hidden_layers_per_session() {
        let mut reg = fresh_registry();
        create_session(&mut reg, "a");
        create_session(&mut reg, "b");

        reg.sessions
            .get_mut("a")
            .unwrap()
            .hidden_layers
            .push("WALLS".to_string());

        assert_eq!(reg.sessions["a"].hidden_layers, vec!["WALLS"]);
        assert!(reg.sessions["b"].hidden_layers.is_empty());
    }
}
