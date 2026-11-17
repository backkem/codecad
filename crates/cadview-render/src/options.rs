//! Render options, parsed from the same JSON `cad.render(...)` takes.

use cadview_core::render::{Camera, View};
use cadview_core::Document;
use serde::{Deserialize, Serialize};

/// Which backend to use. Results always name the one that ran (`Gpu` or `Cpu`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// GPU if available, else CPU (with a logged warning).
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Gpu => "gpu",
            Backend::Cpu => "cpu",
        }
    }
}

impl std::str::FromStr for Backend {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "auto" => Ok(Backend::Auto),
            "gpu" => Ok(Backend::Gpu),
            "cpu" => Ok(Backend::Cpu),
            other => Err(format!("unknown render backend '{other}' (auto|gpu|cpu)")),
        }
    }
}

/// World rectangle to fit in the image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// Largest accepted output edge in pixels.
pub const MAX_EDGE: u32 = 16384;

/// Parse `WxH` (for example `960x640`) into a positive size.
pub fn parse_size(s: &str) -> Result<[u32; 2], String> {
    let parsed = s
        .split_once(['x', 'X'])
        .and_then(|(w, h)| Some([w.trim().parse().ok()?, h.trim().parse().ok()?]));
    match parsed {
        Some([w, h]) if w > 0 && h > 0 => Ok([w, h]),
        _ => Err(format!(
            "size must be WxH with both positive, e.g. 960x640, got '{s}'"
        )),
    }
}

/// `cad.render` options. Unknown fields (`out`, `spp`, ...) are ignored here;
/// the caller handles `out`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RenderOptions {
    /// Image size in CSS pixels; the PNG is `size * pixel_ratio` pixels.
    pub size: [u32; 2],
    /// Region to fit (with the usual 10 % margin). Default: the whole drawing.
    pub bounds: Option<Bounds>,
    pub hidden_layers: Vec<String>,
    pub backend: Backend,
    /// Device pixels per CSS pixel, as on a high-DPI screen: scales the
    /// output size and the 1-px stroke width together.
    pub pixel_ratio: f64,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            size: [960, 640],
            bounds: None,
            hidden_layers: Vec::new(),
            backend: Backend::Auto,
            pixel_ratio: 1.0,
        }
    }
}

impl RenderOptions {
    /// Parse and validate. `""` and `"null"` give the defaults.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let trimmed = json.trim();
        let opts: Self = if trimmed.is_empty() || trimmed == "null" {
            Self::default()
        } else {
            serde_json::from_str(trimmed).map_err(|e| format!("render options: {e}"))?
        };
        opts.validate()?;
        Ok(opts)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(self.pixel_ratio.is_finite() && self.pixel_ratio > 0.0) {
            return Err(format!("pixelRatio must be > 0, got {}", self.pixel_ratio));
        }
        let (w, h) = self.pixel_size();
        if w == 0 || h == 0 {
            return Err(format!("size must be positive, got {}x{}", w, h));
        }
        if w > MAX_EDGE || h > MAX_EDGE {
            return Err(format!("size {w}x{h} exceeds {MAX_EDGE} px per edge"));
        }
        if let Some(b) = self.bounds {
            let ok = b.min.iter().chain(b.max.iter()).all(|v| v.is_finite())
                && b.max[0] > b.min[0]
                && b.max[1] > b.min[1];
            if !ok {
                return Err("bounds need finite min < max on both axes".to_string());
            }
        }
        Ok(())
    }

    /// Output size in device pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        let scale = |v: u32| (v as f64 * self.pixel_ratio).round() as u32;
        (scale(self.size[0]), scale(self.size[1]))
    }

    /// The core view for `doc`: fitted to `bounds` or the whole drawing.
    pub fn view<'a>(&'a self, doc: &Document) -> View<'a> {
        let (w, h) = self.pixel_size();
        let (fw, fh) = (w as f64, h as f64);
        let camera = match self.bounds {
            Some(b) => Camera::fit_checked((b.min[0], b.min[1], b.max[0], b.max[1]), fw, fh),
            None => doc.bounds().and_then(|b| Camera::fit_checked(b, fw, fh)),
        }
        .unwrap_or_default();
        let mut view = View::new(w, h, camera);
        view.hidden_layers = &self.hidden_layers;
        view.pixel_ratio = self.pixel_ratio;
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_camel_case() {
        let o = RenderOptions::from_json("{}").unwrap();
        assert_eq!(o, RenderOptions::default());
        let o = RenderOptions::from_json(
            r#"{"out":"x.png","size":[100,50],"bounds":{"min":[0,0],"max":[10,5]},
                "hiddenLayers":["A"],"backend":"cpu","pixelRatio":2,"spp":1024}"#,
        )
        .unwrap();
        assert_eq!(o.size, [100, 50]);
        assert_eq!(o.pixel_size(), (200, 100));
        assert_eq!(o.hidden_layers, vec!["A"]);
        assert_eq!(o.backend, Backend::Cpu);
        assert_eq!(o.bounds.unwrap().max, [10.0, 5.0]);
    }

    #[test]
    fn rejects_bad_values() {
        assert!(RenderOptions::from_json(r#"{"size":[0,10]}"#).is_err());
        assert!(RenderOptions::from_json(r#"{"pixelRatio":0}"#).is_err());
        assert!(RenderOptions::from_json(r#"{"backend":"metal"}"#).is_err());
        assert!(RenderOptions::from_json(r#"{"size":[20000,10]}"#).is_err());
        assert!(RenderOptions::from_json(r#"{"bounds":{"min":[0,0],"max":[0,5]}}"#).is_err());
    }
}
