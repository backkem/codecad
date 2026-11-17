//! cadview-web: browser shell. Session registry (`session`), wasm-bindgen
//! exports (`exports`), the Vello renderer (`vello_render`) and the egui
//! fallback renderer (`egui_render`). All CAD and drawing decisions live in
//! cadview-core; the Vello scene comes from cadview-render.

mod egui_render;
#[cfg(target_arch = "wasm32")]
mod exports;
mod session;
#[cfg(all(target_arch = "wasm32", feature = "vello-renderer"))]
mod vello_render;

#[cfg(not(target_arch = "wasm32"))]
use eframe::egui;
#[allow(unused_imports)]
use session::{DocumentSession, SESSIONS};

// ── Entry points ──────────────────────────────────────────────────────

#[cfg(feature = "embed-dwg")]
const DWG_BYTES: &[u8] = include_bytes!(concat!(env!("CODECAD_SAMPLE_DWG")));

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use egui_render::CadViewApp;

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
