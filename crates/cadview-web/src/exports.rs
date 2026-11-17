//! wasm-bindgen exports: session management, Yrs sync, cad_call, renderer
//! lifecycle and binary export. Thin wrappers over the session registry.

use crate::egui_render::{start_egui_renderer, RenderCache};
use crate::session::{DocumentSession, SESSIONS};
use cadview_core::render::{Camera, CameraCmd};
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
static NEXT_RENDERER_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
                    let cam = Camera::fit((x0, y0, x1, y1), sw as f64, sh as f64);
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
                        let cam = Camera::fit((bx0, by0, bx1, by1), sw as f64, sh as f64);
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
        crate::vello_render::start(canvas, session_id, &key);
        return format!(
            r#"{{"ok":true,"key":"{}","session":"{}","renderer":"vello"}}"#,
            key, session_id
        );
    }

    let _ = renderer_type; // suppress unused warning when vello feature is off
    start_egui_renderer(canvas, session_id, &key)
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
pub fn export_pdf_bytes(session_id: &str, opts_json: &str) -> Result<Vec<u8>, JsValue> {
    let args: serde_json::Value =
        serde_json::from_str(opts_json).map_err(|e| JsValue::from_str(&format!("savePdf: {e}")))?;
    let opts =
        cadview_core::pdf::PdfOptions::from_args(&args).map_err(|e| JsValue::from_str(&e))?;
    let reg = SESSIONS.lock().unwrap();
    let session = match reg.sessions.get(session_id) {
        Some(s) => s,
        None => return Ok(Vec::new()),
    };
    Ok(cadview_core::export_pdf_bytes(&session.doc, &opts))
}
