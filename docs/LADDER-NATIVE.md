# Ladder N: native desktop CodeCAD

> **The same CodeCAD in a native window.** A desktop app with the web version's plain framing (tabs, a document picker, a layer panel, a canvas). It opens local DWGs offline and connects to a `cadview-server` to sync and run scripts, built on the same `cadview-core` and the same Vello scene builder as the browser. It is a thin shell, like web and server: all CAD logic stays in core, and all 2D drawing code is shared with the web renderer. Later it hosts the 3D viewport from LADDER-3D and ships in the existing release workflow.
>
> Builds on the 2D core and sync (`cadview-core`, `SyncDoc`), the Vello renderer in `cadview-web/src/vello_render.rs`, the server's WebTransport protocol and RPC (`cadview-server/src/transport.rs`), the codelib script sandbox (`cadview-server/src/script.rs`), and the release workflow (`.github/workflows/release.yml`). N5 waits for LADDER-3D 3D2b. Design context: [architecture.md](architecture.md), [3d-design.md](3d-design.md) §9.3 (shared device) and §11 (crate layout). Smoke tests: [research/3d-smoke/](research/3d-smoke/) (`native/`).
>
> **Status:** design done (2026-10-13). A native smoke crate compiles with **one wgpu (30), one winit, one vello**. It combines `eframe 0.36.2` (wgpu), `egui_dock 0.21.1`, `vello 0.11`, `kiss3d 0.47` and a `wtransport 0.7.2` client pinned to a cert hash. The UI-toolkit decision below comes from checking xilem/masonry main. The rungs are **N0a–N6** and the vision is **N🎯**. All are documented and none is started. N0a is the same work as LADDER-3D 3D2a, done once.

## Why this is interesting here

- **Big DWGs deserve native memory and threads.** Floor plans with thousands of entities and many tabs fit browser limits today. 3D evaluation (seconds per rounded part in wasm, design §6), headless renders and local script execution all run faster natively.
- **Offline is a real use case.** On a building site or in a workshop, a desktop app that opens a local DWG, runs a drawing script locally and syncs later is more useful than a browser tab with no server.
- **It proves the core-first rule.** If the desktop app can be built from `cadview-core` plus shell-shared GPU and store crates, with only window and chrome code of its own, the architecture holds. Each piece of logic that turns out to live in `cadview-web` gets moved down, and both shells benefit.
- **One GPU stack everywhere.** wgpu + Vello on the web and natively means the same pixels, and screenshot tests compare the two.

## Key decisions

- **UI chrome: eframe/egui now, Linebender (masonry/xilem) later when it is ready.** We checked the Linebender option first, and it is not usable for this today:
  - The standalone `linebender/masonry` repo is **archived** ("moved"). Masonry now lives in the [xilem monorepo](https://github.com/linebender/xilem).
  - Its last release, [xilem/masonry 0.4.0](https://crates.io/crates/masonry), is from 2025-10-29. Main (last commit 2026-09-14) is still on **vello 0.8 / wgpu 28 / winit 0.30**, two majors behind vello 0.11 (wgpu 30), so it cannot share a device or types with our renderer.
  - Masonry's `Canvas` widget paints through a new backend-neutral `imaging` recording (`imaging 0.0.1`), not a raw `vello::Scene`. Our CAD scene would be re-recorded through an extra layer every frame.
  - **Embedding custom wgpu textures in a widget is an open request** ([xilem#1319](https://github.com/linebender/xilem/issues/1319), open since 2025-08). We need that for the 3D viewport, and it is the clean way to show a Vello-rendered CAD texture.
  - There is no tab or dock widget, and the README calls the project "experimental".

  eframe/egui is already a dependency: the web fallback renderer uses egui 0.34, which moves to 0.36 in N0a. eframe 0.36.2 and egui-wgpu are on **wgpu 30**, the same as vello 0.11 and kiss3d 0.47. egui_dock 0.21.1 (it depends on `egui ^0.36`) gives tabs. egui-wgpu's `register_native_texture` shows our Vello output with zero copies. All chrome goes through a small `Chrome` boundary (tab list, layer list, picker events), so moving to masonry later replaces one module. The **re-evaluation gate**: masonry ships external-texture support and tracks the current vello major.
- **Windowing: winit through eframe** (0.30.13). Not winit 0.31, which is still beta (0.31.0-beta.3).
- **Extract the 2D renderer first.** New shell-shared crate `cadview-render` (GPU-bound, no CAD logic) holds the Vello scene builder (the entity → `Scene` part of `render_frame`), `Camera` and camera commands, LOD and culling, the BezPath cache, and block/text expansion. It compiles for wasm32 and native. `cadview-web` keeps the canvas, rAF and pointer glue, and `cadview-desktop` keeps the window and chrome glue.
- **Sync: the same `SyncDoc`, a native WebTransport client.** `wtransport 0.7.2` has a client with `with_server_certificate_hashes`, which is how the browser trusts the self-signed cert today. The server gets a small authenticated `GET /api/session` (WT port, cert hash). Today these are only injected into the HTML. The stream protocol is unchanged: a sync header, then SV exchange, then continuous updates, plus per-call RPC streams.
- **Scripts: server RPC when connected, an embedded sandbox when offline.** The desktop app links codelib behind a `local-sandbox` feature and runs the same JS engine and preludes the server runs, so there is one script environment everywhere.
- **Local documents through `cadview-store`.** The `DocumentStore` trait, `SingleFileStore` and `FolderStore` move out of `cadview-server` into a shell-shared, native-only crate used by server and desktop. Core stays filesystem-free.
- **Thin shell.** `cadview-desktop` contains window setup, chrome, file dialogs (`rfd`), the WT client loop and the local sandbox host. Anything that needs logic goes into core, `cadview-render` or `cadview-store`.

## Process

Each rung is built by a **builder/tester pair** and sized at **≤ about 2 weeks** of pair work. Every rung ends shippable.

- **The builder** (strongest model) owns the design, the core logic, the docs and the branch. At the start of the rung it spawns **one fast, cheap tester** (Haiku class) and **keeps it alive for the whole rung**, continuing the same tester rather than respawning it, so it keeps its warm build directories, the commands that work and what it has ruled out.
- **The tester** does the build-and-run work: native and wasm builds, UI tests, screenshot and pixel diffs, the sync-latency harness, release-workflow dry runs. It edits only test inputs, scratch files and the small changes the builder names, and decides nothing about the design.
- **The loop.**
  1. The tester first reproduces the previous rung's Done-when as a baseline. If the baseline fails, that is the first report, and nothing else proceeds.
  2. The builder sends a brief: the exact commands, the pass criteria and a time budget.
  3. The tester reports the command run, pass/fail per criterion with numbers, the failing output verbatim, and observations kept separate from inferences.
  4. The builder changes code and repeats.
- **Done** means every Done-when item below has been confirmed in the tester's report. The builder's final report lists what changed, how it was verified and what is open.
- **Hygiene.** One branch per pair. Every pair uses a separate `CARGO_TARGET_DIR`. Generated screenshots stay out of git, except the goldens a rung names.

**Pixel tolerance** used below: an image "matches" a reference when **≤ 0.1 % of pixels differ by more than 8/255 in any channel** (same size, sRGB PNG). The tester's diff script reports the fraction and writes a diff image.

## Rungs

| Rung | Content | Done when |
|---|---|---|
| **N0a** one GPU stack (= 3D 3D2a) | Bump `vello 0.9 → 0.11` and `eframe/egui 0.34 → 0.36` together (eframe 0.34 pins wgpu ^29), so there is one wgpu, 30. Add `GpuContext`: one device per page. | `cargo tree -p cadview-web --target wasm32-unknown-unknown -d` lists no duplicate `wgpu`. Every example DWG matches its committed PNG within the pixel tolerance, in both the Vello and egui renderers. Three tabs create exactly one device (debug counter). |
| **N0b** shared 2D renderer | New `crates/cadview-render`, moved out of `cadview-web` (`vello_render.rs`, `main.rs`): `Camera`, `CameraCmd`, `ViewState` (hidden layers, dirty flag), `SceneBuilder::build(&Document, &ViewState, size) -> vello::Scene`, LOD (cull below 0.5 px, smoothstep fade to 8 px), frustum culling, the BezPath cache, and block/text expansion. `cadview-web` becomes a user of it with no behaviour change. | `cargo build -p cadview-render` succeeds for native and `wasm32-unknown-unknown`. New native unit tests pass: culled-entity count at a fixed zoom, and fill/stroke counts for `01-flange-plate.dwg` equal committed numbers. Every example DWG still matches its PNG within the pixel tolerance. `vello_render.rs` keeps only canvas, rAF, input and surface code (review checklist: no `Shape` match left in it). |
| **N1** native window | `crates/cadview-desktop`: eframe 0.36 (wgpu backend). Vello renders into an `Rgba8Unorm` texture through `cadview-render`, shown with egui-wgpu `register_native_texture` in the central panel. Pan (drag), zoom (wheel, at the cursor), fit (F key). Opens a DWG or JSON from the CLI argument. Demand-driven repaint. Offscreen mode: `--screenshot out.png --size 960x640`. | `cadview-desktop --screenshot` for each example DWG matches the web PNG within the pixel tolerance (CI on a GPU runner or lavapipe). A debug counter shows 0 repaints over 5 s idle, then exactly one after a scripted wheel event. Startup to first frame for `01-flange-plate.dwg` is ≤ 1 s on GPU-A (as defined in LADDER-3D). |
| **N2a** tabs, layers, files | Tabs (egui_dock) with a session per tab (Document + SyncDoc + ViewState). A "+" tab for a new empty drawing. A layer panel (visibility toggles, colour swatches). File → Open / Save as DWG (`rfd`). Drag and drop of DWGs onto the window. | An `egui_kittest` test opens three example DWGs in tabs, hides a layer in one tab (the other tabs' screenshots are unchanged within the pixel tolerance) and saves as DWG. The reloaded file has the same entity count and layer table. |
| **N2b** document picker and `cadview-store` | `crates/cadview-store`, extracted from `cadview-server/src/store.rs` (trait, `SingleFileStore`, `FolderStore`) and used by server and desktop. Document picker: open a folder, show the grouped key list like `DocumentPicker.tsx`, plus recent files. | `cargo test -p cadview-server` and `cargo test -p cadview-store` pass, with the store tests moved as-is. An `egui_kittest` test opens `examples/` as a folder, and the picker lists the same keys, in the same groups, as `GET /api/documents` on a server started on `examples/`. |
| **N3a** connect and sync one document | `GET /api/session` on the server (bearer auth, returns WT port and cert hash). Desktop "Connect…" (URL + token). A wtransport client opens a per-document sync stream with the same `SyncDoc` handshake as `cad.ts`. | Integration harness `crates/cadview-desktop/tests/sync_latency.rs` starts `cadview-server` in-process on localhost, connects the desktop client library and a second WT peer, and applies 200 single-entity edits alternating between peers. **Both converge to identical Yrs state, and p95 edit-to-applied latency ≤ 50 ms** (timestamps carried in the edit). `/api/session` returns 401 without the token. |
| **N3b** server picker, RPC, reconnect | The picker lists `/api/documents` when connected. RPC (save, saveDwg) carries `doc_id`. Reconnect with SV exchange after a drop, refetching `/api/session` (the cert hash rotates on server restart). Offline edits merge on reconnect. | The harness kills and restarts the server mid-run. The client reconnects within 5 s, and all edits made while disconnected arrive (state vectors equal). `saveDwg` via RPC from the desktop writes the file on the server, and a reload has the same entity count. |
| **N4** scripts | A script console panel (editor plus output). Connected: `runScript`/`exec` RPC. Offline: an embedded codelib sandbox (feature `local-sandbox`) runs the same engine and `cad-api-setup.js` against the active tab's Document. Mutations are batched into one Yrs update, and `readFile` is sandboxed to the opened folder. Compiled components are cached in the user cache directory. | `examples/01-flange-plate.js` run offline produces `cad.describe()` and entity JSON identical to the server-run result (golden compare). The same script via RPC gives the same result. An infinite-loop script times out by epoch without blocking the UI (frame counter keeps advancing). The second offline run starts in ≤ 2 s (component cache hit). |
| **N5** 3D viewport | Host `cadview-render3d` on the same device: a 2D/3D toggle per tab, Shaded and Lines modes, Render mode once 3D5a exists, and local headless `cad.render` (native wgpu, HW ray query when available). Requires LADDER-3D 3D2b/3D2c. | Running the LADDER-3D perf scenario natively, the bookshelf orbit has **p95 frame time ≤ 16.7 ms on GPU-A**. After 3D5a, a Render-mode screenshot of the 3D5b reference scene differs from the browser render by mean ꟻLIP ≤ 0.05. One `wgpu::Device` serves egui, Vello and Kiss3d (debug assertion). |
| **N6** packaging | `release.yml` gains a `cadview-desktop` build per target (Windows x64, Linux x64/arm64, macOS x86_64/arm64), archived as `codecad-desktop-<tag>-<target>` next to `codecad`. macOS `.app` bundle (unsigned at first, notarization documented). Windows `.exe` with icon. Linux tarball. `just package-desktop`. | A `workflow_dispatch` run produces all five archives with `.sha256` files. Each job runs `codecad-desktop --version`. Linux x64 also runs `--screenshot` on `01-flange-plate.dwg` under lavapipe and matches within the pixel tolerance. A tag push attaches the archives to the release. |
| **N🎯** vision | A desktop CodeCAD that opens a folder of DWGs in tabs, edits and saves offline, runs drawing scripts locally, connects to a shared server and syncs live with browser users, and shows the 3D model and renders of the same document. It ships from the same tag as the server. | A scripted end-to-end test on a release archive, with no repo checkout:<br>1. Open `examples/` offline, run `01-flange-plate.js`, save as DWG.<br>2. Connect to a server and sync an edit with a WT peer within the N3a latency bound.<br>3. Open the bookshelf in 3D and write a headless render PNG.<br><br>Separately, a review checklist confirms every non-UI change in this ladder landed in core, render, render3d or store. |

### N0a: what landed (2026-11-17)

Same work as LADDER-3D 3D2a (details there).

- **Versions.** vello 0.12.0, wgpu 30.0.2, eframe/egui 0.37.0 (the
  newest pair on wgpu ^30, instead of the planned 0.11/0.36). The wasm32
  bundle has one wgpu (`cargo tree -d` lists none twice). wasm-bindgen
  moved to 0.2.130, since wgpu 30 needs a newer web-sys.
- **`GpuContext`** in `cadview-render` (wasm32 + native): instance,
  adapter, device, queue and `vello::Renderer` (Msaa16 only) from one
  constructor. Headless keeps one per process; the browser shares one per
  page across all canvases via `shared_local()`, and each canvas owns
  only its surface, config, intermediate texture and blitter. Device
  creations log `[CodeCAD] GPU device created (#N)`. A desktop shell can
  build its Vello path on the same type.
- Native GPU and CPU renders of every example are pixel-identical to the
  pre-bump ones. In headless Chromium the Vello viewer reproduces the
  committed PNGs, four canvases on one page create one device, and the
  egui fallback still works with WebGPU disabled.

### N0b: what landed (2026-11-17)

N0b landed with one change to the split described above: the decisions
went to core, not to `cadview-render`.

- **`cadview_core::render`** (pure, wasm32 + native) owns `Camera`
  (`fit` with the 1.1 margin, `apply(CameraCmd)`, world-to-pixel
  transform), `CameraCmd`, `View` (size, camera, hidden layers, pixel
  ratio, background), `SceneCache` (block/text expansion plus the BezPath
  cache, invalidated on entity count change as before) and
  `build_display_list`. The display list is renderer-neutral: background,
  one transform, and ordered items that each borrow an exact `BezPath`
  with a fill (even-odd or non-zero) or a stroke (width, optional dashes),
  colour and final alpha already resolved. Frustum culling, the 0.5 px
  cull, the smoothstep fade to 8 px, transparency, ByLayer colour,
  linetype dashes, the `pixel_ratio / zoom` stroke width and fills before
  strokes all live there, with unit tests.
- **`cadview-render`** holds backends only, with no `Shape` match: the
  Vello scene builder (wasm32 + native, feature `gpu`), a headless GPU
  render (one wgpu device and `vello::Renderer` per process, Msaa16,
  Rgba8Unorm readback), the CPU rasterizer (feature `cpu`), PNG encoding,
  and `render_png(&Document, &RenderOptions)` with `Auto | Gpu | Cpu`.
  `RenderOptions` parses the `cad.render` JSON, so a desktop shell can
  reuse it for `--screenshot`.
- **`cadview-web`** draws through both: `vello_render.rs` keeps canvas,
  rAF, input and surface code. `main.rs` is split into `session.rs`,
  `exports.rs` and `egui_render.rs` (a pure move; the egui renderer's
  drawing is unchanged).
- **`cadview-server`** got the first consumer: `cad.render` (sandbox and
  browser RPC) and `--render <PNG> --size WxH --render-backend`. N1's
  `--screenshot` can reuse `render_png` as is.
- Not yet done for the N0b gate: the browser pixel check against the
  committed PNGs needs the web toolchain (node, pnpm, wasm-bindgen CLI),
  which the machine that did this work did not have.

## Risks

- **egui's look and feel vs a "Linebender ideal".** egui is immediate-mode and looks like egui. Keeping the chrome behind a small boundary and the real work in `cadview-render` keeps a later masonry switch cheap. The gate is masonry shipping external textures (xilem#1319) on a current vello.
- **wgpu major drift** between eframe, vello and kiss3d. They are aligned on 30 today. Pin them and bump in one PR. If eframe lags, use `egui-wgpu` directly on our own winit loop (same API surface, more glue).
- **Moving code out of `cadview-web`** (N0b) can break the browser subtly: camera auto-fit, LOD thresholds, block caches. The pixel-tolerance gate on all examples is the guard. Do it before anything else depends on it.
- **WebTransport client behaviour**: QUIC through firewalls, and the self-signed cert hash rotating on each server start. The client refetches `/api/session` on reconnect, and N3b's restart test covers it.
- **Wasmtime in the desktop binary** adds size and compile time (the first engine compile takes about 25 s with codelib). The component cache and the optional `local-sandbox` feature keep slim builds possible.
- **Headless CI rendering** for screenshots needs lavapipe or a GPU runner, the same unverified risk as LADDER-3D 3D6b. Gate screenshot tests to runners that have one. `--version` always runs.
- **Code signing and notarization** (macOS, Windows SmartScreen) cost money and secrets. Ship unsigned first with documented workarounds, as many open-source CAD tools do.
