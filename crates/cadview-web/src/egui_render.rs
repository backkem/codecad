//! egui fallback renderer: CPU tessellation through eframe's Painter, with
//! its own per-session tessellation cache.

#[cfg(target_arch = "wasm32")]
use crate::session::RepaintFn;
use crate::session::{RendererKey, SessionId, SESSIONS};
use cadview_core::render::{Camera, CameraCmd};
use cadview_core::{
    flatten_bezpath_adaptive, tessellate_ellipse, tessellate_lwpolyline, tessellate_spline,
    triangulate_fill, DrawEntity, EntityId, Shape,
};
use eframe::egui;
use egui::{Color32, Pos2, Stroke};
use std::collections::HashMap;

type CachedFill = (f64, Vec<[f32; 2]>, Vec<u32>);
type CachedCurve = (f64, Vec<Vec<(f64, f64)>>);

/// Render cache for tessellated geometry. Invalidated on document or zoom changes.
pub(crate) struct RenderCache {
    fills: HashMap<EntityId, CachedFill>,
    arcs: HashMap<EntityId, (usize, Vec<(f64, f64)>)>,
    curves: HashMap<EntityId, CachedCurve>,
    expanded: Vec<DrawEntity>,
    entity_count: usize,
}

impl RenderCache {
    pub(crate) fn new() -> Self {
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

impl CadViewApp {
    fn to_color32(c: &cadview_core::Color) -> Color32 {
        Color32::from_rgb(c.r, c.g, c.b)
    }

    /// Resolve an entity's optional color to a concrete Color32,
    /// falling back to the entity's layer color from the document.
    fn resolve_ent_color32(
        ent: &cadview_core::DrawEntity,
        doc: &cadview_core::Document,
    ) -> Color32 {
        let c = ent.color.unwrap_or_else(|| doc.layer_color(&ent.layer));
        Self::to_color32(&c)
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
                        session.camera = Camera::fit(bounds, sw as f64, sh as f64);
                    }
                }
                CameraCmd::FitBounds(x0, y0, x1, y1) => {
                    session.camera = Camera::fit((x0, y0, x1, y1), sw as f64, sh as f64);
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
                session.camera = Camera::fit(bounds, sw as f64, sh as f64);
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

            let base_color = Self::resolve_ent_color32(ent, doc);
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

            let base_color = Self::resolve_ent_color32(ent, doc);
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
