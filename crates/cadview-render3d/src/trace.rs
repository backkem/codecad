//! The presentation-quality backend: a progressive path tracer on plain
//! wgpu compute (no ray-tracing extensions, so it can run on WebGPU).
//!
//! - **Geometry.** Every distinct mesh (shared `Arc<RenderMesh>`) gets a
//!   bounding volume hierarchy over its triangles in its own frame, built
//!   once on the CPU ([`crate::bvh`]) and kept while the mesh is in use.
//!   Items are instances: a second hierarchy over their world boxes points
//!   at the mesh hierarchies, and a ray is moved into each mesh's frame by
//!   the item's rigid placement. The ground is an analytic plane.
//! - **Light.** The core's environment ([`cadview_core::scene3d::Environment`]):
//!   a dome reached by sampling the surfaces' BSDFs, and a sun sampled
//!   directly at every surface (next-event estimation), the two combined
//!   by multiple importance sampling (power heuristic).
//! - **Surfaces.** Lambert diffuse under a GGX specular lobe (exact
//!   dielectric Fresnel from the material's index of refraction, Schlick
//!   for metals; anisotropic for brushed patterns), an optional clear coat,
//!   and smooth or rough glass. The patterns are the core's, evaluated per
//!   hit in the mesh frame.
//! - **Paths** end at an escape, at [`TraceSettings::max_bounces`], or by
//!   Russian roulette from the fourth bounce. Indirect contributions (two
//!   or more scattering events) are clamped in luminance at
//!   [`TraceSettings::clamp`] times the luminance of a white surface lit by
//!   the open environment: that removes fireflies (rare paths to the sun
//!   through glass or off glossy metal) and never touches direct light.
//! - **Progress.** Each call adds a pass of samples to a per-pixel running
//!   sum, one sample at a time in sample order, so the result does not
//!   depend on how the samples are split into passes; random numbers come
//!   from PCG hashes of (pixel, sample, seed). A full-screen pass then
//!   divides by the sample count, applies the exposure and the core's tone
//!   curve, and writes the target.

use crate::bvh::{Aabb, Bvh, Node};
use crate::{Gpu, Progress, Quality, Render3dError, Renderer3d, Target, TARGET_FORMAT};
use cadview_core::scene3d::{mat_inverse, Frame3d, Mat4, Pattern, RenderMesh, MAX_CLIP_PLANES};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Default samples per pixel of a headless render. Measured on the nine 3D
/// examples at 960 x 640 against 4096-sample references: 256 samples leave
/// 1.1 to 2.9 levels (8-bit, root mean square) of noise, visible as grain
/// in shadowed and glossy areas; 1024 leave 0.7 to 1.6, visible only inside
/// polished metal bores. 1024 took 1.2 to 2.4 s each on one discrete GPU.
pub const DEFAULT_SPP: u32 = 1024;

/// Settings of one presentation render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceSettings {
    /// Samples per pixel to reach.
    pub spp: u32,
    /// Seed of the random numbers.
    pub seed: u32,
    /// Most scattering events per path.
    pub max_bounces: u32,
    /// Indirect light is clamped at this many times the luminance of a
    /// white surface lit by the open environment; 0 turns the clamp off.
    pub clamp: f64,
}

impl Default for TraceSettings {
    fn default() -> Self {
        TraceSettings {
            spp: DEFAULT_SPP,
            seed: 0,
            max_bounces: 16,
            clamp: 10.0,
        }
    }
}

/// Where a render's time went.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TraceStats {
    /// Samples per pixel accumulated.
    pub samples: u32,
    /// Compute passes run.
    pub passes: u32,
    /// Triangles and items traced.
    pub triangles: usize,
    pub instances: usize,
    /// Mesh hierarchies built for this frame (the rest came from the cache).
    pub meshes_built: usize,
    /// CPU time building hierarchies (ms).
    pub bvh_ms: f64,
    /// CPU time packing and uploading buffers (ms).
    pub upload_ms: f64,
    /// Wall time of the compute passes, waited for (ms); 0 where the
    /// backend cannot wait (wasm32).
    pub trace_ms: f64,
}

/// Bytes of one item of `Globals` in `trace.wgsl`.
const GLOBALS_SIZE: usize = (14 + MAX_CLIP_PLANES) * 16;
const INSTANCE_SIZE: usize = 2 * 64 + 16;
const MATERIAL_SIZE: usize = 9 * 16;
const TRI_SIZE: usize = 48;
/// A cached mesh hierarchy unused for this many frames is dropped.
const CACHE_FRAMES: u64 = 3;
/// Aim for compute passes of about this long (native), well under the
/// operating systems' GPU watchdogs.
#[cfg(not(target_arch = "wasm32"))]
const PASS_MS: f64 = 60.0;
/// Most samples per pixel in one pass.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PASS_SPP: u32 = 64;

/// One mesh's hierarchy and triangles in leaf order, mesh frame.
struct MeshBvh {
    /// Keeps the allocation (and so the cache key) alive.
    _mesh: Arc<RenderMesh>,
    nodes: Vec<Node>,
    /// p0, e1, e2 as f32 with the vertex indices in the w components.
    tris: Vec<[f32; 12]>,
    normals: Vec<[f32; 4]>,
    /// Box of the mesh (its root).
    bounds: Aabb,
    last_used: u64,
}

impl MeshBvh {
    fn build(mesh: &Arc<RenderMesh>, now: u64) -> MeshBvh {
        let pos: Vec<[f32; 3]> = mesh
            .positions
            .iter()
            .map(|p| [p.x as f32, p.y as f32, p.z as f32])
            .collect();
        let boxes: Vec<Aabb> = mesh
            .triangles
            .iter()
            .map(|t| {
                let mut b = Aabb::EMPTY;
                for &i in t {
                    b.grow_point(pos[i as usize].map(f64::from));
                }
                b
            })
            .collect();
        let bvh = Bvh::build(&boxes);
        let tris = bvh
            .order
            .iter()
            .map(|&ti| {
                let [a, b, c] = mesh.triangles[ti as usize];
                let (p0, p1, p2) = (pos[a as usize], pos[b as usize], pos[c as usize]);
                let e1: [f32; 3] = std::array::from_fn(|k| p1[k] - p0[k]);
                let e2: [f32; 3] = std::array::from_fn(|k| p2[k] - p0[k]);
                [
                    p0[0],
                    p0[1],
                    p0[2],
                    f32::from_bits(a),
                    e1[0],
                    e1[1],
                    e1[2],
                    f32::from_bits(b),
                    e2[0],
                    e2[1],
                    e2[2],
                    f32::from_bits(c),
                ]
            })
            .collect();
        let normals = mesh
            .normals
            .iter()
            .map(|n| [n.x as f32, n.y as f32, n.z as f32, 0.0])
            .collect();
        let mut bounds = Aabb::EMPTY;
        for b in &boxes {
            bounds.grow(b);
        }
        MeshBvh {
            _mesh: Arc::clone(mesh),
            nodes: bvh.nodes,
            tris,
            normals,
            bounds,
            last_used: now,
        }
    }
}

struct State {
    /// Everything that changes the image, packed; a different key starts
    /// a new accumulation.
    key: Vec<u8>,
    width: u32,
    height: u32,
    /// The globals without the per-pass sample range.
    globals: Vec<u8>,
    globals_buf: wgpu::Buffer,
    accum: wgpu::Buffer,
    resolve_buf: wgpu::Buffer,
    bind: wgpu::BindGroup,
    resolve_bind: wgpu::BindGroup,
    samples: u32,
    /// Keeps the scene buffers alive with the bind group.
    _buffers: Vec<wgpu::Buffer>,
}

/// The path tracer: pipelines, the cache of mesh hierarchies and the
/// current accumulation.
pub struct PathTracer {
    pipeline: wgpu::ComputePipeline,
    resolve: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    resolve_layout: wgpu::BindGroupLayout,
    settings: TraceSettings,
    cache: HashMap<usize, MeshBvh>,
    frame_no: u64,
    state: Option<State>,
    stats: TraceStats,
    pass_spp: u32,
}

fn push_f32(buf: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn push_u32(buf: &mut Vec<u8>, values: &[u32]) {
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn mat_f32(m: &Mat4) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] = m[c][r] as f32;
        }
    }
    out
}

fn v4(c: [f64; 3], w: f64) -> [f32; 4] {
    [c[0] as f32, c[1] as f32, c[2] as f32, w as f32]
}

fn lum(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mesh_key(m: &Arc<RenderMesh>) -> usize {
    Arc::as_ptr(m) as usize
}

/// The world box of a mesh-frame box under `model`, padded for the f32
/// transforms in the shader.
fn world_box(b: &Aabb, model: &Mat4) -> Aabb {
    let mut out = Aabb::EMPTY;
    for i in 0..8 {
        let p = [
            if i & 1 == 0 { b.lo[0] } else { b.hi[0] },
            if i & 2 == 0 { b.lo[1] } else { b.hi[1] },
            if i & 4 == 0 { b.lo[2] } else { b.hi[2] },
        ];
        let w: [f64; 3] = std::array::from_fn(|r| {
            model[0][r] * p[0] + model[1][r] * p[1] + model[2][r] * p[2] + model[3][r]
        });
        out.grow_point(w);
    }
    let size = (0..3)
        .map(|k| {
            (out.hi[k] - out.lo[k])
                .abs()
                .max(out.hi[k].abs().max(out.lo[k].abs()))
        })
        .fold(0.0, f64::max);
    let pad = 1e-5 * size + 1e-9;
    for k in 0..3 {
        out.lo[k] -= pad;
        out.hi[k] += pad;
    }
    out
}

impl PathTracer {
    /// Compile the shaders and build the pipelines on `device`.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d_trace"),
            source: wgpu::ShaderSource::Wgsl(include_str!("trace.wgsl").into()),
        });
        let resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d_resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("resolve.wgsl").into()),
        });
        let storage = |binding, read_only, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let cs = wgpu::ShaderStages::COMPUTE;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d_trace"),
            entries: &[
                uniform(cs),
                storage(1, true, cs),
                storage(2, true, cs),
                storage(3, true, cs),
                storage(4, true, cs),
                storage(5, true, cs),
                storage(6, true, cs),
                storage(7, false, cs),
            ],
        });
        let fs = wgpu::ShaderStages::FRAGMENT;
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d_resolve"),
            entries: &[uniform(fs), storage(1, true, fs)],
        });
        let pl = |l: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("render3d_trace"),
                bind_group_layouts: &[Some(l)],
                immediate_size: 0,
            })
        };
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("render3d_trace"),
            layout: Some(&pl(&layout)),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let resolve = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render3d_resolve"),
            layout: Some(&pl(&resolve_layout)),
            vertex: wgpu::VertexState {
                module: &resolve_shader,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &resolve_shader,
                entry_point: Some("fs_resolve"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        PathTracer {
            pipeline,
            resolve,
            layout,
            resolve_layout,
            settings: TraceSettings::default(),
            cache: HashMap::new(),
            frame_no: 0,
            state: None,
            stats: TraceStats::default(),
            pass_spp: 1,
        }
    }

    /// The settings for the next frames. A new seed, bounce limit or clamp
    /// starts the accumulation over; a new sample count only moves the
    /// target.
    pub fn set_settings(&mut self, s: TraceSettings) {
        let old = self.settings;
        self.settings = TraceSettings {
            spp: s.spp.max(1),
            ..s
        };
        if (old.seed, old.max_bounces, old.clamp) != (s.seed, s.max_bounces, s.clamp) {
            self.reset();
        }
    }

    pub fn settings(&self) -> TraceSettings {
        self.settings
    }

    /// Where the last frame's time went.
    pub fn stats(&self) -> TraceStats {
        self.stats
    }

    /// Samples per pixel accumulated so far.
    pub fn samples(&self) -> u32 {
        self.state.as_ref().map_or(0, |s| s.samples)
    }

    /// Mesh hierarchies held in the cache.
    pub fn cached_meshes(&self) -> usize {
        self.cache.len()
    }

    /// The mean linear radiance per pixel accumulated so far (before
    /// exposure and tone curve), row-major from the top-left. Empty before
    /// the first frame. Waits for the GPU.
    pub fn radiance(&self, gpu: Gpu<'_>) -> Result<Vec<[f32; 3]>, Render3dError> {
        let Some(st) = &self.state else {
            return Ok(Vec::new());
        };
        let size = st.width as u64 * st.height as u64 * 16;
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d_trace_readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render3d_trace_readback"),
            });
        enc.copy_buffer_to_buffer(&st.accum, 0, &read, 0, size);
        gpu.queue.submit(Some(enc.finish()));
        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| Render3dError::Gpu(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| Render3dError::Gpu(format!("readback: {e}")))?
            .map_err(|e| Render3dError::Gpu(format!("readback map: {e}")))?;
        let n = st.samples.max(1) as f32;
        let out = slice.get_mapped_range().map(|data| {
            data.as_chunks::<16>()
                .0
                .iter()
                .map(|c| {
                    let f = |k: usize| f32::from_le_bytes(c[4 * k..4 * k + 4].try_into().unwrap());
                    [f(0) / n, f(1) / n, f(2) / n]
                })
                .collect::<Vec<_>>()
        });
        read.unmap();
        out.map_err(|e| Render3dError::Gpu(format!("readback view: {e}")))
    }

    /// Make sure every mesh of `frame` has its hierarchy; drop stale ones.
    fn prepare_meshes(&mut self, frame: &Frame3d<'_>) -> (usize, f64) {
        self.frame_no += 1;
        let now = self.frame_no;
        let mut built = 0;
        #[cfg(not(target_arch = "wasm32"))]
        let t = std::time::Instant::now();
        for it in &frame.items {
            if it.mesh.triangles.is_empty() {
                continue;
            }
            let key = mesh_key(it.mesh);
            if let Some(c) = self.cache.get_mut(&key) {
                c.last_used = now;
                continue;
            }
            self.cache.insert(key, MeshBvh::build(it.mesh, now));
            built += 1;
        }
        self.cache.retain(|_, c| c.last_used + CACHE_FRAMES > now);
        #[cfg(not(target_arch = "wasm32"))]
        let ms = t.elapsed().as_secs_f64() * 1e3;
        #[cfg(target_arch = "wasm32")]
        let ms = 0.0;
        (built, ms)
    }

    /// The globals block for `frame` (sample range left zero) and the
    /// clamp it uses.
    fn globals(&self, frame: &Frame3d<'_>) -> Vec<u8> {
        let mut g = Vec::with_capacity(GLOBALS_SIZE);
        // Camera from the inverse view matrix (rigid) and the projection.
        let inv = mat_inverse(&frame.view).unwrap_or(cadview_core::scene3d::IDENTITY4);
        let col = |c: usize| [inv[c][0], inv[c][1], inv[c][2]];
        let (right, up, back, eye) = (col(0), col(1), col(2), col(3));
        let p = &frame.projection;
        let perspective = frame.eye[3] != 0.0;
        let (hw, hh) = (1.0 / p[0][0], 1.0 / p[1][1]);
        let origin = if perspective {
            eye
        } else {
            // Start at the near plane (it may lie behind the eye).
            let near = frame.near_far.0;
            std::array::from_fn(|k| eye[k] - back[k] * near)
        };
        push_f32(&mut g, &v4(origin, if perspective { 1.0 } else { 0.0 }));
        push_f32(&mut g, &v4(right, hw));
        push_f32(&mut g, &v4(up, hh));
        push_f32(&mut g, &v4(back, 0.0));
        push_u32(&mut g, &[frame.width, frame.height, 0, 0]);
        let env = &frame.environment;
        let mut flags = 0u32;
        if frame.ground.is_some() {
            flags |= 1;
        }
        if env.sun.is_some() {
            flags |= 2;
        }
        if frame.clip_mode == cadview_core::scene3d::ClipMode::All {
            flags |= 4;
        }
        let clips = frame.clip_planes.len().min(MAX_CLIP_PLANES) as u32;
        push_u32(
            &mut g,
            &[self.settings.seed, self.settings.max_bounces, flags, clips],
        );
        match env.sun {
            Some(s) => {
                push_f32(
                    &mut g,
                    &v4([s.direction.x, s.direction.y, s.direction.z], s.cos_radius),
                );
                push_f32(&mut g, &v4(s.radiance, s.solid_angle()));
            }
            None => {
                push_f32(&mut g, &[0.0, 0.0, 1.0, 2.0]);
                push_f32(&mut g, &[0.0, 0.0, 0.0, 1.0]);
            }
        }
        let dome = env.dome;
        push_f32(&mut g, &v4(dome.zenith, 0.0));
        push_f32(&mut g, &v4(dome.horizon, 0.0));
        push_f32(&mut g, &v4(dome.ground, 0.0));
        let (gc, gz) = frame.ground.map_or(([0.0; 3], 0.0), |gr| (gr.color, gr.z));
        push_f32(&mut g, &v4(gc.map(|c| c.clamp(0.0, 1.0)), gz));
        // A white surface in the open: the dome's horizontal irradiance
        // plus the sun's, over pi.
        let mut e = dome.dome_horizontal_irradiance();
        if let Some(s) = env.sun {
            // Irradiance facing the sun per unit radiance: pi sin^2 r.
            let en = std::f64::consts::PI * (1.0 - s.cos_radius * s.cos_radius);
            for (k, ek) in e.iter_mut().enumerate() {
                *ek += s.radiance[k] * en * s.direction.z.max(0.0);
            }
        }
        let white = lum(e) / std::f64::consts::PI;
        let white = if white.is_finite() && white > 1e-6 {
            white
        } else {
            1.0
        };
        let clamp = if self.settings.clamp > 0.0 {
            self.settings.clamp * white
        } else {
            0.0
        };
        push_f32(&mut g, &[clamp as f32, 0.0, 0.0, 0.0]);
        let instances = frame
            .items
            .iter()
            .filter(|it| !it.mesh.triangles.is_empty())
            .count() as u32;
        push_u32(&mut g, &[instances, 0, 0, 0]);
        for i in 0..MAX_CLIP_PLANES {
            let c = frame
                .clip_planes
                .get(i)
                .copied()
                .unwrap_or([0.0, 0.0, 0.0, -1.0]);
            push_f32(&mut g, &c.map(|v| v as f32));
        }
        debug_assert_eq!(g.len(), GLOBALS_SIZE);
        g
    }

    /// Per-item materials, in frame order.
    fn materials(frame: &Frame3d<'_>) -> Vec<u8> {
        let mut out = Vec::with_capacity(frame.items.len().max(1) * MATERIAL_SIZE);
        for it in &frame.items {
            let m = it.material;
            let pf = it.pattern;
            let (kind, scale, color, strength, warp, aniso, circular) = match m.pattern {
                Pattern::Flat => (0.0, 1.0, [0.0; 3], 0.0, 0.0, 0.0, 0.0),
                Pattern::Bands(b) => (1.0, b.spacing, b.color, b.strength, b.warp, 0.0, 0.0),
                Pattern::Brushed(b) => (
                    2.0,
                    b.scale,
                    [0.0; 3],
                    b.streaks,
                    0.0,
                    b.anisotropy,
                    if b.circular { 1.0 } else { 0.0 },
                ),
            };
            push_f32(&mut out, &v4(m.base_color, m.metallic));
            push_f32(
                &mut out,
                &[
                    m.roughness as f32,
                    m.transmission as f32,
                    m.ior as f32,
                    m.clearcoat as f32,
                ],
            );
            push_f32(&mut out, &[m.clearcoat_roughness as f32, kind, 0.0, 0.0]);
            push_f32(&mut out, &v4(m.emission, 0.0));
            push_f32(&mut out, &v4([pf.axis.x, pf.axis.y, pf.axis.z], scale));
            push_f32(&mut out, &v4([pf.origin.x, pf.origin.y, pf.origin.z], 0.0));
            out.truncate(out.len() - 4);
            out.extend_from_slice(&pf.seed.to_le_bytes());
            push_f32(&mut out, &v4(color, strength));
            push_f32(&mut out, &[warp as f32, aniso as f32, circular, 0.0]);
            push_f32(&mut out, &v4(it.section_color, 0.0));
        }
        if out.is_empty() {
            out.resize(MATERIAL_SIZE, 0);
        }
        out
    }

    /// Build the scene buffers and a fresh accumulation for `frame`.
    fn rebuild(&mut self, gpu: Gpu<'_>, frame: &Frame3d<'_>, key: Vec<u8>, globals: Vec<u8>) {
        let (built, bvh_ms) = self.prepare_meshes(frame);
        #[cfg(not(target_arch = "wasm32"))]
        let t = std::time::Instant::now();
        let device = gpu.device;
        // Mesh hierarchies, concatenated in first-seen order.
        let mut blas: Vec<u8> = Vec::new();
        let mut tris: Vec<u8> = Vec::new();
        let mut normals: Vec<u8> = Vec::new();
        let mut roots: HashMap<usize, u32> = HashMap::new();
        let (mut node_base, mut tri_base, mut vert_base) = (0u32, 0u32, 0u32);
        let mut triangles = 0usize;
        for it in &frame.items {
            if it.mesh.triangles.is_empty() {
                continue;
            }
            let key = mesh_key(it.mesh);
            if roots.contains_key(&key) {
                continue;
            }
            let m = &self.cache[&key];
            roots.insert(key, node_base);
            for n in &m.nodes {
                let first = if n.count > 0 {
                    n.first + tri_base
                } else {
                    n.first + node_base
                };
                push_f32(&mut blas, &n.lo);
                push_u32(&mut blas, &[first]);
                push_f32(&mut blas, &n.hi);
                push_u32(&mut blas, &[n.count]);
            }
            for t in &m.tris {
                for (k, v) in t.iter().enumerate() {
                    let v = if k % 4 == 3 {
                        f32::from_bits(v.to_bits() + vert_base)
                    } else {
                        *v
                    };
                    tris.extend_from_slice(&v.to_le_bytes());
                }
            }
            for n in &m.normals {
                push_f32(&mut normals, n);
            }
            node_base += m.nodes.len() as u32;
            tri_base += m.tris.len() as u32;
            vert_base += m.normals.len() as u32;
            triangles += m.tris.len();
        }
        // Instances and the top-level hierarchy over their world boxes.
        let mut inst_boxes = Vec::new();
        let mut inst: Vec<Vec<u8>> = Vec::new();
        for (i, it) in frame.items.iter().enumerate() {
            if it.mesh.triangles.is_empty() {
                continue;
            }
            let m = &self.cache[&mesh_key(it.mesh)];
            inst_boxes.push(world_box(&m.bounds, &it.model));
            let mut b = Vec::with_capacity(INSTANCE_SIZE);
            let to_object = mat_inverse(&it.model).unwrap_or(cadview_core::scene3d::IDENTITY4);
            push_f32(&mut b, &mat_f32(&to_object));
            push_f32(&mut b, &mat_f32(&it.model));
            push_u32(&mut b, &[roots[&mesh_key(it.mesh)], i as u32, 0, 0]);
            inst.push(b);
        }
        let mut tlas: Vec<u8> = Vec::new();
        let mut instances: Vec<u8> = Vec::new();
        if !inst_boxes.is_empty() {
            // Leaves of one instance each (the shader reads a leaf's
            // `first` as the instance index). Splitting halves a run whose
            // boxes share a centre, so the depth cap allows 2^40 instances.
            let top = Bvh::build_with(&inst_boxes, 1);
            for n in &top.nodes {
                let first = if n.count > 0 {
                    assert_eq!(n.count, 1, "one instance per top-level leaf");
                    top.order[n.first as usize]
                } else {
                    n.first
                };
                push_f32(&mut tlas, &n.lo);
                push_u32(&mut tlas, &[first]);
                push_f32(&mut tlas, &n.hi);
                push_u32(&mut tlas, &[n.count]);
            }
            for b in &inst {
                instances.extend_from_slice(b);
            }
        }
        let materials = Self::materials(frame);
        let make = |label, mut data: Vec<u8>, usage| {
            if data.is_empty() {
                data.resize(32, 0);
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &data,
                usage,
            })
        };
        let st = wgpu::BufferUsages::STORAGE;
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d_trace_globals"),
            size: GLOBALS_SIZE as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tlas_buf = make("render3d_trace_tlas", tlas, st);
        let blas_buf = make("render3d_trace_blas", blas, st);
        let tris_buf = make("render3d_trace_tris", tris, st);
        let normals_buf = make("render3d_trace_normals", normals, st);
        let inst_count = inst.len();
        let inst_buf = make("render3d_trace_instances", instances, st);
        let mat_buf = make("render3d_trace_materials", materials, st);
        let (w, h) = (frame.width, frame.height);
        let accum = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d_trace_accum"),
            size: w as u64 * h as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render3d_trace_resolve"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        fn entry(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            }
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render3d_trace"),
            layout: &self.layout,
            entries: &[
                entry(0, &globals_buf),
                entry(1, &tlas_buf),
                entry(2, &blas_buf),
                entry(3, &tris_buf),
                entry(4, &normals_buf),
                entry(5, &inst_buf),
                entry(6, &mat_buf),
                entry(7, &accum),
            ],
        });
        let resolve_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render3d_resolve"),
            layout: &self.resolve_layout,
            entries: &[entry(0, &resolve_buf), entry(1, &accum)],
        });
        #[cfg(not(target_arch = "wasm32"))]
        let upload_ms = t.elapsed().as_secs_f64() * 1e3;
        #[cfg(target_arch = "wasm32")]
        let upload_ms = 0.0;
        self.stats = TraceStats {
            samples: 0,
            passes: 0,
            triangles,
            instances: inst_count,
            meshes_built: built,
            bvh_ms,
            upload_ms,
            trace_ms: 0.0,
        };
        self.pass_spp = 1;
        self.state = Some(State {
            key,
            width: w,
            height: h,
            globals,
            globals_buf,
            accum,
            resolve_buf,
            bind,
            resolve_bind,
            samples: 0,
            _buffers: vec![tlas_buf, blas_buf, tris_buf, normals_buf, inst_buf, mat_buf],
        });
    }

    /// The key of everything that changes the image of `frame`.
    fn key_of(&self, frame: &Frame3d<'_>, globals: &[u8]) -> Vec<u8> {
        let mut k = globals.to_vec();
        k.extend_from_slice(&Self::materials(frame));
        for it in &frame.items {
            k.extend_from_slice(&(mesh_key(it.mesh) as u64).to_le_bytes());
            for c in &it.model {
                for v in c {
                    k.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        k.extend_from_slice(&frame.exposure.to_le_bytes());
        k
    }
}

impl Renderer3d for PathTracer {
    fn quality(&self) -> Quality {
        Quality::Presentation
    }

    fn render(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Frame3d<'_>,
        target: Target<'_>,
    ) -> Result<Progress, Render3dError> {
        let (w, h) = (target.width, target.height);
        if w == 0 || h == 0 || w != frame.width || h != frame.height {
            return Err(Render3dError::InvalidOptions(format!(
                "target {w}x{h} must match the frame {}x{}",
                frame.width, frame.height
            )));
        }
        if frame.clip_planes.len() > MAX_CLIP_PLANES {
            return Err(Render3dError::InvalidOptions(format!(
                "at most {MAX_CLIP_PLANES} clip planes, got {}",
                frame.clip_planes.len()
            )));
        }
        let limit = gpu.device.limits().max_storage_buffer_binding_size;
        let tris: usize = {
            let mut seen = std::collections::HashSet::new();
            frame
                .items
                .iter()
                .filter(|it| seen.insert(mesh_key(it.mesh)))
                .map(|it| it.mesh.triangles.len())
                .sum()
        };
        if (tris * TRI_SIZE) as u64 > limit || w as u64 * h as u64 * 16 > limit {
            return Err(Render3dError::InvalidOptions(format!(
                "the scene ({tris} triangles at {w}x{h}) exceeds the device's {limit}-byte buffer limit"
            )));
        }
        let globals = self.globals(frame);
        let key = self.key_of(frame, &globals);
        if self.state.as_ref().is_none_or(|s| s.key != key) {
            self.rebuild(gpu, frame, key, globals);
        }
        let target_spp = self.settings.spp;
        let pass_spp = self.pass_spp;
        let st = self.state.as_mut().expect("built");
        if st.samples < target_spp {
            let n = pass_spp.min(target_spp - st.samples);
            let mut g = st.globals.clone();
            // size.z, size.w: first sample and samples of this pass.
            g[72..76].copy_from_slice(&st.samples.to_le_bytes());
            g[76..80].copy_from_slice(&n.to_le_bytes());
            gpu.queue.write_buffer(&st.globals_buf, 0, &g);
            let mut enc = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("render3d_trace"),
                });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("render3d_trace"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &st.bind, &[]);
                pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
            }
            #[cfg(not(target_arch = "wasm32"))]
            let t = std::time::Instant::now();
            gpu.queue.submit(Some(enc.finish()));
            st.samples += n;
            self.stats.samples = st.samples;
            self.stats.passes += 1;
            #[cfg(not(target_arch = "wasm32"))]
            {
                // Wait, time the pass and size the next one to PASS_MS.
                gpu.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .map_err(|e| Render3dError::Gpu(format!("poll: {e}")))?;
                let ms = t.elapsed().as_secs_f64() * 1e3;
                self.stats.trace_ms += ms;
                let per = (ms / n as f64).max(1e-3);
                self.pass_spp = ((PASS_MS / per) as u32).clamp(1, MAX_PASS_SPP);
            }
        }
        // Resolve: mean, exposure, tone curve.
        let mut r = Vec::with_capacity(32);
        push_u32(&mut r, &[w, h, 0, 0]);
        let scale = frame.exposure / st.samples.max(1) as f64;
        push_f32(&mut r, &[scale as f32, 0.0, 0.0, 0.0]);
        gpu.queue.write_buffer(&st.resolve_buf, 0, &r);
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render3d_resolve"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render3d_resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.resolve);
            pass.set_bind_group(0, &st.resolve_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(Some(enc.finish()));
        Ok(if st.samples >= target_spp {
            Progress::Done
        } else {
            Progress::Partial {
                samples: st.samples,
                target: target_spp,
            }
        })
    }

    fn reset(&mut self) {
        self.state = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_shader() {
        assert_eq!(GLOBALS_SIZE, 288);
        assert_eq!(INSTANCE_SIZE, 144);
        assert_eq!(MATERIAL_SIZE, 144);
        assert_eq!(std::mem::size_of::<[f32; 12]>(), TRI_SIZE);
        let b = Aabb {
            lo: [0.0; 3],
            hi: [1.0; 3],
        };
        let (lo, hi) = crate::bvh::node_box(&b);
        assert_eq!((lo, hi), ([0.0; 3], [1.0; 3]));
        let m = cadview_core::scene3d::placement_matrix(
            &cadview_core::solid::Placement::translation(10.0, 0.0, 0.0),
        );
        let wb = world_box(&b, &m);
        assert!(wb.lo[0] < 10.0 && wb.hi[0] > 11.0 && wb.hi[0] < 11.001);
    }
}
