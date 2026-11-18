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
| **3D0b** curved corpus, Truck and wasm | Curved corpus entries:<br>• fully rounded board<br>• tapered leg (loft)<br>• turned leg (revolve with beads and coves)<br>• swept handrail<br><br>`TruckKernel` behind the `kernel-truck` feature (no fillets). A wasm runner `node scripts/solid-corpus.mjs`, built the same way as `research/3d-smoke/`.<br><br>**Changed in the build (see "3D0b: what landed"):** the Truck cross-check is dropped, because cad-kernel 0.15.0 in the test harness fills the second-kernel role (with fillets); the wasm runner is a wasmtime host in the harness, since there is no Node on the build machine; corpus names are general shapes (rounded slab, tapered square bar, revolved profile, swept round bar). | Curved entries match the closed form to ≤ 1e-6 relative. Truck agrees with arris to ≤ 1e-6 relative on every non-filleted entry (`cargo test -p cadview-core --features kernel-truck solid::corpus`). The wasm runner passes the full corpus and prints per-entry timings. A table of native and wasm timings and the `cadview-web` wasm size delta is committed in `docs/research/3d-corpus-timings.md`. Any entry arris refuses is listed there, with an upstream issue link. |
| **3D1a** parts in the document | Yrs maps `partdefs`, `parts`, `materials`, `assets` and `scene3d`. A content-hashed evaluation cache. Undo/redo covers parts. JSON save/load includes parts, and DWG export ignores them. Flat-part API in `cad_call` and `cad-api.ts`: `board`, `subtract`, `hole`, `pocket`, `move3d`, `copy3d`, `material`, `parts`.<br><br>**Changed in the build (see "3D1a: what landed"):** a general shape API instead of the flat-part one (`board`, `pocket` and `material` are library code or later rungs, design 4.0); two maps (`partdefs`, `parts`), not five; the DWG check compares 2D content, not bytes. | A core test runs two Yrs peers. One applies a script that builds the bookshelf parts, and the other converges to byte-identical `partdefs`/`parts` maps. `cad.parts()` returns identical bboxes and volumes on both. `saveDwg` output for a document with parts is byte-identical to the same document without them. `undo()` after a `subtract` restores the previous volume exactly. |
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

### 3D0b: what landed (2026-11-18)

- **Curved corpus** (`cadview-core/src/solid/corpus/curved.rs`, kernel
  neutral, named in general shape language): a 400 x 200 x 20 slab with
  all 12 edges rounded r = 4, a tapered square bar (ruled loft from 40 x
  40 to 24 x 24 over 300), a revolved profile with a bead and a cove (one
  revolve of lines and arcs), and a round bar (r = 6) swept along a line,
  a quarter arc and a line. Each closed form is derived in its doc
  comment (Steiner, prismatoid, discs, Pappus) and checked numerically in
  core's tests.
- **Gate:** `cargo test -p solid-harness --test corpus` passes on arris:
  every curved part a valid single solid with a closed mesh, volume
  errors 2.9e-16, 7.4e-16, 2.4e-16 and 4.9e-16 relative (gate 1e-6),
  box errors 0, 0, 0 and 2.8e-14 mm. cad-kernel builds the same four
  (printed, not gated): errors 1.5e-15, 7.4e-16, 1.7e-15, 6.1e-16, boxes
  exact.
- **Adapters.** Revolve, loft, sweep and fillet in both (design 4.1
  semantics, now written into `types.rs`); `Feature::Sweep` gained the
  profile's `plane`, and `Path3::check` states what a sweep path must be.
  Tapered extrudes stay unsupported. No corpus entry is refused by either
  kernel.
- **Harness:** reference meanings for the curved features, a positional
  band for the volume enclosure, curved catalogue cases, three curved
  mutants (all caught), scoreboards regenerated; see
  [3d-test-harness.md](3d-test-harness.md), "Slice 5". arris has no
  recorded wrong case left; cad-kernel has 20, 10 of them from its mesher
  on torus faces.
- **Truck is dropped from this rung.** The planned Truck cross-check was
  a second, unrelated kernel to compare against; cad-kernel fills that
  role in the harness, with fillets, which Truck lacks.
- **Wasm:** see [research/3d-corpus-timings.md](research/3d-corpus-timings.md)
  for the size arris adds and native against wasm timings, measured with
  a wasmtime host in the harness instead of the planned Node runner (the
  build machine has no Node).
- **The adapter ships (after 3D0b).** `ArrisKernel` moved, unchanged,
  from the harness to the new crate `crates/cadview-solid`, the only
  crate that names arris; it compiles for wasm32. When it moved, nothing
  linked it yet; cadview-core still links no kernel. The harness tests it
  from there, its arris scoreboard came out byte-identical, and a test
  fails if that scoreboard records a wrong case. The harness gates got
  fast again: scoreboard 262 s to 50 s, mutants 54 to 4 minutes, in
  release; see [3d-test-harness.md](3d-test-harness.md), "After slice 5".

### 3D1a: what landed (2026-11-18)

Parts in the document and the first 3D script API. Reference:
`docs/api-surface.md`, "Solid parts".

- **Part model.** A definition (`PartDef`: feature tree, shape only) placed
  by a part (`Part`: definition, placement, attributes). Ids from the one
  document counter, printed `p_<n>` (parts) and `d_<n>` (definitions).
  `copy3d` shares the definition; a modifying call changes the definition
  and so every part that shares it; `unique: true` gives a copy its own.
  Positions in modifying calls are world coordinates, moved into the
  part's frame (`frame: "local"` skips that). A part used as a tool is
  stored as its definition and its placement relative to the target at the
  time of the call. The feature types got serde derives and nothing else.
- **Calls** (`cadview-core/src/dispatch_3d.rs`): `extrude`, `revolve`,
  `sweep`, `loft`; `cut`, `union`, `intersect` with an inline body or
  another part; `hole` (blind, through, counterbore); `fillet3d`,
  `chamfer3d` on edge selectors; `move3d`, `rotate3d`, `copy3d` (with
  `count` for a row), `remove3d` (unused definitions go too);
  `setPartAttrs`, `removePartAttrs`; `parts`, `part`, `partDef`;
  `describe()` counts parts, part attributes and names the kernel. Parts
  are selected with the entity selector's shape (`ids`, `attrs`, `has`,
  `bounds`, plus `def`). Typed in `web/src/cad-api.ts`.
- **Evaluation without a kernel in core.** `solid::eval`: an object-safe
  `Evaluator` (any `SolidKernel` through `KernelEvaluator`) that a shell
  puts on the document as a `SolidEnv` (never synced, serialized or
  undone). With one, every new or changed definition, and every definition
  that uses it as a tool, is evaluated before it is stored; a refusal fails
  the call naming the call, the part, the feature and the reason, and the
  document stays as it was. Without one, input is checked and stored, and
  geometry queries say "this build has no solid kernel". The cache lives in
  the `SolidEnv`, keyed by the bytes of the features and of every tool
  definition reached; solids, mass properties and render meshes (per chord)
  are cached; least recently used entries go past 4096. Core links no
  kernel (`cargo tree -p cadview-core -e normal` shows none).
- **Document.** Definitions and parts on `Document`, in undo snapshots,
  in `clear`, in `saveJson`/`loadJson` (files from before load with none),
  and in two Yrs maps, `partdefs` and `parts`, diffed by content hash.
  `saveDwg` writes the 2D content only and reports the parts it dropped.
- **Server.** `cadview-server` depends on `cadview-solid` (`ArrisKernel`)
  and gives every document one process-wide `SolidEnv` (`src/solids.rs`),
  so documents share the cache. Loads and sync rebuilds keep it
  (`Document::replace_keeping_solids`). `cad.render` draws the parts when
  it gets a `view`: core builds the `Scene3d` (`solid::scene`: selection,
  colours by part, definition or attribute, chord a quarter pixel rounded
  down to a power of two, named views, projection, edges, occlusion,
  ground, up to four clip planes, explode), `cadview-render3d` draws it on
  the shared GPU device. The browser stores and syncs parts but has no
  kernel and no 3D view yet.
- **Evidence.** Core (`solid/tests.rs`, on a closed-form fake kernel): every
  call, 32 degenerate inputs that each fail and change nothing, a refused
  feature with its exact message, undo after a cut restoring the definition
  and the volume from the cache, JSON round trip (bit for bit) and old
  files, DWG 2D content equal with and without parts, two Yrs peers that
  build a seven-call assembly in a batch and end with byte-identical part
  maps and equal `parts()` (then the other way round with a move and a
  removal), and the render scene. Server (through the script entry, on
  arris): a twelve-part assembly (a base plate with four through holes and
  a slot with exact arcs, two uprights sharing one definition with a groove
  and a blind hole, a shelf with two holes and a lip added by `union`, four
  pins and four feet as copies) matches hand-computed volumes to 1e-9
  relative, boxes to 1e-9 mm and two centroids, and survives `saveJson`,
  `clear`, `loadJson` unchanged; every call once more against its closed
  form (bulge profile, clockwise profile, revolved tube, swept bar along a
  line and a half turn, ruled loft, a profile from a 2D entity, intersect
  with a part, union, counterbore, one-edge fillet by cap and by point,
  chamfer, a local-frame cut); a tapered extrude, a round larger than the
  plate and a revolve across its own axis are refused and the document is
  unchanged; 3D renders of the assembly (plain with a ground, a cutaway, an
  exploded view coloured by an attribute) come back at the requested size
  with 23 to 36 % of the pixels drawn.
- **Times** (debug server, one GPU machine, 960x640): the first 3D render
  of the assembly 1.0 s (device, pipelines and tessellation at chord 0.25
  mm, 680 triangles), the next four 90 to 380 ms each, with meshes from
  the cache, including PNG encoding.
- **2D unchanged.** The twelve examples rendered with `--exec ...
  --render` differ from the committed PNGs on at most 0.048 % of pixels
  (the known anti-aliasing difference, as before).
- **Typed API.** The calls are typed in `web/src/cad-api.ts` and in the
  rebuilt sandbox prelude (`cad-client/cad-api-setup.js`), so scripts call
  `cad.extrude`, `cad.cut`, `cad.parts` and the rest directly; the server
  tests above use the typed calls end to end.
- **Open.** No kernel in the browser (geometry
  queries say so), no 3D viewport, no interference or measuring, no
  projection, no materials, assets or `scene3d` map, no live tool links or
  clearances, no CPU fallback for 3D renders. `fillet3d`'s `fallback:
  "visual"` is stored but a kernel refusal still fails the call.

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
  Done in the first 3D2b slice (below).

### 3D2b, first slice: what landed (2026-11-18)

A headless, shaded 3D render to PNG. The viewport (browser canvas,
orbit input, React toggle, perf harness) is still open, and so are scene
sync from `Document`, the server `render` RPC and `cad.render` for 3D
(3D6b).

- **Core.** `cadview-core/src/scene3d.rs` decides everything once:
  `RenderMesh` derives normals from a bare `TriMesh` (angle-weighted,
  split at a 40° crease angle) and classifies its edges with the same
  angle; `build_frame` adds the smooth edges that are silhouettes from
  the current eye; `Camera3d` is an orbit camera (Z up, right-handed,
  WebGPU depth 0..1) with named views, exact fit for both projections and
  a computed clip range; `shade` and `tone_map` (Khronos PBR Neutral) are
  the reference every backend reproduces.
- **Backend.** `cadview-render3d` has the `Renderer3d` trait (a frame may
  take several calls, so a progressive path tracer fits) and one backend
  of our own on wgpu: 4x MSAA, Depth32Float, per-pixel shading, and sharp
  edges and silhouettes as 1.5 px screen-space lines. It renders on
  `cadview-render`'s process-wide device; no second device is opened.
  `Quality::Presentation` returns `NotAvailable`.
- **Kiss3d was tried and not adopted for this path.** It resolves to the
  single wgpu 30.0.2 and rendered a correct, byte-reproducible box on the
  shared device, but its context and resource managers are thread-local.
  Used from a second thread it panicked in its texture manager and then
  aborted the whole process during thread-local teardown, and dropping
  its last surface calls `destroy()` on the device it was given (the
  shared one). A server renders from any worker thread, so that does not
  fit headless rendering. It would also add 98 crates to `Cargo.lock`
  (an AV1 encoder and a parser generator among them). It stays the
  candidate for the presentation render (its path tracer), on a thread of
  its own, or as the tracer kernel to fork (design §9.5).
- **Evidence** (`cargo test -p cadview-render3d`, one GPU machine).
  Silhouette area of a unit box (front, top, right, isometric) and a
  sphere (front, top, isometric) against the closed form: within 0.06 %
  (tolerance a quarter pixel along the outline). Face pixels equal the
  core's reference shading exactly (tolerance 2 levels). A near box hides
  the far one at the sampled points, in both projections. Smooth shading:
  largest second difference across a 48-facet cylinder 2 levels, 13 with
  flat facets. Edges: 1.50 px of ink on a box edge, none across a
  cylinder's side, ink at both outlines. Two renders are byte-identical.
  1x and 2x pixel ratio agree to 0.42 levels on average.
- **Times** (release, 960x640): first render 974 ms (device and pipeline
  creation), warm 1.6 to 2.8 ms including PNG encoding. wgpu's software
  adapter on Windows (DX12 WARP) renders the same frame in about 19 ms,
  within 1 level of the GPU image on every pixel. lavapipe (Linux) is
  not measured yet.
- **Cost.** One new crate, no new third-party crates.

### 3D2b, second slice: what landed (2026-11-18)

The working render against the list in `docs/3d-examples.md`, on
procedural meshes (the example recipes come later). Every parameter a
second backend would have to agree on is in `scene3d.rs`; the raster
backend only carries it out.

- **Outlines on any background.** 3D renders default to a light neutral
  background (`LIGHT_BACKGROUND`); the 2D viewer's dark one is
  `Background::VIEWER`. `EdgeStyle::outline` draws the background side of
  every silhouette 1 px wider, in the line colour when that contrasts at
  least 3:1 (WCAG) with the background, otherwise in a light grey. The
  backend draws it only where no part was drawn (stencil), so it never
  covers a face.
- **Depth cues: screen-space ambient occlusion and a ground.** Chosen over
  a shadow map because the default key light is a headlight, whose
  shadows fall mostly behind the parts, while occlusion darkens pockets,
  holes, gaps and the ground next to a part from any light. It is a
  deterministic, WebGPU-friendly post pass: core gives the 16 kernel
  points, the 4 x 4 rotation pattern, the radius (15 % of the scene
  diagonal by default), strength, bias and depth range; the backend reads
  depth sample 0, averages each 4 x 4 block and multiplies the colour.
  Open flat faces come out at exactly 1 and are left untouched. The
  optional ground (`Ground`: height defaults to the lowest point, colour
  defaults to the background, so it shows only the contact shadow) is
  never cut and is not part of the fit.
- **Cutaway.** Up to four `ClipPlane`s. Front faces on the removed side are
  discarded and a line marks the cut; back faces, which can then only be
  seen through a cut, are drawn flat in the item's section colour (given,
  or derived from its base colour leaning toward a brick red), and a
  full-screen pass moves those pixels' depth onto the cut itself, so the
  ground and edges behind a cut face stay hidden. This relies on closed
  meshes. The camera fit uses the part of the scene box the planes keep.
- **Exploded views.** `explode_offsets` and `Scene3d::explode` move items
  away from the centre of all their boxes by a factor along chosen axes.
- **Many parts.** The backend caches vertex and index buffers per shared
  `Arc<RenderMesh>` (dropped after three frames unused) and draws the items
  that share a mesh in one instanced call, from a storage buffer.
  `Scene3d::add_shared` adds another placement of a prepared mesh.
- **Contact sheet and palette.** `contact_sheet` lays renders out in a
  labelled grid; captions are TEXT entities drawn by the 2D renderer, so no
  font dependency. `palette_color(i)` hands out muted colours (OKLab
  chroma 0.075, three lightness steps, golden-angle hues).
- **Evidence** (`cargo test -p cadview-render3d`, one GPU machine).
  Outline contrast at 18 points around a box: 5.13:1 on the light
  background, 7.60:1 on the dark one, 1.11:1 on the dark one with the
  outline off (threshold 3). Floor of a 12 mm pocket seen from the top: 27
  levels darker than the top face (threshold 20), which keeps its
  reference shading. Ground 0.5 mm from a cube: 35 levels darker than 8 mm
  away, which is the background colour (threshold 15). Without ground or
  occlusion the image is byte-identical to occlusion at strength 0, and
  with every new feature off the five first-slice images are
  byte-identical to the first slice's output. Cutaway: section colour
  within 1 level on material, the cavity wall's reference shading through
  the cut, also with a ground. 500 boxes sharing one mesh prepare one mesh
  and draw the same bytes as 500 separate meshes.
- **Times** (release, 960x640, warm, including readback): 500 boxes
  sharing a mesh 1.5 ms (first render with a cold cache 1.6 ms); 500
  separate meshes 1.8 ms (first 7 ms). The default scene with occlusion
  2.1 to 3.4 ms including PNG encoding. On DX12 WARP 31 ms, within 2 levels
  of the GPU image.

### 3D examples, first gallery (2026-11-18)

Nine of the eleven targets in [3d-examples.md](3d-examples.md) are scripts
in `examples/3d/` (01 to 08, and the timber frame of 11), with shared
helpers in `examples/3d/lib/` loaded by `cad.use`. Each checks its parts
against closed forms it computes (volumes to 1e-9 relative, boxes, counts)
and writes an isometric working render, committed, plus the other views to
`target/3d-views/`. `the_3d_examples_run_and_check_themselves` (server)
runs them all through the script entry, renders to a temporary folder.
What each simplifies is listed in 3d-examples.md.

- **Render changes.** Three more isometric corners (`iso-sw`, `iso-ne`,
  `iso-nw`): the play tower's stairs are edge-on from `iso`. `clipMode:
  "all"` removes only what lies beyond every clip plane, so two planes
  give a quarter cutaway (`ClipMode` in `scene3d.rs`, the shader's
  removal test, cut line and cut depth). A part's palette colour follows
  its place in the document, so views that `select` some parts keep the
  colours of the full view. `any` mode renders as before (the existing
  cutaway and first-slice tests are unchanged).
- **API fix.** A call whose result the kernel builds but cannot measure
  used to store the change and still report a failure. It now fails as a
  whole, names the call and changes nothing (create, modify, `copy3d`).
- **Findings.** Where two cylinders meet (a cross bore, a round boss in a
  skirt, a tee) the arris adapter evaluates the solid but cannot give its
  box ("a tight bounding box of a NURBS edge is not supported by this
  kernel yet"), so the part can be neither queried nor rendered. An arc
  whose circle crosses a revolve axis is refused ("a spindle torus"),
  though the arc stays clear of it. Every call re-evaluates the whole
  feature list, so a part with n features costs O(n^2): the enclosure base
  (20 features) takes 7.6 s of the enclosure's 11 s. A `near` point on
  +x of a revolve's sketch picks the seam edge.
- **Times** (release, one GPU machine, 960x640): whole script runs 0.4 to
  3.5 s, the enclosure 11.7 s; evaluation is most of it. Two to three
  renders per example take 0.36 to 0.56 s together, the GPU device's
  creation included. Two runs give byte-identical PNGs.

### 3D1b: what landed (2026-11-18)

Checks between parts, and three fixes the gallery asked for. Reference:
`docs/api-surface.md`, "Solid parts" and "Checks between parts". Not
landed from the row above: tapered extrude (still refused) and the turned
leg by `revolve` against its corpus volume (the corpus gate covers it on
the trait; no API test was added).

- **Evaluation is no longer quadratic.** The cache continues a definition
  from its longest cached prefix of features (`Solids::solid`, keyed by
  the prefix's features and the tools they reach) through a new optional
  `SolidKernel::evaluate_from`, whose default evaluates from scratch. arris
  continues on a copy of the prefix's model: a model is append-only with
  ids minted in order, so the copy holds what a from-scratch run holds at
  that point, the copy shares storage, and the result is the same B-rep.
  Evidence: volume, centroid, box and mesh bit for bit against from
  scratch on the enclosure base, a bookshelf side and a cross-bored
  revolve (`cadview-solid/tests/checks.rs`), and on every evaluated prefix
  of the harness catalogue (88 continuations equal, 21 refused alike,
  `solid-harness/tests/capabilities.rs`). Enclosure example (release
  server, headless, no renders): build 9.2 s before, 3.4 s after; its
  20-call base 7.7 s before (calls growing from 9 to 944 ms), 2.5 s after
  (9 to 218 ms). The piston 2.9 to 1.8 s. What a call still costs grows
  with the part, not with its feature count: on the finished base, the
  feature's own boolean 57 ms, the kernel's full validity check of the
  result 71 ms, the volume integral 23 ms.
- **Parts where two cylinders meet have boxes.** A new optional
  `SolidKernel::bbox_within(brep, frame, tol) -> TightBox {aabb, tol}`
  (default: the exact `bbox`); `bbox` stays exact-or-refused, so the
  harness demands exact boxes as before and both scoreboards came out
  byte-identical. arris brackets each NURBS edge's extremes: the curve
  over a parameter range lies in the box of its segment's control points
  (positive weights, knot insertion), pieces are halved until no control
  box reaches more than the tolerance past the curve points found, and
  boxes are padded for rounding. Parts report `bboxTol` (0 exact, else
  1e-9 mm). Cross-bored cylinder: box off its closed form by 4.6e-10 mm;
  two crossing cylinders (sides decided by the intersection curves)
  5.2e-10 mm; a bored tee exact (its NURBS edges lie inside). A second
  through hole on a cross-bored part works too (the hole's reach uses
  the bracketed box). Catalogue: 236 boxes equal to the exact one with
  tolerance 0, 2 bracketed. NURBS faces still have no box.
- **Checks between parts** (`cadview-core/src/setquery3d.rs`):
  `overlapping3d`, `pairsWithin3d`, `inside3d`, `distance3d`, named after
  the 2D set queries with the part calls' `3d`, the same options (`differ`,
  `attrs`, `limit`, `strict`) and `skipped` for pairs the kernel cannot
  answer. A sweep-and-prune broad phase on world boxes; the kernel answers
  candidates through new optional `distances` and `interferences` (arris:
  every solid copied once into one scratch model, `query::distance` on the
  exact carriers, `common` measured on its exact faces, and `V(a) - V(a -
  b)` where `common` refuses). Gate: a shelf 2 mm too long gives exactly
  one record, the right side and that shelf, 10584.000000000004 mm^3 against
  10584 (3.4e-16 relative); the correct bookshelf gives none; two parallel
  plates 3.5 apart come back 3.5 exactly, two shelves their 326.4 opening
  to 1e-9. A hundred blocks: `overlapping3d` 26 ms, `pairsWithin3d` 0.7 s
  (182 pairs), `inside3d` with a box 0.5 ms.
- **`mirror3d`** makes a mirrored definition (a reflection is not a rigid
  motion): every definition mirrors across its own YZ plane, so all images
  of one definition share one mirrored definition, placed by `M P S`;
  mirroring back finds the original definition.
- **`near` at a seam.** The seam won because the point at +x of a revolve's
  sketch lies on the circular edge and on the revolve's seam at once (they
  meet there), at distance 0 to both, and the seam came first. Seams (both
  sides one face) are never picked now, and among equally near edges one
  the point lies inside beats one it meets at an end vertex. The flange's
  chamfer near (82.5, 0, 18) gives its Pappus volume.
- **Examples.** 03 and 11 check, through the new calls, that no two parts
  overlap (none found in either), that every shelf touches both sides, the
  shelf gap, and that the swing beam rests on the head rail and both legs.
  They use the typed calls. The server test runs them and the ladder gate
  through the script entry. The play tower gained a gabled timber roof
  (27 parts: wall plates, rafters seated with a birdsmouth cut by the
  plate, a ridge board, butted roof boards, ridge caps; 63 parts in all),
  checked the same way: no overlaps, every rafter on a plate and the ridge,
  and 328.8 mm from the swing beam to the nearest rafter, to its closed
  form. Its committed picture changed.
- **Open.** A shelf lengthened by `union` (its end face split by an edge
  lying in the dado's bottom face) is a contact arris refuses for `common`
  and both cuts: that pair lands in `skipped`. No `nearest3d`, no point
  operand for `distance3d`, no interference against NURBS faces (arris
  refuses those, so they are skipped). Example 05 still uses square pin
  bosses; the box limit that forced them is gone.

### 3D5a, first slice: what landed (2026-11-18)

`cad.render({ quality: "presentation" })` path traces the document's parts:
the same scene, camera and options as the working render, headless and
deterministic, on the shared GPU device with plain compute shaders (no
ray-tracing extensions). The browser viewport, `cad.view3d`, the perf
harness, the denoiser, HDRI environments, texture sets and the look gate
(3D5b) are still open.

- **Origin.** A tracer of our own, not a fork of Kiss3d's: its kernel loses
  sky light (an environment it never samples is weighted by multiple
  importance sampling anyway) and its diffuse sampling and evaluation
  disagree, so a fork would have meant rewriting the sampling core. The
  evidence and what was given up are in design §9, "Decision update
  (2026-11-18, 3D5a)". No code is derived from it; no new dependencies.
- **Backend.** `PathTracer` (`cadview-render3d/src/trace.rs`, `trace.wgsl`,
  `resolve.wgsl`) behind `Renderer3d`, progressive (`Partial` until the
  target), with a two-level BVH (`bvh.rs`: binned SAH per mesh, instances
  for shared meshes), next-event estimation of the sun with the power
  heuristic, Russian roulette, a luminance clamp on indirect light only,
  and the core's tone curve. `render_rgba_stats` and `render_radiance`
  expose its statistics and its linear radiance.
- **Core.** `Material` gains transmission, index of refraction, emission,
  a clear coat and a `Pattern` without texture coordinates: `Bands`
  (rings about an axis: wood grain) and `Brushed` (streaks along an axis or
  in circles about it, anisotropic highlights). Axes can be the part's
  longest, shortest or symmetry axis (principal axes of its surface), so
  presets need not know how a part was modelled. `Environment` (a dome of
  zenith, horizon and ground colours plus a `Sun` of adjustable direction,
  frame and size), presets `studio` (default) and `sky`, and `exposure` in
  stops. The working render ignores all of these except colour, metallic and
  roughness.
- **Options.** `quality`, `spp` (default 1024), `seed`, `materials: {attr,
  map, default}` (attribute values to physical parameters, like `colorBy`),
  `environment` and `exposure`; the result reports `samples` and a `trace`
  block (passes, triangles, BVH, upload, trace and PNG times).
  `examples/3d/lib/materials.js` holds the names: oak, walnut, ply,
  timber, steel, stainless, aluminium, turned steel and aluminium, brass,
  glass, pcb, plastics, paints, rubber.
- **Evidence** (`cargo test -p cadview-render3d --test present`, one GPU
  machine). Furnace, white objects in a uniform environment of radiance 1,
  256 spp: a Lambert block with a pocket 0.99851, a smooth glass sphere
  0.99991, a mirror sphere 0.99992 (tolerance 0.5 %). A Lambert plate under
  a uniform upper hemisphere against `rho (1 + cos tilt) / 2`: -0.00, -0.05
  and +0.09 % at 0, 60 and 90 degrees; under a small sun against the cosine
  law: -0.00 % at 0 and 60 degrees (tolerance 1 %). Ground under a box in
  the sun 5.5 times darker than open ground (threshold 4); open ground
  0.4356 against the closed form 0.4353 (tolerance 2 %). A 20 mm glass slab
  tilted 45 degrees moves an edge by 13.169 px against the closed form
  13.166 px (tolerance 0.1 px). Same seed, same bytes; another seed,
  different bytes. Error against a 4096-sample reference at 16, 64, 256 spp
  falls by 2.04 and 1.94 (one over the square root of the samples). The
  silhouette covers the working render's pixels within 0.03 to 0.16 px per
  outline pixel over a box and a sphere in three views (tolerance 0.25), no
  pixel flipped. The tracer's albedo equals the core's pattern to a median
  of 0.0004. The clamp moves a studio scene's mean by -0.08 %; a glass
  sphere in sunlight loses its caustic (-6.6 % of the mean, reported).
- **Examples** (release, 960 x 640, 1024 spp, studio light, the
  development machine's discrete GPU). Tracing 1.16 s (bracket) to 2.37 s
  (enclosure with the lid lifted); BVH builds 0.1 to 3.3 ms, uploads under
  1 ms, PNG encoding 4 to 7 ms; pipelines compile once per process (about
  0.3 s on a cold driver cache). The bookshelf reaches 256 spp in 0.5 s
  here, against the 3D5a aim of 30 s on a mid-range GPU in Chrome; the gate
  itself is a browser measurement and is not claimed. Noise against
  4096-sample references, root mean square in 8-bit levels: 1.1 to 2.9 at
  256 spp, 0.7 to 1.6 at 1024 (the reference's own noise included). What
  each example still lacks is in [3d-examples.md](3d-examples.md).
- **Open.** A denoiser (deterministic, e.g. a-trous on albedo and normal
  guides). HDRI environments (reading Radiance `.hdr` is a small decoder,
  but sampling one well needs a 2D distribution and a third light strategy).
  Image textures and Poly Haven sets (assets, a manifest, UVs from the
  kernel). Next-event estimation of emissive surfaces (they are reached by
  BSDF sampling only). Caustics from small lights (clamped away).
  Low-discrepancy sampling (independent random numbers today). The look
  gate against Cycles (3D5b). Examples 9 and 10.

### 3D5a, second slice: what landed (2026-11-18)

A denoiser, true rough glass, glass that lets the sun through, portals for
interiors, a studio backdrop without a horizon line, a placed camera, and
examples 9 (room) and 10 (still life). Evidence is in
`cargo test -p cadview-render3d --release --test present` (one GPU
machine); every first-slice test passes unchanged in substance.

- **Denoiser** (`denoise.wgsl`, `TraceSettings::denoise`, render option
  `denoise`, on by default in `cad.render`). The tracer sums per pixel what
  it knows exactly for every sample: first-hit albedo, normal and
  position, followed through mirrors and clear glass. An edge-avoiding
  a-trous filter (5 scales, 1 to 16 px) smooths the irradiance (radiance
  over albedo) where those agree and where the brightness difference is
  within the two pixels' own noise, then multiplies the albedo back, so
  grain and streak patterns stay exact. Deterministic; a few ms at
  960 x 640. Tests: on a 4096-sample studio scene it changes 0.46 display
  levels (root mean square), more than 2 levels on 0.4 % of pixels, and the
  change brings the image closer to the truth (estimated from two other
  references). At 64 samples, with direct light and sky, it cuts the error
  against a 4096-sample reference 2.6 times; a box edge stays 0.77 px wide
  (10 to 90 %), and a soft shadow edge goes from 1.74 to 1.61 px (it
  steepens a little, never blurs). On the small studio scene with a polished
  sphere it cuts the display error 1.7 times but the linear error only 1.24
  times: rare bright paths (the key light focused by the sphere) are spread,
  not removed. The furnace and irradiance tests run without it.
- **Default samples stay 1024.** Denoised, 256 samples look converged on
  every example looked at (01, 03, 06, 10; 01 and 06 measured: 1.25 and
  1.00 levels against a 4096-sample raw reference, like 1024 raw); 1024
  denoised is 0.86 and 0.67. The room uses 512 (see below).
- **Material paths.** The clear coat in the furnace loses 1.1 % (coat
  roughness 0.05) and 2.4 % (0.5), single-scattering loss, no gain. Rough
  glass lost 31 % at roughness 0.6: a microfacet that sends light the wrong
  way now is drawn again and no shadowing weight is applied, so the energy
  goes on along the lobe (0.04 % from white in the furnace, roughness 0.3
  and 0.6; the lobe's shape at grazing angles is approximate). The bounce
  cap went from 16 to 64 events (light trapped in glass hit it). An emitter
  is seen at its own radiance exactly; a plate lit by a sphere of known
  size agrees with `rho L R^2 h / d^3` within 0.6 % at 16384 samples.
  Emitters are still found by BSDF sampling only, so they converge slowly.
- **Glass passes the sun.** A shadow ray now crosses glass, attenuated by
  Fresnel and tint and not bent; paths that reach the sun through glass
  from a surface that sampled it are not counted again. A pane casts 0.9232
  of the open floor's light against the closed form 0.9216. A thick glass
  body's focused caustic is not drawn (the clamp removed most of it before;
  the glass sphere in the sun now loses 0.13 % to the clamp, was 6.6 %).
- **Portals** (`Portal` in `scene3d.rs`, render option `portals`): up to
  four rectangles through which the dome is sampled directly, weighted
  against BSDF sampling. A closed room lit by a uniform sky through a
  ceiling opening: the floor matches the form factor within 0.02 % at 256
  samples with the portal, its pixels spread 0.22 % against 27 % without;
  16384 samples without agree within 0.2 %. In example 9 they help less
  (raw noise 7.2 to 6.1 levels at 256 samples): there the noise is the sunlit
  floor's light bouncing onto the walls.
- **Studio backdrop** (`Environment::sweep`, on for "studio"): what is seen
  just above the horizon blends from the open ground's own radiance into
  the dome over 20 degrees, so a level view has no floor-to-wall line (rows
  either side of the horizon differ by 0.2 %, were 70 %); lighting is
  unchanged.
- **Placed camera** (`Camera3d::placed`; render option `camera: {eye,
  target, fov | height, up}`, `fit`): perspective with a field of view or
  orthographic with a height, any up direction (default +Z, where the old
  rule is kept bit for bit), both qualities, fitted only if asked. A known
  point lands within 0.06 px (presentation) and 0.15 px (working) of its
  closed-form pixel, perspective and orthographic; from eye height the
  horizon is at row 80.786 against the closed form 80.788.
- **Ground material** (`Ground::material`, option `ground.material`) with
  a new generic pattern, `Mottle` (two octaves of soft value noise toward a
  second colour): the tracer's ground albedo equals the core's to 0.0002;
  `materials.js` has `lawn`. **Aerial haze** (`Environment::haze`, 150 m
  for "sky"): ground seen far away fades into the horizon colour; the rows
  either side of the horizon differ by 6.3 %, were 76 %. Example 11 gained
  an eye-height view on a lawn (its picture unchanged).
- **Determinism.** The same accumulation filters to the same pixels. Two
  identical 4096-sample renders can differ in the last bit of the raw sums
  in a few hundred pixels (GPU arithmetic, seen with this slice's shader);
  no 8-bit pixel changed in the four compared.
- **Examples.** 10, the still life: 11 parts, 38 checks (Pappus volumes of
  every revolved profile, no arc's circle crossing the axis, everything on
  the table, no overlaps); camera render 5.3 s at 1024 samples. 9, the
  room: 07's living room with the bookshelf and stool built by
  `lib/bookshelf.js` and `lib/stool.js` (03 and 08 now use them; their
  pictures are byte-identical), 25 parts, 17 checks; interior 17 s at 512
  samples. Details and what still looks wrong are in
  [3d-examples.md](3d-examples.md).
- **Open.** Next-event estimation of emitters and of bright surfaces (the
  room's sun patch). Caustics. Brushing that follows a swept part's own
  direction. HDR environments. Haze on parts (only the ground fades).

### 3D in the browser: 3D2b's viewport and 3D5a's browser side (2026-11-18)

The browser shows parts in both render modes, from the same core code as
the server, and evaluates them itself once it loads the kernel module.
Design: [3d-design.md](3d-design.md) section 14. Reference:
`docs/api-surface.md`, "Solid parts in the browser".

- **Viewport** (`cadview-web/src/view3d.rs`). A canvas with its own surface
  on the page's one `GpuContext`; demand-driven frames; drag to orbit,
  right- or shift-drag to pan, wheel to zoom; named views, fit, orthographic
  and perspective. The scene is `solid::scene::parts_scene`, the same call
  and options as `cad.render`.
- **Working mode**: the raster backend, one frame per change.
- **Presentation mode**: the path tracer, progressive. The working render
  is shown while the camera moves; once still, samples are added in
  batches, each submitted only after the GPU finished the last and sized to
  about 24 ms of GPU time; accumulation restarts on any change and stops at
  the target. A first sample slower than 40 ms traces at half resolution;
  the target is 30 s of work at the measured speed, 16 to 256 samples.
- **UI.** A 2D/3D switch per tab when the document has parts, and
  Working/Presentation, named views, Ortho/Persp and Fit in the 3D view,
  with a sample counter. Without WebGPU the 3D view says so in words.
- **Prebaked scenes** (`solid/baked.rs`, `cad.bake3d`, `GET /api/bake3d`):
  meshes, exact boxes and volumes, placements, attributes and render
  options; read as an evaluator, so `parts()` on a bake gives the kernel's
  numbers bit for bit. The embed viewer shows one with
  `<codecad-viewer scene="part.bake3d">` and a mode switch; the app opens
  one with `?scene=` or a dropped `.bake3d`. The gallery bakes to 17 to
  139 KB each (3 to 58 KB gzipped).
- **The app with a server** asks the server for the bake of the current
  document (an interim source until the page loads the kernel by default)
  and asks again when the parts change.
- **The kernel module** (`crates/cadview-kernel-wasm`, `just
  build-kernel`): arris behind core's byte interface (`solid/remote.rs`),
  4.33 MB, 1.42 MB gzipped, fetched only by the first call that needs
  geometry or `await cad.loadKernel()`, content-hashed with a manifest and a
  wire version checked three times. Calls are synchronous on the page's
  thread, like the script API. A test reads the evaluator traits and fails
  for any method that does not cross the boundary.
- **Evidence** (headless Chromium on its software GPU, SwiftShader; the
  native references rendered by the same code on the same software GPU
  through Vulkan, `cargo run -p cadview-render3d --example bake`):
  - the bookshelf, the pipe flange and the play tower frame, prebaked, in
    the embed viewer: working and presentation (16 samples, full
    resolution, seed 0) both equal the native render on every pixel (`pngdiff`: 0 of 614 400 over 8/255; root mean square 0.00);
  - one GPU device per page: the embed viewer, and the app with a 2D tab
    and a 3D tab, each log one `GPU device created`;
  - idle: no frame and no animation-frame callback in 3 s, in working mode
    and after the sample target;
  - orbit: a 120 px drag turns the camera 55 degrees, draws 14 working
    frames while moving and restarts the accumulation;
  - network: the prebaked page fetches the page, the viewer script, the
    wasm-bindgen glue, the main module and the 3.6 KB bake, nothing else;
    the app with a server fetched `/api/bake3d` once (19 KB) for its 3D
    tab and nothing of the kernel; a page without a server fetched nothing
    of the kernel until the first call that needed geometry, which was
    refused with the call to await and started one fetch of the manifest
    and the module (1.43 MB on the wire, 0.3 s on loopback, 0.12 s of it
    to compile), and nothing after;
  - kernel equivalence: the 3D1a assembly script run in that page gives
    all 120 numbers (volume, box, centroid of twelve parts) bit-identical
    to the server's, the same ids and `describe()`, and identical results
    of `overlapping3d`, `pairsWithin3d` and `distance3d`; then both modes
    draw it from the page's own geometry.
- **Times on the software GPU** (not real-GPU numbers): bookshelf first
  frame submitted 1.4 to 10 s after navigation (module download and
  compile, device; on a cold run SwiftShader's shader compilation made the
  first raster frame take 9.2 s of GPU time); warm working frames 170 to 183 ms of GPU time, 0.4 to 0.9 ms of
  CPU; one path-traced sample at 960 x 640 1.6 to 1.7 s, so the adaptive
  view traced at half resolution and reached its target of 38 to 64
  samples in 18 to 28 s (three runs); 16 samples at full resolution took
  25 to 35 s. The kernel page, 1000 x 688: 82 samples at half resolution
  in 33 s. On this machine's GPU,
  natively, the bookshelf's 256 samples take 0.5 s (3D5a); a real browser
  GPU was not measured.
- **Sizes.** Main module 25.33 MB before, 25.94 MB after (8.53 to 8.71 MB
  gzipped, +177 KB): the raster, the tracer, the view, the bake reader and
  the kernel proxy; the tracer alone is 97 KB raw, 29 KB gzipped. The embed viewer's script 28.5 to 38.7 KB.
- **Also fixed.** The page applied the server's echo of its own empty
  flushes (two zero bytes, sent after every read-only call such as the
  layer panel's poll) as a document change: a rebuild and a frame in every
  view of the session, about once a second. Empty updates are skipped now.
- **On the combined tree** (with 3D3a's projection and 3D5a's second
  slice): the bookshelf's working render, and its presentation at 16
  samples both denoised and raw, equal native on every pixel; one device;
  flat frame counters; the prebaked page fetches no kernel. The assembly
  still matches the server bit for bit, and `cad.project` (front and iso
  of a plate with a hole and a pin) through the page's kernel gives the
  server's 34 entities, 419 numbers identical: `topology`, `silhouettes`
  and `ray_hits` cross the boundary (wire version 2). The view follows the
  denoiser (on unless `denoise: false`, as `cad.render`) and a placed
  `camera`. Natively on a discrete GPU, against 4096 samples: 16 denoised
  samples leave 1.63 levels of noise (RMS), between 64 raw (2.38) and 256
  raw (1.21); 64 denoised leave 0.97. So a clean image needs about four
  to eight times fewer samples. The second slice's tracer is slower per
  sample on the software GPU (6.6 s at 960 x 640, was 1.7 s), so the
  adaptive view there settles on 16 samples at half resolution (41 s);
  each denoised batch costs about 1 s more there.
- **Open.** The kernel runs on the page's thread (a heavy part blocks for
  a fraction of a second); a Worker needs cross-origin isolation or JSPI.
  The projection's trait methods (`topology`, `silhouettes`, `ray_hits`)
  got their requests in 3D3a (wire version 2). Two
  3D tabs at once (only the active tab has a canvas), the perf harness
  (`web/tests/perf3d.mjs`) and the 3D2b/3D5a time gates on GPU-A and GPU-B,
  zoom-adaptive meshes (the bake's one chord serves every zoom), lines mode,
  a part list in the layer panel, bakes committed for the site, and the
  site's 3D gallery. The release and site builds do not build the kernel
  module (no CI change here), so they show bakes and server bakes only.

### 3D3a: what landed (2026-11-18)

Projected views with hidden lines as exact 2D entities. Reference:
`docs/api-surface.md`, "Projected views"; design section 7.

- **The call.** `cad.project({view | from+up, parts, origin, at, scale,
  hidden, silhouettes, smooth, layers, partAttrs, name})` makes one block
  definition per view (true size) and one insert at `at` scaled by
  `scale`; projecting again with the same name replaces the block and moves
  its insert. Every entity carries `partId`, `edge` (visible or hidden) and
  `curve` (edge or silhouette). The script names the layers; a layer the
  call creates gets plain defaults. `cad.viewPoints` (and `toSheet` on the
  typed result) maps 3D points to the sheet. Views by the names `cad.render`
  uses, or by a direction. First- and third-angle arrangement is script
  code (`examples/3d/lib/views.js`). Not built from the row above: the
  `3D_*` layer names (no layer has meaning in the core), `3D_CENTER` axes,
  the committed golden JSON (the numbers are in the tests instead), and the
  Truck adapter (dropped in 3D0b).
- **Kernel capabilities.** Optional `SolidKernel::topology` (faces with
  closed-form surfaces, edges with exact curves: line, circle, ellipse,
  rational B-spline; the faces each bounds; seam and smooth flags),
  `silhouettes` (core's closed forms in `solid/silhouette.rs` for
  cylinders, elliptic cylinders, cones, spheres, and tori seen along or
  across their axis, clipped by the kernel to each face, the ends moved
  onto the face's boundary edge by Newton's method) and `ray_hits` (arris
  `ray_fire` on the exact faces). Defaults refuse. They cross the browser's
  kernel boundary too (`solid/remote.rs`, wire version 2).
- **Hidden lines** (`solid/hlr.rs`, `solid/oracle.rs`): exact projection;
  splits at crossings in the view (closed form where one curve is straight,
  Newton otherwise), at end points of other curves lying on a curve, and at
  the folds of conics seen edge on; one oracle question per piece; merge on
  carriers with visible winning. The oracle is a mesh (chord max(0.01 mm,
  1e-5 of the size)) in a bounding volume hierarchy; it is trusted only
  away from the mesh's outline in the view (contour sides between triangles
  facing opposite ways), otherwise four rays 8 chords to the side decide
  (the rule the exact fallback uses for rays inside a face), and failing
  that the kernel's exact rays. Nothing tessellated is stored or drawn.
- **Evidence** (`cadview-solid/tests/projection.rs`, arris). A 100 x 60 x 10
  plate with a 10 mm hole and an 8 mm slot: top, front and right views equal
  hand-computed lines, circle and arcs with a worst error of 0 mm; the side
  view's four hidden lines (y = 25, 26, 34, 35) are there with hidden lines
  on and absent with them off. Iso view of a 10 mm hole: one ellipse,
  semi-axes 5.000000000000001 and 2.886751345948129 against 5 and 5 / sqrt 3
  (see the note below). A cylinder of radius 7: its silhouettes are the
  lines x = -7 and 7 exactly; turned 90 degrees, off by 7e-15 mm. The
  oracle against exact rays on all 13 corpus parts in six views: 1710
  pieces, 5130 samples at 5, 50 and 95 % of each piece, 0 disagree (at 2 and
  98 %, 4 samples disagree, all next to the fold of an edge-on circle at
  the top of a bead, where the ray is 0.007 mm inside the circle and the
  kernel marks its own hits as grazes). The same view twice, and in a fresh
  document, gives the same block bytes. Harness (`capabilities.rs`, against
  the certified reference): 2026 edges of 117 parts on the boundary; 818
  silhouette curves in four views on the boundary with |n . v| at most
  2.9e-12; 11 700 rays whose hits lie on the boundary, 2902 segments
  between crossings inside or outside as the reference says. Scoreboards
  unchanged. Server: the bookshelf's front, left, top and iso views saved
  to DWG and reloaded hold only LINE (56, 44, 22, 195), ARC (0, 1, 2, 0) and
  ELLIPSE (0, 0, 0, 8), every one with its `partId`.
- **The iso ellipse's minor axis.** The ladder row asked for semi-axes 5 and
  5 cos(35.264 degrees) = 4.08. A circle seen at 54.736 degrees from its
  normal has minor semi-axis 5 cos(54.736) = 5 sin(35.264) = 5 / sqrt 3 =
  2.887; the test checks that.
- **Times** (release, one machine): the example's pillow block 26 to 42 ms
  per principal view and 282 ms for the iso view; the bookshelf 18 to 58
  ms per view (debug); the play tower, now 142 parts, 56, 39 and 65 ms for
  front, right and top and 667 ms for the iso view (2404 entities; 96 % of
  it classification, 12 exact rays). Before the outline-only margin and the
  side rays the pillow block took minutes: an exact arris ray on it costs
  about 55 ms (it rebuilds its face index per ray).
- **Example** `examples/3d/12-projected-views.js`: a pillow block (fillets,
  counterbores, a chamfered bore, an oil hole into the bore) drawn on an A3
  sheet in first angle with an isometric view and dimensions placed
  through `toSheet`, written as PNG and PDF; its checks read the views back.
- **Open.** Silhouettes of tori seen obliquely and of B-spline faces (listed
  in `open`, missing from the view); B-spline edges that lie on no conic
  are fitted cubic splines (0.01 mm), not exact; parts must not
  interpenetrate; sections, hatching, centre lines and automatic
  dimensions are later rungs; the browser's own kernel answers the new
  calls through the wire but was not tested in a browser. The typed calls
  are in `web/src/cad-api.ts`, but `cad-client/cad-api-setup.js` is not
  rebuilt: scripts reach them through `__cadCall` (lines marked `RAW:`).
  An oil hole whose axis meets the bore's is refused by arris 0.11.1 (the
  example's is offset 8 mm).

## Risks

- **arris is about two months old with one maintainer, and breaks its API every minor release (pre-1.0).** ogeom, the first choice, changed its licence within weeks. The mitigations are an exact pin on a permissive release, the Truck differential corpus from 3D0b onward, the right to vendor, and documents that store no kernel data. If arris stalls before 3D3a, `TruckKernel` plus our own HLR adapter and closed-form silhouettes carry 3D0–3D3 (the HLR adapter is ours in any case, since arris has no HLR). Roundovers then drop to `fallback: "visual"` until a kernel can do them.
- **Evaluation latency in wasm.** The smoke test took 3.3 s for a fully rounded board. Content-hash caching, time-slicing and showing the previous B-rep keep the UI responsive. A Web Worker evaluator is the escape hatch (design §13), and the 3D0b timing table decides whether 3D2b needs it.
- **Kiss3d as an embedded backend.** It has thread-local managers, an engine-shaped API and the look question. The `Renderer3d` trait, the 3D5b gate and the fork fallback bound the risk. Replacing Shaded mode with a thin raster pass of our own is a contained rewrite behind the same trait. The first 3D2b slice confirmed the thread-local problem at run time and took that route for the working render; Kiss3d now matters only for the presentation render.
- **Three crates must agree on wgpu** (Vello, Kiss3d, egui). They agree on wgpu 30 today. Pin and bump them together. `GpuContext` turns a mismatch into a compile error, and 3D2a's `cargo tree -d` check keeps it that way.
- **Headless rendering without a GPU** (lavapipe) is unverified. If it is too slow: lower spp and denoise, run the 3D example export on a GPU host, or fall back to the headless-Chromium path.
- **Projection quality edge cases**: tangent-edge clutter on rounded parts, near-degenerate ellipses, torus silhouettes on turned parts, and the visibility oracle near grazing faces. Goldens grow with every bug found, `tangent_edges: false` is the default, and exact ray hits against the faces cross-check the oracle.
- **Poly Haven availability and licence drift.** Assets are CC0 today. The manifest pins hashes, so a changed or removed upstream asset fails loudly at fetch, and nothing is vendored into git.
