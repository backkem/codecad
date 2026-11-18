# 3D in CodeCAD: design

Status: proposal (2026-10-13). The build-out plan is [LADDER-3D.md](LADDER-3D.md).
The native desktop shell that later hosts the same 3D viewport is
[LADDER-NATIVE.md](LADDER-NATIVE.md).

CodeCAD gets solid parts made from CAD primitives (extrude, revolve, sweep,
loft, booleans, roundovers). They live in the same document as the 2D
drawing and sync over the same Yrs channel. A 3D viewport renders them, from
an interactive raster view up to a progressive path tracer. Parts project
back into exact 2D geometry, so the existing 2D tooling (dimensions, title
block, DWG export) produces the shop drawing. Everything stays in Rust,
compiles to `wasm32-unknown-unknown`, and runs the same code on both peers.

## 1. Goals and non-goals

### Goals

1. **Building and DIY modelling with CAD primitives.** The workhorse is the
   flat part: a 2D profile, a thickness and a placement (a shelf, a side, a
   plywood back, a slab). Around it: revolve (turned legs), sweep (handrails,
   mouldings), loft and taper (tapered legs), booleans for joinery (dados,
   rabbets, mortises, holes), and edge roundovers and chamfers.
2. **Accuracy you can trust.** The 3D model is used to check the 2D work
   against reality, so the 2D abstract-geometry rule applies here too. The
   document stores the exact feature tree and the kernel evaluates exact
   analytic B-rep. Tessellation happens only at render time and adapts to
   zoom. Volumes, distances and cut-list dimensions come from exact geometry,
   never from a mesh.
3. **3D to 2D for manufacturing.** The output is orthographic views, sections
   with hatching, hidden lines and dimensions, written into the 2D `Document`
   as ordinary entities. Also a cut list for flat parts.
4. **Good-looking renders.** PBR materials (wood, concrete, paint, glass),
   HDRI image-based lighting, a progressive path tracer with global
   illumination, and filmic tone mapping (AgX or Khronos PBR Neutral).
5. **Assets for context.** External glTF/GLB models (plants, chairs, people)
   are placed by reference with a transform. They appear in renders and as
   optional plan symbols. They are excluded from projection, cut lists and
   DWG.
6. **Same ABI.** Everything goes through `cad_call` and `web/src/cad-api.ts`
   (`cad.extrude`, `cad.revolve`, `cad.subtract`, `cad.placeAsset`,
   `cad.project`, `cad.render`, ...), so agents, the sandbox and the browser
   share one API.

### Non-goals (for this design)

- A general mechanical CAD system. There are no assemblies with mates or
  joints, no sketch constraint solver, no sheet metal and no surfacing
  beyond sweep and loft.
- Interactive 3D direct modelling (push/pull with the mouse). Creation is
  scripted, with a simple pick-and-inspect UI. Interactive tools can come
  later on top of the same feature tree.
- STEP/IGES import as a first-class workflow. The chosen kernel can read
  STEP; exposing it is an open question (§13), not a rung.
- Live associative drawings. Projected views are regenerated on request
  (`cad.project` again, or a "refresh views" button), not on every edit.
- Hardware ray tracing. WebGPU has none, and the path tracer runs as
  compute shaders.
- C++ kernels (OpenCascade and its bindings), by decision.

## 2. Verification summary (evidence)

Each claim from the brief was checked on 2026-10-13 against crates.io, the
GitHub API, the repositories and, where cheap, a compile-and-run smoke test.
The smoke-test sources are in [research/3d-smoke/](research/3d-smoke/)
(see its README for how to rerun them). All of them model the same test part: a
600 x 300 x 18 mm board, minus a Ø10 through hole, minus a 12 x 6 dado along
its length. That is two booleans. The closed-form volume is
**3 195 386.283 mm³**. With all 12 edges rounded at r = 3 before the cuts,
it is **3 188 376.6 mm³** (Minkowski box plus the dado-end correction).
Builds used `opt-level = "s"`, LTO and `wasm32-unknown-unknown`, and the
wasm was run in Node with stubbed imports.

### Kernels

| Kernel | Status (verified) | Smoke result | Verdict |
|---|---|---|---|
| **[ogeom](https://github.com/gilbertorconde/ogeom-rs)** ([crates.io](https://crates.io/crates/ogeom) 0.10.0, MIT/Apache-2.0) | Pure-Rust B-rep with a data model like OCCT's (shared topology, per-entity tolerance, operation history). Has booleans, fillets and chamfers (with vertex blends), extrude/revolve/sweep/loft, offset/shell, **HLR (polygonal and exact), sections with hatch**, STEP/IGES/STL. Repo created 2026-08-04 (first crates.io release 2026-09-18), so about ten weeks old, with about 222 k LoC and 1 309 commits by one author (2 by others), 4 stars, near-daily releases. | Native: plain board in 5-12 ms, **exact volume 3 195 386.283**, valid B-rep, 11 faces. Rounded board (12-edge fillet, then 2 cuts) in 0.7-1.1 s, valid B-rep, 33 faces, **volume 3 188 376.635** (matches closed form). Front-view HLR: 55 visible and 46 hidden curves in 9 ms. **wasm: builds, no imports at all, 3.2 MB (1.08 MB gzip), rounded board runs in 3.3 s in Node.** | **Primary backend**, behind a trait, pinned, with a second backend for differential tests. Young, single maintainer and mostly machine-written, so §5 covers the mitigations. |
| **[Truck](https://github.com/ricosjp/truck)** (truck-modeling 0.6.0, truck-shapeops 0.4.0, Apache-2.0) | Mature B-rep/NURBS kernel (ricosjp, 1.6 k stars). The last crates.io release was **2024-09-20**. Master is active, though since mid-2026 mostly weekly `cargo upgrade` merges. The unreleased master adds a **single-edge fillet** (low-level, face-based, outside `truck_modeling::Solid`), `EdgeBlendSurface`, offsets and a new 2D `truck-drafting` crate ([CHANGELOG](https://github.com/ricosjp/truck/blob/master/CHANGELOG.md)). | Native: 2 booleans in 344-389 ms, 13 faces. Mesh volume converges to the exact value (3 195 386.538 at tol 0.001). **wasm: builds, 949 KB (260 KB gzip), runs in 1.37 s**. It needs the wasm-bindgen placeholder imports, and rayon falls back to one thread. Master fillet tests (`create_simple_fillet`, `create_fillet_with_side`) pass. | **Secondary backend** for differential testing and the fallback if ogeom stalls. It has no usable roundovers through its released API. |
| **[vcad-kernel](https://github.com/ecto/vcad)** 0.10.0 | B-rep with fillet, shell, sweep and loft, plus a sprawling umbrella crate (it pulls in `qcd`, `neutronics`, `photonics`, ...). crates.io says MIT and the GitHub repo says Apache-2.0. | Plain board: **analytic B-rep**, volume 3 195 386.66 (mesh). Rounded board: works, but the result's `fidelity()` is **`TriangleSoup`**: booleans after a fillet silently fell back to mesh CSG. `Solid::difference` is documented to return the target **unchanged** on a kernel error. wasm: 1.3 MB, runs in 1.2 s. | Rejected. Silent degradation to a mesh breaks the accuracy goal. |
| **[csgrs](https://github.com/timschmidt/csgrs)** | crates.io 0.20.1 (2025-07) **no longer resolves**: it hard-depends on `core2 ^0.4`, whose only version is yanked. Main is an unreleased 0.23 rewrite on exact-arithmetic "hyper*" crates. | On main: box minus box is fine, and box minus an 8-segment cylinder is fine. **Box minus a cylinder with ≥ 16 segments fails** with `predicate could not be decided: Real sign classification`. | Rejected for now. It was mesh-based anyway, so wrong for the accuracy goal. |
| **[Fornjot](https://github.com/hannobraun/fornjot)** | **Archived.** The README says the project "has been shut down". The last push was 2026-06-19, which fits "discontinued June 2026". | n/a | Excluded. |
| **[BREP.io](https://github.com/mmiscool/BREP.io)** (mmiscool) | **TypeScript**, not Rust. Triangle meshes with B-rep-style topology. The license is custom ("Autodrop3d LLC", reported as NOASSERTION). | n/a | Excluded (not Rust, not exact). |
| opencascade-rs | C++ | n/a | Excluded by decision. |

### Renderers

| Renderer | Status (verified) | Verdict |
|---|---|---|
| **[Kiss3d](https://github.com/dimforge/kiss3d) 0.47.0** (2026-10-04, BSD-3-Clause) | The claim holds, and it goes further than the brief said. **wgpu 30**. PBR StandardMaterial (clearcoat, anisotropy, transmission). **Equirectangular HDRI skybox + IBL**. Shadow maps, SSAO, OIT. Linear HDR film with **AgX, ACES, Khronos PBR Neutral (default) and Tony McMapface** tone mapping. A **progressive Monte-Carlo path tracer** (`RayTracer`) with one WGSL kernel and two backends: a software **compute BVH** (works on WebGPU) and a HW ray-query backend picked at runtime. It has a two-level BVH, MIS with area lights, HDRI IBL and an à-trous denoiser. `OffscreenSurface` renders headless natively and **on wasm** (GPU side), and `output_view` exposes the tone-mapped texture. `Context::init(instance, device, queue, adapter, format)` **accepts an external device**. glTF loader. Mostly one author (sebcrozet, 655 commits). The path tracer is about 1.7 k lines of WGSL. Source: [CHANGELOG](https://github.com/dimforge/kiss3d/blob/master/CHANGELOG.md). | **Adopted** as the 3D render backend behind our own trait (§9). Revised 2026-11-18 after a run-time test: the working render is our own raster, Kiss3d stays the presentation candidate (§9, decision update). The "dry renders" concern becomes a gate in 3D5b: a look bake-off against a Cycles reference. If it fails, we fork its BSD-3 tracer kernel into our crate rather than starting from zero. |
| [Renderling](https://github.com/schell/renderling) | "Alpha", NLnet-funded, actively pushed (2026-10-11). **crates.io 0.4.9 dates from 2024-09**, and main is an unreleased 0.6.0. **Main is on wgpu 26**, while Vello 0.11 and Kiss3d are on wgpu 30, so it **cannot share a device with Vello** without an upstream bump. PBR, IBL, shadows and bloom, but **no path tracer**. Main builds for wasm32 (our smoke test). | Not adopted. The earlier recommendation is revised. |
| Bevy 0.20 (2026-10-08) | A full engine that owns the app loop. Its ray-traced GI (Solari) needs **hardware** ray tracing, which WebGPU lacks ([0.17 notes](https://bevy.org/news/bevy-0-17/), [0.20 notes](https://bevy.org/news/bevy-0-20/)). MSRV 1.97. | Too heavy to embed, and its GI is not available on the web. |
| three-d 0.19 | OpenGL/WebGL (glow), so it cannot share a wgpu device with Vello. | Excluded. |
| rend3 | [Archived](https://github.com/BVE-Reborn/rend3), "maintenance mode", last push 2024-07-08. The archive date (claimed June 2025) is not visible through the API. | Excluded. |
| "threers", "Penumbra" | **Not found** on crates.io, GitHub search or the web under these names. | Unverified, ignored. |

### Compatibility facts that shape the design

- **One wgpu across the stack is possible today.** `vello 0.11`,
  `kiss3d 0.47` and `eframe/egui-wgpu 0.36.2` all depend on **wgpu 30**, and
  `egui_dock 0.21.1` depends on `egui ^0.36` (not on wgpu directly), so it
  follows eframe. A native smoke crate with all of them plus
  `wtransport 0.7.2` compiles with **no duplicate wgpu, winit or vello**. It
  hands one `egui_wgpu::RenderState` device to both `vello::Renderer::new`
  and `kiss3d::context::Context::init`. A wasm32 smoke crate with
  `kiss3d 0.47` and `vello 0.11` compiles and resolves to a single
  `wgpu v30.0.1`. CodeCAD is on vello 0.9 / wgpu 29 today, and `cadview-web`'s
  `eframe/egui 0.34` (the fallback renderer) also pins `wgpu ^29`. So
  **bumping vello 0.9 → 0.11 and eframe/egui 0.34 → 0.36 together is a
  prerequisite** (rung 3D2a and N0a). Bumping only one leaves two wgpu
  majors in the web bundle.
- The current browser bundle is 24 MB of wasm. ogeom adds about 3.2 MB raw
  (1.1 MB gzip).
- The research machine had no GPU and no Vulkan driver, so **no GPU render
  was run**.
  All renderer evidence is compile-level plus the documentation. Rungs 3D2b
  and 3D5a/3D5b must prove the runtime behaviour.

## 3. Architecture overview

```
 cadview-core (wasm32 + native, zero global state)
 ├── document.rs   Document { entities (2D), layers, blocks, ..., solids: SolidStore }
 ├── solid/        feature tree (PartDef, Feature, Profile2d, Placement, Material, AssetRef)
 │   ├── kernel.rs     trait SolidKernel  ── impl ArrisKernel in the crate cadview-solid
 │   ├── eval.rs       evaluator a shell supplies + content-keyed cache: PartDef -> solid, meshes (never synced)
 │   ├── mesh.rs       tessellation cache: (Brep hash, tolerance bucket) -> Mesh (never synced)
 │   ├── project.rs    3D -> 2D: HLR, sections, analytic re-fit -> Shape entities in a block
 │   ├── cutlist.rs    flat-part cut list from the feature tree (exact, no kernel needed)
 │   └── check.rs      interference, measure3d, validity
 ├── dispatch*.rs  cad_call: extrude, revolve, subtract, project, cutList, placeAsset ...
 └── sync.rs       new YMaps: "parts", "partdefs", "materials", "assets", "scene3d"

 cadview-render   (shared 2D Vello scene builder; LADDER-NATIVE N0b; GPU-bound, no CAD logic)
 cadview-render3d (new, wasm32 + native): SceneSync(Document -> kiss3d scene),
                  orbit camera, modes Shaded | Lines | Render(path tracer), HDRI,
                  tone mapping, headless render to PNG (native)
 cadview-web      one wgpu device per page; Vello (2D) + render3d (3D) per canvas
 cadview-server   GET /api/files/<key> for GLB/HDRI/textures; render RPC (headless)
 cadview-desktop  (LADDER-NATIVE) same crates, eframe chrome
```

The core-first rule holds. The feature tree, evaluation, tessellation,
projection, cut lists and interference all live in core and run identically
in the browser, on the server and in the sandbox. Only GPU code lives
outside core, in `cadview-render3d`, which both shells share.

## 4. Data model

### 4.0 Revision (2026-11-18): the core speaks a general shape language

This design was written around DIY furniture, and woodworking terms
leaked into the core's types and API: a part "kind" (flat or turned), a
grain direction, stock sizes on materials, a `cad.board` call and a cut
list computed in core. That conflicts with the rule the project has since
written down (`architecture.md`, "Semantics live in code"): the core
holds exact geometry and answers generic questions, and what the
geometry means lives in the script.

The revision, which overrides the sections below wherever they disagree:

- **Parts are shape only.** A `PartDef` is an id, a name and an ordered
  list of features. The features are a general shape language: extrude,
  revolve, sweep, loft, hole, boolean, fillet, chamfer. `PartKind`,
  `GrainDir` and the "fast path for flat parts" (sections 4.1 and 5) are
  dropped; they were removed from the code before anything used them.
- **Meaning rides on attributes.** Grain, stock, a cut-list label and
  anything else a domain needs are free-form attributes on parts and
  materials, which the core stores, syncs and filters on without
  interpreting them. This is the same mechanism planned for 2D entities.
  `Part.label` and `Material.stock` in section 4.1 become attributes.
  A material's appearance and its section hatch stay: they are rendering
  and drafting properties, not domain meaning.
- **Woodworking is a script library, fully decoupled.** It is TypeScript
  or JavaScript that runs in the script sandbox (and in the browser), and
  it uses nothing but the public `cad.*` API. It is not compiled into the
  core, not part of the `cad` API prelude, and the core has no knowledge
  of it: it could live in another repository. `cad.board` (section 10)
  becomes a library helper over `cad.extrude`. The cut list (section 7.6)
  is computed by the library from generic part queries: the bounding box
  in the part's own frame, the volume, the material and the attributes.
  The library knows the sizes it asked for, so it can still report exact
  numbers as entered and compare them with the kernel's box. `cutlist.rs`
  (section 3) is not built in core.
- **What the core owes such a library** is generic: attributes on parts,
  `cad.parts()` with filtering at the source, exact measures (box in a
  given frame, volume, distance, interference), projection and sections.
  A different domain (sheet metal, circuit boards, 3D printing) uses the
  same calls with a different library.
- **Scripts load a library by name.** `const draft = await cad.use("drafting")`
  runs `lib/drafting.js` once and returns what it offers (rung A7 on
  `LADDER-API.md`, landed). Shipping domain libraries is still not a
  priority.
- **The examples and the vision are unchanged.** The bookshelf script in
  section 10.1 still reads the same; `board`, `cutList` and
  `cutListTable` come from the library it loads.

Ladder rungs that named the moved pieces (3D1a `board`, 3D3b cut lists)
keep their gates. What changes is where the code lives.

### 4.1 Types (cadview-core `solid/`)

```rust
/// A reusable solid definition in its own local frame (millimetres, f64).
pub struct PartDef {
    pub id: DefId,
    pub name: String,                  // "shelf", "side"
    pub kind: PartKind,                // Flat { thickness, grain: GrainDir } | Turned | General
    pub features: Vec<Feature>,        // ordered, evaluated top to bottom
}

/// A placed occurrence of a definition: what the user sees and selects.
pub struct Part {
    pub id: EntityId,                  // same id space as 2D entities, stable u64
    pub def: DefId,
    pub placement: Placement,          // rotation (unit quaternion) + translation, f64
    pub material: Option<String>,      // None = layer's material
    pub layer: String,                 // layers work in 3D too: visibility, material default
    pub label: Option<String>,         // cut-list name override ("shelf 2")
}

pub enum Feature {
    Extrude  { id: FeatId, profile: Profile2d, plane: Plane3, distance: f64,
               taper_deg: f64, op: Op },
    Revolve  { id: FeatId, profile: Profile2d, plane: Plane3, axis: Line2, angle_deg: f64, op: Op },
    Sweep    { id: FeatId, profile: Profile2d, plane: Plane3, path: Path3, op: Op }, // path starts at plane origin along its normal
    Loft     { id: FeatId, sections: Vec<(Profile2d, Plane3)>, ruled: bool, op: Op },
    Hole     { id: FeatId, at: Point3, dir: Vec3, dia: f64, depth: Depth, cbore: Option<(f64, f64)> },
    Boolean  { id: FeatId, op: Op, tool: ToolRef },             // joinery
    Fillet   { id: FeatId, edges: EdgeSel, radius: f64, fallback: RoundFallback },
    Chamfer  { id: FeatId, edges: EdgeSel, d1: f64, d2: f64 },
}

pub enum Op { Add, Subtract, Intersect }

/// A tool body for a boolean: another part as placed in the scene (joinery:
/// "cut the shelf's shape into the side"), or an inline def.
pub enum ToolRef { Part { part: EntityId, clearance: f64 }, Def(DefId, Placement) }

/// The 2D section of a feature, using the same exact primitives as 2D Shapes:
/// lines, arcs, bulge polylines, circles, ellipses, splines. Never flattened.
pub struct Profile2d { pub outer: Vec<Seg2>, pub holes: Vec<Vec<Seg2>> }

/// Edge selection by meaning, not by kernel topology (see 4.3).
pub enum EdgeSel {
    Cap { feature: FeatId, side: CapSide /* Start | End | Both */, segs: Option<Vec<usize>> },
    Lateral { feature: FeatId, segs: Vec<usize> },  // edges swept from profile vertices
    Near { points: Vec<Point3> },                   // resolved to the nearest edges at eval
    All,
}

pub enum RoundFallback { Error, Visual }   // Visual = render-only bevel + 2D note

pub struct Material {
    pub name: String,                  // "oak", "birch-ply-18", "concrete"
    pub pbr: Pbr,                      // base colour, roughness, metallic, clearcoat, ...
    pub textures: Option<AssetKey>,    // store key of a PBR texture set (Poly Haven)
    pub section_hatch: HatchSpec,      // pattern, angle, spacing for sections
    pub stock: Option<StockSpec>,      // nominal thickness, sheet size, grain: for cut lists
}

pub struct AssetRef {
    pub id: EntityId,
    pub key: AssetKey,                 // document-store key, e.g. "assets/polyhaven/potted_plant_01.glb"
    pub placement: Placement,
    pub scale: f64,
    pub layer: String,
    pub plan_symbol: PlanSymbol,       // None | Footprint | Block(name)
}
```

Profiles reuse the 2D geometry types (kurbo `Line`/`Arc`, LwPolyline bulges,
circles, splines). `cad.extrude` can therefore take existing 2D entity ids,
which makes "draw the outline in 2D, then extrude it" natural and keeps the
profile exact.

### 4.2 Yrs mapping

Five new YMaps sit beside `entities`, `layers`, `blocks`, `linetypes` and
`meta`. They use the existing pattern: one bincode blob per key, so a
mutation is one map write and diffs stay small.

| YMap | Key | Value (bincode) | Typical size |
|---|---|---|---|
| `partdefs` | `DefId` | `PartDef` (feature list, profiles) | 0.2-5 KB |
| `parts` | `EntityId` | `Part` (def ref, placement, material, layer) | ~100 B |
| `materials` | name | `Material` | ~200 B |
| `assets` | `EntityId` | `AssetRef` | ~150 B |
| `scene3d` | `"default"` / view names | camera bookmarks, HDRI key, exposure, tone map | ~200 B |

The **feature tree is the only synced geometry**. B-reps, meshes, BVHs and
textures are derived, never synced, and cached by content hash on each peer.
Both peers run the same kernel code, so they evaluate the same tree to the
same B-rep up to floating-point determinism. Nothing depends on bit
equality: selections are semantic (4.3), and projected 2D views are written
back as ordinary entities by whichever peer ran `cad.project`.

Conflict granularity matches 2D: concurrent edits to the same `PartDef` are
last-writer-wins on that def, and edits to different parts merge. This is
fine for scripted, agent-driven work. A finer split (features as a YArray)
is possible later without changing the API.

DWG export ignores all five maps. DWG save stays byte-identical for a
document whose 2D content is unchanged, and rung 3D1a tests this. JSON
`cad.save` includes them.

**As built in 3D1a (2026-11-18).** Two of the five maps exist: `partdefs`
(key `d_<n>`) and `parts` (key `p_<n>`); materials, assets and `scene3d`
wait for the rungs that use them. A `Part` is `{id, def, placement,
attrs}`: material, layer and label became attributes or render options
(4.0). Part and definition ids come from the document's one id counter.
DWG bytes differ between two saves even with no change, so 3D1a compares
the DWG's 2D content (entities and layers after a reload) with and without
parts, and `saveDwg` reports the parts it left out. JSON is `saveJson`
(lossless), not `cad.save`. The cache lives with the evaluator the shell
supplies (`solid::eval::SolidEnv`), not in core's own state; see
`docs/api-surface.md`, "Solid parts".

### 4.3 Stable references (the "topological naming" problem)

Fillets, chamfers and dimension anchors need to name edges and faces across
re-evaluation. We **never store kernel edge ids**. Selections are expressed
against our own feature tree:

- `Cap{feature, side, segs}`: edges of an extrude's start or end cap,
  indexed by **profile segment index**. We own the profile, so the index is
  stable. "Round over the top front edge" is `Cap{f1, End, [2]}`.
- `Lateral{feature, segs}`: edges swept from profile vertices.
- `Near{points}`: picks the nearest edge at evaluation time. It is robust to
  small edits, and the API uses it when a script picks by coordinates.

The kernel adapter resolves selectors after each feature, using operation
history (arris returns a `Provenance` record on every op, naming extrude
edges by profile segment) to follow edges through later booleans. A selector that no longer resolves is an evaluation error
with a clear message. It never silently picks a different edge.

### 4.4 Placement, frames, units

Units are document units (mm), with f64 everywhere in core, as in 2D. The
world frame has Z up. The 2D drawing plane is world XY at z = 0, which makes
the plan view the natural default and lets a 2D floor plan sit under the 3D
model. `Placement` is a rigid transform (unit quaternion plus translation).
Scale is only allowed on assets. Parts use explicit dimensions, so cut lists
stay honest.

## 5. Kernel choice and boundary

### Decision update (2026-11-18): arris for now

ogeom changed licence: releases up to 0.11.5 are `MIT OR Apache-2.0`, and
from 0.12.0 on every ogeom crate is published as `LGPL-2.1-only`. CodeCAD
is MIT and statically links its kernel into the server binary and the wasm
bundle, so ogeom was dropped in 3D0a before anything with it landed.

- **The ladder continues on arris, for now.** The test harness
  (3d-test-harness.md) ran arris and a second candidate, cad-kernel, side
  by side; the evidence is in
  [research/kernel-comparison.md](research/kernel-comparison.md). arris
  has what the next rungs need (sections, projection, ray hits,
  provenance, centroids) and was wrong far less often under fuzzing, at
  the cost of refusing more often near exact contacts. The choice is
  provisional: everything goes through the `SolidKernel` trait, documents
  store no kernel data, cad-kernel stays on the scoreboard as a second
  opinion, and a kernel of our own remains possible with the harness as
  its gate.
- **The arris adapter is in a shipped crate, `cadview-solid`; nothing
  links it yet.** `ArrisKernel` moved there from the harness unchanged
  (the exact pin, default features off, `paranoid`, the refusals of what
  arris's checker rejects and of meshes that are not closed). It is the
  only crate that names arris and it compiles for wasm32. cadview-core
  still links no kernel; the server and the browser bundle will link
  `cadview-solid` when they evaluate parts. The harness tests the adapter
  from there, and a test fails if arris's committed scoreboard records
  any wrong case (3d-test-harness.md, section 5). It records none since
  3D0b. cad-kernel stays in the harness, test-only.
- **Reference backend: [arris](https://github.com/arris-labs/arris)**, pinned
  to `=0.11.1` (`MIT OR Apache-2.0`, pure Rust, no `unsafe`). Default
  features are off: no serde and no rayon, so it starts no threads, and it
  keeps no process-wide state. It passes the whole prismatic corpus (3D0a)
  and builds the smoke part above, fully rounded (12-edge fillet, then the
  hole and the dado), as a valid B-rep of 33 faces at volume
  3 188 376.635, with volumes integrated on the exact faces.
- **What arris does not have:** hidden-line removal and silhouettes. It
  has the parts to build them on: exact planar sections
  (`ops::section`), projection of edges to a plane as exact 2D curves
  (`query::project_to_plane`), exact ray hits against the faces
  (`query::ray_fire`), tessellation to a chord tolerance, and full access
  to face surfaces and edge curves. §7 therefore uses the adapter path the
  design already planned for Truck (our own oracle and closed-form
  silhouettes) for every backend.
- Everything below that names ogeom is the 2026-10-13 evaluation. The
  evidence tables in §2 are kept as recorded.

### Decision (2026-10-13)

- **Primary backend: ogeom** (superseded by the update above). The smoke
  tests used 0.10, and 3D0a first pinned `=0.18.1`. It was
  the only candidate that passed every smoke test: exact volume on booleans,
  a 12-edge fillet with corner blends followed by booleans that still yield
  a valid B-rep at the closed-form volume, built-in HLR (exact silhouettes
  and a mesh visibility test),
  section with hatch, and a pure wasm build with no imports.
- **Secondary backend: Truck** (crates.io 0.6/0.4), compiled in tests and
  behind a cargo feature. Every corpus part is evaluated on both, and the
  volumes, bounding boxes and projected outlines are compared. Disagreement
  is a test failure that points at one kernel or the other. Truck has no
  usable fillets, so roundover corpus entries only check the primary
  backend against closed-form values.
- **Our own fast path for flat parts.** Cut lists, nominal dimensions and
  grain come straight from the feature tree (`PartKind::Flat`), with no
  kernel involved. The kernel's bounding box confirms them (and the check
  fails loudly if a feature pokes outside the declared thickness).

### Why not "our own parametric layer + mesh booleans" (the earlier recommendation)

The feature tree is still ours, as recommended. The mesh-boolean half is
dropped. Mesh booleans cannot give exact volumes, exact projected arcs or
real roundovers. csgrs no longer resolves from crates.io and fails
box-minus-cylinder on main. vcad shows what happens when mesh CSG is used
as a fallback: silent `TriangleSoup`. Truck was evaluated seriously, as the
brief suggested. It works and is the safe second backend, but its released
API has no roundovers, its booleans were about 30x slower than ogeom's on
the test part, and it has had no release for two years.

### Mitigating the kernel's risk

arris is about two months old (repo created 2026-09-17, first release
0.1.0 the same day, 0.11.1 on 2026-11-16), has one author and 4 stars,
ships about 195 k lines over eight crates, and is developed with heavy
machine assistance. Pre-1.0, every minor release breaks the API (its
changelog lists each break with a fix). ogeom, the previous choice, had
the same profile and then changed its licence. A competent dependency can
still die or change terms. The plan:

1. **Trait boundary.** All kernel use goes through `SolidKernel` (below). No
   kernel type leaks into the feature tree, the sync format or the API;
   `cadview-solid/src/arris.rs` is the only file that names arris.
2. **Exact pin + vendoring right.** arris 0.11.1 is `MIT OR Apache-2.0`,
   and so is every crate it adds, so if upstream disappears or relicenses
   we vendor the pinned crate set and maintain the subset we use
   (booleans, blends, measure, mesh, section). A licence change upstream
   only affects versions after the pin.
3. **A differential corpus.** It runs in the test harness's scoreboards
   on both backends (arris and, since slice 4, cad-kernel in place of
   Truck), so a regression from a version bump is caught before it
   reaches users. The wasm run (3D0b) is a development-time wasmtime host,
   not a CI step (research/3d-corpus-timings.md).
4. **Swappable without migration.** Documents store feature trees, not
   kernel data, so switching backends never rewrites a document.

### The trait

```rust
pub trait SolidKernel {
    type Brep: Clone;
    fn evaluate(&self, def: &PartDef, tools: &dyn ToolResolver) -> Result<Evaluated<Self::Brep>, EvalError>;
    fn tessellate(&self, b: &Self::Brep, tol: Tess) -> Mesh;               // positions, normals, uv, face ids
    fn edges(&self, b: &Self::Brep, tol: Tess) -> Vec<EdgeCurve3>;         // for edge overlay + picking
    fn project(&self, parts: &[(&Self::Brep, Placement)], view: &ViewSpec) -> Drawing2d;
    fn section(&self, parts: &[(&Self::Brep, Placement)], plane: &Plane3) -> Vec<SectionRegion>;
    fn mass(&self, b: &Self::Brep) -> MassProps;                           // exact volume, centroid
    fn bbox(&self, b: &Self::Brep, frame: &Placement) -> Aabb3;
    fn distance(&self, a: &Self::Brep, b: &Self::Brep) -> f64;             // interference, measure3d
}

pub struct Evaluated<B> { pub brep: B, pub face_tags: Vec<FaceTag>, pub edge_tags: Vec<EdgeTag>,
                          pub degraded: Vec<Degradation> /* e.g. fillet fell back to Visual */ }
```

`FaceTag`/`EdgeTag` map kernel entities back to `(FeatId, role, seg)`. This
is how selection, hatching per part and cut-list face naming work without
kernel ids.

### Roundovers: exact first, visual fallback

The evidence says exact fillets are feasible: ogeom (smoke test) and arris
(3D0a) both round all 12 edges of a board and keep booleans exact
afterwards. So the default is `RoundFallback::Error`, a real fillet in the
B-rep. arris refuses with a typed error rather than guesses on corners and
chains its closed forms do not cover. For those, `fallback: "visual"` gives a
render-only bevel: the renderer smooths the edge normals and the mesh gets a
small bevel strip. `cad.project` then writes a 2D note ("R3 roundover, all
outer edges") on the view instead of an arc, and cut lists ignore roundovers
in either case. Every fallback is recorded in `Evaluated.degraded` and shown
by `cad.parts()`, so it is never silent.

## 6. Tessellation strategy and accuracy budget

| Quantity | Source | Budget |
|---|---|---|
| Model geometry | feature tree, f64 mm | exact (analytic) |
| Kernel tolerance | arris `Precision::DEFAULT` | default entity tolerance 1e-7 mm, maximum 1e-2 mm (arris defaults) |
| Volume, centroid, distance, interference | kernel, exact surfaces | ≤ 1e-6 relative (corpus-checked against closed form) |
| Cut-list L x W x T | feature tree (flat parts) | exact numbers as entered; the kernel bbox must agree to 1e-6 mm |
| Projected 2D edges (lines, circles, ellipses) | analytic re-fit of the source edge (§7) | exact |
| Projected 2D edges (other curves, silhouettes of curved faces) | spline fit | ≤ 0.01 mm chordal |
| Hidden/visible split points | parameters on the exact edge, refined against the visibility oracle (§7.3) | ≤ 0.01 mm along the edge |
| Render mesh (viewport) | tessellation at runtime | chord ≤ 0.5 px at current zoom, angle ≤ 5° |
| Render mesh (path tracer / headless) | same, fixed for the shot | chord ≤ 0.25 px at output resolution |

Rules:

- **Tessellate only in the render path.** `cadview-render3d` asks core for
  `mesh(part, Tess { chord, angle })`. Core picks a tolerance bucket (powers
  of 2 between 0.005 mm and 2 mm) from the requested chord and caches by
  `(brep_hash, bucket)`. Zooming in past a bucket triggers re-tessellation
  of the visible parts within a per-frame time budget, so the frame never
  blocks. The coarser mesh is shown until the finer one is ready. This is
  the 3D analogue of the 2D BezPath cache.
- **Flat faces are free.** Planar faces tessellate exactly at any tolerance.
  For DIY furniture most triangles come from holes, roundovers and turned
  parts, which is where the zoom-adaptive bucket pays off. Smoke data: the
  rounded board is 3.5 k triangles at chord 0.1 mm and 65 k at 0.01 mm
  with ogeom; with arris (3D0a) it is 608, 3 672 and 32 592 triangles at
  0.1, 0.01 and 0.001 mm, all watertight.
- **Evaluation is cached by content hash**, and only edited defs
  re-evaluate. Smoke timings (ogeom): about 10 ms for a flat part with two
  cuts, about 1 s natively and 3 s in wasm for a fully rounded board. arris
  builds the fully rounded board in about 22 ms natively (release, 3D0a);
  in wasm (wasmtime, 3D0b) the rounded slab of the curved corpus takes
  15 ms and the slowest corpus part, a swept round bar, 283 ms
  (research/3d-corpus-timings.md). Evaluation
  therefore runs off the input path: in the browser it is time-sliced
  between frames, and fully rounded parts show their previous B-rep until
  the new one is ready. Moving evaluation into a Web Worker is an open
  question (§13).

## 7. 3D to 2D projection

`cad.project` writes ordinary 2D entities into the `Document`, so
dimensions, title block, PDF and DWG export work unchanged.

### 7.0 Revision (2026-11-18, rung 3D3a): what was built differs

Where the sections below disagree, this wins (`docs/api-surface.md`,
"Projected views", has the details):

- **No standard layers.** The `3D_*` table in 7.1 gave layer names meaning
  in the core, which the generic-core rule forbids. The script names the
  layers for visible, hidden and silhouette curves (plain defaults
  `visible` and `hidden`); centre lines, section and note layers are
  script work. Blocks are named `view-<view>` (a script picks another), not
  the anonymous `*V_<name>`. First- or third-angle arrangement is a script
  library, not an option.
- **Splits are crossings, not bisection.** Visibility along a curve can
  only change where, in the view, it crosses another projected edge or
  silhouette, where another curve ends on it, or where a conic seen edge on
  folds back. HLR splits there, at exact parameters (closed form where one
  curve is straight, Newton's method otherwise), and asks the oracle once
  per piece. Bisection remains a fallback when a long piece's samples
  disagree.
- **The oracle trusts the mesh only away from its outline.** A mesh hit
  near a triangle side where the mesh's facing turns (its outline in the
  view), within 50 chords of the point, or at a grazing angle is unsure;
  four rays 8 chords to the side then decide (visible if any escapes), and
  the kernel's exact `ray_fire` only when those are unsure too. arris
  rebuilds its face index per ray (about 55 ms on a part with B-spline
  edges), so the exact rays are a last resort, not the fallback for every
  near graze.
- **Silhouettes** are closed forms in `solid/silhouette.rs` (core,
  kernel-neutral); the kernel clips them to its faces (`silhouettes`). Tori
  are closed form only seen along or across their axis; seen obliquely,
  and on B-spline faces, they are listed as open, not fitted.
- **Smooth edges** (round boundaries) are drawn where they lie on the
  outline of the view (the faces' normal across the view along the edge),
  and everywhere with `smooth: true`.
- **B-spline edges** lying on a line, circle or ellipse of the view (a curve
  on a cylinder seen along its axis) are drawn as that conic; others as a
  cubic spline (Hermite pieces) within 0.01 mm.

### 7.1 Views

`ViewSpec { dir, up, kind: Ortho | Iso | Section(plane), scale, at: [x, y], parts: filter, hidden: bool, tangent_edges: bool }`.
The standard names are `front`, `back`, `left`, `right`, `top`, `bottom` and
`iso`, with first- or third-angle arrangement chosen by option (ISO default:
first angle).

Each view becomes **one block definition** (`*V_<name>`) plus one
`BlockInsert` at `at`, with the scale applied in the insert. Re-projecting
replaces the block content and keeps the insert, so dimensions placed on the
sheet next to it remain, and stale ones are easy to spot. The block's
entities sit on standard layers:

| Layer | Content | Linetype / weight |
|---|---|---|
| `3D_VISIBLE` | visible edges and silhouettes | Continuous, 0.5 mm |
| `3D_HIDDEN` | hidden edges | Hidden, 0.25 mm |
| `3D_CENTER` | axes of revolved features and holes | Center, 0.18 mm |
| `3D_SECTION` | section outlines | Continuous, 0.5 mm |
| `3D_HATCH` | section hatch per material | Continuous, 0.18 mm |
| `3D_NOTES` | roundover notes, part labels | — |

### 7.2 Exact paths for prismatic and turned parts

The HLR adapter (§7.3) returns runs of curves, each split into visible and
hidden pieces and tagged with its **source edge** (a kernel edge of the
B-rep). The adapter re-fits each run analytically from the source edge's 3D curve and
the parameter interval of the run:

- a 3D line becomes a `Shape::Line`.
- a circle in a plane parallel to the view becomes a `Shape::Arc` or
  `Circle`.
- a circle in an oblique plane becomes a `Shape::Ellipse` (with params),
  which covers holes in an iso view and turned legs seen from above.
- a circle in a plane containing the view direction becomes a line.
- anything else (spline edges, silhouettes of cylinders, cones and tori)
  becomes a `Shape::Spline` fitted to ≤ 0.01 mm. Cylinder and cone
  silhouettes in principal views are lines, and the adapter detects and
  emits them as lines.

For the target domain (boards, dados, holes, turned legs) this means the DWG
contains real `LINE`, `ARC`, `CIRCLE` and `ELLIPSE` entities. That is
checkable, and rung 3D3a tests it.

### 7.3 Hidden lines, and how this fits the abstract-geometry rule

**Every emitted curve is an exact source curve.** HLR output is a set of
parameter intervals on exact 3D curves, mapped to 2D by §7.2. The curves
come from two sources:

- **Model edges.** These are lines, circles and B-splines from the B-rep.
- **Silhouettes of curved faces.** These are computed in closed form by
  our own code in `project.rs` (arris has no HLR, §5). For a cylinder or
  cone they are rulings (lines), and for a torus or a surface of
  revolution they are exact curves fitted as in §7.2.

Polylines and silhouettes taken from mesh edges are **never written to
the Document**.

**The mesh is only a visibility oracle.** The question "is the point at
parameter t of this edge hidden?" is answered by casting a ray toward the
eye against a fine tessellation (0.01 mm chord) of the placed parts. That
is fast and robust, and it is what a polygonal HLR does internally.
Each visibility change is located by bisection on the exact edge's
parameter, to 1e-4 mm along the edge. The emitted run is the exact curve
restricted to `[t0, t1]`. A mesh only ever decides *where* a line turns
dashed, to a stated tolerance. It never decides *what* the line is.

arris's `query::ray_fire` (a ray against the exact faces, no mesh) is the
fallback for oracle edge cases (near-grazing faces) and the cross-check
in the 3D3a corpus tests. (In the 2026-10-13 smoke test, ogeom's own
polygonal HLR gave 55 visible and 46 hidden curves for the rounded
board's front view in 9 ms.) The same oracle algorithm runs in our
adapter (`project.rs`) on every backend's mesh, with silhouettes from our
own closed-form code for planes, cylinders, cones and surfaces of
revolution (enough for the target domain). The algorithm is small and is
ours, so the backend never decides the look of a drawing.

**What the rule allows.** AGENTS.md says "never flatten, tessellate, or
triangulate at load time or in the API layer; flattening happens only
during rendering". The oracle tessellates inside an API call
(`cad.project`), but nothing tessellated is stored, synced or emitted, and
the result is exact geometry with a stated split tolerance. This design
treats the rule as covering **stored and emitted geometry**. A transient
mesh used as a numeric oracle is allowed, as long as it never becomes
output and its tolerance is documented. Proposed clarification for
AGENTS.md (not applied, for the maintainer to accept):

> Derived meshes may be used transiently inside an operation as a numeric
> oracle (visibility, ray picking, coarse interference pre-checks),
> provided the operation's stored or emitted geometry is still exact
> (source curves, split at refined parameters), its tolerance is
> documented, and the mesh is never synced or written to the document.

### 7.4 Sections and hatching

A section view cuts the placed parts with a plane through the kernel
(`section`), producing planar regions tagged by part. Each region becomes an
exact outline (arcs stay arcs) plus `addHatch` with the part material's
`section_hatch` (wood: lines at 45°, alternating per adjacent part; plywood:
the same plus a ply-direction note; concrete: the cross/stipple pattern;
steel: dense 45° lines). The part behind the plane is projected with HLR as
in 7.3. Section lines and labels ("A-A") on the parent view use the existing
`draft` helpers.

### 7.5 Dimensions on projected views

`cad.project` returns, for each view, a `toSheet(p3) -> [x, y]` mapping and
named anchors (`view.anchor(part, "bbox.min")`, `view.edge(part, sel)`).
Scripts then call the existing `cad.measure`, `dimRadius` and so on with
sheet coordinates. No new dimension system is needed.

### 7.6 Cut lists

`cad.cutList({ by: "material" })` walks `parts` and groups
`PartKind::Flat` parts by `(label, material, L, W, T)`. Two sides that differ
only by a drilled hole are one row with qty 2, and the per-part differences
go in the notes. It emits rows of
`{ label, qty, length, width, thickness, material, grain, notes }`. Length
follows the grain, and width is the other in-plane extent of the profile
bbox. Notes collect edge banding, roundovers and holes ("4x Ø5 shelf pins").
The numbers come from the feature tree and are confirmed against the kernel
bbox. `cad.cutListTable(rows, { at })` draws the table as 2D text and lines
next to the title block. Non-flat parts (turned legs) get a "stock" row
(blank size = bbox rounded up by a configurable allowance). Sheet nesting is
out of scope: rows can be exported as CSV through the returned JS array.

### 7.7 Interference and measure (the reality check)

The point of 3D here is to catch 2D mistakes, so two queries ship with the
projection rung:

- `cad.interference()` evaluates pairwise `common` volume between parts that
  are not deliberate joinery (a part that has a `Boolean{tool: Part}`
  against another is exempt for that pair). It returns
  `[{a, b, volume, bbox}]`. Typical catches: a shelf 2 mm too long for its
  dado, or a drawer that hits the frame.
- `cad.measure3d(a, b)` is the exact minimum distance between parts, faces
  or points.

## 8. Assets

- **Reference, not content.** `AssetRef.key` is a document-store key
  (`assets/polyhaven/<slug>/<slug>_2k.glb`). Assets sync as the ~150-byte
  `AssetRef`. Bytes come from `GET /api/files/<key>` (bearer token, cached,
  `ETag`). In single-file mode the server serves keys relative to the DWG's
  folder.
- **Excluded** from `cad.project`, `cad.cutList`, `cad.interference` and DWG
  export. JSON save keeps the reference.
- **Plan symbol.** `plan_symbol: Footprint` makes `cad.project` (top view)
  and the 2D plan show the asset's bbox outline with a light diagonal on
  layer `3D_ASSETS`. `Block(name)` places a user 2D block instead (a chair
  symbol). The plan symbol is generated, not stored. It is regenerated with
  the view.
- **Loading.** The `gltf` crate (1.4.1) is already a dependency of
  kiss3d 0.47, whose glTF loader handles meshes, PBR materials and textures.
  Assets are render-only, so they never enter the kernel.
- **Sourcing.** [Poly Haven](https://polyhaven.com/license) models, HDRIs
  and textures are CC0. A `just fetch-assets` script downloads a pinned
  manifest (slug, resolution, sha256) into `assets/`, which is `.gitignore`d.
  Example scripts reference only manifest slugs, so a fresh clone runs
  `just fetch-assets` once. The site/Pages build fetches the same manifest.

## 9. Renderer architecture

### Decision update (2026-11-18): our own raster for the working render

Two render qualities draw the same scene and camera: a fast **working**
render (shaded raster with edges) and a **presentation** render (path
traced). The first 3D2b slice ran Kiss3d 0.47.0 at run time, on a GPU,
for the first time:

- **What held.** It resolves to the workspace's single wgpu 30.0.2 (no
  second wgpu, naga or vello). `Context::init` accepted the `GpuContext`
  device, `OffscreenSurface` rendered a correct shaded box headless, and
  two renders were byte-identical (554 ms first, 137 ms warm, debug).
- **What failed.** Its context and resource managers are thread-local.
  On a second thread it panicked in its texture manager and then aborted
  the process during thread-local teardown. Dropping its last surface
  calls `destroy()` on the device it was handed, which is the device Vello
  shares; avoiding that means bumping an internal window counter, and
  then teardown panics at exit instead. A server renders on any worker
  thread, so this does not fit the headless path. It would also add 98
  crates to `Cargo.lock` (all permissive licences; among them an AV1
  encoder from `image`'s default formats, a parser generator for `wesl`,
  and `winit`, which cannot be switched off). A debug probe binary was
  39 MB against 17 MB for our own backend.

So the working render is a small rasterizer of our own in
`cadview-render3d` (§9.1's `KissBackend` row becomes `RasterRenderer`),
and the decisions it needs (normals, edges, camera, shading, tone curve)
live in `cadview-core/src/scene3d.rs`. Every face pixel it draws equals
core's reference `shade` plus `tone_map`, which a Kiss3d backend could
not offer. The `Renderer3d` trait lets a frame take several calls, so
the presentation render can be Kiss3d's `RayTracer` behind the same
trait, run on one dedicated thread that owns all Kiss3d state, or its
BSD-3 tracer kernel forked into the crate (§9.5). That choice is made at
3D5a. Easy with our backend: per-part colours, edges, cutaway clip planes
(a plane test in the fragment shader). A soft ground contact shadow needs
a shadow or ambient-occlusion pass of our own, which Kiss3d already has.

### Decision update (2026-11-18, 3D5a): our own path tracer for the presentation render

The presentation render is a path tracer of our own, `PathTracer` in
`cadview-render3d` (`trace.rs`, `trace.wgsl`, `resolve.wgsl`, `bvh.rs`).
No code is taken from Kiss3d; there is no notices file because nothing
needs one. Kiss3d stays out of `Cargo.lock`.

Kiss3d's tracer was read in full (0.47.0: `rt_preamble.wgsl`,
`rt_intersect_bvh.wgsl`, `rt_kernel.wgsl`, 1.4k lines of WGSL, and the
Rust around it) as the candidate to fork. What decided against it:

- **It loses energy under an environment.** The environment is never
  sampled directly, yet a bounce ray that escapes is weighted by the power
  heuristic against a uniform-sphere pdf of `1/4pi` that no strategy
  draws from. Light from the sky is lost, the more so at grazing angles:
  a furnace test cannot pass. That is likely part of the "dry" look.
- **Its diffuse sampling and evaluation disagree.** The evaluated diffuse
  lobe carries a `(1 - F)` factor; the sampled one does not.
- **Smaller faults:** one non-finite sample zeroes the pixel's whole
  running mean; a full traversal stack drops nodes silently; ray offsets
  are a fixed 1e-4 in world units (our parts span 10 mm to 3 m); Y is up
  and the environment turns about Y; the modules are joined by WESL
  imports, which need the `wesl` crate or a rewrite to plain WGSL.
- **What it has that we lack:** an a-trous denoiser (127 lines), texture
  arrays, point and spot lights, alpha cut-outs, fog, depth of field,
  subsurface, and a hardware ray-query path.

Fixing the first two means rewriting the sampling core, and the rest is
features this slice does not want, so a fork would have kept little more
than the triangle and box tests. Our own tracer is about 1000 lines of
WGSL and 1400 of Rust with its unit tests, written against the core's scene from the start,
and checked against closed forms (below). What we gave up: the denoiser
(at 1024 samples the examples need none except inside polished bores), image
textures, lights other than the sun, depth of field and hardware ray
queries.

How it works. Every distinct mesh gets a bounding volume hierarchy over
its triangles in its own frame (binned surface-area heuristic, built once
on the CPU and cached while the mesh is in use); a second hierarchy over
the items' world boxes points at them, so repeated parts are instances.
One compute invocation per pixel traces one path per sample: a camera ray
from the core's camera (both projections, the same pixel grid as the
raster, so silhouettes agree), the closest hit in both levels (ordered
traversal, re-tested pops), the core's material and pattern at the hit,
then next-event estimation of the sun (uniform in its cone, power-heuristic
weighted against the BSDF), and a BSDF sample for the next ray (Lambert
under GGX with exact dielectric Fresnel, visible-normal sampling,
anisotropic for brushed patterns, a clear coat, smooth or rough glass as a
delta event). The dome is reached by BSDF sampling only (it is smooth).
The ground is an analytic plane. Paths stop on escape, after 16 events, or
by Russian roulette from the fourth. Ray origins are moved off the surface
by a few units in the last place (Wächter and Binder, Ray Tracing Gems
2019), so the offset scales with the coordinates. Cut-away views are
traced too: hits beyond the clip planes are skipped, and a ray that meets
the inside of a part through a cut is shaded flat in the section colour on
the plane it crossed.

Determinism and progress. Each pixel keeps a running sum, and a pass adds
its samples to it one at a time in sample order; sample `s` of pixel `p`
draws its random numbers from PCG hashes of `(p, s, seed)`. The image is
therefore the same however samples are split into passes (passes are sized
to about 60 ms on native, so no pass trips a GPU watchdog). A full-screen
pass divides by the sample count, applies the exposure and the core's tone
curve, and writes the sRGB target, so a viewport can show every pass.

Firefly control: indirect contributions (after two or more scattering
events) are clamped in luminance at 10 times that of a white surface lit by
the open environment. Direct light and what the camera sees are never
clamped. In a studio scene with polished and brushed metal it moved the mean
by 0.08 % (inside the noise); a glass sphere in sunlight loses its caustic,
6.6 % of the image's mean (a known bias: caustics from a small sun reach a
path tracer only through rare paths).

What a second backend must agree on lives in `scene3d.rs`: the physical
`Material` (transmission, index of refraction, emission, clear coat, a
procedural `Pattern`), `pattern_albedo`, `pattern_roughness`,
`streak_direction`, the `Environment` (dome and `Sun`, with the sun's
radiance normalised so a surface facing it receives its stated
irradiance), `exposure_scale` and `tone_map`. The working render ignores
transmission, emission, the coat, the pattern, the environment and the
exposure.

Second slice (see LADDER-3D.md, "3D5a, second slice"): paths now end after
64 events, not 16; the dome is also sampled directly through `Portal`
rectangles; sun shadow rays pass through glass (attenuated, not bent);
rough glass keeps its energy along the lobe; and an edge-aware a-trous
denoiser (`denoise.wgsl`), guided by first-hit albedo, normal and
position, is on by default in `cad.render`. `Environment::sweep` and
`Scene3d::portals` join the decisions a second backend must agree on.

### 9.1 Layers

```
 cadview-render3d
 ├── trait Renderer3d { sync(&Document), set_camera, set_mode, frame(&mut Target) -> FrameState }
 ├── KissBackend   (kiss3d 0.47: raster PBR + RayTracer + IBL + tonemap)
 ├── scene_sync.rs Document -> GPU scene: parts (core meshes, per-part material),
 │                 edges overlay (core edge curves, tessellated to 0.5 px), assets (glTF),
 │                 HDRI (store key), lights (sun from scene3d: date/latitude optional)
 ├── camera.rs     orbit/pan/zoom, ortho <-> perspective, named views, fit
 └── headless.rs   (native) OffscreenSurface -> PNG, fixed spp, deterministic seed
```

### 9.2 Modes

- **Shaded** (default, interactive): kiss3d's rasterizer with PBR, IBL from
  the scene HDRI, shadow maps, SSAO and MSAA. It is demand-driven like the
  2D renderer: zero frames when idle.
- **Lines**: exact part edges (tessellated to screen tolerance) over flat
  shading, hidden edges dashed. It is the on-screen twin of `cad.project`.
- **Render**: our `PathTracer` (compute BVH on plain WebGPU; see the 3D5a
  decision update above). It is progressive, accumulating a pass of
  samples per call while idle, and stops at the target spp (default 1024
  headless; 256 is the browser's aim). Any camera move resets the
  accumulation (`Renderer3d::reset`, or a changed frame). Rendering stops
  as soon as the target is reached, which keeps the zero-idle-frames
  property. An edge-aware denoiser runs on each resolve (second slice).

Tone mapping is AgX by default for renders and Khronos PBR Neutral for
Shaded mode (more faithful colours for checking materials). Exposure is an
EV value in `scene3d`.

### 9.3 Sharing the device with Vello

Today `vello_render.rs` creates a device per canvas (`init_gpu`). The plan
is to change that to **one `wgpu::Device` per page**, held in a small
`GpuContext` registry in `cadview-web` and handed to both Vello and
`kiss3d::context::Context::init` (compile-proven on wgpu 30). Each canvas
keeps its own surface. A 3D canvas renders kiss3d into an `OffscreenSurface`
and blits its `output_view` to the canvas with the same `TextureBlitter`
path Vello uses. Overlays (selection, dimensions-in-3D later) can be Vello
scenes composited over the 3D texture. One device means one place for
limits and features, and no cross-device texture copies.

Kiss3d keeps thread-local global managers. That is acceptable in a shell
crate (core keeps zero global state). Version 0.47 fixed the
"second `OffscreenSurface` replaces managers" bug, which matters for several
3D tabs. Rung 3D2c proves two 3D canvases at once.

### 9.4 Headless server-side rendering

`cadview-server` gets a `render` RPC (and the sandbox gets `cad.render()` as
an `rpc-call`). It runs `cadview-render3d::headless` natively: the same
scene sync, then a fixed spp, a fixed seed and a PNG. It is used for example
PNGs (960x640) and for agents to check their work visually. On a GPU-less
host, wgpu uses Vulkan through lavapipe. That is **unverified** (the research
machine had no Vulkan driver), and rung 3D6b must prove it in CI with a time budget.
Fallback: the existing headless-Chromium `render.mjs` path with WebGPU.

### 9.5 If the look gate fails

The maintainer found Kiss3d renders "dry". Most of that look usually comes
from content, not the integrator: no HDRI, a lone point light, low-albedo
default materials, a clamped tone map. Rung 3D5b sets up a reference scene
(bookshelf, Poly Haven HDRI, oak and plaster PBR textures) and compares it
against Blender Cycles with the same assets, using ꟻLIP as the metric.
Superseded in part by the 3D5a decision update: the tracer is our own
(`cadview-render3d/src/trace.wgsl`), so a failed gate means fixing its BSDF,
sampling or environment there. The gate also needs what this slice left
out: image environments (HDRI) and texture sets.

## 10. Scripting API sketch

**Superseded where it differs by the API built in 3D1a** (reference:
`docs/api-surface.md`, "Solid parts"): `subtract` is `cut` (with `union`
and `intersect`), taking an inline tool body or another part; `roundover`
is `fillet3d`; `copy3d` shares the definition by default (`unique: true`
for an independent copy), the reverse of the sketch below, so repeated
parts evaluate once; `board`, `pocket`, `material` and the cut list are
library code or later rungs (4.0); 3D renders go through the existing
`cad.render` with a `view`.

New `cad_call` methods, exposed through `cad-api.ts`. All units are document
mm and all angles degrees, as in the 2D API. Every creation call returns the
created object with an `id`, and all mutations accept the usual target forms
(id, array, filtered objects).

```js
// Materials (presets carry PBR + section hatch + stock defaults)
cad.material("oak", { preset: "wood-oak", textures: "polyhaven:oak_veneer_01" })
cad.material("ply", { preset: "birch-ply" })

// Flat part: the DIY workhorse. profile = rectangle size, ring, or 2D entity ids
cad.board({ name, size: [w, l] | profile, thickness, plane: "xy"|"xz"|"yz"|{origin,normal,xdir},
            at: [x,y,z], material, grain: "length"|"width", layer })

// General features (return a part)
cad.extrude(profile, { plane, at, distance, taper, material, name })
cad.revolve(profile, { plane, axis: [[x0,y0],[x1,y1]], angle: 360, at, material, name })
cad.sweep(profile, path3d, { material, name })
cad.loft([{ profile, plane, at }, ...], { ruled: false, material, name })

// Modify (adds features to the part's def)
cad.subtract(part, tools, { keepTools: true, clearance: 0 })   // joinery
cad.union(part, tools); cad.intersect(part, tools)
cad.hole(part, { at, dir, dia, depth: "through" | mm, counterbore: [dia, depth] })
cad.pocket(part, profile, { plane, depth })                    // dado/rabbet as a profile cut
cad.roundover(part, { edges: "outer-top" | EdgeSel | points, r, fallback: "visual" })
cad.chamfer3d(part, { edges, d })

// Placement and queries
cad.move3d(target, [dx,dy,dz]); cad.rotate3d(target, deg, axis, about)
cad.copy3d(target, [dx,dy,dz], { shareDef: false }) // default: independent copy (own PartDef);
                                                    // shareDef: true = instance of the same def
cad.parts(filter?) // [{id, name, def, material, bbox, volume, kind, degraded}]
cad.measure3d(a, b) // exact min distance
cad.interference() // [{a, b, volume}]

// Assets
cad.placeAsset("assets/polyhaven/potted_plant_01/potted_plant_01_2k.glb",
               { at, rotate: deg, scale: 1, planSymbol: "footprint" | blockName | null })

// 3D -> 2D
const views = cad.project({ parts, views: ["front","left","top","iso"], at: [x,y], gap: 40,
                            scale: 1/10, hidden: true,
                            sections: [{ name: "A-A", plane: { origin, normal }, in: "front" }] })
// views.front.toSheet([x,y,z]) -> [sx, sy]; views.front.anchor(part, "bbox.max")
const rows = cad.cutList({ by: "material" })
cad.cutListTable(rows, { at: [x,y] })

// Rendering (client: viewport state; server/sandbox: headless RPC)
cad.view3d({ camera: { eye, target, fov }, hdri: "polyhaven:studio_small_09",
             exposure: 0, tonemap: "agx", mode: "shaded" | "lines" | "render" })
await cad.render({ out: "renders/bookshelf.png", size: [960, 640], spp: 1024 })
```

### 10.1 Worked example: a bookshelf

An 800 x 1800 x 300 mm oak bookshelf: two 18 mm sides with 6 mm-deep dados,
five shelves, a top that sits on the sides, and a 6 mm plywood back in a
rabbet. The script models
it, checks it, renders it with a plant beside it, projects a shop drawing
and emits a cut list. It is the target script for the 🎯 rung
(`examples/07-bookshelf.js`).

```js
const draft = await cad.use("drafting");
const { P, dim } = draft;

// ── Parameters ─────────────────────────────────────────────────────
const W = 800, H = 1800, D = 300;      // outside dimensions
const T = 18, BACK = 6, DADO = 6;      // board, back panel, dado depth
const nShelf = 5;
cad.material("oak", { preset: "wood-oak", textures: "polyhaven:oak_veneer_01" });
cad.material("ply", { preset: "birch-ply" });

// ── Parts (x = width, y = depth, z = up) ───────────────────────────
const sideL = cad.board({ name: "side", size: [D, H - T], thickness: T, plane: "yz",
                          at: [0, 0, 0], material: "oak", grain: "length" });
const sideR = cad.copy3d(sideL, [W - T, 0, 0]);
const inner = W - 2 * T + 2 * DADO;    // shelves run into both dados
const shelves = [];
for (let i = 0; i < nShelf; i++) {
  const z = 60 + i * ((H - 60 - T) / nShelf);
  shelves.push(cad.board({ name: "shelf", size: [inner, D - BACK], thickness: T,
                           plane: "xy", at: [T - DADO, BACK, z], material: "oak" }));
}
const top = cad.board({ name: "top", size: [W, D], thickness: T, plane: "xy",
                        at: [0, 0, H - T], material: "oak" });
const back = cad.board({ name: "back", size: [W - 2 * T + 2 * DADO, H - T], thickness: BACK,
                         plane: "xz", at: [T - DADO, 0, 0], material: "ply" });

// ── Joinery: cut the shelves and back into the sides ───────────────
for (const side of [sideL, sideR]) {
  cad.subtract(side, [...shelves, back], { keepTools: true });   // dados + rabbet
  cad.roundover(side, { edges: "outer-front", r: 3 });
}
cad.roundover(top, { edges: "outer-top", r: 3 });
cad.hole(sideL, { at: [T, D - 40, 1000], dir: [-1, 0, 0], dia: 5, depth: 10 }); // one shelf-pin hole

// ── Reality check ──────────────────────────────────────────────────
const clashes = cad.interference();
if (clashes.length) throw new Error("parts collide: " + JSON.stringify(clashes));

// ── Render with context ────────────────────────────────────────────
cad.placeAsset("assets/polyhaven/potted_plant_01/potted_plant_01_2k.glb",
               { at: [W + 250, 150, 0], planSymbol: "footprint" });
cad.view3d({ camera: { eye: [2400, -2600, 1500], target: [400, 150, 900], fov: 35 },
             hdri: "polyhaven:studio_small_09", tonemap: "agx" });
await cad.render({ out: "examples/07-bookshelf-render.png", size: [960, 640], spp: 1024 });

// ── Shop drawing on an A3 sheet at 1:10 ────────────────────────────
draft.setup({ scale: 10 });
draft.sheet({ at: [0, 0], w: 420, h: 297, title: "BOOKSHELF 800x1800", subtitle: "OAK / BIRCH PLY",
              number: "CC-007", scale: "1:10" });
const v = cad.project({
  parts: cad.parts(), views: ["front", "left", "top"], at: [P(40), P(60)], gap: P(25),
  scale: 1 / 10, hidden: true,
  sections: [{ name: "A-A", in: "front", plane: { origin: [W / 2, 0, 0], normal: [1, 0, 0] } }],
});
dim(v.front.toSheet([0, 0, 0]), v.front.toSheet([W, 0, 0]), { offset: -P(8) });          // 800
dim(v.front.toSheet([0, 0, 0]), v.front.toSheet([0, 0, H]), { dir: "v", offset: -P(8) }); // 1800
dim(v.left.toSheet([0, 0, 0]),  v.left.toSheet([0, D, 0]),  { offset: -P(8) });          // 300

// ── Cut list ───────────────────────────────────────────────────────
const rows = cad.cutList({ by: "material" });
cad.cutListTable(rows, { at: [P(250), P(200)] });
console.log(rows.map(r => `${r.qty}x ${r.label} ${r.length}x${r.width}x${r.thickness} ${r.material}`).join("\n"));
return cad.saveDwg("examples/07-bookshelf.dwg");
```

Expected cut list (checked by hand, and a golden test in rung 3D3b):

| Qty | Part | L | W | T | Material | Notes |
|---|---|---|---|---|---|---|
| 2 | side | 1782 | 300 | 18 | oak | 5 dados 18x6, rabbet 6x6, R3 front edges; left only: 1x Ø5 hole |
| 5 | shelf | 776 | 294 | 18 | oak | |
| 1 | top | 800 | 300 | 18 | oak | R3 top edges |
| 1 | back | 1782 | 776 | 6 | ply | |

## 11. Crate layout

| Crate | Kind | Change | Targets |
|---|---|---|---|
| `cadview-core` | core | new `solid/` module (types, eval, kernel trait, mesh checks and cache, project, section, cutlist, check); `dispatch_3d.rs`; sync maps. Links no kernel | wasm32 + native |
| `cadview-solid` | core | new: `ArrisKernel`, the only crate that names arris (exact pin). Built and tested; no shipped crate links it yet | wasm32 + native |
| `cadview-render` | shell-shared | new (LADDER-NATIVE N0b): the 2D Vello scene builder, camera, LOD and path cache, extracted from `cadview-web` | wasm32 + native |
| `cadview-render3d` | shell-shared | new: `Renderer3d` trait, Kiss3d backend, scene sync, camera, headless | wasm32 + native |
| `cadview-store` | shell-shared | new (LADDER-NATIVE N2b): `DocumentStore` trait, `SingleFileStore`, `FolderStore`, moved out of `cadview-server` | native |
| `cadview-web` | shell | `GpuContext` (one device per page), vello 0.9 → 0.11 and eframe/egui 0.34 → 0.36, 3D canvas start/stop, wasm exports | wasm32 |
| `cadview-server` | shell | `/api/files/<key>`, `/api/session`, `render` RPC, `cad.render` in the sandbox via `rpc-call`; uses `cadview-store` | native |
| `cadview-desktop` | shell | new (LADDER-NATIVE): eframe window, chrome, WT client, optional local sandbox | native |
| `web/src` | shell | `cad-api.ts` 3D methods, 2D/3D toggle per tab in `ViewportContainer`, part list in the layer panel | TS |
| `examples/` | — | `07-bookshelf.js`, `08-turned-stool.js`, `assets.manifest.json` | — |

**Why shell-shared crates don't break core-first.** The rule says CAD logic
goes in core, and web and server stay thin. `cadview-render` and
`cadview-render3d` exist only because their code is **GPU-bound**: wgpu,
Vello, Kiss3d and shaders. That code cannot go in core, which must stay
GPU-free so the server and sandbox can use it without a graphics stack.
Without these crates it would be duplicated in two shells (web and
desktop). `cadview-store` is the same case for filesystem access (server
and desktop). These crates hold **no CAD logic**. They read the `Document`
and core-computed data (meshes, edge curves, tessellation buckets, scene
settings) and turn them into pixels, or bytes into documents. Any decision
about geometry (what a part is, how it projects, what the cut list says,
which tolerance applies) stays in core. A review checklist item enforces
it: no `cadview-core` types are constructed or mutated in these crates
except through `cad_call` or core's public query functions.

## 12. Risks

- **Kernel maturity and bus factor (arris).** Mitigations in §5: trait, pin,
  differential corpus, vendoring right, kernel-free documents.
- **Evaluation latency in the browser.** Fully rounded parts took seconds
  in wasm in the smoke test. Caching, time-slicing and possibly a worker
  (§13) handle it. Rung 3D0b records per-corpus-part timings native and wasm.
- **Kiss3d fit.** Global thread-local managers, an engine-shaped API and the
  "dry" look. The trait boundary plus the 3D5b look gate with a fork fallback
  bound the risk.
- **wgpu major bumps across Vello, Kiss3d and egui.** They are aligned on
  wgpu 30 today and will drift. Pin all three and bump together. The single
  `GpuContext` makes a mismatch a compile error, not a runtime surprise.
- **Headless rendering without a GPU** (lavapipe) is unproven here. It could
  be too slow for 1024 spp. Fallbacks: fewer spp plus the denoiser, a
  GPU-enabled runner for releases, or the Chromium path.
- **wasm size** grows by the kernel (ogeom measured 3.2 MB; arris is
  measured in 3D0b: about 4.1 MB of code and data, 1.4 MB gzipped,
  research/3d-corpus-timings.md) and a few MB for Kiss3d. That is acceptable against today's 24 MB, but rungs 3D0b and 3D2a record it.
- **Floating-point divergence between peers** (wasm vs native) in kernel
  results. Nothing synced depends on it (§4.2). Projections are written
  once by one peer.

## 13. Open questions

1. Should evaluation run in a Web Worker (a second wasm instance with its
   own `Document` replica fed by Yrs updates)? That would keep the UI fluid
   for heavy parts but doubles memory. Decide after rung 3D0b timings.
   Answered for now in section 14.2: the kernel module runs on the page's
   thread, so the script API stays synchronous; a Worker needs
   cross-origin isolation or JSPI.
2. STEP export of parts (arris and Truck both write STEP), for CNC shops.
   It is cheap to expose. Should it be a rung or a one-liner later?
3. Should a section's hatch respect wood grain direction per part
   (end grain vs long grain), as some furniture-drawing conventions do?
4. Do turned parts (revolve) get a lathe-style "turning drawing" with
   diameters at stations as a projection option?
5. Should DWG export of 3D be supported, as 3DSOLID/ACIS via acadrust? That
   is likely never, since the 2D projection is the deliverable. This
   confirms the non-goal.
6. Should Kiss3d's HW ray-query backend be used on native desktop
   (LADDER-NATIVE) by default? It makes renders faster and gives the same
   images in principle. Verify equivalence in 3D5b.

## 14. 3D in the browser

Status: built (2026-11-18), phase 1 and phase 2 below. The browser runs the
same core code as the server for parts: the same scene builder
(`solid::scene::parts_scene`), the same evaluation cache (`solid::eval`),
the same raster and path tracer (`cadview-render3d`). What differs is
where the kernel's answers come from: a prebaked file, the server, or a
kernel module the page loads itself.

### 14.1 The split: what loads when

| Piece | File | Size raw / gzip | Fetched | Cached, versioned |
|---|---|---|---|---|
| Main module: 2D (Vello, egui), core, the 3D view (raster and path tracer), the bake reader, the kernel proxy | `cadview-web_bg-<hash>.wasm` | 25.94 / 8.71 MB (before this work 25.33 / 8.53 MB) | page load | Vite's content hash in the name |
| A prebaked scene | `<name>.bake3d` | 17 to 139 KB / 3 to 58 KB for the gallery | when a viewer shows it | the page names it; a format version in its header, refused when it differs |
| The kernel module | `kernel/cadview-kernel-<sha256 prefix>.wasm` and `kernel/manifest.json` | 4.33 / 1.42 MB | the first call that needs geometry, or `cad.loadKernel()` | content hash in the name (cache forever); the manifest is fetched fresh; a wire version in the manifest and in the module, checked against the main module's |

- **The 3D renderers stay in the main module.** Together with the view, the
  bake reader and the kernel proxy they add 608 KB raw and 177 KB gzipped
  (2 % of the module); the path tracer alone is 97 KB raw and 29 KB
  gzipped of that (measured by building the view without it). A split
  would not pay even if it were bigger: the tracer must draw on the
  page's one GPU device, which lives in the main module's memory as a
  `wgpu::Device`. A second module would bring its own wgpu and open its own
  device (breaking one device per page), and wgpu has no way to share a
  device between two modules through JavaScript.
- **Prebaked scenes are data files**, fetched by whoever shows them. Their
  format is versioned on its own (section 14.3).
- **The kernel is a second WebAssembly module** (crate
  `cadview-kernel-wasm`): arris through `cadview-solid`, behind core's byte
  interface (section 14.2). `just build-kernel` writes it with a manifest:
  `{file, wire, bytes, sha256}`. The page fetches the manifest with
  `cache: no-cache`, refuses a wire version other than its own, then
  compiles the module from a streaming fetch (`compileStreaming`, so the
  browser's code cache keeps the compiled code across visits), checks the
  module's own `ck_wire()` and says hello over the interface, which checks
  the version a third time. A piece built for another wire version is
  never used: its manifest names that version, and the module answers
  with it.
- **Linking the kernel small.** rustc exports every `#[no_mangle]` symbol of
  the dependency tree from a wasm module. cadview-core links printpdf, whose
  wasm-bindgen API is exported on wasm unconditionally; as exports those
  functions are roots, and they keep about 10 MB of code and data the
  kernel never calls (the PDF reader and the HTML layout engine with its
  fonts). `crates/cadview-kernel-wasm/link.sh` drops every export but the
  module's own, so the linker can drop the rest: 15.4 MB (5.5 MB gzipped)
  became 4.33 MB (1.42 MB gzipped; symbol names stripped too), with no
  imports. The main module carries the same printpdf weight; whether core needs printpdf's default `html`
  feature is worth a look (not changed here).

### 14.2 The kernel boundary

Core gets its kernel through the object-safe `Evaluator` (solid/eval.rs).
In the browser that evaluator is a proxy, `RemoteEvaluator`
(`cadview-core/src/solid/remote.rs`), and the kernel module runs a
`KernelServer` over the real one. Both are the same file.

- **One request type, one reply type.** `Request` has a variant per
  `Evaluator` and `Solid` method (`Evaluate`, `EvaluateFrom`, `Distances`,
  `Interferences`, `Mass`, `Bbox`, `Check`, `Tessellate`, and since 3D3a
  `Topology`, `Silhouettes`, `RayHits`) plus `Hello` and `Release`;
  `Reply` carries the answers as the serializable mirrors in
  `solid/wire.rs`, or as the kernel types themselves where they derive
  serde (the 3D3a ones do). Both are bincode, wire version 2. Definitions travel with every tool
  definition they reach (the page's resolver, flattened), so the module
  needs no copy of the document.
- **Solids stay in the module.** A reply names a solid by a handle. The
  page's evaluation cache (`Solids`) keys handles by content, as it keys
  B-reps natively, and caches boxes, volumes and meshes on the page, so each
  is asked across the boundary once. `evaluate_from` continues a cached
  prefix by its handle, so a modifying call costs about one feature here
  too. When the cache drops a solid, its handle is released with the next
  request (a drop never makes a call of its own); when the environment is
  dropped, all of its handles are released.
- **It cannot fall out of step silently.** `COVERAGE` lists which request
  each trait method becomes, and `every_trait_method_crosses_the_boundary`
  reads eval.rs: a method of `Solid` or `Evaluator` without an entry, or
  without a proxy method in remote.rs (which would fall back to the trait's
  default in the browser), fails the test. Kernel answers are mirrored in
  wire.rs by destructuring, so a field added to a kernel type fails to
  compile there. The wire types use their own `Wire` trait, not `From`:
  `From` impls between them and the kernel types made `?` ambiguous in the
  arris adapter.
- **Synchronous, like the script API.** The module runs on the page's
  thread; the main module calls `globalThis.__codecadKernel.call(bytes)`
  and gets the reply bytes back in the same call, so `cad.parts()` and every
  modifying call stay synchronous, as on the server. A Worker would need
  either `Atomics.wait` on shared memory (cross-origin isolation: COOP and
  COEP headers, which GitHub Pages cannot send without a service-worker
  shim) or JSPI (Chromium only) to keep that API; open question 1 of
  section 13 is answered "not now".
- **Before the module is loaded.** A page that can load a kernel puts a
  placeholder evaluator (`NotLoaded`) on its documents. A call that needs
  geometry fails with: "the solid kernel is not loaded in this page yet: run
  `await cad.loadKernel()` once (about 1.5 MB), then call again", and that
  failed call starts the fetch, once. A script that builds parts awaits
  `cad.loadKernel()` first; the 3D view loads it itself when it has neither
  a bake nor a server to ask. Without a kernel module in the build the
  message stays and `cad.loadKernel()` says the build has none. Parts that
  arrive by sync are stored as before; only geometry needs the kernel.
- **How long a call blocks.** Measured in headless Chromium on the 3D1a
  assembly (section 14.5): 2.5 s for the whole script (22 part calls
  and three set queries), the slowest call `overlapping3d` over the
  twelve parts at 1.3 s, then `distance3d` at 0.6 s and a cut at 0.15 s
  (a debug server, whose arris is optimised, took 1.5 and 0.7 s for the
  two queries). 3D0b measured wasm at about 1.5 times native for
  evaluation, so the gallery's heaviest call (a feature on the enclosure
  base, 57 + 71 + 23 ms natively in release) is about a quarter of a second
  in the page. The 3D view keeps its last image while a call runs; nothing
  else is done about it yet.
- **A trap.** wasm32 aborts on panic. The loader then starts a fresh
  instance (its solids are gone) and the call fails with a message asking
  for `cad.loadKernel()` again, which installs a fresh environment.

**From the sibling repositories** (ideas only, nothing taken as code):
wasmnet's browser ladder and decisions 0053 and 0065 name modules by the
hash of their bytes and serve them cache-forever with a small manifest,
compile from a real fetch so the engine's code cache holds, and cross the
module boundary with plain alloc, call and free exports over bytes. Their
open item "no version handshake between the two sides" is why this
boundary checks the wire version three times. codelib's browser host kit
runs a module in a Worker with a wall-clock deadline and stubs imports a
module declares but never calls; the stubs are used here, the Worker is
the model for a later non-blocking kernel.

### 14.3 Prebaked scenes

A bake is a snapshot of the evaluation cache, not a picture. Written by a
peer with a kernel (`cad.bake3d({out, ...render options})` in a script,
`GET /api/bake3d?doc=<key>` for the app), read by any page
(`solid::baked::Baked`).

- **Contents.** The document's definitions and parts (placements,
  attributes) as JSON text, so a file survives a new optional field in the
  feature tree; the render options it was baked with (view and projection,
  the default camera; colours, materials, environment, ground); and per
  distinct solid (definitions with equal features count once, shared
  definitions once) its mesh at one chord, its exact box in its own frame
  and at every placement a part gives it, and its mass properties.
  Positions are f32 (what the GPU draws); boxes and volumes stay f64.
- **Format.** `CCB3`, a u32 format version, then bincode. A reader
  refuses another magic or version and says to bake again.
- **Loaded, it is an evaluator.** `BakedEvaluator` answers `evaluate` by
  content key, and its solids answer mass, box (at a baked placement) and
  mesh from the file. So a viewer builds its scene with the server's code,
  and `cad.parts()` on a bake returns the kernel's numbers bit for bit.
  Anything else (a new or edited definition, a new placement, a query
  between parts) fails with "the prebaked scene does not hold this answer
  ... it needs the solid kernel": the moment to load it.
- **Chord.** A quarter pixel at 1280 px (`BAKE_PIXELS`), the rule
  `cad.render` uses, so views up to about twice that size show no facet
  wider than half a pixel. Where the kernel refuses that chord for a solid
  (the enclosure's lid does), the bake doubles it up to three times and
  reports `coarser`.
- **Sizes**, the gallery baked with its isometric view and the material
  presets:

  | Example | Parts | Solids | Triangles | Chord (mm) | Raw | Gzip |
  |---|---:|---:|---:|---:|---:|---:|
  | 01 pipe flange | 1 | 1 | 2 454 | 0.031 | 48 486 | 18 341 |
  | 02 mounting bracket | 1 | 1 | 1 344 | 0.016 | 30 029 | 10 701 |
  | 03 bookshelf | 12 | 8 | 296 | 0.25 | 17 448 | 3 339 |
  | 04 enclosure | 9 | 8 | 3 840 | 0.016 (one solid coarser) | 90 581 | 27 804 |
  | 05 piston and rod | 3 | 3 | 5 424 | 0.031 | 105 163 | 38 761 |
  | 06 spur gears | 2 | 2 | 3 782 | 0.031 | 137 613 | 41 899 |
  | 07 pipe spool | 7 | 5 | 6 154 | 0.25 | 120 000 | 53 301 |
  | 08 turned stool | 7 | 3 | 7 298 | 0.125 | 139 121 | 57 540 |
  | 11 play tower frame | 63 | 17 | 236 | 1 | 41 632 | 6 915 |

- **Where they would live.** About 0.73 MB raw for the nine; small enough to
  commit next to the examples (`examples/3d/*.bake3d`), made by the same
  export step that writes the example PNGs. Not committed in this change.

### 14.4 The viewport

`cadview-web/src/view3d.rs`: a canvas with its own surface on the page's
shared `GpuContext` (never a second device), a render target in the
renderers' sRGB format with a plain view for the blit, and the frame loop.

- **Demand-driven.** A frame is drawn when the camera, the scene, the
  options or the canvas size change; none while idle. Input: drag to
  orbit, right-drag or shift-drag to pan, wheel to zoom; named views, fit,
  orthographic and perspective through `view3d_command`.
- **Working:** the raster backend, one frame per change.
- **Presentation:** the path tracer. While the camera moves the view draws
  the working render; once it has been still for 150 ms it adds samples in
  batches. A batch is submitted only when the GPU reported the previous one
  done (`on_submitted_work_done`), and sized from the measured GPU time per
  sample to about 24 ms, so the page never queues more than one batch.
  Accumulation restarts when the camera, scene or size change and stops at
  the target. Every image shown goes through the tracer's denoiser unless
  the options say `denoise: false`, as in `cad.render`; a camera the
  options place (`camera`) is used as given, a named view is fitted.
- **A slow GPU.** If the first sample at full resolution takes longer than
  40 ms, the view traces at half resolution (the blit scales up), and it
  lowers the sample target so the image converges in about 30 s: the
  target is 30 s divided by the measured time per sample, between 16 and
  256 (3D5a's browser aim). On the test browser's software GPU that gave
  half resolution and a target of 38 to 64 samples, reached in 18 to 28 s,
  across runs, for the bookshelf.
- **No WebGPU, no 3D.** The view and the embed viewer say so in words; 2D
  keeps its egui fallback.
- **The scene's source**, in order: the page's kernel, when loaded; else a
  bake the session holds; else, in the app with a server, the server's
  bake of the document, fetched again when the document's parts change.
  The server bake is an interim source until pages load the kernel by
  default; it keeps the app's 3D view working without the 1.5 MB download.

### 14.5 Equivalence, honestly

What the browser does now, from the same core code:

- show parts in both render modes, from a bake (no kernel), from the
  server's bake, or from its own kernel;
- with the kernel module loaded: every part call (`extrude` ... `fillet3d`,
  `move3d`, `copy3d`, `mirror3d`), `parts()`, `part()`, the set queries
  (`overlapping3d`, `pairsWithin3d`, `inside3d`, `distance3d`) and their
  refusals, with the server's numbers. On the 3D1a assembly all 120
  numbers (volume, box and centroid of twelve parts) are bit-identical to
  the server's, and so are the ids, `describe()` and the results of
  `overlapping3d`, `pairsWithin3d` and `distance3d`;
- `cad.project` through the page's kernel (topology, silhouettes and ray
  hits cross the boundary): the same entities as the server, number for
  number;
- with a bake only: `parts()` (bit for bit) and the views.

What only the server does, and why:

- `cad.render` and `cad.bake3d` write files, and scripts read libraries
  (`cad.use`) from the server's disk: the page has no file system. The
  page's own images are on screen; a PNG is a screenshot away.
- Running scripts in the sandbox with time and memory limits; in the page a
  script is the page's own JavaScript.
- Bit-identical numbers are not promised: the kernel's floating-point code
  is the same, but wasm's math library is not the platform's.
