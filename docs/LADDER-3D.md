# Ladder 3D: solids, renders and shop drawings

> **Model it in 3D, check it, render it, and build it from the 2D drawing it produces.** Parts are exact CAD solids (boards, turned legs, sweeps, lofts, joinery, roundovers) stored as a feature tree in the document and synced like everything else. The 3D view shows a part as it really is, from an interactive shaded view to a path-traced render with HDRI light and real materials. One call projects the parts back into exact 2D views, sections, hidden lines and a cut list, so the existing 2D tooling turns a model into a shop drawing and a DWG.
>
> Builds on the 2D core (document model, `cad_call`, Yrs sync, Vello renderer, DWG export, hatch and dimension helpers in `examples/lib/drafting.js`), the server-side sandbox (`/api/run`), and the example export pipeline. Design: [3d-design.md](3d-design.md). The smoke tests behind the decisions are in [research/3d-smoke/](research/3d-smoke/). The native desktop shell hosts the same viewport later ([LADDER-NATIVE.md](LADDER-NATIVE.md), N5).
>
> **Status:** design done (2026-10-13). Kernel and renderer evidence comes from smoke tests (3d-design.md §2), and an independent tester has rerun them:
> - **ogeom:** exact booleans, fillets and HLR; runs in wasm with no imports (3.2 MB). Dropped in 3D0a, after ogeom relicensed to `LGPL-2.1-only` from 0.12.0. **arris** now runs the corpus as a test-only reference backend, and which kernel ships is open (see the 3D0a note).
> - **Truck:** works as a second backend (949 KB wasm).
> - **vcad and csgrs:** rejected.
> - **Kiss3d 0.47:** shares wgpu 30 with Vello 0.11 (compile-proven). No GPU render has been run yet.
>
> The rungs are **3D0a–3D6b** and the vision is **3D🎯**. All are documented and none is started. 3D2a is the same work as LADDER-NATIVE N0a, done once.

## Why this is interesting here

- **The 2D drawing finally gets a cross-check.** DIY and building work goes wrong where a 2D plan says one thing and the real object needs another: a shelf 2 mm too long for its dado, a door that hits a radiator. Exact solids, `cad.interference()` and `cad.measure3d()` catch this before wood is cut. That is only worth anything if the 3D numbers are exact, hence the abstract-geometry rule in 3D.
- **The deliverable is still a 2D drawing.** Projection into the existing `Document` (blocks, layers, linetypes, `measure`, `addHatch`, title block, DWG) reuses everything the 2D work built. 3D is a source of exact 2D geometry, not a parallel product.
- **Agents can model.** The same `cad_call` ABI means an agent can write `cad.board(...)`, `cad.subtract(...)`, `cad.project(...)` and `cad.render(...)` in one script, run it on the server, and look at both the PNG and the DWG.
- **All Rust, both peers, one GPU device.** The kernel compiles to `wasm32-unknown-unknown`, and the renderer shares the wgpu device with Vello. Nothing here needs C++ or a native-only path.

## Key decisions

- **The feature tree is the document; B-rep and meshes are caches.** `PartDef` (ordered features over exact 2D profiles), `Part` (placement, material, layer), `Material`, `AssetRef` and `scene3d` are new Yrs maps of bincode blobs, following the 2D pattern. B-reps, meshes and BVHs are derived on each peer, keyed by content hash, and never synced (design §4).
- **Kernel: arris for now, behind `SolidKernel`.** ogeom was the first choice (the only candidate that passed every smoke test) until it relicensed to `LGPL-2.1-only`. arris (`MIT OR Apache-2.0`, pinned `=0.11.1`) runs the corpus as a dependency of the test-only `solid-harness` crate, so no shipped crate links a kernel yet. The test harness ran it against a second candidate, and the choice of arris is provisional (design §5). What arris gives:
  - exact volumes integrated on the faces (the prismatic corpus to about 1e-16 relative);
  - a 12-edge fillet followed by booleans that still give a valid B-rep at the closed-form volume;
  - exact planar sections, edge projection to exact 2D curves and exact ray hits, but **no built-in HLR**: the HLR adapter of design §7.3 is ours for every backend;
  - no threads and no process-wide state; core with arris checks for wasm32 (its wasm size and speed are measured in 3D0b).

  The harness's evidence on arris and cad-kernel 0.15.0, side by side, is in [research/kernel-comparison.md](research/kernel-comparison.md). Decided on 2026-11-18: the ladder continues on arris for now, with cad-kernel kept on the scoreboard as a second opinion (design §5).

  Truck runs the same corpus as a differential check. vcad degraded to triangle soup after a fillet. csgrs failed box-minus-cylinder and does not resolve from crates.io. arris is about two months old with one author. That risk is handled with an exact pin, the trait, the corpus, and the right to vendor it (MIT/Apache) (design §5).
- **Selections are semantic, never kernel ids.** Edges are named by `(feature, cap or lateral, profile segment)` or by nearest point, and resolved through the kernel's operation history. A selector that stops resolving is an error, never a silent re-pick.
- **Roundovers are exact by default.** If the kernel refuses (interfering chains), `fallback: "visual"` gives a render-only bevel plus a 2D note. Every fallback is listed in `cad.parts()[i].degraded`.
- **Tessellation only for rendering and as a transient oracle.** Render meshes adapt to zoom (≤ 0.5 px chord, power-of-two buckets between 0.005 and 2 mm) and are cached per (B-rep hash, bucket). Projection, cut lists, volumes and interference never take their *numbers or geometry* from a mesh. HLR uses a mesh only to decide visibility, and its output is exact source curves split at refined parameters (design §7.3, with a proposed AGENTS.md clarification).
- **Projection writes exact 2D entities.** HLR runs are re-fitted from their source edge into `Line`/`Arc`/`Circle`/`Ellipse`, and only general curves become splines (≤ 0.01 mm). Each view is one block plus one insert on `3D_*` layers with standard linetypes, regenerated on request.
- **Renderer: Kiss3d 0.47 behind `Renderer3d`**, with one wgpu device per page shared with Vello. Kiss3d supplies:
  - raster PBR;
  - HDRI IBL;
  - AgX/Neutral tone mapping;
  - a compute-BVH path tracer that works on WebGPU;
  - headless offscreen rendering.

  Renderling was dropped: it is alpha, on wgpu 26, and has no path tracer. The "dry render" worry becomes a measurable look gate (3D5b). If that gate fails, we fork Kiss3d's BSD-3 WGSL tracer instead of writing one from scratch.
- **One GPU stack bump first.** vello 0.9 → 0.11 and eframe/egui 0.34 → 0.36 move together (eframe 0.34 pins wgpu ^29). That gives one wgpu 30 in the web bundle before any 3D code lands (3D2a = NATIVE N0a).
- **Assets by reference.** A GLB, HDRI or texture is a document-store key plus a transform. Assets are served by `/api/files`, excluded from projection, cut lists and DWG, and sourced from a pinned Poly Haven (CC0) manifest fetched into a git-ignored `assets/`.
- **Core-first.** Feature tree, evaluation, tessellation, projection, sections, cut lists and interference live in `cadview-core`. GPU code lives in the shell-shared `cadview-render3d` (GPU-bound, no CAD logic, used by web and desktop). Server and web stay thin.

## Process

Each rung is built by a **builder/tester pair** and sized at **≤ about 2 weeks** of pair work. Every rung ends shippable on `main` behind no flag, or behind a documented feature flag.

- **The builder** (strongest model) owns the design, the core logic, the docs and the branch. At the start of the rung it spawns **one fast, cheap tester** (Haiku class) and **keeps it alive for the whole rung**, continuing the same tester rather than respawning it, so it keeps its warm build directories, the commands that work and what it has ruled out.
- **The tester** does the build-and-run work: cargo builds (native and wasm32), corpus runs, perf harnesses, headless renders, ꟻLIP comparisons, screenshot diffs, bisecting. It edits only test inputs, scratch files and the small changes the builder names. It decides nothing about the design.
- **The loop.**
  1. The tester first reproduces the rung's baseline (the previous rung's Done-when). If the baseline fails, that is the first report, and nothing else proceeds.
  2. The builder sends a brief: the exact commands, the pass criteria and a time budget.
  3. The tester reports in a fixed shape: the command run, pass/fail per criterion with numbers, the failing output verbatim, and observations kept separate from inferences.
  4. The builder changes code and repeats.
- **Done** means every Done-when item below has been confirmed by the tester's report. The builder's final report lists what changed, how it was verified (commands and numbers) and what is open.
- **Hygiene.** One branch per pair. Every pair uses a separate `CARGO_TARGET_DIR` so parallel pairs never lock each other's builds. Big outputs (renders, corpora, target dirs) stay out of git, except the small committed goldens a rung names.

**Reference hardware** for performance gates: **GPU-A** is a desktop RTX 3060-class discrete GPU (about 13 TFLOPS FP32). **GPU-B** is an Intel Iris Xe-class integrated GPU. Browser gates run in current stable Chrome with WebGPU. The harness records the actual adapter name, and a run on other hardware reports numbers without claiming the gate.

## Rungs

| Rung | Content | Done when |
|---|---|---|
| **3D0a** kernel boundary and prismatic corpus | `cadview-core/src/solid/`: the `PartDef`, `Feature`, `Profile2d` and `EdgeSel` types (no sync yet), the `SolidKernel` trait and `ArrisKernel` pinned to an exact version (`=0.11.1`). A **prismatic corpus** with closed-form volume and bbox per entry:<br>• board<br>• board with through hole<br>• dado<br>• rabbet<br>• mortise and tenon pair<br>• half-lap<br>• counterbored hole<br>• chamfered slab | `cargo test -p cadview-core solid::corpus::prismatic` passes. Every entry is a valid B-rep, and its volume matches the closed form to ≤ 1e-9 relative. The bbox matches to 1e-9 mm. Per-entry native timings are recorded in the test output. |
| **3D0b** curved corpus, Truck and wasm | Curved corpus entries:<br>• fully rounded board<br>• tapered leg (loft)<br>• turned leg (revolve with beads and coves)<br>• swept handrail<br><br>`TruckKernel` behind the `kernel-truck` feature (no fillets). A wasm runner `node scripts/solid-corpus.mjs`, built the same way as `research/3d-smoke/`. | Curved entries match the closed form to ≤ 1e-6 relative. Truck agrees with arris to ≤ 1e-6 relative on every non-filleted entry (`cargo test -p cadview-core --features kernel-truck solid::corpus`). The wasm runner passes the full corpus and prints per-entry timings. A table of native and wasm timings and the `cadview-web` wasm size delta is committed in `docs/research/3d-corpus-timings.md`. Any entry arris refuses is listed there, with an upstream issue link. |
| **3D1a** parts in the document | Yrs maps `partdefs`, `parts`, `materials`, `assets` and `scene3d`. A content-hashed evaluation cache. Undo/redo covers parts. JSON save/load includes parts, and DWG export ignores them. Flat-part API in `cad_call` and `cad-api.ts`: `board`, `subtract`, `hole`, `pocket`, `move3d`, `copy3d`, `material`, `parts`. | A core test runs two Yrs peers. One applies a script that builds the bookshelf parts, and the other converges to byte-identical `partdefs`/`parts` maps. `cad.parts()` returns identical bboxes and volumes on both. `saveDwg` output for a document with parts is byte-identical to the same document without them. `undo()` after a `subtract` restores the previous volume exactly. |
| **3D1b** general features and checks | `extrude` (with taper), `revolve`, `union`, `intersect`, `rotate3d`, `measure3d`, `interference`. `describe()` reports parts. `POST /api/run` runs 3D scripts in the sandbox. | Via `curl /api/run`, the bookshelf script with one shelf 2 mm too long gets back exactly one interference record (the right pair, volume equal to the closed form to 1e-6 relative). With the correct shelf it gets none. `measure3d` between two parallel boards returns the gap to 1e-9 mm. A turned leg built by `revolve` matches its 3D0b corpus volume. |
| **3D2a** one GPU stack (= NATIVE N0a) | Bump `vello 0.9 → 0.11` and `eframe/egui 0.34 → 0.36` together, so the web bundle has one wgpu, 30. Add `GpuContext`: one `wgpu::Device` per page, shared by every canvas (today each Vello canvas creates its own). No 3D code yet. | `cargo tree -p cadview-web --target wasm32-unknown-unknown -d` lists no duplicate `wgpu`. Every example DWG renders in both the Vello and the egui renderer with **≤ 0.1 % of pixels differing by more than 8/255 in any channel** from the committed PNGs (`examples/lib/render.mjs` plus a diff script). Opening three tabs creates exactly one device (debug counter). |
| **3D2b** 3D viewport, shaded | New crate `cadview-render3d` with the `Renderer3d` trait and the Kiss3d backend initialised on the `GpuContext` device. Scene sync from `Document`: part meshes from core's tessellation cache, materials, layer visibility. Orbit, pan and zoom; ortho/perspective; named views. Shaded mode only. React: a 2D/3D toggle per tab. Wasm export `cadview_frame_stats()` returns frame times. | Perf harness `web/tests/perf3d.mjs` (Chrome DevTools Protocol) loads the bookshelf scene (12 parts, no assets) at 1920x1080 and orbits it for 10 s. **p95 frame time ≤ 16.7 ms on GPU-A and ≤ 33 ms on GPU-B.** After the orbit stops, the frame counter stays flat for 5 s (zero idle frames). The 2D/3D toggle keeps the 2D camera unchanged. |
| **3D2c** lines mode, adaptive meshes, multi-tab | Lines mode (exact part edges tessellated to 0.5 px, hidden edges dashed). Zoom-adaptive re-tessellation with a per-frame time budget, where the coarse mesh stays visible until the fine one is ready. Two 3D tabs plus one 2D tab on one device. Part list in the layer panel (select, hide, show `degraded`). | Unit test: at three zoom levels, the chosen bucket's maximum chord deviation is ≤ 0.5 px (measured against the exact edges). Under the perf harness with the camera zooming 1x to 50x on the rounded tabletop, **no frame exceeds 50 ms on GPU-A** (re-tessellation is time-sliced). Two 3D tabs and one 2D tab render at once, with one device (debug counter). |
| **3D3a** views and hidden lines | `cad.project` with front/back/left/right/top/bottom/iso views and first- or third-angle layout. HLR as in design §7.3: the mesh is only the visibility oracle, silhouettes are closed-form, and split parameters are refined to 1e-4 mm. Analytic re-fit to `Line`/`Arc`/`Circle`/`Ellipse` (splines only for general curves). View blocks plus inserts on `3D_VISIBLE`, `3D_HIDDEN` and `3D_CENTER`. The same oracle algorithm in our adapter for `TruckKernel`. | Golden test against hand-computed coordinates committed as `crates/cadview-core/tests/golden/board_views.json`: the front, left and top views of the corpus "board with hole and dado" match to 1e-9 mm, with the hole's hidden lines on `3D_HIDDEN`. An iso view of a Ø10 hole yields one `ELLIPSE` with semi-axes 5 and 5·cos(35.264°) to 1e-9. The bookshelf views saved to DWG and reloaded contain **only LINE, ARC, CIRCLE and ELLIPSE** inside the view blocks. The visibility oracle agrees with exact ray hits against the faces (arris `query::ray_fire`) on every corpus entry. |
| **3D3b** dimensions and cut lists | `toSheet` and `anchor` for dimensions on views. `cad.cutList` (grouped by label, material and L×W×T, with notes) and `cad.cutListTable`. Stock rows for non-flat parts. | The bookshelf cut list equals the hand table in 3d-design.md §10.1, cell for cell (test). Dimensions placed through `toSheet` on the front view read 800 and 1800 in the saved DWG's TEXT entities. Changing `W` to 900 and re-running changes exactly the expected cut-list cells (test). |
| **3D4a** sections and hatching | Section views (`sections: [{plane, in, name}]`): planar regions per part through the kernel, exact outlines, `addHatch` from each material's `section_hatch` with alternating angles for adjacent parts, and section line plus label on the parent view. | **Golden:** `crates/cadview-core/tests/golden/bookshelf_section_aa.json` (exact entities: type, endpoints, centres, radii, hatch boundary and angle per part). It was derived by hand from the §10.1 parameters, the derivation is committed as `bookshelf_section_aa.md`, and the builder and tester check it independently. The test matches it to 1e-9 mm. A DXF of the same golden is committed for visual review. Adjacent parts' hatch angles differ by 90°. |
| **3D4b** roundovers, sweep and loft | `cad.roundover` and `chamfer3d`, exact, with `fallback: "visual"` (render bevel plus a `3D_NOTES` note). `cad.sweep` and `cad.loft` in the API. Example `examples/08-turned-stool.js`: revolved legs, tapered stretchers, rounded seat. | The rounded tabletop's volume equals the closed form to 1e-6 relative. A forced interfering roundover chain fails with `fallback: "error"`. With `"visual"` it writes the note and lists `degraded` in `cad.parts()`. `08-turned-stool.js` runs via `/api/run` and exports a DWG whose leg section matches the golden `turned_leg_section.json` (hand-derived like 3D4a). |
| **3D5a** path-traced rendering | Render mode: Kiss3d `RayTracer` with progressive accumulation up to a target spp, the à-trous denoiser, HDRI IBL from the store, AgX/Neutral tone mapping and exposure from `scene3d`. Material presets with Poly Haven PBR texture sets: oak, walnut, birch ply, plaster, concrete, brushed steel, glass, white paint. `cad.view3d`. | In Chrome on GPU-A, the bookshelf with the studio HDRI reaches **256 spp at 960x640 in ≤ 30 s** (≤ 4 min on GPU-B), measured by the perf harness. The frame counter is flat for 5 s after the target spp. A camera move resets accumulation (the spp counter returns to 0). Every material preset renders without validation errors in Chrome and natively. |
| **3D5b** look gate | A reference scene (bookshelf, `studio_small_09` HDRI, oak and plaster, a fixed camera and exposure) rendered here and in Blender Cycles with the same assets. A Blender script under `docs/research/look-gate/` makes the reference reproducible. | **Objective gate:** mean ꟻLIP ≤ 0.10 (NVIDIA ꟻLIP, `pip install flip-evaluator`) between our 1024-spp render and the Cycles 4096-spp reference at 960x640, run by the tester. **Second gate:** the maintainer signs off on the side-by-side. If the objective gate fails, the tracer kernel is forked into `cadview-render3d/tracer/` and fixed until it passes, and the outcome is recorded in 3d-design.md §9.5. |
| **3D6a** assets | `cad.placeAsset`, `AssetRef` sync, `GET /api/files/<key>` (auth, ETag), glTF loading in render3d, plan symbols (`footprint` or a block) in the 2D plan and top views. `just fetch-assets` reads a pinned Poly Haven manifest (slug, resolution, sha256) into git-ignored `assets/`. | The bookshelf script places a plant and a chair. Both show in the 3D view, the top view has their footprints, and neither appears in the cut list, the section or the DWG (tests). A fresh clone runs `just fetch-assets`, all manifest hashes verify, and a corrupted file fails the fetch. `/api/files` returns 401 without the token and 304 on a matching ETag. |
| **3D6b** headless renders | Server `render` RPC and sandbox `cad.render({out, size, spp})` through `cadview-render3d::headless` (native wgpu; lavapipe on GPU-less hosts). The example export pipeline gains 3D PNGs. | `curl /api/run` with `cad.render` writes a 960x640 PNG **on the GPU-less CI runner in ≤ 10 min** at the configured spp. On a GPU-A host the same scene renders in ≤ 30 s. The two PNGs differ by mean ꟻLIP ≤ 0.05. If lavapipe misses the time budget, the fallback named in design §9.4 is adopted, and its gate replaces this one. |
| **3D🎯** vision | `examples/07-bookshelf.js` runs the full design §10.1 script: model, interference check, plant and chair, path-traced render, an A3 shop drawing at 1:10 with three views, section A-A, dimensions, a cut-list table and a title block. It runs from the browser console and from `/api/run`. A room variant drops the bookshelf into a floor-plan DWG and renders it in place. | One `just export 07-bookshelf` produces:<br>• `07-bookshelf.dwg`, which opens in a third-party DWG viewer with views, hatch, dimensions and table<br>• `07-bookshelf.png` (the drawing)<br>• `07-bookshelf-render.png` (path-traced)<br><br>The cut list printed by the script matches the drawing's table. Changing `W = 900` and re-running updates the parts, views, dimensions, cut list and render with no manual fix-ups. All goldens still pass. |

### 3D0a: what landed (2026-11-17)

- **Reference kernel: arris `=0.11.1`** (`MIT OR Apache-2.0`), test-only:
  since the first harness slice a dependency of the test-only
  `solid-harness` crate (before that a dev-dependency of cadview-core), so the
  server and the wasm bundle link no kernel yet. The choice of the kernel
  that ships is open. 3D0a was
  first built on ogeom `=0.18.1` (it passed the same corpus), but ogeom
  is `MIT OR Apache-2.0` only up to 0.11.5: from 0.12.0 every ogeom crate
  is `LGPL-2.1-only`, which does not fit an MIT project that links the
  kernel statically into the server binary and the wasm bundle. Nothing
  with ogeom landed. Design §5 has the decision update.
- **Code.** `crates/cadview-core/src/solid/`: `types.rs` (the 4.1 feature
  tree; profiles are exact `Line`/`Arc`/`Circle`/B-spline segments, and
  `LwPolyline` bulges convert to exact arcs), `kernel.rs` (`SolidKernel`
  with `evaluate`, `mass`, `bbox`, `check`; the rest of the designed trait
  comes with later rungs), `select.rs` (edge selectors resolved to exact
  3D target curves from our own feature tree, kernel-neutral),
  `arris_kernel.rs` (the only file that names arris; now
  `crates/solid-harness/src/kernels/arris.rs`) and
  `corpus/prismatic.rs`. Implemented: extrude without taper, hole with an
  optional counterbore, boolean with an inline tool definition, symmetric
  chamfer, and ops add/subtract/intersect. Elliptic profile segments and
  loops also extrude (arris has them). Every other feature returns
  `EvalError::Unsupported`. No change to the trait or the types was needed.
- **Mapping.** A profile becomes an arris `Profile` (`LineTo`, `ArcTo`
  through the arc's midpoint, `EllipseTo`, or a whole `Circle`/`Ellipse`
  loop) and `ops::extrude` sweeps it; a hole is `ops::primitive_cylinder`
  plus `ops::cut`; a tool definition is evaluated into the same model and
  moved by `ops::transform`; ops are `fuse`/`cut`/`common`; a chamfer is
  `ops::chamfer` on the edges `select.rs` picks. Volume is
  `ops::measure::mass_properties` (a flux integral over the exact faces,
  no mesh). Validity is `check::check` at `Level::Full` plus
  `check::lumps` for the solid count. arris has no body bounding box, so
  `bbox` is computed from the exact geometry: every vertex, the exact box
  of every edge curve, and the interior extremes of sphere and torus faces
  kept when they lie on the face (plane, cylinder and cone faces take their
  extremes on their edges). NURBS edges and faces are refused there.
- **Gate.** `cargo test -p cadview-core solid::corpus::prismatic` passed
  (since the first harness slice: `cargo test -p solid-harness --test
  corpus`).
  All 9 parts (8 entries, the mortise and tenon entry has two) are valid
  single solids with no undecided checker rows. The worst relative volume
  error is 4.3e-16 (rail with tenon) and every bounding box matches
  exactly (error 0).
- **Timings** (one machine, `--test-threads=1`, evaluation only, debug /
  release in ms): board 7.7 / 0.10, through hole 343 / 12, dado 36 / 0.60,
  rabbet 28 / 0.46, mortised stile 26 / 0.44, tenoned rail 25 / 0.37,
  half-lap 34 / 0.52, counterbored hole 919 / 33, chamfered slab 22 / 0.32.
  The whole corpus, including the full checks, takes 3.3 s in debug and
  0.09 s in release. Debug builds of arris also run its checker on every
  operation's input and output, which is where most of the debug time
  goes; the cylinder cuts are the slow ones. If debug test time becomes a
  problem, a `[profile.dev.package]` opt-level override for the arris
  crates is the fix.
- **Cost.** Against the last commit, arris adds 17 entries to
  `Cargo.lock` (788 to 805): the 8 arris crates, nalgebra 0.35, simba,
  wide, safe_arch, robust, and four glam versions that are locked but not
  built. Every one is MIT, Apache-2.0, BSD or Zlib licensed (nalgebra 0.35
  and simba are Apache-2.0). Default features are off (no serde, no
  rayon). The first `cargo check` of cadview-core with arris took 25 s and
  the first debug test build 46 s. Both
  `cargo check -p cadview-web --target wasm32-unknown-unknown` and the
  native workspace check pass. The wasm size delta is measured in 3D0b.
- **Notes for 3D0b and later.** A chamfer `Cap` selector picks every edge
  lying on the cap's profile curves, so edges split by later features are
  all picked; an arc segment's target is its whole circle. `d1 != d2` has
  no defined face convention in 4.1 and is unsupported. arris starts no
  threads with default features off and keeps no process-wide state, so
  core stays free of global state. Every arris operation returns a
  provenance record that names extrude edges by profile segment
  (`StartEdge`, `EndEdge`, `Rise`) and follows them through later
  operations; on a dado cut across a cap it picks exactly the edges
  `select.rs` matches geometrically, so selectors could move to it.
  arris panics in debug builds when an operation's own output fails its
  checker (a kernel bug by its contract); release builds skip that check.
  The forward-look tests in the arris adapter record the smoke part
  (fully rounded, valid, 33 faces, volume 3 188 376.635 in 22 ms release),
  tessellation, an exact section and exact ray hits.

### 3D2a: what landed (2026-11-17)

Same work as LADDER-NATIVE N0a.

- **Versions.** vello 0.12.0, wgpu 30.0.2 and eframe/egui 0.37.0, not
  the 0.11/0.36 pair planned above: vello 0.11 and 0.12 and eframe 0.36
  and 0.37 all need wgpu ^30, so the newest pair was taken. kurbo 0.13,
  peniko 0.6.1 and vello_cpu 0.4.0 are unchanged.
  `cargo tree -p cadview-web --target wasm32-unknown-unknown -d` lists
  no duplicate wgpu: one wgpu 30.0.2 serves vello, eframe/egui-wgpu,
  `cadview-render` and `cadview-web`. wgpu 30 needs web-sys 0.3.104 or
  newer, so wasm-bindgen moved from 0.2.121 to 0.2.130, and the
  wasm-bindgen CLI pin in the workflows with it.
- **`GpuContext`** (`cadview-render`, feature `gpu`, wasm32 + native)
  owns the instance, adapter, device, queue and `vello::Renderer`, built
  by one constructor that asks for Msaa16 only (the browser used to
  compile all three AA pipelines). Headless render keeps one per process.
  The browser takes it from `shared_local()`: one device per page, the
  first canvas creates it, canvases that start while it is pending wait
  for the same result, and each canvas keeps only its surface, surface
  config, intermediate texture and blitter. Every device creation logs
  `[CodeCAD] GPU device created (#N)`.
- **Pixels.** Native GPU and CPU renders of all 12 examples are identical
  to the pre-bump renders (0 differing pixels). GPU against the committed
  PNGs: at most 0.046 % of pixels differ by more than 8/255, as before.
- **Browser.** In headless Chromium the Vello viewer reproduces all 12
  committed PNGs (0 pixels over the threshold, through `render.mjs`), and
  app screenshots are identical before and after the bump. Four canvases
  started on one page log exactly one device creation. With WebGPU
  disabled the app still falls back to egui and draws.
- **The egui half of the gate was not measurable as written.** egui
  anti-aliases differently from Vello, so it differs from the committed
  (Vello) PNGs on 3 to 5 % of pixels at any version. Measured instead:
  egui before against after the bump, at most 0.10 % of pixels.

**Notes (2026-11-17).**

- **3D6b builds on the 2D headless path.** `cad.render` already exists for
  2D (LADDER-NATIVE N0b): the server RPC, the sandbox method, the
  `--render` CLI flag, `RenderOptions` parsing (it already accepts and
  ignores `spp`), a process-wide wgpu device with Vello, readback to
  RGBA8, PNG encoding, and `Auto | Gpu | Cpu` selection with a CPU
  fallback. 3D6b adds a 3D branch behind the same handler
  (`crates/cadview-server/src/render.rs`) and should share the device
  that `cadview-render`'s GPU backend creates rather than open a second
  one.
- **Re-check 3D2a's version claim.** crates.io now has vello 0.12.0,
  wgpu 30.0.2 and eframe 0.37.0. The "one wgpu, 30" result above was
  measured against vello 0.11 and eframe 0.36; before 3D2a, re-run the
  smoke crates and decide which vello/eframe pair actually shares one
  wgpu. Done in 3D2a: vello 0.12 and eframe 0.37 share wgpu 30.0.2 (see
  above). Kiss3d still has to be checked against that wgpu in 3D2b.

## Risks

- **arris is about two months old with one maintainer, and breaks its API every minor release (pre-1.0).** ogeom, the first choice, changed its licence within weeks. The mitigations are an exact pin on a permissive release, the Truck differential corpus from 3D0b onward, the right to vendor, and documents that store no kernel data. If arris stalls before 3D3a, `TruckKernel` plus our own HLR adapter and closed-form silhouettes carry 3D0–3D3 (the HLR adapter is ours in any case, since arris has no HLR). Roundovers then drop to `fallback: "visual"` until a kernel can do them.
- **Evaluation latency in wasm.** The smoke test took 3.3 s for a fully rounded board. Content-hash caching, time-slicing and showing the previous B-rep keep the UI responsive. A Web Worker evaluator is the escape hatch (design §13), and the 3D0b timing table decides whether 3D2b needs it.
- **Kiss3d as an embedded backend.** It has thread-local managers, an engine-shaped API and the look question. The `Renderer3d` trait, the 3D5b gate and the fork fallback bound the risk. Replacing Shaded mode with a thin raster pass of our own is a contained rewrite behind the same trait.
- **Three crates must agree on wgpu** (Vello, Kiss3d, egui). They agree on wgpu 30 today. Pin and bump them together. `GpuContext` turns a mismatch into a compile error, and 3D2a's `cargo tree -d` check keeps it that way.
- **Headless rendering without a GPU** (lavapipe) is unverified. If it is too slow: lower spp and denoise, run the 3D example export on a GPU host, or fall back to the headless-Chromium path.
- **Projection quality edge cases**: tangent-edge clutter on rounded parts, near-degenerate ellipses, torus silhouettes on turned parts, and the visibility oracle near grazing faces. Goldens grow with every bug found, `tangent_edges: false` is the default, and exact ray hits against the faces cross-check the oracle.
- **Poly Haven availability and licence drift.** Assets are CC0 today. The manifest pins hashes, so a changed or removed upstream asset fails loudly at fetch, and nothing is vendored into git.
