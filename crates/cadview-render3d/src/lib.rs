//! cadview-render3d: backends for `cadview_core::scene3d` frames.
//!
//! cadview-core decides what a 3D view shows (meshes with derived normals,
//! which edges are drawn, camera matrices, clip range, light directions,
//! shading formula and tone curve) and hands over a [`Frame3d`]. This crate
//! only turns that frame into pixels. It never looks at a part or a kernel.
//!
//! - [`Renderer3d`]: the backend trait. A backend draws a frame into a
//!   texture on a caller's device. It may need several calls to finish (a
//!   progressive path tracer accumulates samples), and says so through
//!   [`Progress`].
//! - [`raster::RasterRenderer`] (wasm32 + native): the working-quality
//!   backend: depth-tested triangles with 4x MSAA, per-pixel shading that
//!   mirrors `cadview_core::scene3d::shade`, the tone curve in the shader,
//!   sharp edges and silhouettes as screen-space lines with an outline
//!   that contrasts with the background, screen-space ambient occlusion,
//!   a ground plane, clip planes with filled cut faces, and a cache of
//!   prepared meshes (repeated parts drawn instanced).
//! - [`options::RenderOptions3d`]: size, pixel ratio, background override,
//!   quality level.
//! - `render_rgba` / `render_png` (native): headless render on the
//!   process-wide `cadview_render` GPU context, the same device the 2D
//!   renderer uses. `contact_sheet` lays several renders out as one
//!   labelled grid.

pub mod options;
pub mod raster;

#[cfg(not(target_arch = "wasm32"))]
mod headless;
#[cfg(not(target_arch = "wasm32"))]
pub use headless::*;

pub use cadview_core::scene3d::{Frame3d, Scene3d};
pub use options::{Quality, RenderOptions3d};

/// The texture format every [`Renderer3d`] draws into: 8-bit sRGB, so the
/// bytes read back are the image (the hardware encodes linear shader
/// output to sRGB).
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// A 3D rendering failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Render3dError {
    /// No backend for this quality level yet.
    NotAvailable(Quality),
    /// Options out of range.
    InvalidOptions(String),
    /// The GPU is missing or failed.
    Gpu(String),
}

impl std::fmt::Display for Render3dError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Render3dError::NotAvailable(q) => {
                write!(f, "the {} render is not available yet", q.as_str())
            }
            Render3dError::InvalidOptions(e) => write!(f, "render options: {e}"),
            Render3dError::Gpu(e) => write!(f, "GPU: {e}"),
        }
    }
}

impl std::error::Error for Render3dError {}

/// How far a frame got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// The image in the target is final.
    Done,
    /// The target holds a preview; call again with the same frame to
    /// refine it. `samples` of `target` are accumulated so far.
    Partial { samples: u32, target: u32 },
}

/// The device a renderer draws on (owned by the caller: one per process or
/// page, shared with the 2D renderer).
#[derive(Clone, Copy)]
pub struct Gpu<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
}

/// Where a renderer draws: a single-sample [`TARGET_FORMAT`] texture view
/// of `width` x `height` that allows `RENDER_ATTACHMENT`.
#[derive(Clone, Copy)]
pub struct Target<'a> {
    pub view: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

/// A 3D backend.
///
/// The caller owns the device, the target and the frame; the renderer owns
/// only its pipelines and scratch resources. A raster backend finishes in
/// one call. A progressive one returns [`Progress::Partial`] until it has
/// its samples, keeping its accumulation between calls; [`Self::reset`]
/// discards it when the scene or camera changes. That is all a viewport
/// needs to stay demand-driven: call while `Partial`, stop at `Done`.
pub trait Renderer3d {
    /// The quality level this backend produces.
    fn quality(&self) -> Quality;

    /// Draw `frame` (or the next refinement of it) into `target`.
    fn render(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Frame3d<'_>,
        target: Target<'_>,
    ) -> Result<Progress, Render3dError>;

    /// Forget accumulated work. The default does nothing (raster backends
    /// keep none).
    fn reset(&mut self) {}
}
