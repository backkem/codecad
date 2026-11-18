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
//!
//! Materials work the same way: `materials: {attr, map, default}` maps the
//! values of one attribute to physical parameters (colour, metallic,
//! roughness, transmission, index of refraction, emission, clear coat, a
//! procedural pattern). The names in the map are the script's; the core
//! sees only numbers. `environment` (a studio or sky dome with a sun) and
//! `exposure` (stops) light the presentation render.

use serde_json::Value;

use crate::document::Document;
use crate::scene3d::{
    palette_color, srgb_to_linear, Background, Bands, Brushed, Camera3d, ClipMode, ClipPlane,
    Environment, Ground, LightFrame, Material, NamedView, Occlusion, Pattern, PatternAxis,
    Projection, Scene3d, VIEWER_BACKGROUND,
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

fn num(v: &Value, key: &str, what: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(x) => x
            .as_f64()
            .filter(|f| f.is_finite() && (lo..=hi).contains(f))
            .map(Some)
            .ok_or_else(|| format!("{what}.{key} is a number from {lo} to {hi}")),
    }
}

fn linear(c: [u8; 3]) -> [f64; 3] {
    c.map(srgb_to_linear)
}

fn pattern_axis(v: Option<&Value>, what: &str) -> Result<PatternAxis, String> {
    match v {
        None | Some(Value::Null) => Ok(PatternAxis::Longest),
        Some(Value::String(s)) if s == "long" || s == "longest" => Ok(PatternAxis::Longest),
        Some(Value::String(s)) if s == "short" || s == "shortest" => Ok(PatternAxis::Shortest),
        Some(Value::String(s)) if s == "symmetry" => Ok(PatternAxis::Symmetry),
        Some(a) => {
            let p = p3(a, &format!("{what}.axis"))?;
            Vec3::new(p.x, p.y, p.z)
                .normalized()
                .map(PatternAxis::Vector)
                .ok_or_else(|| format!("{what}.axis must be a non-zero vector"))
        }
    }
}

fn pattern(v: &Value, base: &Material, what: &str) -> Result<Pattern, String> {
    let what = format!("{what}.pattern");
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("flat");
    match kind {
        "flat" => Ok(Pattern::Flat),
        "bands" => {
            let spacing = num(v, "spacing", &what, 1e-3, 1e6)?.unwrap_or(4.0);
            let color = match v.get("color") {
                None | Some(Value::Null) => base.base_color.map(|c| c * 0.45),
                Some(c) => linear(rgb(c, &format!("{what}.color"))?),
            };
            Ok(Pattern::Bands(Bands {
                axis: pattern_axis(v.get("axis"), &what)?,
                spacing,
                color,
                strength: num(v, "strength", &what, 0.0, 1.0)?.unwrap_or(0.6),
                warp: num(v, "warp", &what, 0.0, 1e6)?.unwrap_or(0.6 * spacing),
                distance: num(v, "distance", &what, -1e7, 1e7)?.unwrap_or(40.0 * spacing),
            }))
        }
        "brushed" => Ok(Pattern::Brushed(Brushed {
            axis: pattern_axis(v.get("axis"), &what)?,
            anisotropy: num(v, "anisotropy", &what, 0.0, 1.0)?.unwrap_or(0.7),
            streaks: num(v, "streaks", &what, 0.0, 1.0)?.unwrap_or(0.5),
            scale: num(v, "scale", &what, 1e-4, 1e4)?.unwrap_or(0.15),
            circular: match v.get("circular") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(format!("{what}.circular is true or false")),
            },
        })),
        other => Err(format!(
            "{what}.kind is \"flat\", \"bands\" or \"brushed\", got '{other}'"
        )),
    }
}

/// One material of the `materials` option over `base` (the part's colour
/// from `colorBy`): `color` (sRGB), `metallic`, `roughness`,
/// `transmission`, `ior`, `emission` (a strength, or `{color, strength}`),
/// `clearcoat`, `clearcoatRoughness`, `pattern`.
fn material(v: &Value, base: Material, what: &str) -> Result<Material, String> {
    if !v.is_object() {
        return Err(format!("{what} is an object of material parameters"));
    }
    let mut m = base;
    if let Some(c) = v.get("color").filter(|c| !c.is_null()) {
        m.base_color = linear(rgb(c, &format!("{what}.color"))?);
    }
    let set = |field: &mut f64, key: &str, lo: f64, hi: f64| -> Result<(), String> {
        if let Some(x) = num(v, key, what, lo, hi)? {
            *field = x;
        }
        Ok(())
    };
    set(&mut m.metallic, "metallic", 0.0, 1.0)?;
    set(&mut m.roughness, "roughness", 0.0, 1.0)?;
    set(&mut m.transmission, "transmission", 0.0, 1.0)?;
    set(&mut m.ior, "ior", 1.0, 3.0)?;
    set(&mut m.clearcoat, "clearcoat", 0.0, 1.0)?;
    set(&mut m.clearcoat_roughness, "clearcoatRoughness", 0.0, 1.0)?;
    match v.get("emission") {
        None | Some(Value::Null) => {}
        Some(Value::Number(n)) => {
            let s = n
                .as_f64()
                .filter(|s| s.is_finite() && *s >= 0.0)
                .ok_or_else(|| format!("{what}.emission is a strength >= 0"))?;
            m.emission = [s; 3];
        }
        Some(e @ Value::Object(_)) => {
            let s = num(e, "strength", &format!("{what}.emission"), 0.0, 1e6)?.unwrap_or(1.0);
            let c = match e.get("color") {
                None | Some(Value::Null) => [1.0; 3],
                Some(c) => linear(rgb(c, &format!("{what}.emission.color"))?),
            };
            m.emission = c.map(|c| c * s);
        }
        Some(_) => {
            return Err(format!(
                "{what}.emission is a strength or {{color, strength}}"
            ))
        }
    }
    if let Some(p) = v.get("pattern").filter(|p| !p.is_null()) {
        m.pattern = pattern(p, &m, what)?;
    }
    Ok(m)
}

/// The `materials` render option: `{attr, map: {value: material}, default?}`.
/// A part whose attribute value is a key of `map` gets that material over
/// its `colorBy` colour; other parts get `default`, or keep the colour as a
/// plain material. The core gives the attribute no other meaning.
struct MaterialMap {
    attr: Option<String>,
    map: Vec<(String, Value)>,
    default: Option<Value>,
}

impl MaterialMap {
    fn from_options(o: &Value) -> Result<MaterialMap, String> {
        let none = MaterialMap {
            attr: None,
            map: Vec::new(),
            default: None,
        };
        let Some(v) = o.get("materials").filter(|v| !v.is_null()) else {
            return Ok(none);
        };
        let shape = "materials is {attr: \"key\", map: {value: {color, roughness, ...}}, default?}";
        if !v.is_object() {
            return Err(shape.into());
        }
        let attr = match v.get("attr") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(shape.into()),
        };
        let map = match v.get("map") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            Some(_) => return Err(shape.into()),
        };
        if attr.is_none() && !map.is_empty() {
            return Err("materials.map needs materials.attr (the attribute to look up)".into());
        }
        let m = MaterialMap {
            attr,
            map,
            default: v.get("default").filter(|d| !d.is_null()).cloned(),
        };
        // Check every entry now, so a mistake shows even when no part uses it.
        for (k, entry) in &m.map {
            material(entry, Material::default(), &format!("materials.map.{k}"))?;
        }
        if let Some(d) = &m.default {
            material(d, Material::default(), "materials.default")?;
        }
        Ok(m)
    }

    fn material_of(&self, p: &crate::solid::Part, base: Material) -> Material {
        let value = self
            .attr
            .as_ref()
            .and_then(|k| p.attrs.get(k))
            .map(|v| match v {
                crate::attrs::AttrValue::Str(s) => s.clone(),
                crate::attrs::AttrValue::Num(n) => n.to_string(),
                crate::attrs::AttrValue::Bool(b) => b.to_string(),
            });
        let entry = value
            .and_then(|v| self.map.iter().find(|(k, _)| *k == v))
            .map(|(k, e)| (format!("materials.map.{k}"), e))
            .or_else(|| {
                self.default
                    .as_ref()
                    .map(|d| ("materials.default".into(), d))
            });
        match entry {
            // Checked in from_options.
            Some((what, e)) => material(e, base, &what).unwrap_or(base),
            None => base,
        }
    }
}

/// The `environment` render option: `"studio"`, `"sky"`, or `{preset?,
/// zenith?, horizon?, ground?, intensity?, sun?}` with sRGB colours and `sun`
/// false or `{direction, angle, irradiance, color, frame: "camera" |
/// "world"}`.
fn environment(v: &Value) -> Result<Environment, String> {
    let preset = |name: &str| match name {
        "studio" => Ok(Environment::studio()),
        "sky" => Ok(Environment::sky()),
        other => Err(format!(
            "environment is \"studio\", \"sky\" or an object, got '{other}'"
        )),
    };
    let obj = match v {
        Value::String(s) => return preset(s),
        Value::Object(_) => v,
        _ => return Err("environment is \"studio\", \"sky\" or an object".into()),
    };
    let mut env = match obj.get("preset").and_then(Value::as_str) {
        None => Environment::studio(),
        Some(p) => preset(p)?,
    };
    for (key, field) in [
        ("zenith", &mut env.zenith),
        ("horizon", &mut env.horizon),
        ("ground", &mut env.ground),
    ] {
        if let Some(c) = obj.get(key).filter(|c| !c.is_null()) {
            *field = linear(rgb(c, &format!("environment.{key}"))?);
        }
    }
    if let Some(k) = num(obj, "intensity", "environment", 0.0, 1e6)? {
        env.intensity = k;
    }
    match obj.get("sun") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(false)) => env.sun = None,
        Some(s @ Value::Object(_)) => {
            let mut sun = env.sun.unwrap_or(crate::scene3d::Sun {
                direction: Vec3::new(-0.45, -0.45, 0.766),
                frame: LightFrame::World,
                angle_deg: 0.53,
                irradiance: 2.0,
                color: [1.0; 3],
            });
            if let Some(d) = s.get("direction").filter(|d| !d.is_null()) {
                let p = p3(d, "environment.sun.direction")?;
                sun.direction = Vec3::new(p.x, p.y, p.z)
                    .normalized()
                    .ok_or("environment.sun.direction must be a non-zero vector")?;
                // A direction given without a frame is a world direction.
                sun.frame = LightFrame::World;
            }
            match s.get("frame").and_then(Value::as_str) {
                None => {}
                Some("camera") => sun.frame = LightFrame::Camera,
                Some("world") => sun.frame = LightFrame::World,
                Some(f) => {
                    return Err(format!(
                        "environment.sun.frame is \"camera\" or \"world\", got '{f}'"
                    ))
                }
            }
            if let Some(a) = num(
                s,
                "angle",
                "environment.sun",
                crate::scene3d::MIN_SUN_ANGLE_DEG,
                90.0,
            )? {
                sun.angle_deg = a;
            }
            if let Some(e) = num(s, "irradiance", "environment.sun", 0.0, 1e6)? {
                sun.irradiance = e;
            }
            if let Some(c) = s.get("color").filter(|c| !c.is_null()) {
                sun.color = linear(rgb(c, "environment.sun.color")?);
            }
            env.sun = Some(sun);
        }
        Some(_) => {
            return Err(
                "environment.sun is false or {direction, angle, irradiance, color, frame}".into(),
            )
        }
    }
    Ok(env)
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
    let materials = MaterialMap::from_options(o)?;
    let mut triangles = 0;
    for p in &parts {
        let def = &doc.part_defs[&p.def];
        let mesh = solids
            .render_mesh(&doc.part_defs, def, chord)
            .map_err(|e| format!("part {}: {e}", part_key(p.id)))?;
        triangles += mesh.triangles.len();
        let [r, g, b] = colors[&p.id];
        let base = Material::from_srgb8(r, g, b);
        scene.add_shared(mesh, p.placement, materials.material_of(p, base));
    }
    if let Some(env) = o.get("environment").filter(|v| !v.is_null()) {
        scene.environment = environment(env)?;
    }
    match o.get("exposure") {
        None | Some(Value::Null) => {}
        Some(v) => {
            scene.exposure = v
                .as_f64()
                .filter(|e| e.is_finite() && e.abs() <= 20.0)
                .ok_or("exposure is a number of stops from -20 to 20")?
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn materials_option_maps_attribute_values_to_parameters() {
        let o = json!({"materials": {
            "attr": "material",
            "map": {
                "oak": {"color": "#b08a5a", "roughness": 0.6, "clearcoat": 0.3,
                        "pattern": {"kind": "bands", "spacing": 3, "axis": "long"}},
                "glass": {"transmission": 1, "ior": 1.52, "roughness": 0},
                "lamp": {"emission": {"color": [255, 128, 0], "strength": 4}},
                "turned": {"metallic": 1, "pattern": {"kind": "brushed", "axis": "symmetry", "circular": true}}
            },
            "default": {"roughness": 0.9}
        }});
        let m = MaterialMap::from_options(&o).unwrap();
        assert_eq!(m.attr.as_deref(), Some("material"));
        let base = Material::from_srgb8(10, 20, 30);
        let oak = material(
            &m.map.iter().find(|(k, _)| k == "oak").unwrap().1,
            base,
            "t",
        )
        .unwrap();
        assert_eq!(oak.base_color, [0xb0, 0x8a, 0x5a].map(srgb_to_linear));
        assert_eq!((oak.roughness, oak.clearcoat), (0.6, 0.3));
        match oak.pattern {
            Pattern::Bands(b) => {
                assert_eq!((b.spacing, b.axis), (3.0, PatternAxis::Longest));
                // The ring colour defaults to a darker base colour.
                assert!(b.color[0] < oak.base_color[0]);
            }
            other => panic!("{other:?}"),
        }
        let glass = material(
            &m.map.iter().find(|(k, _)| k == "glass").unwrap().1,
            base,
            "t",
        )
        .unwrap();
        assert_eq!((glass.transmission, glass.ior), (1.0, 1.52));
        // No colour given: the part's colour stays.
        assert_eq!(glass.base_color, base.base_color);
        let lamp = material(
            &m.map.iter().find(|(k, _)| k == "lamp").unwrap().1,
            base,
            "t",
        )
        .unwrap();
        assert_eq!(lamp.emission, [4.0, 4.0 * srgb_to_linear(128), 0.0]);
        let turned = material(
            &m.map.iter().find(|(k, _)| k == "turned").unwrap().1,
            base,
            "t",
        )
        .unwrap();
        assert!(matches!(
            turned.pattern,
            Pattern::Brushed(Brushed {
                axis: PatternAxis::Symmetry,
                circular: true,
                ..
            })
        ));

        for bad in [
            json!({"materials": 5}),
            json!({"materials": {"map": {"oak": {}}}}),
            json!({"materials": {"attr": "m", "map": {"oak": {"roughness": 2}}}}),
            json!({"materials": {"attr": "m", "map": {"oak": {"pattern": {"kind": "marble"}}}}}),
            json!({"materials": {"attr": "m", "map": {"oak": {"ior": 0.5}}}}),
            json!({"materials": {"attr": "m", "default": {"emission": "bright"}}}),
        ] {
            assert!(MaterialMap::from_options(&bad).is_err(), "{bad}");
        }
        assert!(MaterialMap::from_options(&json!({}))
            .unwrap()
            .map
            .is_empty());
    }

    #[test]
    fn environment_option_presets_and_overrides() {
        assert_eq!(environment(&json!("sky")).unwrap(), Environment::sky());
        assert_eq!(
            environment(&json!("studio")).unwrap(),
            Environment::studio()
        );
        let e = environment(&json!({
            "preset": "sky", "intensity": 2, "zenith": "#000000",
            "sun": {"direction": [0, 0, 1], "angle": 5, "irradiance": 3}
        }))
        .unwrap();
        assert_eq!((e.intensity, e.zenith), (2.0, [0.0; 3]));
        let sun = e.sun.unwrap();
        assert_eq!(
            (sun.angle_deg, sun.irradiance, sun.frame),
            (5.0, 3.0, LightFrame::World)
        );
        assert_eq!(sun.direction, Vec3::Z);
        assert!(environment(&json!({"sun": false})).unwrap().sun.is_none());
        for bad in [
            json!("night"),
            json!(3),
            json!({"sun": {"angle": 0}}),
            json!({"sun": {"direction": [0, 0, 0]}}),
            json!({"sun": {"frame": "eye"}}),
            json!({"intensity": -1}),
        ] {
            assert!(environment(&bad).is_err(), "{bad}");
        }
    }
}
