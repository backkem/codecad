//! Options for a headless 3D render, modelled on `cadview_render::RenderOptions`.

use crate::Render3dError;
use cadview_render::MAX_EDGE;

/// Most samples per pixel a presentation render may ask for.
pub const MAX_SPP: u32 = 1 << 16;

/// Render quality. Both levels draw the same scene and camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// Fast rasterized shading with edges, for checking a model while
    /// building it.
    #[default]
    Working,
    /// Path traced with materials and environment light, for showing the
    /// result ([`crate::trace::PathTracer`]).
    Presentation,
}

impl Quality {
    pub fn as_str(self) -> &'static str {
        match self {
            Quality::Working => "working",
            Quality::Presentation => "presentation",
        }
    }
}

impl std::str::FromStr for Quality {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "working" => Ok(Quality::Working),
            "presentation" => Ok(Quality::Presentation),
            other => Err(format!("unknown quality '{other}' (working|presentation)")),
        }
    }
}

/// Options for one headless render of a [`cadview_core::scene3d::Scene3d`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOptions3d {
    /// Image size in CSS pixels; the PNG is `size * pixel_ratio` pixels.
    pub size: [u32; 2],
    /// Device pixels per CSS pixel, as on a high-DPI screen: scales the
    /// output size and the edge line width together. The picture stays
    /// the same.
    pub pixel_ratio: f64,
    /// Overrides the scene's background (sRGB).
    pub background: Option<[u8; 3]>,
    pub quality: Quality,
    /// Fit the camera to the whole scene before rendering (keeping its
    /// view direction and projection), with the 2D renderer's 10 % margin.
    pub fit: bool,
    /// Presentation only: samples per pixel to accumulate.
    pub spp: u32,
    /// Presentation only: the seed of the random numbers. The same scene,
    /// samples and seed give the same bytes.
    pub seed: u32,
}

impl Default for RenderOptions3d {
    fn default() -> Self {
        Self {
            size: [960, 640],
            pixel_ratio: 1.0,
            background: None,
            quality: Quality::Working,
            fit: true,
            spp: crate::trace::DEFAULT_SPP,
            seed: 0,
        }
    }
}

impl RenderOptions3d {
    pub fn validate(&self) -> Result<(), Render3dError> {
        let bad = |e: String| Err(Render3dError::InvalidOptions(e));
        if !(self.pixel_ratio.is_finite() && self.pixel_ratio > 0.0) {
            return bad(format!("pixelRatio must be > 0, got {}", self.pixel_ratio));
        }
        let (w, h) = self.pixel_size();
        if w == 0 || h == 0 {
            return bad(format!("size must be positive, got {w}x{h}"));
        }
        if w > MAX_EDGE || h > MAX_EDGE {
            return bad(format!("size {w}x{h} exceeds {MAX_EDGE} px per edge"));
        }
        if self.spp == 0 || self.spp > MAX_SPP {
            return bad(format!("spp must be 1 to {MAX_SPP}, got {}", self.spp));
        }
        Ok(())
    }

    /// Output size in device pixels.
    pub fn pixel_size(&self) -> (u32, u32) {
        let scale = |v: u32| (v as f64 * self.pixel_ratio).round() as u32;
        (scale(self.size[0]), scale(self.size[1]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_validation() {
        let o = RenderOptions3d {
            size: [100, 50],
            pixel_ratio: 2.0,
            ..Default::default()
        };
        assert_eq!(o.pixel_size(), (200, 100));
        assert!(o.validate().is_ok());
        for bad in [
            RenderOptions3d {
                size: [0, 10],
                ..Default::default()
            },
            RenderOptions3d {
                pixel_ratio: 0.0,
                ..Default::default()
            },
            RenderOptions3d {
                size: [20000, 10],
                ..Default::default()
            },
        ] {
            assert!(matches!(
                bad.validate(),
                Err(Render3dError::InvalidOptions(_))
            ));
        }
        assert_eq!("presentation".parse::<Quality>(), Ok(Quality::Presentation));
        assert!("final".parse::<Quality>().is_err());
    }
}
