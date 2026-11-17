//! Renderer-neutral 2D drawing: camera, view, scene cache, display list.
//!
//! Every rendering decision lives here: which entities are drawn, in what
//! order, with what colour, alpha, stroke width and dash pattern, and how
//! world coordinates map to pixels. Backends (the Vello GPU scene builder,
//! the CPU rasterizer, future shells) only turn a [`DisplayList`] into
//! pixels and never look at a [`Shape`].
//!
//! Geometry stays exact: items carry `kurbo::BezPath` references in world
//! coordinates plus one world-to-pixel [`Affine`]. Nothing is flattened here.

use crate::{Color, Document, DrawEntity, EntityId, Shape};
use kurbo::{Affine, BezPath};
use std::collections::HashMap;

/// Entities whose screen-space bbox diagonal is below this (pixels) are culled.
pub const CULL_THRESHOLD_PX: f64 = 0.5;
/// Between the cull threshold and this (pixels), stroke alpha fades in with
/// a smoothstep.
pub const FADE_THRESHOLD_PX: f64 = 8.0;
/// Fixed alpha for solid fills (hatches): fills read as a tint under strokes.
pub const FILL_ALPHA: u8 = 35;
/// Margin factor used by [`Camera::fit`] (1.1 = 10 % around the bounds).
pub const FIT_MARGIN: f64 = 1.1;
/// Default viewport background, rgb(30, 30, 30).
pub const DEFAULT_BACKGROUND: Color = Color {
    r: 30,
    g: 30,
    b: 30,
    a: 255,
};

// ── Camera ─────────────────────────────────────────────────────────────

/// 2D camera: the world point at the centre of the screen and the zoom in
/// pixels per world unit. Y is up in world space, down on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub center_x: f64,
    pub center_y: f64,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self::new()
    }
}

impl Camera {
    pub fn new() -> Self {
        Self {
            center_x: 0.0,
            center_y: 0.0,
            zoom: 1.0,
        }
    }

    /// Camera that shows `bounds` (x0, y0, x1, y1) centred on a screen of
    /// `screen_w` x `screen_h` pixels, with a [`FIT_MARGIN`] margin.
    pub fn fit(bounds: (f64, f64, f64, f64), screen_w: f64, screen_h: f64) -> Self {
        let (x0, y0, x1, y1) = bounds;
        let dw = x1 - x0;
        let dh = y1 - y0;
        let zoom_x = screen_w / (dw * FIT_MARGIN);
        let zoom_y = screen_h / (dh * FIT_MARGIN);
        Camera {
            center_x: (x0 + x1) / 2.0,
            center_y: (y0 + y1) / 2.0,
            zoom: zoom_x.min(zoom_y),
        }
    }

    /// Like [`Camera::fit`], but `None` when the result is unusable (a
    /// zero-size or non-finite bounds giving an infinite or NaN zoom).
    pub fn fit_checked(bounds: (f64, f64, f64, f64), screen_w: f64, screen_h: f64) -> Option<Self> {
        let cam = Self::fit(bounds, screen_w, screen_h);
        let ok = cam.zoom.is_finite()
            && cam.zoom > 0.0
            && cam.center_x.is_finite()
            && cam.center_y.is_finite();
        ok.then_some(cam)
    }

    /// Camera fitted to the whole document, or `None` for an empty one.
    pub fn fit_document(doc: &Document, screen_w: f64, screen_h: f64) -> Option<Self> {
        doc.bounds().map(|b| Self::fit(b, screen_w, screen_h))
    }

    /// Apply a viewport command. `FitAll` leaves the camera alone on an
    /// empty document.
    pub fn apply(&mut self, cmd: CameraCmd, doc: &Document, screen_w: f64, screen_h: f64) {
        match cmd {
            CameraCmd::FitAll => {
                if let Some(cam) = Self::fit_document(doc, screen_w, screen_h) {
                    *self = cam;
                }
            }
            CameraCmd::FitBounds(x0, y0, x1, y1) => {
                *self = Self::fit((x0, y0, x1, y1), screen_w, screen_h);
            }
            CameraCmd::SetView(cx, cy, zoom) => {
                self.center_x = cx;
                self.center_y = cy;
                self.zoom = zoom;
            }
        }
    }

    /// World-to-pixel transform: centre on screen, zoom, flip Y.
    pub fn transform(&self, screen_w: f64, screen_h: f64) -> Affine {
        Affine::translate((screen_w / 2.0, screen_h / 2.0))
            * Affine::scale_non_uniform(self.zoom, -self.zoom)
            * Affine::translate((-self.center_x, -self.center_y))
    }

    /// Visible world rectangle (x0, y0, x1, y1).
    pub fn view_rect(&self, screen_w: f64, screen_h: f64) -> (f64, f64, f64, f64) {
        let half_w = (screen_w / 2.0) / self.zoom;
        let half_h = (screen_h / 2.0) / self.zoom;
        (
            self.center_x - half_w,
            self.center_y - half_h,
            self.center_x + half_w,
            self.center_y + half_h,
        )
    }
}

/// A pending viewport change, applied by the renderer once it knows the
/// screen size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraCmd {
    FitAll,
    FitBounds(f64, f64, f64, f64),
    SetView(f64, f64, f64),
}

// ── View ───────────────────────────────────────────────────────────────

/// What to draw and how: output size, camera, hidden layers, pixel ratio
/// and background.
#[derive(Debug, Clone)]
pub struct View<'a> {
    /// Output size in device pixels.
    pub width: u32,
    pub height: u32,
    pub camera: Camera,
    /// Layers to skip.
    pub hidden_layers: &'a [String],
    /// Device pixels per CSS pixel. Strokes are this many pixels wide.
    pub pixel_ratio: f64,
    pub background: Color,
}

impl<'a> View<'a> {
    /// A view with the default background and pixel ratio 1.
    pub fn new(width: u32, height: u32, camera: Camera) -> Self {
        Self {
            width,
            height,
            camera,
            hidden_layers: &[],
            pixel_ratio: 1.0,
            background: DEFAULT_BACKGROUND,
        }
    }
}

// ── Scene cache ────────────────────────────────────────────────────────

/// Expanded blocks/text and per-entity BezPaths, reused across frames.
///
/// Invalidated when the document's entity count changes (the same rule the
/// browser renderer has always used). Keep one per document per renderer;
/// a one-shot render can use a fresh one.
#[derive(Debug, Default)]
pub struct SceneCache {
    /// Block inserts and text expanded to world-space entities.
    expanded: Vec<DrawEntity>,
    /// Entity count the cache was built for.
    entity_count: usize,
    /// Paths of the document's own entities, by ID.
    direct_paths: HashMap<EntityId, Option<BezPath>>,
    /// Paths of `expanded`, by index (expanded IDs are synthetic and could
    /// collide with real ones, so they get their own table).
    expanded_paths: Vec<Option<BezPath>>,
    /// Bumped whenever the cached paths are thrown away.
    generation: u64,
}

fn is_expandable(shape: &Shape) -> bool {
    matches!(
        shape,
        Shape::BlockInsert { .. } | Shape::Text { .. } | Shape::MText { .. }
    )
}

impl SceneCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Entity count the cache currently reflects.
    pub fn entity_count(&self) -> usize {
        self.entity_count
    }

    /// Number of expanded (block/text) entities.
    pub fn expanded_len(&self) -> usize {
        self.expanded.len()
    }

    /// Counts how often the cached paths have been thrown away. A
    /// [`PathKey`] names the same path for as long as this stays the same,
    /// so backends that keep their own per-path data (flattened points,
    /// triangles) drop it when the generation changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Throw the cached paths away. The entity count catches adds and
    /// removes; call this when entities may have changed in place (moved,
    /// edited, document replaced), which the count cannot see.
    pub fn invalidate(&mut self) {
        self.expanded.clear();
        self.expanded_paths.clear();
        self.direct_paths.clear();
        self.generation += 1;
    }

    /// Bring the cache up to date with `doc`.
    pub fn update(&mut self, doc: &Document) {
        let entity_count = doc.entities.len();
        if self.entity_count != entity_count {
            self.invalidate();
            self.entity_count = entity_count;
        }
        if self.expanded.is_empty() && doc.entities.iter().any(|e| is_expandable(&e.shape)) {
            self.expanded = crate::expand_for_render(doc);
            self.expanded_paths.clear();
        }
        if self.expanded_paths.len() != self.expanded.len() {
            self.expanded_paths = self.expanded.iter().map(|e| e.shape.to_bezpath()).collect();
        }
        for ent in &doc.entities {
            if !is_expandable(&ent.shape) {
                self.direct_paths
                    .entry(ent.id)
                    .or_insert_with(|| ent.shape.to_bezpath());
            }
        }
    }

    /// All renderable entities with their paths: the document's own
    /// entities (minus blocks and text), then the expanded ones.
    fn renderables<'a>(
        &'a self,
        doc: &'a Document,
    ) -> impl Iterator<Item = (PathKey, &'a DrawEntity, Option<&'a BezPath>)> + 'a {
        let direct = doc
            .entities
            .iter()
            .filter(|e| !is_expandable(&e.shape))
            .map(|e| {
                let path = self.direct_paths.get(&e.id).and_then(|p| p.as_ref());
                (PathKey::Entity(e.id), e, path)
            });
        let expanded = self
            .expanded
            .iter()
            .zip(self.expanded_paths.iter())
            .enumerate()
            .map(|(i, (e, p))| (PathKey::Expanded(i), e, p.as_ref()));
        direct.chain(expanded)
    }
}

// ── Display list ───────────────────────────────────────────────────────

/// A renderer-neutral frame: background, one world-to-pixel transform, and
/// items in paint order.
#[derive(Debug, Clone)]
pub struct DisplayList<'a> {
    /// Output size in pixels.
    pub width: u32,
    pub height: u32,
    /// Opaque background colour.
    pub background: Color,
    /// World -> pixel, applies to every item.
    pub transform: Affine,
    /// Paint order.
    pub items: Vec<DrawItem<'a>>,
}

/// Which cached path an item draws: one of the document's own entities, or
/// the n-th entity expanded from a block insert or text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathKey {
    Entity(EntityId),
    Expanded(usize),
}

/// One path to fill or stroke.
#[derive(Debug, Clone)]
pub struct DrawItem<'a> {
    /// Names `path` within one [`SceneCache::generation`].
    pub key: PathKey,
    /// Exact geometry in world coordinates.
    pub path: &'a BezPath,
    /// Straight (non-premultiplied) RGBA8, final alpha already resolved.
    pub color: Color,
    pub paint: Paint<'a>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint<'a> {
    /// `even_odd: false` means non-zero.
    Fill { even_odd: bool },
    /// Width in world units. Style is `kurbo::Stroke::new(width)` defaults;
    /// dashes (world units) start at offset 0.0.
    Stroke {
        width: f64,
        dashes: Option<&'a [f64]>,
    },
}

/// Smoothstep alpha for an entity whose bbox diagonal is `diag_px` pixels.
/// Returns `None` when the entity is culled.
pub fn lod_alpha(diag_px: f64, base_alpha: u8) -> Option<u8> {
    if diag_px < CULL_THRESHOLD_PX {
        return None;
    }
    if diag_px < FADE_THRESHOLD_PX {
        let t = ((diag_px - CULL_THRESHOLD_PX) / (FADE_THRESHOLD_PX - CULL_THRESHOLD_PX))
            .clamp(0.0, 1.0) as f32;
        let smoothstep = t * t * (3.0 - 2.0 * t);
        Some((smoothstep * base_alpha as f32) as u8)
    } else {
        Some(base_alpha)
    }
}

/// A dash pattern both backends can draw: non-empty, finite, no negative
/// entries and a positive period. Anything else is drawn solid.
pub fn usable_dashes(pattern: &[f64]) -> bool {
    !pattern.is_empty()
        && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)
        && pattern.iter().sum::<f64>() > 0.0
}

/// Below this period on screen (device px) a dash pattern draws solid; the
/// dashes would blur into a line anyway.
pub const MIN_DASH_PERIOD_PX: f64 = 4.0;
/// Dashes per frame; dashed entities past it draw solid. Vello dashes on
/// the CPU, one subpath per dash (about 6 GPU lines each), into fixed-size
/// GPU buffers (2^21 lines), and drops the whole frame without an error
/// when they overflow. Patterns are in world units, so a wide view of a
/// drawing with millimetre patterns asks for hundreds of thousands of
/// dashes. The budget leaves the rest of the buffer for everything else.
pub const DASH_BUDGET: f64 = 100_000.0;

/// Whether to stroke `path` with `pattern` (already [`usable_dashes`]) at
/// this zoom, charging the dashes it produces against `budget`.
fn dash_lod(pattern: &[f64], path: &BezPath, zoom: f64, budget: &mut f64) -> bool {
    use kurbo::Shape as _;
    let period: f64 = pattern.iter().sum();
    if period * zoom < MIN_DASH_PERIOD_PX {
        return false;
    }
    let dashes_per_period = pattern.len().div_ceil(2) as f64;
    let dashes = (path.perimeter(period * 0.1) / period).ceil() * dashes_per_period;
    if dashes > *budget {
        return false;
    }
    *budget -= dashes;
    true
}

/// Update `cache` for `doc`, then build the display list (see
/// [`display_list`]). The returned list keeps `cache` borrowed; to read the
/// cache while a list is alive, call [`SceneCache::update`] and
/// [`display_list`] separately.
pub fn build_display_list<'a>(
    doc: &'a Document,
    cache: &'a mut SceneCache,
    view: &View<'_>,
) -> DisplayList<'a> {
    cache.update(doc);
    display_list(doc, cache, view)
}

/// The display list for `doc` as seen through `view`, borrowing paths from
/// a `cache` already brought up to date with [`SceneCache::update`].
///
/// Pass 1 fills solid fills (even-odd, alpha [`FILL_ALPHA`]); pass 2
/// strokes everything else, with frustum culling, sub-pixel culling, LOD
/// alpha fade, entity transparency, ByLayer colour and linetype dashes.
/// Dash patterns too dense to see ([`MIN_DASH_PERIOD_PX`]) or past the
/// frame's [`DASH_BUDGET`] draw solid. Strokes are `pixel_ratio` pixels wide.
pub fn display_list<'a>(
    doc: &'a Document,
    cache: &'a SceneCache,
    view: &View<'_>,
) -> DisplayList<'a> {
    let sw = view.width as f64;
    let sh = view.height as f64;
    let cam = view.camera;
    let transform = cam.transform(sw, sh);
    let (view_x0, view_y0, view_x1, view_y1) = cam.view_rect(sw, sh);

    let visible = |ent: &DrawEntity| -> Option<(f64, f64, f64, f64)> {
        if view.hidden_layers.iter().any(|n| n == &ent.layer) {
            return None;
        }
        let (bx0, by0, bx1, by1) = ent.shape.bbox();
        if bx1 < view_x0 || bx0 > view_x1 || by1 < view_y0 || by0 > view_y1 {
            return None;
        }
        Some((bx0, by0, bx1, by1))
    };

    let mut items = Vec::new();

    // Pass 1: solid fills.
    for (key, ent, path) in cache.renderables(doc) {
        if !matches!(&ent.shape, Shape::SolidFill { .. }) {
            continue;
        }
        if visible(ent).is_none() {
            continue;
        }
        if let Some(path) = path {
            let c = doc.resolve_color(ent);
            items.push(DrawItem {
                key,
                path,
                color: Color {
                    r: c.r,
                    g: c.g,
                    b: c.b,
                    a: FILL_ALPHA,
                },
                paint: Paint::Fill { even_odd: true },
            });
        }
    }

    // Pass 2: strokes. 1 CSS pixel = pixel_ratio device pixels.
    let stroke_width = view.pixel_ratio / cam.zoom;
    let mut dash_budget = DASH_BUDGET;
    for (key, ent, path) in cache.renderables(doc) {
        if matches!(&ent.shape, Shape::SolidFill { .. }) {
            continue;
        }
        let Some((bx0, by0, bx1, by1)) = visible(ent) else {
            continue;
        };
        let dw = bx1 - bx0;
        let dh = by1 - by0;
        let diag_screen = (dw * dw + dh * dh).sqrt() * cam.zoom;
        if diag_screen < CULL_THRESHOLD_PX {
            continue;
        }

        let c = doc.resolve_color(ent);
        let base_alpha = if ent.transparency > 0 {
            ((255 - ent.transparency) as f32 / 255.0 * c.a as f32) as u8
        } else {
            c.a
        };
        let Some(alpha) = lod_alpha(diag_screen, base_alpha) else {
            continue;
        };

        if let Some(path) = path {
            items.push(DrawItem {
                key,
                path,
                color: Color {
                    r: c.r,
                    g: c.g,
                    b: c.b,
                    a: alpha,
                },
                paint: Paint::Stroke {
                    width: stroke_width,
                    dashes: doc
                        .resolve_linetype_pattern(ent)
                        .filter(|p| usable_dashes(p))
                        .filter(|p| dash_lod(p, path, cam.zoom, &mut dash_budget)),
                },
            });
        }
    }

    DisplayList {
        width: view.width,
        height: view.height,
        background: view.background,
        transform,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn line(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, layer: &str) -> EntityId {
        doc.add_line(Point::new(x0, y0), Point::new(x1, y1), layer, None)
    }

    fn push(doc: &mut Document, shape: Shape) {
        let id = doc.alloc_id();
        doc.entities.push(DrawEntity {
            id,
            layer: "0".to_string(),
            color: Some(Color::rgb(255, 0, 0)),
            linetype: None,
            lineweight: None,
            transparency: 0,
            shape,
        });
    }

    fn square_fill() -> Shape {
        use crate::FillEdge::LineTo;
        Shape::SolidFill {
            boundary: vec![
                LineTo(Point::new(0.0, 0.0)),
                LineTo(Point::new(10.0, 0.0)),
                LineTo(Point::new(10.0, 10.0)),
                LineTo(Point::new(0.0, 10.0)),
            ],
            holes: vec![],
        }
    }

    #[test]
    fn camera_fit_centres_and_keeps_margin() {
        let cam = Camera::fit((0.0, 0.0, 100.0, 50.0), 960.0, 640.0);
        assert_eq!((cam.center_x, cam.center_y), (50.0, 25.0));
        // x limits: 960 / 110 = 8.727..; y: 640 / 55 = 11.63..
        assert!((cam.zoom - 960.0 / 110.0).abs() < 1e-12);

        // Bounds land inside the screen with the margin on the tight axis.
        let t = cam.transform(960.0, 640.0);
        let p0 = t * Point::new(0.0, 0.0);
        let p1 = t * Point::new(100.0, 50.0);
        assert!((p0.x - (960.0 - 960.0 / 1.1) / 2.0).abs() < 1e-9);
        assert!((p1.x - p0.x - 960.0 / 1.1).abs() < 1e-9);
        // Y is flipped: world y=0 is below world y=50 on screen.
        assert!(p0.y > p1.y);
        assert!(((p0.y + p1.y) / 2.0 - 320.0).abs() < 1e-9);
    }

    #[test]
    fn camera_cmds_apply() {
        let mut doc = Document::new();
        line(&mut doc, -10.0, -10.0, 10.0, 10.0, "0");
        let mut cam = Camera::new();
        cam.apply(CameraCmd::FitAll, &doc, 200.0, 200.0);
        assert_eq!((cam.center_x, cam.center_y), (0.0, 0.0));
        assert!((cam.zoom - 200.0 / 22.0).abs() < 1e-12);
        cam.apply(CameraCmd::SetView(5.0, 6.0, 3.0), &doc, 200.0, 200.0);
        assert_eq!(
            cam,
            Camera {
                center_x: 5.0,
                center_y: 6.0,
                zoom: 3.0
            }
        );
        cam.apply(
            CameraCmd::FitBounds(0.0, 0.0, 10.0, 10.0),
            &doc,
            200.0,
            200.0,
        );
        assert_eq!((cam.center_x, cam.center_y), (5.0, 5.0));

        // FitAll on an empty document leaves the camera alone.
        let before = cam;
        cam.apply(CameraCmd::FitAll, &Document::new(), 200.0, 200.0);
        assert_eq!(cam, before);
    }

    #[test]
    fn hidden_layers_are_skipped() {
        let mut doc = Document::new();
        line(&mut doc, 0.0, 0.0, 100.0, 0.0, "A");
        line(&mut doc, 0.0, 10.0, 100.0, 10.0, "B");
        let cam = Camera::fit_document(&doc, 400.0, 300.0).unwrap();
        let hidden = vec!["B".to_string()];
        let mut view = View::new(400, 300, cam);
        let mut cache = SceneCache::new();
        assert_eq!(build_display_list(&doc, &mut cache, &view).items.len(), 2);
        view.hidden_layers = &hidden;
        let list = build_display_list(&doc, &mut cache, &view);
        assert_eq!(list.items.len(), 1);
        assert_eq!(
            list.items[0].path.elements()[0],
            kurbo::PathEl::MoveTo(Point::new(0.0, 0.0))
        );
    }

    #[test]
    fn frustum_culling_drops_offscreen_entities() {
        let mut doc = Document::new();
        for i in 0..10 {
            let x = i as f64 * 100.0;
            line(&mut doc, x, 0.0, x + 50.0, 50.0, "0");
        }
        // View the first 3 lines only: world x in [-10, 260] at zoom 2.
        let cam = Camera {
            center_x: 125.0,
            center_y: 25.0,
            zoom: 2.0,
        };
        let view = View::new(540, 200, cam); // half width 135 world units
        let mut cache = SceneCache::new();
        let list = build_display_list(&doc, &mut cache, &view);
        // x ranges [0,50] [100,150] [200,250] intersect [-10, 260]; [300,..] doesn't.
        assert_eq!(list.items.len(), 3);
    }

    #[test]
    fn sub_pixel_cull_and_lod_fade() {
        let mut doc = Document::new();
        // Diagonal of a 3x4 box is 5 world units.
        line(&mut doc, 0.0, 0.0, 3.0, 4.0, "0");
        let mut cache = SceneCache::new();
        let at = |zoom: f64, cache: &mut SceneCache, doc: &Document| {
            let view = View::new(
                100,
                100,
                Camera {
                    center_x: 1.5,
                    center_y: 2.0,
                    zoom,
                },
            );
            build_display_list(doc, cache, &view)
                .items
                .first()
                .map(|i| i.color.a)
        };
        // 5 * 0.09 = 0.45 px: culled.
        assert_eq!(at(0.09, &mut cache, &doc), None);
        // 5 * 0.85 = 4.25 px: t = (4.25-0.5)/7.5 = 0.5, smoothstep 0.5 -> 127.
        assert_eq!(at(0.85, &mut cache, &doc), Some(127));
        // 5 * 2 = 10 px: full alpha.
        assert_eq!(at(2.0, &mut cache, &doc), Some(255));
        assert_eq!(lod_alpha(0.49, 255), None);
        assert_eq!(lod_alpha(8.0, 200), Some(200));
    }

    #[test]
    fn fills_paint_before_strokes() {
        let mut doc = Document::new();
        // Stroke first in document order, fill second.
        line(&mut doc, 0.0, 0.0, 10.0, 10.0, "0");
        push(&mut doc, square_fill());
        let cam = Camera::fit_document(&doc, 200.0, 200.0).unwrap();
        let mut cache = SceneCache::new();
        let list = build_display_list(&doc, &mut cache, &View::new(200, 200, cam));
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.items[0].paint, Paint::Fill { even_odd: true });
        assert_eq!(list.items[0].color.a, FILL_ALPHA);
        assert!(matches!(list.items[1].paint, Paint::Stroke { .. }));
    }

    #[test]
    fn linetype_becomes_dashes_and_width_follows_pixel_ratio() {
        let mut doc = Document::new();
        let id = line(&mut doc, 0.0, 0.0, 100.0, 0.0, "0");
        doc.entity_mut(id).unwrap().linetype = Some("Dashed".to_string());
        line(&mut doc, 0.0, 10.0, 100.0, 10.0, "0");
        let cam = Camera {
            center_x: 50.0,
            center_y: 5.0,
            zoom: 4.0,
        };
        let mut view = View::new(800, 200, cam);
        view.pixel_ratio = 2.0;
        let mut cache = SceneCache::new();
        let list = build_display_list(&doc, &mut cache, &view);
        assert_eq!(
            list.items[0].paint,
            Paint::Stroke {
                width: 0.5,
                dashes: Some(&[5.0, 2.5][..])
            }
        );
        assert_eq!(
            list.items[1].paint,
            Paint::Stroke {
                width: 0.5,
                dashes: None
            }
        );
    }

    #[test]
    fn dense_or_over_budget_dashes_draw_solid() {
        let dashes_at = |zoom: f64, lines: usize, len: f64| -> usize {
            let mut doc = Document::new();
            for i in 0..lines {
                let y = i as f64;
                let id = line(&mut doc, 0.0, y, len, y, "0");
                doc.entity_mut(id).unwrap().linetype = Some("Dashed".to_string());
            }
            let cam = Camera {
                center_x: len / 2.0,
                center_y: lines as f64 / 2.0,
                zoom,
            };
            let mut cache = SceneCache::new();
            build_display_list(&doc, &mut cache, &View::new(4000, 4000, cam))
                .items
                .iter()
                .filter(|i| {
                    matches!(
                        i.paint,
                        Paint::Stroke {
                            dashes: Some(_),
                            ..
                        }
                    )
                })
                .count()
        };
        // "Dashed" is [5, 2.5]: a 7.5-unit period.
        // 7.5 * 0.6 = 4.5 px per repeat: dashed. 7.5 * 0.5 = 3.75 px: solid.
        assert_eq!(dashes_at(0.6, 1, 100.0), 1);
        assert_eq!(dashes_at(0.5, 1, 100.0), 0);
        // Each 300 000-unit line asks for 40 000 dashes; the budget of
        // 100 000 covers two of the three, the third draws solid.
        assert_eq!(dashes_at(1.0, 3, 300_000.0), 2);
    }

    #[test]
    fn invalidate_picks_up_in_place_edits() {
        let mut doc = Document::new();
        let id = line(&mut doc, 0.0, 0.0, 10.0, 10.0, "0");
        let view = View::new(200, 200, Camera::new());
        let mut cache = SceneCache::new();
        let start = |cache: &mut SceneCache, doc: &Document| {
            let list = build_display_list(doc, cache, &view);
            assert_eq!(list.items[0].key, PathKey::Entity(id));
            list.items[0].path.elements()[0]
        };
        assert_eq!(
            start(&mut cache, &doc),
            kurbo::PathEl::MoveTo(Point::new(0.0, 0.0))
        );

        // Same entity count, so the cache cannot see the edit by itself.
        doc.entity_mut(id).unwrap().shape = Shape::Line(kurbo::Line::new((5.0, 5.0), (10.0, 10.0)));
        assert_eq!(
            start(&mut cache, &doc),
            kurbo::PathEl::MoveTo(Point::new(0.0, 0.0))
        );

        let generation = cache.generation();
        cache.invalidate();
        assert_eq!(
            start(&mut cache, &doc),
            kurbo::PathEl::MoveTo(Point::new(5.0, 5.0))
        );
        assert!(cache.generation() > generation);
    }

    #[test]
    fn transparency_scales_alpha() {
        let mut doc = Document::new();
        let id = line(&mut doc, 0.0, 0.0, 100.0, 0.0, "0");
        doc.entity_mut(id).unwrap().transparency = 128;
        let cam = Camera::fit_document(&doc, 200.0, 200.0).unwrap();
        let mut cache = SceneCache::new();
        let list = build_display_list(&doc, &mut cache, &View::new(200, 200, cam));
        assert_eq!(
            list.items[0].color.a,
            ((255 - 128) as f32 / 255.0 * 255.0) as u8
        );
    }

    #[test]
    fn blocks_expand_and_cache_invalidates_on_count_change() {
        let mut doc = Document::new();
        push(
            &mut doc,
            Shape::Text {
                text: "Hi".to_string(),
                position: Point::new(0.0, 0.0),
                height: 10.0,
                rotation: 0.0,
            },
        );
        let cam = Camera::fit_document(&doc, 400.0, 200.0).unwrap();
        let mut cache = SceneCache::new();
        let n_text = build_display_list(&doc, &mut cache, &View::new(400, 200, cam))
            .items
            .len();
        assert!(n_text > 0, "text expands to glyph curves");
        assert_eq!(cache.entity_count(), 1);
        line(&mut doc, 0.0, 0.0, 10.0, 0.0, "0");
        cache.update(&doc);
        assert_eq!(cache.entity_count(), 2);
        let list = display_list(&doc, &cache, &View::new(400, 200, cam));
        assert_eq!(list.items.len(), n_text + 1);
    }

    #[test]
    fn degenerate_dash_patterns_draw_solid() {
        assert!(usable_dashes(&[5.0, 2.5]));
        assert!(usable_dashes(&[0.0, 2.5]));
        assert!(!usable_dashes(&[]));
        assert!(!usable_dashes(&[0.0, 0.0]));
        assert!(!usable_dashes(&[5.0, -1.0]));
        assert!(!usable_dashes(&[f64::NAN, 1.0]));
        assert!(!usable_dashes(&[f64::INFINITY, 1.0]));

        let mut doc = Document::new();
        doc.linetypes.insert(
            "BAD".to_string(),
            crate::LinetypeDef {
                name: "BAD".to_string(),
                description: String::new(),
                pattern: vec![0.0, 0.0],
            },
        );
        let id = line(&mut doc, 0.0, 0.0, 100.0, 0.0, "0");
        doc.entity_mut(id).unwrap().linetype = Some("BAD".to_string());
        let cam = Camera::fit_document(&doc, 200.0, 200.0).unwrap();
        let mut cache = SceneCache::new();
        let list = build_display_list(&doc, &mut cache, &View::new(200, 200, cam));
        assert!(matches!(
            list.items[0].paint,
            Paint::Stroke { dashes: None, .. }
        ));
    }
}
