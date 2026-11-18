//! The working-quality backend: a small wgpu rasterizer.
//!
//! Two render passes over a 4x MSAA colour target and a depth-stencil
//! buffer, resolved into the caller's single-sample [`TARGET_FORMAT`]
//! texture, with the ambient-occlusion passes in between:
//!
//! 1. **Surfaces.** Every part (stencil 1), depth-tested and shaded per
//!    pixel with the core's formula and tone curve. Faces get a
//!    slope-scaled depth bias so the lines on them win the depth test. With
//!    clip planes, front faces on the removed side are discarded and a line
//!    of the edge colour marks where a plane cuts them; back faces, which
//!    can then only be seen through a cut (meshes are closed), are drawn
//!    flat in the part's section colour (stencil 2), and a full-screen pass
//!    moves the depth of those pixels onto the cut itself, so the ground
//!    and the edges behind a cut face stay hidden. Then the ground square
//!    (stencil 0), which the clip planes do not cut.
//! 2. **Ambient occlusion** (when the frame has it): visibility per pixel
//!    from depth sample 0 into a half-float texture, then a 4 x 4 blur (see
//!    `ao.wgsl`).
//! 3. **Lines.** The visibility multiplied into the colour (not on cut
//!    faces); every item's visible edges as screen-space quads of the edge
//!    width, depth-tested without writing depth, pulled one pixel toward the
//!    eye; then, if the frame has an outline, the same edges a pixel wider
//!    in the outline colour, only where no part was drawn (stencil 0), so
//!    the background side of each silhouette contrasts with the background.
//!
//! Without clip planes, back faces are drawn with the normal flipped, so an
//! open or wrongly oriented mesh still shades.
//!
//! Meshes are prepared once: vertex and index buffers are cached per shared
//! mesh (`Arc<RenderMesh>`; the cache holds a clone, so the allocation and
//! its address stay valid) and dropped after a few frames unused. Items that
//! share a mesh are drawn in one instanced call, reading their matrices and
//! material from a storage buffer. Matrices are composed in f64 by the core
//! and cast to f32 here.

use crate::{Gpu, Progress, Quality, Render3dError, Renderer3d, Target, TARGET_FORMAT};
use cadview_core::scene3d::{
    ao_rotation, mat_inverse, mat_mul, specular_exponent, srgb_to_linear, Frame3d, Mat4,
    RenderMesh, MAX_CLIP_PLANES,
};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// MSAA sample count (supported for these formats on every WebGPU device).
pub const SAMPLE_COUNT: u32 = 4;
/// Depth-stencil format: the stencil tells parts (1), cut faces (2) and
/// the rest (0) apart.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;
/// Format of the ambient-occlusion textures.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// Byte size of one item in the storage buffer (two matrices, three vec4s).
const ITEM_SIZE: usize = 176;
/// Byte size of the globals block.
const GLOBALS_SIZE: usize = 2 * 64 + 6 * 16 + 4 * 16 + 5 * 16;
/// Byte size of the ambient-occlusion block.
const AO_SIZE: usize = 2 * 64 + 2 * 16 * 16 + 2 * 16;
/// Bytes per edge instance: two points and the item index.
const EDGE_SIZE: u64 = 28;
/// A cached mesh unused for this many frames is dropped.
const CACHE_FRAMES: u64 = 3;

const STENCIL_GROUND: u32 = 0;
const STENCIL_PART: u32 = 1;
const STENCIL_CUT: u32 = 2;

/// Pipelines, the mesh cache, and the targets of the last size drawn.
pub struct RasterRenderer {
    mesh: wgpu::RenderPipeline,
    mesh_front: wgpu::RenderPipeline,
    mesh_cut: wgpu::RenderPipeline,
    cut_depth: wgpu::RenderPipeline,
    ground: wgpu::RenderPipeline,
    edge: wgpu::RenderPipeline,
    outline: wgpu::RenderPipeline,
    ao: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    main_layout: wgpu::BindGroupLayout,
    ao_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    scratch: Option<Scratch>,
    cache: HashMap<usize, CachedMesh>,
    frame_no: u64,
}

struct Scratch {
    width: u32,
    height: u32,
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    depth_only: wgpu::TextureView,
    ao_raw: wgpu::TextureView,
    ao: wgpu::TextureView,
}

struct CachedMesh {
    /// Keeps the allocation (and so the key) alive.
    _mesh: Arc<RenderMesh>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    last_used: u64,
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

fn push_f32(buf: &mut Vec<u8>, values: &[f32]) {
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

fn rgb_w(c: [f64; 3], w: f64) -> [f32; 4] {
    [c[0] as f32, c[1] as f32, c[2] as f32, w as f32]
}

/// Stencil state that writes `reference` wherever the depth test passes.
fn stencil_write() -> wgpu::StencilState {
    let face = wgpu::StencilFaceState {
        compare: wgpu::CompareFunction::Always,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: wgpu::StencilOperation::Replace,
    };
    wgpu::StencilState {
        front: face,
        back: face,
        read_mask: 0xff,
        write_mask: 0xff,
    }
}

/// Stencil state that only tests against `reference` with `compare`.
fn stencil_test(compare: wgpu::CompareFunction) -> wgpu::StencilState {
    let face = wgpu::StencilFaceState {
        compare,
        fail_op: wgpu::StencilOperation::Keep,
        depth_fail_op: wgpu::StencilOperation::Keep,
        pass_op: wgpu::StencilOperation::Keep,
    };
    wgpu::StencilState {
        front: face,
        back: face,
        read_mask: 0xff,
        write_mask: 0,
    }
}

impl RasterRenderer {
    /// Compile the shaders and build every pipeline on `device`.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d_raster"),
            source: wgpu::ShaderSource::Wgsl(include_str!("raster.wgsl").into()),
        });
        let ao_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render3d_ao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ao.wgsl").into()),
        });
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let main_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d_raster"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: vf,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(GLOBALS_SIZE as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: vf,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(ITEM_SIZE as u64),
                    },
                    count: None,
                },
            ],
        });
        let ao_texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let ao_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d_ao"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(AO_SIZE as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: true,
                    },
                    count: None,
                },
                ao_texture(2),
            ],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render3d_composite"),
            entries: &[ao_texture(2)],
        });
        let layout_of = |l: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("render3d"),
                bind_group_layouts: &[Some(l)],
                immediate_size: 0,
            })
        };
        let main_pl = layout_of(&main_layout);
        let ao_pl = layout_of(&ao_layout);
        let composite_pl = layout_of(&composite_layout);

        let multisample = wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let opaque = [Some(wgpu::ColorTargetState {
            format: TARGET_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        // dst * src: multiply what is drawn by the visibility.
        let multiply = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::Src,
            operation: wgpu::BlendOperation::Add,
        };
        let multiplied = [Some(wgpu::ColorTargetState {
            format: TARGET_FORMAT,
            blend: Some(wgpu::BlendState {
                color: multiply,
                alpha: wgpu::BlendComponent::REPLACE,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let ao_target = [Some(wgpu::ColorTargetState {
            format: AO_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let mesh_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 24,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &mesh_attrs,
        })];
        let edge_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32];
        let edge_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: EDGE_SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &edge_attrs,
        })];
        let face_bias = wgpu::DepthBiasState {
            constant: 0,
            slope_scale: 1.0,
            clamp: 0.0,
        };

        struct Desc<'a> {
            label: &'a str,
            module: &'a wgpu::ShaderModule,
            layout: &'a wgpu::PipelineLayout,
            vs: &'a str,
            fs: &'a str,
            buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
            cull: Option<wgpu::Face>,
            depth: Option<wgpu::DepthStencilState>,
            targets: &'a [Option<wgpu::ColorTargetState>],
            multisample: wgpu::MultisampleState,
        }
        let build = |d: Desc<'_>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(d.label),
                layout: Some(d.layout),
                vertex: wgpu::VertexState {
                    module: d.module,
                    entry_point: Some(d.vs),
                    compilation_options: Default::default(),
                    buffers: d.buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: d.cull,
                    ..Default::default()
                },
                depth_stencil: d.depth,
                multisample: d.multisample,
                fragment: Some(wgpu::FragmentState {
                    module: d.module,
                    entry_point: Some(d.fs),
                    compilation_options: Default::default(),
                    targets: d.targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let surface = |bias| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: stencil_write(),
                bias,
            })
        };
        let overlay = |depth: wgpu::CompareFunction, stencil: wgpu::CompareFunction| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(depth),
                stencil: stencil_test(stencil),
                bias: Default::default(),
            })
        };
        let mesh_desc = |label, fs, cull, bias| Desc {
            label,
            module: &shader,
            layout: &main_pl,
            vs: "vs_mesh",
            fs,
            buffers: &mesh_buffers,
            cull,
            depth: surface(bias),
            targets: &opaque,
            multisample,
        };
        let mesh = build(mesh_desc("render3d_mesh", "fs_mesh", None, face_bias));
        let mesh_front = build(mesh_desc(
            "render3d_mesh_front",
            "fs_mesh_clip",
            Some(wgpu::Face::Back),
            face_bias,
        ));
        // Back faces lose every tie with the front faces at a silhouette
        // (their depth is moved onto the cut afterwards anyway).
        let mesh_cut = build(mesh_desc(
            "render3d_mesh_cut",
            "fs_cap",
            Some(wgpu::Face::Front),
            wgpu::DepthBiasState {
                constant: 64,
                slope_scale: 4.0,
                clamp: 0.0,
            },
        ));
        let no_color = [Some(wgpu::ColorTargetState {
            format: TARGET_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        })];
        let cut_depth = build(Desc {
            label: "render3d_cut_depth",
            module: &shader,
            layout: &main_pl,
            vs: "vs_full",
            fs: "fs_cut_depth",
            buffers: &[],
            cull: None,
            depth: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: stencil_test(wgpu::CompareFunction::Equal),
                bias: Default::default(),
            }),
            targets: &no_color,
            multisample,
        });
        let ground = build(Desc {
            label: "render3d_ground",
            module: &shader,
            layout: &main_pl,
            vs: "vs_ground",
            fs: "fs_ground",
            buffers: &[],
            cull: None,
            depth: surface(Default::default()),
            targets: &opaque,
            multisample,
        });
        let edge = build(Desc {
            label: "render3d_edges",
            module: &shader,
            layout: &main_pl,
            vs: "vs_edge",
            fs: "fs_edge",
            buffers: &edge_buffers,
            cull: None,
            depth: overlay(
                wgpu::CompareFunction::LessEqual,
                wgpu::CompareFunction::Always,
            ),
            targets: &opaque,
            multisample,
        });
        let outline = build(Desc {
            label: "render3d_outline",
            module: &shader,
            layout: &main_pl,
            vs: "vs_outline",
            fs: "fs_outline",
            buffers: &edge_buffers,
            cull: None,
            depth: overlay(
                wgpu::CompareFunction::LessEqual,
                wgpu::CompareFunction::Equal,
            ),
            targets: &opaque,
            multisample,
        });
        let single = wgpu::MultisampleState::default();
        let ao = build(Desc {
            label: "render3d_ao",
            module: &ao_shader,
            layout: &ao_pl,
            vs: "vs_full",
            fs: "fs_ao",
            buffers: &[],
            cull: None,
            depth: None,
            targets: &ao_target,
            multisample: single,
        });
        let blur = build(Desc {
            label: "render3d_ao_blur",
            module: &ao_shader,
            layout: &ao_pl,
            vs: "vs_full",
            fs: "fs_blur",
            buffers: &[],
            cull: None,
            depth: None,
            targets: &ao_target,
            multisample: single,
        });
        let composite = build(Desc {
            label: "render3d_ao_composite",
            module: &ao_shader,
            layout: &composite_pl,
            vs: "vs_full",
            fs: "fs_composite",
            buffers: &[],
            cull: None,
            depth: overlay(
                wgpu::CompareFunction::Always,
                wgpu::CompareFunction::NotEqual,
            ),
            targets: &multiplied,
            multisample,
        });

        Self {
            mesh,
            mesh_front,
            mesh_cut,
            cut_depth,
            ground,
            edge,
            outline,
            ao,
            blur,
            composite,
            main_layout,
            ao_layout,
            composite_layout,
            scratch: None,
            cache: HashMap::new(),
            frame_no: 0,
        }
    }

    /// Meshes held in the cache of prepared geometry.
    pub fn cached_meshes(&self) -> usize {
        self.cache.len()
    }

    fn scratch(&mut self, device: &wgpu::Device, width: u32, height: u32) -> &Scratch {
        let reuse = self
            .scratch
            .as_ref()
            .is_some_and(|s| s.width == width && s.height == height);
        if !reuse {
            let make = |format, samples, usage, label| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let attach = wgpu::TextureUsages::RENDER_ATTACHMENT;
            let both = attach | wgpu::TextureUsages::TEXTURE_BINDING;
            let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
            let depth = make(DEPTH_FORMAT, SAMPLE_COUNT, both, "render3d_depth");
            self.scratch = Some(Scratch {
                width,
                height,
                color: view(&make(TARGET_FORMAT, SAMPLE_COUNT, attach, "render3d_msaa")),
                depth_only: depth.create_view(&wgpu::TextureViewDescriptor {
                    aspect: wgpu::TextureAspect::DepthOnly,
                    ..Default::default()
                }),
                depth: view(&depth),
                ao_raw: view(&make(AO_FORMAT, 1, both, "render3d_ao_raw")),
                ao: view(&make(AO_FORMAT, 1, both, "render3d_ao")),
            });
        }
        self.scratch.as_ref().unwrap()
    }
}

/// One frame's items, grouped by shared mesh.
struct Packed {
    /// Storage-buffer bytes, items in group order.
    items: Vec<u8>,
    /// Edge instances: two mesh-frame points and the index into `items`.
    edges: Vec<u8>,
    edge_count: u32,
    /// Per group: the cache key and the instance range in `items`.
    groups: Vec<(usize, std::ops::Range<u32>)>,
}

fn mesh_key(m: &Arc<RenderMesh>) -> usize {
    Arc::as_ptr(m) as usize
}

fn pack(frame: &Frame3d<'_>) -> Packed {
    // Groups in first-seen order, items within a group in scene order.
    let mut group_of: HashMap<usize, usize> = HashMap::new();
    let mut members: Vec<(usize, Vec<usize>)> = Vec::new();
    // Empty meshes draw nothing (and cannot back a buffer).
    for (i, it) in frame.items.iter().enumerate() {
        if it.mesh.triangles.is_empty() {
            continue;
        }
        let key = mesh_key(it.mesh);
        let g = *group_of.entry(key).or_insert_with(|| {
            members.push((key, Vec::new()));
            members.len() - 1
        });
        members[g].1.push(i);
    }
    let mut p = Packed {
        items: Vec::with_capacity(frame.items.len().max(1) * ITEM_SIZE),
        edges: Vec::new(),
        edge_count: 0,
        groups: Vec::with_capacity(members.len()),
    };
    let mut slot = 0u32;
    for (key, list) in &members {
        let first = slot;
        for &i in list {
            let it = &frame.items[i];
            let start = p.items.len();
            push_f32(
                &mut p.items,
                &mat_f32(&mat_mul(&frame.view_projection, &it.model)),
            );
            push_f32(&mut p.items, &mat_f32(&it.model));
            let mat = it.material;
            push_f32(&mut p.items, &rgb_w(mat.base_color, mat.metallic));
            push_f32(
                &mut p.items,
                &[
                    mat.roughness as f32,
                    specular_exponent(mat.roughness) as f32,
                    0.0,
                    0.0,
                ],
            );
            push_f32(&mut p.items, &rgb_w(it.section_color, 0.0));
            debug_assert_eq!(p.items.len() - start, ITEM_SIZE);
            let m = it.mesh;
            for [a, b] in &it.edges {
                let (a, b) = (m.positions[*a as usize], m.positions[*b as usize]);
                push_f32(
                    &mut p.edges,
                    &[
                        a.x as f32, a.y as f32, a.z as f32, b.x as f32, b.y as f32, b.z as f32,
                    ],
                );
                p.edges.extend_from_slice(&slot.to_le_bytes());
                p.edge_count += 1;
            }
            slot += 1;
        }
        p.groups.push((*key, first..slot));
    }
    if p.items.is_empty() {
        p.items.resize(ITEM_SIZE, 0);
    }
    p
}

fn globals(frame: &Frame3d<'_>) -> Vec<u8> {
    let mut g = Vec::with_capacity(GLOBALS_SIZE);
    push_f32(&mut g, &mat_f32(&frame.view_projection));
    let inv = mat_inverse(&frame.view_projection).unwrap_or(cadview_core::scene3d::IDENTITY4);
    push_f32(&mut g, &mat_f32(&inv));
    push_f32(&mut g, &frame.eye.map(|v| v as f32));
    let (k, f, l) = (frame.key_direction, frame.fill_direction, frame.lighting);
    push_f32(&mut g, &rgb_w([k.x, k.y, k.z], l.key_intensity));
    push_f32(&mut g, &rgb_w([f.x, f.y, f.z], l.fill_intensity));
    let (color, width) = match frame.edges {
        Some(e) => (e.color.map(srgb_to_linear), e.width_px),
        None => ([0.0; 3], 0.0),
    };
    push_f32(
        &mut g,
        &[
            l.ambient as f32,
            frame.width as f32,
            frame.height as f32,
            (width * frame.pixel_ratio * 0.5) as f32,
        ],
    );
    push_f32(&mut g, &rgb_w(color, frame.pixel_world));
    let (o_color, o_width) = match frame.outline {
        Some(o) => (o.color.map(srgb_to_linear), o.width_px),
        None => ([0.0; 3], 0.0),
    };
    push_f32(&mut g, &rgb_w(o_color, o_width * frame.pixel_ratio * 0.5));
    for i in 0..MAX_CLIP_PLANES {
        let p = frame
            .clip_planes
            .get(i)
            .copied()
            .unwrap_or([0.0, 0.0, 0.0, -1.0]);
        push_f32(&mut g, &p.map(|v| v as f32));
    }
    push_f32(
        &mut g,
        &[
            frame.clip_planes.len() as f32,
            (width * frame.pixel_ratio) as f32,
            // 1: remove only what lies beyond every plane.
            match frame.clip_mode {
                cadview_core::scene3d::ClipMode::Any => 0.0,
                cadview_core::scene3d::ClipMode::All => 1.0,
            },
            0.0,
        ],
    );
    let ground = frame.ground;
    let gr = ground.map_or([0.0; 3], |g| g.color);
    push_f32(&mut g, &rgb_w(gr, ground.map_or(0.0, |g| g.z)));
    let (c, fade, half) = ground.map_or(([0.0; 2], (0.0, 1.0), 0.0), |g| {
        (g.center, g.fade, g.half_size)
    });
    push_f32(
        &mut g,
        &[c[0] as f32, c[1] as f32, fade.0 as f32, fade.1 as f32],
    );
    push_f32(&mut g, &[half as f32, 0.0, 0.0, 0.0]);
    push_f32(&mut g, &rgb_w(frame.background, 0.0));
    debug_assert_eq!(g.len(), GLOBALS_SIZE);
    g
}

fn ao_block(frame: &Frame3d<'_>) -> Vec<u8> {
    let mut a = Vec::with_capacity(AO_SIZE);
    let Some(o) = frame.occlusion else {
        return a;
    };
    push_f32(&mut a, &mat_f32(&frame.projection));
    let inv = mat_inverse(&frame.projection).unwrap_or(cadview_core::scene3d::IDENTITY4);
    push_f32(&mut a, &mat_f32(&inv));
    for k in o.kernel {
        push_f32(&mut a, &rgb_w([k.x, k.y, k.z], 0.0));
    }
    for y in 0..4 {
        for x in 0..4 {
            let (c, s) = ao_rotation(x, y);
            push_f32(&mut a, &[c as f32, s as f32, 0.0, 0.0]);
        }
    }
    push_f32(
        &mut a,
        &[
            o.radius as f32,
            o.strength as f32,
            o.bias as f32,
            frame.eye[3] as f32,
        ],
    );
    push_f32(
        &mut a,
        &[
            frame.width as f32,
            frame.height as f32,
            o.range.0 as f32,
            o.range.1 as f32,
        ],
    );
    debug_assert_eq!(a.len(), AO_SIZE);
    a
}

impl RasterRenderer {
    /// Make sure every mesh of `frame` has its buffers, mark it used, and
    /// drop the ones unused for [`CACHE_FRAMES`] frames.
    fn prepare_meshes(&mut self, device: &wgpu::Device, frame: &Frame3d<'_>) {
        self.frame_no += 1;
        let now = self.frame_no;
        for it in frame
            .items
            .iter()
            .filter(|it| !it.mesh.triangles.is_empty())
        {
            let key = mesh_key(it.mesh);
            if let Some(c) = self.cache.get_mut(&key) {
                c.last_used = now;
                continue;
            }
            let m: &RenderMesh = it.mesh;
            let mut vertices = Vec::with_capacity(m.positions.len() * 24);
            for (pos, n) in m.positions.iter().zip(&m.normals) {
                push_f32(
                    &mut vertices,
                    &[
                        pos.x as f32,
                        pos.y as f32,
                        pos.z as f32,
                        n.x as f32,
                        n.y as f32,
                        n.z as f32,
                    ],
                );
            }
            let indices: Vec<u8> = m
                .triangles
                .iter()
                .flatten()
                .flat_map(|i| i.to_le_bytes())
                .collect();
            let buffer = |label, data: &[u8], usage| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: data,
                    usage,
                })
            };
            self.cache.insert(
                key,
                CachedMesh {
                    _mesh: Arc::clone(it.mesh),
                    vertices: buffer("render3d_vertices", &vertices, wgpu::BufferUsages::VERTEX),
                    indices: buffer("render3d_indices", &indices, wgpu::BufferUsages::INDEX),
                    index_count: (m.triangles.len() * 3) as u32,
                    last_used: now,
                },
            );
        }
        self.cache.retain(|_, c| c.last_used + CACHE_FRAMES > now);
    }
}

impl Renderer3d for RasterRenderer {
    fn quality(&self) -> Quality {
        Quality::Working
    }

    fn render(
        &mut self,
        gpu: Gpu<'_>,
        frame: &Frame3d<'_>,
        target: Target<'_>,
    ) -> Result<Progress, Render3dError> {
        let (w, h) = (target.width, target.height);
        if w == 0 || h == 0 {
            return Err(Render3dError::InvalidOptions(
                "render size must be positive".to_string(),
            ));
        }
        if frame.clip_planes.len() > MAX_CLIP_PLANES {
            return Err(Render3dError::InvalidOptions(format!(
                "at most {MAX_CLIP_PLANES} clip planes, got {}",
                frame.clip_planes.len()
            )));
        }
        let device = gpu.device;
        self.prepare_meshes(device, frame);
        let packed = pack(frame);
        let buffer = |label, contents: &[u8], usage| {
            // Zero-size buffers cannot be bound; keep a minimum.
            let mut data = contents.to_vec();
            if data.is_empty() {
                data.resize(32, 0);
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &data,
                usage,
            })
        };
        let gbuf = buffer(
            "render3d_globals",
            &globals(frame),
            wgpu::BufferUsages::UNIFORM,
        );
        let item_buf = buffer("render3d_items", &packed.items, wgpu::BufferUsages::STORAGE);
        let ebuf = buffer("render3d_edges", &packed.edges, wgpu::BufferUsages::VERTEX);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render3d_raster"),
            layout: &self.main_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: gbuf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: item_buf.as_entire_binding(),
                },
            ],
        });
        let occlusion = frame.occlusion.is_some() && !packed.groups.is_empty();
        let clipping = !frame.clip_planes.is_empty();
        let draw_edges = frame.edges.is_some() && packed.edge_count > 0;
        self.scratch(device, w, h);
        let s = self.scratch.as_ref().unwrap();
        let bg = frame.background;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render3d"),
        });

        // 1. Surfaces.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render3d_surfaces"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s.color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0],
                            g: bg[1],
                            b: bg[2],
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &s.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &bind, &[]);
            for (key, instances) in &packed.groups {
                let c = &self.cache[key];
                pass.set_vertex_buffer(0, c.vertices.slice(..));
                pass.set_index_buffer(c.indices.slice(..), wgpu::IndexFormat::Uint32);
                if clipping {
                    pass.set_pipeline(&self.mesh_front);
                    pass.set_stencil_reference(STENCIL_PART);
                    pass.draw_indexed(0..c.index_count, 0, instances.clone());
                    pass.set_pipeline(&self.mesh_cut);
                    pass.set_stencil_reference(STENCIL_CUT);
                    pass.draw_indexed(0..c.index_count, 0, instances.clone());
                } else {
                    pass.set_pipeline(&self.mesh);
                    pass.set_stencil_reference(STENCIL_PART);
                    pass.draw_indexed(0..c.index_count, 0, instances.clone());
                }
            }
            if clipping {
                pass.set_pipeline(&self.cut_depth);
                pass.set_stencil_reference(STENCIL_CUT);
                pass.draw(0..3, 0..1);
            }
            if frame.ground.is_some() {
                pass.set_pipeline(&self.ground);
                pass.set_stencil_reference(STENCIL_GROUND);
                pass.draw(0..6, 0..1);
            }
        }

        // 2. Ambient occlusion.
        let ao_buf;
        let mut composite_bind = None;
        if occlusion {
            ao_buf = buffer("render3d_ao", &ao_block(frame), wgpu::BufferUsages::UNIFORM);
            let ao_bind = |input: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("render3d_ao"),
                    layout: &self.ao_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: ao_buf.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&s.depth_only),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(input),
                        },
                    ],
                })
            };
            for (pipeline, input, output, label) in [
                (&self.ao, &s.ao, &s.ao_raw, "render3d_ao"),
                (&self.blur, &s.ao_raw, &s.ao, "render3d_ao_blur"),
            ] {
                let bind = ao_bind(input);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            composite_bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("render3d_composite"),
                layout: &self.composite_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&s.ao),
                }],
            }));
        }

        // 3. Occlusion into the colour, then lines; resolve.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render3d_lines"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &s.color,
                    depth_slice: None,
                    resolve_target: Some(target.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &s.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(cb) = &composite_bind {
                pass.set_pipeline(&self.composite);
                pass.set_bind_group(0, cb, &[]);
                pass.set_stencil_reference(STENCIL_CUT);
                pass.draw(0..3, 0..1);
            }
            if draw_edges {
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, ebuf.slice(..));
                pass.set_pipeline(&self.edge);
                pass.draw(0..6, 0..packed.edge_count);
                if frame.outline.is_some() {
                    pass.set_pipeline(&self.outline);
                    pass.set_stencil_reference(STENCIL_GROUND);
                    pass.draw(0..6, 0..packed.edge_count);
                }
            }
        }
        gpu.queue.submit(Some(encoder.finish()));
        Ok(Progress::Done)
    }
}
