//! Session registry: per-document state (document, sync, camera, render
//! caches, hidden layers) and the global registry the exports operate on.

use crate::egui_render::RenderCache;
use cadview_core::render::{Camera, CameraCmd};
use cadview_core::sync::SyncDoc;
use cadview_core::Document;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub(crate) type SessionId = String;
pub(crate) type RendererKey = String;
// CanvasId removed: renderers now receive HtmlCanvasElement directly.

/// Per-document session. Bundles all state for one open drawing.
#[allow(dead_code)]
pub(crate) struct DocumentSession {
    pub(crate) doc: Document,
    pub(crate) sync: SyncDoc,
    pub(crate) sync_sv_before: Option<Vec<u8>>,
    pub(crate) hidden_layers: Vec<String>,
    pub(crate) camera_cmd: Option<CameraCmd>,
    pub(crate) camera: Camera,
    pub(crate) screen_size: (f32, f32),
    pub(crate) initialized: bool,
    pub(crate) last_entity_count: usize,
    pub(crate) cache: RenderCache,
    /// Set by client-side commands (setLayerVisible, etc.) to signal
    /// renderers that a repaint is needed even without entity changes.
    pub(crate) render_dirty: bool,
}

impl DocumentSession {
    pub(crate) fn new(client_id: u64) -> Self {
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
pub(crate) struct RepaintFn(pub(crate) Box<dyn Fn() + 'static>);
unsafe impl Send for RepaintFn {}
#[allow(dead_code)]
impl RepaintFn {
    pub(crate) fn call(&self) {
        (self.0)();
    }
}

/// Global session registry. Single static, replaces all per-document statics.
pub(crate) struct SessionRegistry {
    pub(crate) sessions: HashMap<SessionId, DocumentSession>,
    /// Which session `cad_call` targets (set by `session_use`).
    pub(crate) js_target: Option<SessionId>,
    /// Map of renderer_key -> session_id for active renderers.
    pub(crate) renderers: HashMap<RendererKey, SessionId>,
    /// Repaint callbacks per renderer_key. Renderers register on start.
    #[allow(dead_code)]
    pub(crate) repaint_fns: HashMap<RendererKey, RepaintFn>,
    /// Monotonic counter for Yrs client IDs (each session needs a unique one).
    pub(crate) next_client_id: u64,
}

impl SessionRegistry {
    pub(crate) fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            js_target: None,
            renderers: HashMap::new(),
            repaint_fns: HashMap::new(),
            next_client_id: 100, // start at 100 to avoid conflict with server (1)
        }
    }
}

pub(crate) static SESSIONS: LazyLock<Mutex<SessionRegistry>> =
    LazyLock::new(|| Mutex::new(SessionRegistry::new()));

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
