//! egui fallback renderer: draws cadview-core's display list through
//! eframe's Painter. egui only draws polylines and triangles, so each path
//! is flattened (fills are triangulated) here and cached per path.

#[cfg(target_arch = "wasm32")]
use crate::session::RepaintFn;
use crate::session::{RendererKey, SessionId, SESSIONS};
use cadview_core::render::{display_list, CameraCmd, Paint, PathKey, SceneCache, View};
use cadview_core::{flatten_bezpath_adaptive, triangulate_path};
use eframe::egui;
use egui::{Color32, Pos2, Stroke};
use kurbo::{BezPath, Point};
use std::collections::HashMap;

/// Largest distance, in screen points, between a curve and its polyline.
const MAX_ERROR_PX: f64 = 0.5;

/// One display-list path flattened to what egui can draw, in world
/// coordinates.
enum Flat {
    /// The polylines of a stroke, already cut into dashes when `dashed`.
    Lines {
        dashed: bool,
        contours: Vec<Vec<Point>>,
    },
    /// The triangles of a fill.
    Mesh {
        vertices: Vec<[f32; 2]>,
        indices: Vec<u32>,
    },
}

/// Per-session render cache: core's scene cache plus the flattened form of
/// each path.
pub(crate) struct RenderCache {
    scene: SceneCache,
    /// Flattened paths with the world tolerance they were made at.
    flat: HashMap<PathKey, (f64, Flat)>,
    /// The scene generation `flat` belongs to.
    generation: u64,
    /// The session's `doc_rev` the scene was last brought up to date with.
    doc_rev: u64,
}

impl RenderCache {
    pub(crate) fn new() -> Self {
        Self {
            scene: SceneCache::new(),
            flat: HashMap::new(),
            generation: 0,
            doc_rev: 0,
        }
    }
}

/// Flatten `path` for `paint` at `tolerance` (world units).
fn flatten(path: &BezPath, paint: &Paint<'_>, tolerance: f64) -> Flat {
    match paint {
        Paint::Fill { .. } => {
            let (vertices, indices) = triangulate_path(path, tolerance);
            Flat::Mesh { vertices, indices }
        }
        Paint::Stroke {
            dashes: Some(pattern),
            ..
        } => {
            let dashed: BezPath = kurbo::dash(path.iter(), 0.0, pattern).collect();
            Flat::Lines {
                dashed: true,
                contours: flatten_bezpath_adaptive(&dashed, tolerance),
            }
        }
        Paint::Stroke { dashes: None, .. } => Flat::Lines {
            dashed: false,
            contours: flatten_bezpath_adaptive(path, tolerance),
        },
    }
}

/// Whether a cached entry made at `cached_tolerance` still serves `paint`
/// at `tolerance`: not too coarse, and dashed exactly when the paint is.
fn still_fits(cached_tolerance: f64, flat: &Flat, paint: &Paint<'_>, tolerance: f64) -> bool {
    if tolerance < cached_tolerance * 0.5 {
        return false;
    }
    match (flat, paint) {
        (Flat::Mesh { .. }, Paint::Fill { .. }) => true,
        (Flat::Lines { dashed, .. }, Paint::Stroke { dashes, .. }) => *dashed == dashes.is_some(),
        _ => false,
    }
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn start_egui_renderer(
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

// ── Renderer ──────────────────────────────────────────────────────────

pub(crate) struct CadViewApp {
    pub(crate) session_id: SessionId,
    pub(crate) renderer_key: RendererKey,
}

impl eframe::App for CadViewApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let sw = rect.width();
        let sh = rect.height();

        // Check if this renderer is still registered
        let mut reg = SESSIONS.lock().unwrap();
        let still_active = reg.renderers.get(&self.renderer_key) == Some(&self.session_id);
        if !still_active {
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_rgb(30, 30, 30));
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
            session
                .camera
                .apply(cmd, &session.doc, sw as f64, sh as f64);
            session.initialized = true;
        } else if auto_fit {
            session
                .camera
                .apply(CameraCmd::FitAll, &session.doc, sw as f64, sh as f64);
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

        // What to draw is decided by cadview-core. Sizes are in egui
        // points, so strokes come out one point wide.
        let camera = session.camera;
        let mut view = View::new(sw.round() as u32, sh.round() as u32, camera);
        view.hidden_layers = &session.hidden_layers;

        let cache = &mut session.cache;
        // Entities edited in place keep the count, so the cache cannot tell.
        if cache.doc_rev != session.doc_rev {
            cache.scene.invalidate();
            cache.doc_rev = session.doc_rev;
        }
        cache.scene.update(&session.doc);
        if cache.generation != cache.scene.generation() {
            cache.flat.clear();
            cache.generation = cache.scene.generation();
        }
        let list = display_list(&session.doc, &cache.scene, &view);

        let painter = ui.painter_at(rect);
        let bg = list.background;
        painter.rect_filled(rect, 0.0, Color32::from_rgb(bg.r, bg.g, bg.b));

        // The exact (unrounded) panel size, so the picture stays put under
        // the cursor maths above.
        let to_screen = camera.transform(sw as f64, sh as f64);
        let screen = |p: Point| -> Pos2 {
            let p = to_screen * p;
            Pos2::new(p.x as f32, p.y as f32)
        };
        let tolerance = MAX_ERROR_PX / camera.zoom;

        for item in &list.items {
            let fits = cache
                .flat
                .get(&item.key)
                .is_some_and(|(tol, flat)| still_fits(*tol, flat, &item.paint, tolerance));
            if !fits {
                let flat = flatten(item.path, &item.paint, tolerance);
                cache.flat.insert(item.key, (tolerance, flat));
            }
            let c = item.color;
            let color = Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a);

            match (&cache.flat[&item.key].1, &item.paint) {
                (Flat::Mesh { vertices, indices }, _) => {
                    if indices.is_empty() {
                        continue;
                    }
                    let mut mesh = egui::Mesh::default();
                    mesh.vertices
                        .extend(vertices.iter().map(|&[x, y]| egui::epaint::Vertex {
                            pos: screen(Point::new(x as f64, y as f64)),
                            uv: Pos2::ZERO,
                            color,
                        }));
                    mesh.indices.clone_from(indices);
                    painter.add(egui::Shape::mesh(mesh));
                }
                (Flat::Lines { contours, .. }, Paint::Stroke { width, .. }) => {
                    let stroke = Stroke::new((width * camera.zoom) as f32, color);
                    for contour in contours {
                        match contour.as_slice() {
                            [] | [_] => {}
                            [a, b] => {
                                painter.line_segment([screen(*a), screen(*b)], stroke);
                            }
                            points => {
                                let points = points.iter().map(|p| screen(*p)).collect();
                                painter.add(egui::epaint::PathShape::line(points, stroke));
                            }
                        }
                    }
                }
                (Flat::Lines { .. }, Paint::Fill { .. }) => {}
            }
        }

        painter.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!(
                "{}/{} entities | zoom {:.2}x | {}",
                list.items.len(),
                session.doc.entities.len(),
                camera.zoom,
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
