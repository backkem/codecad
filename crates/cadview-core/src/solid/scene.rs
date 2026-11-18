//! The document's parts as a [`Scene3d`]: the 3D branch of `cad.render`.
//!
//! Every decision about what a part render shows is made here, in core,
//! from the render options a script passes (the same JSON object as the 2D
//! `cad.render`): which parts, their colours, the tessellation tolerance,
//! the view, edges, depth cues, cutaways and exploded views. Meshes come
//! from the document's evaluator (`Document::solids`), so this code links
//! no kernel; a shell only hands the scene to a backend
//! (`cadview-render3d`).
//!
//! Colours are a render option, not part data: by default each part gets
//! the next palette colour; `colorBy: "def"` gives the parts of one
//! definition one colour; `colorBy: {attr: "key"}` gives parts with equal
//! values of that attribute one colour, and a value written `"#rrggbb"` is
//! used as the colour itself. The core gives the attribute no meaning
//! beyond this option.

use serde_json::Value;

use crate::document::Document;
use crate::scene3d::{
    palette_color, Background, Camera3d, ClipMode, ClipPlane, Ground, Material, NamedView,
    Occlusion, Projection, Scene3d, VIEWER_BACKGROUND,
};
use crate::solid::kernel::Aabb3;
use crate::solid::part::part_key;
use crate::solid::types::{Point3, Vec3};

/// Margin the camera fit leaves around the scene (the 2D renderer's).
const FIT_SPAN: f64 = 1.2;

/// A scene of parts and what went into it.
#[derive(Debug, Clone)]
pub struct PartsScene {
    pub scene: Scene3d,
    /// Parts drawn.
    pub parts: usize,
    /// Triangles over all drawn parts.
    pub triangles: usize,
    /// Tessellation chord (mm).
    pub chord: f64,
    /// The kernel that tessellated.
    pub kernel: String,
}

/// Is this `cad.render` call a 3D one? `mode: "3d" | "2d"` decides; without
/// it a `view` option means 3D, and so does a document that has parts and
/// no 2D entities.
pub fn wants_3d(doc: &Document, o: &Value) -> Result<bool, String> {
    match o.get("mode").and_then(Value::as_str) {
        Some("3d") => Ok(true),
        Some("2d") => Ok(false),
        Some(other) => Err(format!("mode is \"2d\" or \"3d\", got '{other}'")),
        None if o.get("mode").is_some_and(|m| !m.is_null()) => {
            Err("mode is \"2d\" or \"3d\"".into())
        }
        None => Ok(o.get("view").is_some_and(|v| !v.is_null())
            || (!doc.parts.is_empty() && doc.entities.is_empty())),
    }
}

/// The chord (mm) for an image of `pixels` (the shorter side) showing a
/// scene whose box has diagonal `diag`: a quarter pixel, rounded down to a
/// power of two between 2^-7 and 2 mm, so nearby sizes share a cached mesh.
pub fn auto_chord(diag: f64, pixels: u32) -> f64 {
    let per_px = diag.max(1e-9) * FIT_SPAN / pixels.max(1) as f64;
    let c = 0.25 * per_px;
    2f64.powi(c.log2().floor() as i32).clamp(2f64.powi(-7), 2.0)
}

fn rgb(v: &Value, what: &str) -> Result<[u8; 3], String> {
    if let Some(s) = v.as_str() {
        return hex_color(s).ok_or_else(|| format!("{what}: colours are [r, g, b] or \"#rrggbb\""));
    }
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("{what}: colours are [r, g, b] or \"#rrggbb\""))?;
    let mut out = [0u8; 3];
    for (o, x) in out.iter_mut().zip(a) {
        *o = x
            .as_u64()
            .filter(|n| *n <= 255)
            .ok_or_else(|| format!("{what}: colour channels are 0 to 255"))? as u8;
    }
    Ok(out)
}

/// `#rrggbb` as sRGB bytes.
fn hex_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

fn p3(v: &Value, what: &str) -> Result<Point3, String> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("{what} is [x, y, z]"))?;
    let f = |i: usize| a[i].as_f64().ok_or_else(|| format!("{what} is [x, y, z]"));
    Ok(Point3::new(f(0)?, f(1)?, f(2)?))
}

fn view_of(o: &Value) -> Result<NamedView, String> {
    match o.get("view").and_then(Value::as_str) {
        None => Ok(NamedView::Isometric),
        // "iso" is the south-east isometric; the other three corners are
        // "iso-sw", "iso-ne" and "iso-nw".
        Some("iso" | "iso-se") => Ok(NamedView::Isometric),
        Some(s) => s.parse::<NamedView>(),
    }
}

fn clip_planes(o: &Value) -> Result<Vec<ClipPlane>, String> {
    let one = |c: &Value| -> Result<ClipPlane, String> {
        let point = p3(c.get("point").unwrap_or(&Value::Null), "clip point")?;
        let n = p3(c.get("normal").unwrap_or(&Value::Null), "clip normal")?;
        let normal = Vec3::new(n.x, n.y, n.z);
        normal
            .normalized()
            .ok_or("clip normal must be a non-zero vector")?;
        Ok(ClipPlane { point, normal })
    };
    match o.get("clip") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(a)) => {
            if a.len() > crate::scene3d::MAX_CLIP_PLANES {
                return Err(format!(
                    "at most {} clip planes",
                    crate::scene3d::MAX_CLIP_PLANES
                ));
            }
            a.iter().map(one).collect()
        }
        Some(c) => Ok(vec![one(c)?]),
    }
}

/// Builds the scene `cad.render` draws for the document's parts, for an
/// image of `pixels` (width, height in device pixels).
pub fn parts_scene(doc: &Document, o: &Value, pixels: (u32, u32)) -> Result<PartsScene, String> {
    let solids = doc.solids.get()?;
    let sel = o.get("select").cloned().unwrap_or(Value::Null);
    let ids = crate::dispatch_3d::select_parts_value(doc, &sel)?;
    if ids.is_empty() {
        return Err("no parts to draw (the document has none, or select picks none)".into());
    }
    let parts: Vec<_> = ids
        .iter()
        .map(|id| doc.part(*id).expect("selected"))
        .collect();

    // World boxes: the scene's size sets the tessellation tolerance.
    let mut all: Option<Aabb3> = None;
    for p in &parts {
        let def = &doc.part_defs[&p.def];
        let b = solids
            .bbox(&doc.part_defs, def, &p.placement)
            .map_err(|e| format!("part {}: {e}", part_key(p.id)))?;
        all = Some(match all {
            None => b,
            Some(a) => Aabb3::new(
                Point3::new(
                    a.min.x.min(b.min.x),
                    a.min.y.min(b.min.y),
                    a.min.z.min(b.min.z),
                ),
                Point3::new(
                    a.max.x.max(b.max.x),
                    a.max.y.max(b.max.y),
                    a.max.z.max(b.max.z),
                ),
            ),
        });
    }
    let all = all.expect("at least one part");
    let diag = all.min.distance(all.max);
    let chord = match o.get("chord") {
        None | Some(Value::Null) => auto_chord(diag, pixels.0.min(pixels.1)),
        Some(v) => v
            .as_f64()
            .filter(|c| c.is_finite() && *c > 0.0)
            .ok_or("chord must be a positive number (mm)")?,
    };

    // Colours.
    enum By {
        Part,
        Def,
        Attr(String),
    }
    let by = match o.get("colorBy") {
        None | Some(Value::Null) => By::Part,
        Some(Value::String(s)) if s == "part" => By::Part,
        Some(Value::String(s)) if s == "def" => By::Def,
        Some(v) if v.get("attr").and_then(Value::as_str).is_some() => {
            By::Attr(v["attr"].as_str().expect("checked").to_string())
        }
        Some(_) => return Err("colorBy is \"part\", \"def\" or {attr: \"key\"}".into()),
    };
    // A part's colour follows its place among all of the document's parts,
    // not among the selected ones, so a view of some parts shows each in
    // the colour it has in a view of all of them. Groups (by definition or
    // attribute value) are numbered in the same document order.
    let mut groups: Vec<String> = Vec::new();
    let mut color_of = |p: &crate::solid::Part, i: usize| -> [u8; 3] {
        let key = match &by {
            By::Part => return palette_color(i),
            By::Def => p.def.0.to_string(),
            By::Attr(k) => match p.attrs.get(k) {
                None => return [200, 200, 200],
                Some(crate::attrs::AttrValue::Str(s)) if hex_color(s).is_some() => {
                    return hex_color(s).expect("checked")
                }
                Some(v) => format!("{v:?}"),
            },
        };
        let n = groups.iter().position(|g| *g == key).unwrap_or_else(|| {
            groups.push(key);
            groups.len() - 1
        });
        palette_color(n)
    };

    let projection = match o.get("projection").and_then(Value::as_str) {
        None | Some("orthographic") => Projection::Orthographic,
        Some("perspective") => Projection::Perspective,
        Some(other) => {
            return Err(format!(
                "projection is \"orthographic\" or \"perspective\", got '{other}'"
            ))
        }
    };
    let mut scene = Scene3d {
        camera: Camera3d::new(view_of(o)?, projection),
        ..Scene3d::default()
    };
    // Number the colour groups in document order first (see color_of).
    let mut colors: std::collections::HashMap<crate::types::EntityId, [u8; 3]> =
        std::collections::HashMap::new();
    for (i, p) in doc.parts.iter().enumerate() {
        colors.insert(p.id, color_of(p, i));
    }
    let mut triangles = 0;
    for p in &parts {
        let def = &doc.part_defs[&p.def];
        let mesh = solids
            .render_mesh(&doc.part_defs, def, chord)
            .map_err(|e| format!("part {}: {e}", part_key(p.id)))?;
        triangles += mesh.triangles.len();
        let [r, g, b] = colors[&p.id];
        scene.add_shared(mesh, p.placement, Material::from_srgb8(r, g, b));
    }

    match o.get("background") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "viewer" => {
            scene.background = Background::Solid(VIEWER_BACKGROUND)
        }
        Some(Value::String(s)) if s == "light" => scene.background = Background::default(),
        Some(v) => scene.background = Background::Solid(rgb(v, "background")?),
    }
    if o.get("edges").and_then(Value::as_bool) == Some(false) {
        scene.edges = None;
    }
    match o.get("occlusion") {
        None | Some(Value::Null) | Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => scene.occlusion = None,
        Some(v) => {
            let strength = v
                .as_f64()
                .filter(|s| (0.0..=1.0).contains(s))
                .ok_or("occlusion is true, false or a strength from 0 to 1")?;
            scene.occlusion = Some(Occlusion {
                strength,
                ..Occlusion::default()
            });
        }
    }
    match o.get("ground") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => scene.ground = Some(Ground::default()),
        Some(g @ Value::Object(_)) => {
            scene.ground = Some(Ground {
                height: g.get("height").and_then(Value::as_f64),
                color: match g.get("color") {
                    None | Some(Value::Null) => None,
                    Some(c) => Some(rgb(c, "ground color")?),
                },
            })
        }
        Some(_) => return Err("ground is true, false or {height?, color?}".into()),
    }
    scene.clip_planes = clip_planes(o)?;
    scene.clip_mode = match o.get("clipMode").and_then(Value::as_str) {
        None if o.get("clipMode").is_none_or(Value::is_null) => ClipMode::Any,
        Some("any") => ClipMode::Any,
        Some("all") => ClipMode::All,
        _ => return Err(
            "clipMode is \"any\" (remove what is beyond any plane) or \"all\" (beyond every plane)"
                .into(),
        ),
    };
    match o.get("explode") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let (factor, axes) = match v {
                Value::Number(n) => (n.as_f64().unwrap_or(0.0), "xyz".to_string()),
                Value::Object(m) => (
                    m.get("factor").and_then(Value::as_f64).unwrap_or(0.5),
                    m.get("axes")
                        .and_then(Value::as_str)
                        .unwrap_or("xyz")
                        .to_string(),
                ),
                _ => return Err("explode is a factor or {factor, axes: \"xyz\"}".into()),
            };
            if !(factor.is_finite() && factor >= 0.0) {
                return Err("explode factor must be a number >= 0".into());
            }
            if axes.is_empty() || !axes.chars().all(|c| "xyz".contains(c)) {
                return Err("explode axes is a string of x, y and z".into());
            }
            scene.explode(
                factor,
                [axes.contains('x'), axes.contains('y'), axes.contains('z')],
            );
        }
    }

    Ok(PartsScene {
        scene,
        parts: parts.len(),
        triangles,
        chord,
        kernel: solids.kernel_name(),
    })
}
