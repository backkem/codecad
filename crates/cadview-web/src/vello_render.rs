//! Vello GPU renderer for cadview-web.
//!
//! Renders the Document model using Vello's compute-based pipeline.
//! Requires WebGPU; the JS host detects capability and falls back to
//! the egui renderer on browsers without WebGPU support.

use crate::{Camera, CameraCmd, SESSIONS};
use cadview_core::{DrawEntity, EntityId, Shape};
use kurbo::{Affine, BezPath};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use vello::peniko;
use vello::Scene;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

/// The rAF callback, held in a cell so it can schedule itself.
type FrameCb = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

/// Per-renderer GPU state.
struct VelloState {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    renderer: vello::Renderer,
    /// Intermediate Rgba8Unorm texture for Vello's compute output.
    /// Required because Vello writes via storage binding (needs Rgba8Unorm)
    /// but surface textures on Windows are Bgra8Unorm.
    target_texture: wgpu::Texture,
    target_view: wgpu::TextureView,
    blitter: wgpu::util::TextureBlitter,
    session_id: String,
    renderer_key: String,
    canvas: HtmlCanvasElement,
    /// Block/text expansion cache. Cleared on entity count change.
    expanded: Vec<DrawEntity>,
    expanded_count: usize,
    /// BezPath cache per entity ID. Avoids re-converting every frame.
    path_cache: HashMap<EntityId, Option<BezPath>>,
}

impl VelloState {
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface.configure(&self.device, &self.surface_config);
        let (tex, view) = create_target_texture(&self.device, width, height);
        self.target_texture = tex;
        self.target_view = view;
    }
}

fn create_target_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vello_target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        format: wgpu::TextureFormat::Rgba8Unorm,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Start a Vello renderer on the given canvas for the given session.
pub fn start(canvas: HtmlCanvasElement, session_id: &str, renderer_key: &str) {
    let session_id = session_id.to_string();
    let renderer_key = renderer_key.to_string();

    wasm_bindgen_futures::spawn_local(async move {
        let (
            device,
            queue,
            surface,
            surface_config,
            renderer,
            target_texture,
            target_view,
            blitter,
        ) = match init_gpu(&canvas).await {
            Ok(s) => s,
            Err(e) => {
                web_sys::console::error_1(&format!("Vello init failed: {e}").into());
                return;
            }
        };

        let state = Rc::new(RefCell::new(VelloState {
            device,
            queue,
            surface,
            surface_config,
            renderer,
            target_texture,
            target_view,
            blitter,
            session_id,
            renderer_key,
            canvas: canvas.clone(),
            expanded: Vec::new(),
            expanded_count: 0,
            path_cache: HashMap::new(),
        }));

        // Input handlers
        let input = Rc::new(RefCell::new(InputState::new()));

        // Demand-driven rendering: only run rAF when something changed.
        // `frame_pending` tracks whether a rAF is already scheduled.
        let frame_pending = Rc::new(Cell::new(false));
        let cb: FrameCb = Rc::new(RefCell::new(None));

        // schedule_frame: request a single rAF if one isn't already queued.
        // Called by input handlers when they have new data.
        let schedule_frame: Rc<dyn Fn()> = {
            let cb = Rc::clone(&cb);
            let pending = Rc::clone(&frame_pending);
            Rc::new(move || {
                if pending.get() {
                    return;
                }
                pending.set(true);
                let borrowed = cb.borrow();
                if let Some(ref f) = *borrowed {
                    let _ = web_sys::window()
                        .unwrap()
                        .request_animation_frame(f.as_ref().unchecked_ref());
                }
            })
        };

        attach_input_listeners(&canvas, Rc::clone(&input), Rc::clone(&schedule_frame));

        // Register repaint callback and renderer mapping so that
        // client-side commands (zoomTo, setLayerVisible, Yrs sync) can wake us up.
        {
            let sched = Rc::clone(&schedule_frame);
            let rk = state.borrow().renderer_key.clone();
            let sid = state.borrow().session_id.clone();
            let mut reg = SESSIONS.lock().unwrap();
            reg.renderers.insert(rk.clone(), sid);
            reg.repaint_fns.insert(
                rk,
                crate::RepaintFn(Box::new(move || {
                    sched();
                })),
            );
        }

        // Re-render on canvas resize (browser window resize, layout change)
        {
            let sched = Rc::clone(&schedule_frame);
            let observer_cb =
                Closure::<dyn FnMut(js_sys::Array)>::new(move |_entries: js_sys::Array| {
                    sched();
                });
            let observer =
                web_sys::ResizeObserver::new(observer_cb.as_ref().unchecked_ref()).unwrap();
            observer.observe(&canvas);
            observer_cb.forget();
            // Prevent GC of the observer by leaking the reference.
            // It lives for the lifetime of the canvas, which is fine.
            std::mem::forget(observer);
        }

        let state_clone = Rc::clone(&state);
        let input_clone = Rc::clone(&input);
        let pending_clone = Rc::clone(&frame_pending);
        let schedule_clone = Rc::clone(&schedule_frame);

        *cb.borrow_mut() = Some(Closure::new(move || {
            pending_clone.set(false);

            let still_active = {
                let reg = SESSIONS.lock().unwrap();
                let s = state_clone.borrow();
                reg.renderers
                    .get(&s.renderer_key)
                    .is_some_and(|sid| sid == &s.session_id)
            };
            if !still_active {
                return;
            }

            // Process input
            let has_input = {
                let delta = input_clone.borrow_mut().take();
                let has = delta.pan_dx.abs() > 0.1
                    || delta.pan_dy.abs() > 0.1
                    || (delta.zoom_factor - 1.0).abs() > 1e-6;
                let mut reg = SESSIONS.lock().unwrap();
                let s = state_clone.borrow();
                if let Some(session) = reg.sessions.get_mut(&s.session_id) {
                    apply_input_to_camera(
                        &mut session.camera,
                        &delta,
                        s.surface_config.width,
                        s.surface_config.height,
                    );
                }
                has
            };

            // Handle resize
            let resized = {
                let s = state_clone.borrow();
                let c = &s.canvas;
                let dpr = web_sys::window().unwrap().device_pixel_ratio();
                let w = (c.client_width() as f64 * dpr) as u32;
                let h = (c.client_height() as f64 * dpr) as u32;
                c.set_width(w);
                c.set_height(h);
                drop(s);
                let mut s = state_clone.borrow_mut();
                if w != s.surface_config.width || h != s.surface_config.height {
                    s.resize(w, h);
                    true
                } else {
                    false
                }
            };

            // Check for camera commands, entity changes, or dirty flag
            let has_update = {
                let mut reg = SESSIONS.lock().unwrap();
                let s = state_clone.borrow();
                if let Some(session) = reg.sessions.get_mut(&s.session_id) {
                    let dirty = session.render_dirty;
                    session.render_dirty = false;
                    session.camera_cmd.is_some()
                        || session.doc.entities.len() != s.expanded_count
                        || dirty
                } else {
                    false
                }
            };

            render_frame(&mut state_clone.borrow_mut());

            // If there's ongoing interaction, keep the loop alive for
            // smooth continuous pan/zoom (pointer might still be held).
            if has_input || resized || has_update {
                schedule_clone();
            }
        }));

        // Kick off first frame
        schedule_frame();
    });
}

// ── GPU init ──────────────────────────────────────────────────────────

async fn init_gpu(
    canvas: &HtmlCanvasElement,
) -> Result<
    (
        wgpu::Device,
        wgpu::Queue,
        wgpu::Surface<'static>,
        wgpu::SurfaceConfiguration,
        vello::Renderer,
        wgpu::Texture,
        wgpu::TextureView,
        wgpu::util::TextureBlitter,
    ),
    String,
> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(desc);

    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| format!("create_surface: {e}"))?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("request_adapter: {e}"))?;

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("cadview-vello"),
            required_features: adapter.features(),
            required_limits: adapter.limits(),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("request_device: {e}"))?;

    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let width = ((canvas.client_width() as f64 * dpr) as u32).max(1);
    let height = ((canvas.client_height() as f64 * dpr) as u32).max(1);

    let caps = surface.get_capabilities(&adapter);
    let format = caps.formats.first().copied().ok_or("no surface format")?;

    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
    };
    surface.configure(&device, &surface_config);

    let renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport::all(),
            num_init_threads: std::num::NonZeroUsize::new(1),
            pipeline_cache: None,
        },
    )
    .map_err(|e| format!("vello Renderer: {e}"))?;

    let (target_texture, target_view) = create_target_texture(&device, width, height);
    let blitter = wgpu::util::TextureBlitterBuilder::new(&device, format).build();

    Ok((
        device,
        queue,
        surface,
        surface_config,
        renderer,
        target_texture,
        target_view,
        blitter,
    ))
}

// ── Render ────────────────────────────────────────────────────────────

fn render_frame(state: &mut VelloState) {
    let width = state.surface_config.width;
    let height = state.surface_config.height;
    if width == 0 || height == 0 {
        return;
    }

    let mut scene = Scene::new();

    // Background
    scene.fill(
        peniko::Fill::NonZero,
        Affine::IDENTITY,
        peniko::Color::from_rgb8(30, 30, 30),
        None,
        &kurbo::Rect::new(0.0, 0.0, width as f64, height as f64),
    );

    // Lock session
    let mut reg = SESSIONS.lock().unwrap();
    let Some(session) = reg.sessions.get_mut(&state.session_id) else {
        return;
    };

    let sw = width as f64;
    let sh = height as f64;
    session.screen_size = (width as f32, height as f32);

    // Camera commands / auto-fit
    let entity_count = session.doc.entities.len();
    let camera_cmd = session.camera_cmd.take();
    let auto_fit = !session.initialized || (session.last_entity_count == 0 && entity_count > 0);

    if let Some(cmd) = camera_cmd {
        match cmd {
            CameraCmd::FitAll => {
                if let Some(bounds) = session.doc.bounds() {
                    session.camera = Camera::fit(bounds, width as f32, height as f32);
                }
            }
            CameraCmd::FitBounds(x0, y0, x1, y1) => {
                session.camera = Camera::fit((x0, y0, x1, y1), width as f32, height as f32);
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
            session.camera = Camera::fit(bounds, width as f32, height as f32);
        }
        session.initialized = true;
    }
    session.last_entity_count = entity_count;

    let cam_cx = session.camera.center_x;
    let cam_cy = session.camera.center_y;
    let cam_zoom = session.camera.zoom;

    // World-to-screen: center on screen, zoom, flip Y
    let transform = Affine::translate((sw / 2.0, sh / 2.0))
        * Affine::scale_non_uniform(cam_zoom, -cam_zoom)
        * Affine::translate((-cam_cx, -cam_cy));

    let half_w = (sw / 2.0) / cam_zoom;
    let half_h = (sh / 2.0) / cam_zoom;
    let view_x0 = cam_cx - half_w;
    let view_x1 = cam_cx + half_w;
    let view_y0 = cam_cy - half_h;
    let view_y1 = cam_cy + half_h;

    const CULL_THRESHOLD: f64 = 0.5;
    const FADE_THRESHOLD: f64 = 8.0;

    // Block/text expansion
    if state.expanded_count != entity_count {
        state.expanded.clear();
        state.expanded_count = entity_count;
        state.path_cache.clear();
    }
    if state.expanded.is_empty()
        && session.doc.entities.iter().any(|e| {
            matches!(
                &e.shape,
                Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. }
            )
        })
    {
        expand_blocks_and_text(&session.doc, &mut state.expanded);
    }

    // All renderable entities
    let all_entities: Vec<&DrawEntity> = session
        .doc
        .entities
        .iter()
        .filter(|e| {
            !matches!(
                &e.shape,
                Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. }
            )
        })
        .chain(state.expanded.iter())
        .collect();

    // Ensure path cache is populated for visible entities
    for ent in &all_entities {
        state
            .path_cache
            .entry(ent.id)
            .or_insert_with(|| ent.shape.to_bezpath());
    }

    // Pass 1: SolidFill
    for ent in &all_entities {
        if !matches!(&ent.shape, Shape::SolidFill { .. }) {
            continue;
        }
        if session.hidden_layers.iter().any(|n| n == &ent.layer) {
            continue;
        }
        let (bx0, by0, bx1, by1) = ent.shape.bbox();
        if bx1 < view_x0 || bx0 > view_x1 || by1 < view_y0 || by0 > view_y1 {
            continue;
        }

        if let Some(Some(bezpath)) = state.path_cache.get(&ent.id) {
            let c = session.doc.resolve_color(ent);
            let fill_color = peniko::Color::from_rgba8(c.r, c.g, c.b, 35);
            scene.fill(peniko::Fill::EvenOdd, transform, fill_color, None, bezpath);
        }
    }

    // Pass 2: Strokes
    // 1 CSS pixel = dpr device pixels. Without this, strokes are
    // 1 device pixel (0.5 CSS px on 2x displays), too thin for clean AA.
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let stroke_width = dpr / cam_zoom;
    let solid_stroke = kurbo::Stroke::new(stroke_width);

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

        let c = session.doc.resolve_color(ent);
        let base_alpha = if ent.transparency > 0 {
            ((255 - ent.transparency) as f32 / 255.0 * c.a as f32) as u8
        } else {
            c.a
        };
        let alpha = if diag_screen < FADE_THRESHOLD {
            let t = ((diag_screen - CULL_THRESHOLD) / (FADE_THRESHOLD - CULL_THRESHOLD))
                .clamp(0.0, 1.0) as f32;
            let smoothstep = t * t * (3.0 - 2.0 * t);
            (smoothstep * base_alpha as f32) as u8
        } else {
            base_alpha
        };

        let color = peniko::Color::from_rgba8(c.r, c.g, c.b, alpha);

        if let Some(Some(bezpath)) = state.path_cache.get(&ent.id) {
            let stroke = if let Some(pattern) = session.doc.resolve_linetype_pattern(ent) {
                solid_stroke
                    .clone()
                    .with_dashes(0.0, pattern.iter().copied())
            } else {
                solid_stroke.clone()
            };
            scene.stroke(&stroke, transform, color, None, bezpath);
        }
    }

    drop(reg);

    // Render to intermediate Rgba8 texture, then blit to surface
    let params = vello::RenderParams {
        base_color: peniko::Color::BLACK,
        width,
        height,
        antialiasing_method: vello::AaConfig::Msaa16,
    };

    state
        .renderer
        .render_to_texture(
            &state.device,
            &state.queue,
            &scene,
            &state.target_view,
            &params,
        )
        .expect("vello render failed");

    let surface_texture = match state.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
        _ => return,
    };
    let surface_view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vello_blit"),
        });
    state.blitter.copy(
        &state.device,
        &mut encoder,
        &state.target_view,
        &surface_view,
    );
    state.queue.submit(Some(encoder.finish()));

    surface_texture.present();
}

fn expand_blocks_and_text(doc: &cadview_core::Document, expanded: &mut Vec<DrawEntity>) {
    let result = cadview_core::expand_for_render(doc);
    web_sys::console::log_1(
        &format!("expand_blocks_and_text: {} expanded entities", result.len()).into(),
    );
    expanded.extend(result);
}

// ── Input handling ────────────────────────────────────────────────────

struct InputDelta {
    pan_dx: f64,
    pan_dy: f64,
    zoom_factor: f64,
    zoom_center: Option<(f64, f64)>,
}

struct InputState {
    pan_dx: f64,
    pan_dy: f64,
    zoom_factor: f64,
    zoom_center: Option<(f64, f64)>,
    pointer_down: bool,
    last_x: f64,
    last_y: f64,
}

impl InputState {
    fn new() -> Self {
        Self {
            pan_dx: 0.0,
            pan_dy: 0.0,
            zoom_factor: 1.0,
            zoom_center: None,
            pointer_down: false,
            last_x: 0.0,
            last_y: 0.0,
        }
    }

    fn take(&mut self) -> InputDelta {
        let delta = InputDelta {
            pan_dx: self.pan_dx,
            pan_dy: self.pan_dy,
            zoom_factor: self.zoom_factor,
            zoom_center: self.zoom_center.take(),
        };
        self.pan_dx = 0.0;
        self.pan_dy = 0.0;
        self.zoom_factor = 1.0;
        delta
    }
}

fn attach_input_listeners(
    canvas: &HtmlCanvasElement,
    input: Rc<RefCell<InputState>>,
    schedule: Rc<dyn Fn()>,
) {
    // Pointer down
    {
        let inp = Rc::clone(&input);
        let sched = Rc::clone(&schedule);
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                let mut s = inp.borrow_mut();
                s.pointer_down = true;
                s.last_x = e.offset_x() as f64;
                s.last_y = e.offset_y() as f64;
                sched();
            });
        canvas
            .add_event_listener_with_callback("pointerdown", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Pointer move (pan)
    {
        let inp = Rc::clone(&input);
        let sched = Rc::clone(&schedule);
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
                let mut s = inp.borrow_mut();
                if s.pointer_down {
                    let x = e.offset_x() as f64;
                    let y = e.offset_y() as f64;
                    s.pan_dx += x - s.last_x;
                    s.pan_dy += y - s.last_y;
                    s.last_x = x;
                    s.last_y = y;
                    sched();
                }
            });
        canvas
            .add_event_listener_with_callback("pointermove", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Pointer up
    {
        let inp = Rc::clone(&input);
        let cb =
            Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |_e: web_sys::PointerEvent| {
                inp.borrow_mut().pointer_down = false;
            });
        canvas
            .add_event_listener_with_callback("pointerup", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Wheel (zoom)
    {
        let inp = Rc::clone(&input);
        let sched = Rc::clone(&schedule);
        let cb = Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |e: web_sys::WheelEvent| {
            e.prevent_default();
            let delta_y = e.delta_y();
            let factor = if delta_y < 0.0 { 1.08 } else { 1.0 / 1.08 };
            let mut s = inp.borrow_mut();
            s.zoom_factor *= factor;
            s.zoom_center = Some((e.offset_x() as f64, e.offset_y() as f64));
            sched();
        });
        let opts = web_sys::AddEventListenerOptions::new();
        opts.set_passive(false);
        canvas
            .add_event_listener_with_callback_and_add_event_listener_options(
                "wheel",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }
}

fn apply_input_to_camera(camera: &mut Camera, input: &InputDelta, width: u32, height: u32) {
    let sw = width as f64;
    let sh = height as f64;

    // Pan
    if input.pan_dx.abs() > 0.1 || input.pan_dy.abs() > 0.1 {
        let dpr = web_sys::window().unwrap().device_pixel_ratio();
        camera.center_x -= (input.pan_dx * dpr) / camera.zoom;
        camera.center_y += (input.pan_dy * dpr) / camera.zoom;
    }

    // Zoom toward mouse
    if (input.zoom_factor - 1.0).abs() > 1e-6 {
        if let Some((mx_css, my_css)) = input.zoom_center {
            let dpr = web_sys::window().unwrap().device_pixel_ratio();
            let mx = mx_css * dpr;
            let my = my_css * dpr;
            let world_x = (mx - sw / 2.0) / camera.zoom + camera.center_x;
            let world_y = -(my - sh / 2.0) / camera.zoom + camera.center_y;
            camera.zoom *= input.zoom_factor;
            camera.center_x = world_x - (mx - sw / 2.0) / camera.zoom;
            camera.center_y = world_y + (my - sh / 2.0) / camera.zoom;
        } else {
            camera.zoom *= input.zoom_factor;
        }
    }
}
