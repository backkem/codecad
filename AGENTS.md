# CodeCAD - AI-native 2D CAD

Browser-based 2D CAD viewer and editor with Rust server backend.
Reads .dwg via acadrust (Lines, Arcs, Circles, LwPolylines, Ellipses,
Splines, Hatches, Text/MText, Dimensions, Block Inserts). Dual GPU
renderer: Vello (compute-based, WebGPU) with egui/eframe fallback.
Multi-document tabs with per-session Yrs CRDT sync over WebTransport.

## Core-first rule

All CAD/app logic belongs in **cadview-core**. Browser and server
crates are thin shells. Specifically:

- **cadview-core**: document model, cad_call dispatcher, geometry,
  text, DWG loading, Yrs CRDT sync (SyncDoc), entity serialization.
  Compiles for both wasm32 and native. Both peers run the same code.
  Zero global state.
- **cadview-web**: session registry, dual renderer (Vello GPU +
  egui fallback), wasm-bindgen exports, viewport commands, JS/HTML
  glue. Delegates all mutation and sync to cadview-core.
- **cadview-server**: HTTP server (axum), document store + registry,
  WebTransport session management, per-document broadcast, RPC
  dispatch, server-side script execution. Delegates all mutation
  and sync to cadview-core. Scripts run in a
  [codelib](https://github.com/backkem/codelib) sandbox (embedded JS engine on wasmtime):
  timeouts, a memory cap, stdout/stderr capture, and `host.call`
  as the only way out, which `script.rs` answers with the document.

When adding new functionality, default to putting it in core. Only
put code in web/server if it genuinely cannot compile on the other
target (e.g. filesystem access, browser DOM, GPU rendering).

## Abstract geometry rule

The document model stores abstract/mathematical geometry only (arcs
as center+radius+angles, curves as BezPaths, fill boundaries as
FillEdge sequences with arc/line/polyline edges). Never flatten,
tessellate, or triangulate at load time or in the API layer.
Flattening to polylines/triangles happens only during rendering,
with zoom-adaptive tolerance and caching. This keeps Yrs sync lean,
enables PDF export at arbitrary tolerance, and gives pixel-perfect
quality at any zoom level.

When adding or bumping a dependency, always `cargo search <crate>` to
check the latest published version. Don't assume the version in
Cargo.toml is current.

## Generic core rule

CodeCAD is code plus CAD. **Code** brings the meaning: the use case, the
domain, the rules. **CAD** is the core: generic, exact drawing and shape
work. The core must not learn what a drawing is about.

- **No domain knowledge in Rust.** No layer names or prefixes with
  built-in behaviour, no part "kinds", no rules that serve one trade. A
  domain library is TypeScript or JavaScript that runs in the script
  sandbox and uses only the public `cad.*` API.
- **Before adding something to the core or the API, ask: would another
  domain need the same thing?** If yes, add the generic mechanism: a
  query over whole sets, an attribute the core stores but does not
  interpret, an option the script passes. If no, it belongs in a script.
- **When a rule is too slow in script, move the geometric primitive it
  needs into the core, not the rule.**

See `docs/architecture.md`, "Semantics live in code".

## Public repository

This repository is public. Everything committed, and every commit
message, is published.

- **No tool attribution in commits.** No `Co-Authored-By` trailers for
  AI assistants, no "generated with" lines, no links to chat or agent
  sessions. Commit messages describe the change and nothing else.
- **Nothing machine-specific.** No setup scripts for one person's
  environment, no private hostnames, drive letters, home paths or
  internal tool names, in files or in commit messages. Describe a
  requirement generically (the tool, its version, where the version is
  pinned) and keep environment-specific helpers outside the repo.

## Crate layout

```
Cargo.toml                    # workspace
crates/
    cadview-core/             # shared Rust library (wasm32 + native)
      src/lib.rs              # Document model, DWG loading, cad_call dispatcher,
                              #   bincode serialize/deserialize
      src/sync.rs             # Yrs CRDT sync (SyncDoc): diffing, state vectors,
                              #   update encoding. Used by both web and server.
      src/geo.rs              # geometry helpers (distance, lerp, polygon tests)
      src/text.rs             # skrifa glyph outlines -> polylines
      src/render.rs           # Camera, CameraCmd, View, SceneCache (block/text
                              #   expansion + BezPath cache), build_display_list:
                              #   every 2D drawing decision (culling, LOD, colour,
                              #   linetype, paint order) as a renderer-neutral list
      src/scene3d.rs          # 3D twin of render.rs: Scene3d (meshes, placements,
                              #   materials, lighting, edges), Camera3d (orbit,
                              #   named views, fit, matrices), RenderMesh (normals
                              #   and sharp edges at a crease angle), build_frame
                              #   (silhouettes per view), reference shade + tone curve
      src/solid/              # 3D parts (docs/3d-design.md), no sync or API yet
        types.rs              #   feature tree: PartDef, Feature, Profile2d (exact
                              #   2D segments), EdgeSel, Plane3, Placement
        kernel.rs             #   SolidKernel trait (optional capabilities default
                              #   to Unsupported: tessellate), TriMesh, EvalError,
                              #   MassProps (centroid optional), Aabb3. Core links
                              #   no kernel
        select.rs             #   EdgeSel -> exact 3D target curves (kernel-neutral)
        corpus/               #   reference parts with closed-form volume + bbox,
                              #   generic over SolidKernel (prismatic.rs: 3D0a)
    cadview-render/           # backends for the display list (no CAD logic)
      src/scene.rs            # display list -> vello::Scene (wasm32 + native)
      src/context.rs          # GpuContext: adapter, device, queue, vello::Renderer
                              #   (Msaa16); one constructor for browser and
                              #   headless, shared_local() = one per page
      src/gpu.rs              # headless render on a process-wide GpuContext
      src/cpu.rs              # CPU rasterizer (vello_cpu), the fallback without a GPU
      src/options.rs          # RenderOptions (the cad.render JSON), Backend
      src/lib.rs              # render_png / render_rgba: Auto|Gpu|Cpu, PNG encode
      examples/pngdiff.rs     # compare two PNGs (fraction of differing pixels)
      examples/render.rs      # batch DWG -> PNG with timings
    cadview-render3d/         # backends for core's Frame3d (no CAD logic, no kernel)
      src/lib.rs              # Renderer3d trait (progressive-capable), Gpu, Target
      src/raster.rs           # working-quality raster: MSAA 4x, depth, shading
                              #   that mirrors scene3d::shade, edge lines (+ .wgsl)
      src/options.rs          # RenderOptions3d: size, pixelRatio, background,
                              #   quality (working | presentation, not yet)
      src/headless.rs         # render_rgba / render_png on cadview-render's
                              #   process-wide GPU device (native)
      examples/shapes.rs      # procedural meshes -> PNGs, timings, --fallback
    cadview-web/              # wasm-bindgen target (browser renderer)
      src/main.rs             # entry points, module wiring
      src/session.rs          # SessionRegistry, DocumentSession, SESSIONS
      src/exports.rs          # wasm-bindgen exports (session_*, cad_call, yrs_*,
                              #   start_renderer, export_*)
      src/egui_render.rs      # egui fallback renderer: draws core's display list,
                              #   flattening each path to polylines or triangles
      src/vello_render.rs     # Vello renderer shell: per-canvas wgpu surface on
                              #   the page's shared GpuContext, demand-driven
                              #   rAF, pointer/wheel input; draws core's display
                              #   list through cadview-render's scene builder
    cadview-server/           # native Rust server
      src/main.rs             # entry: HTTP (axum) + WebTransport (wtransport)
      src/store.rs            # DocumentStore trait (SingleFileStore, FolderStore)
      src/registry.rs         # DocumentRegistry: lazy-loading, per-doc SyncDoc + broadcast
      src/assets.rs           # AssetProvider trait: embedded (rust-embed) or disk
      src/http.rs             # static file serving, /api/documents, /api/run, auth
      src/transport.rs        # WebTransport: stream classification, per-doc sync, RPC
      src/script.rs           # server-side JS execution (codelib): run_script,
                              #   exec_file, host.call bridge, nested exec
      src/library.rs          # cad.use: library name -> file (path-sandboxed),
                              #   ES module -> async function transform
      src/render.rs           # cad.render handler (sandbox, WT RPC and --render)
      src/sync.rs             # re-exports cadview_core::sync::SyncDoc
    solid-harness/            # test tool (publish = false), docs/3d-test-harness.md;
                              #   no shipped crate may depend on it
      src/reference/          # certified reference meaning of each feature:
                              #   sign-exact distance bounds in interval arithmetic
      src/interval.rs         # outward-rounded intervals (no dependency)
      src/layers/             # evidence layers: closed_form, enclosure, shell,
                              #   mesh_invariants, validity (the kernel's own check),
                              #   equivalence, algebra, continuity
      src/cases.rs            # catalogue: corpus parts, extra closed forms,
                              #   equivalent pairs, algebra/motion cases, sweeps,
                              #   regression cases found by the fuzzer
      src/kernels/arris.rs    # ArrisKernel (arris =0.11.1), the only file naming arris
      src/kernels/cad_kernel.rs # CadKernel (cad-kernel =0.15.0), the only file
                              #   naming cad-kernel
      src/kernels/broken.rs   # broken adapters (mutants) that test the harness
      src/fuzz.rs             # fuzzer library: seeded recipe generator, judge
                              #   without closed forms, shrinker, case printer
      src/scoreboard.rs       # committed per-kernel scoreboard + ratchet
      scoreboards/<kernel>.txt # one scoreboard per kernel (arris, cad-kernel)
      examples/fuzz.rs        # the fuzzer driver (development-time, never in CI)
      examples/compare.rs     # both kernels on the catalogue and the sweeps
      examples/timings.rs     # evaluation and tessellation times per kernel
  cad-client/                 # build output for sandbox
    cad-api-setup.js          # Vite-built IIFE (from cad-api.ts), a codelib prelude
  web/                        # frontend (TypeScript + React + Vite)
    package.json              # pnpm project (react, vite, typescript)
    vite.config.ts            # builds to ../dist/, aliases cadview-wasm
    vite.sandbox.config.ts    # Vite lib mode: emits cad-api-setup.js for sandbox
    src/
      cad-api.ts              # shared typed CAD API factory (source of truth)
      cad-api-sandbox.ts      # sandbox entry: wires __cadCall globals to buildCadApi
      cad.ts                  # browser client: imports cad-api.ts, adds sessions/WT/Yrs
      App.tsx                 # React root: tabs, viewports, layer panel, drag-drop
      TabBar.tsx              # tab strip with close buttons + "+" add
      DocumentPicker.tsx      # file list from /api/documents, folder grouping
      ViewportContainer.tsx   # per-session canvas + eframe WebRunner lifecycle
      LayerPanel.tsx          # layer visibility toggle panel
      index.css               # CodeCAD brand styles (void/amber/off-white)
  SKILL.md                    # drawing skill: techniques, patterns, creed
  dist/                       # build output (WASM + Vite bundle)
  docs/
    architecture.md           # stack, design decisions, AI drawing vision
    api-surface.md            # JS API design (generic core + domain exercises)
    _todo.md                  # working TODO
```

## Architecture

```
 Server (cadview-server)          WebTransport            Browser (cadview-web)
 ┌─────────────────────┐   per-doc sync streams    ┌───────────────────────┐
 │ DocumentStore       │◄═════════════════════════►│ SessionRegistry       │
 │  (SingleFile/Folder)│   {type:"document",id}    │  N sessions in memory │
 │                     │   + Yrs SV exchange        │  M visible (rendered) │
 │ DocumentRegistry    │                           │  1 js_target for API  │
 │  lazy load from     │   RPC bidi streams        │                       │
 │  store on first     │   (save, loadDwg,         │ Per-session:          │
 │  sync request       │    runScript, exec)       │  Document + SyncDoc   │
 │                     │                           │  Camera + RenderCache │
 │ Per-document:       │   Auth: bearer token       │  Hidden layers        │
 │  Document + SyncDoc │   (HTTP header +           │                       │
 │  broadcast channel  │    injected into HTML)     │ Per-viewport:         │
 │                     │                           │  <canvas> + WebRunner │
 │ Sandbox (codelib)   │                           │                       │
 │  JS scripts via     │   POST /api/run           │ React: TabBar, Picker │
 │  host.call: cad.*,  │◄─── (HTTP, curl, MCP) ───│  ViewportContainer    │
 │  rpc.*, readFile    │                           │                       │
 │  batch -> Yrs sync  │                           │ Shared: cad-api.ts    │
 └─────────────────────┘                           └───────────────────────┘
```

### Multi-document model

**Three-tier session state:**
- **Open** = in memory, syncing via WebTransport (all sessions)
- **Visible** = has a `<canvas>` + eframe WebRunner rendering it
- **JS target** = which session `cad.*` calls operate on (set by `cad.useSession()`)

Each session bundles: Document, SyncDoc, Camera, RenderCache, hidden
layers, screen size. Sessions are fully isolated (no shared state).

**Document sources:**
- Server-loaded DWG (via store, synced to server)
- Drag-and-drop DWG (parsed in WASM, then synced to server)
- New empty drawing (created in WASM, synced to server on create)

### Command categories

- **Document mutations** (addLine, remove, move...): execute on js_target
  session locally, Yrs sync via bincode-serialized entity blobs
- **Server-only RPC** (save, saveDwg): always server, includes doc_id
  for routing. Browser flushes pending Yrs updates before RPC.
- **Client-only** (zoomTo, fitView, setLayerVisible): target js_target
  session's camera/layer state
- **Server-side scripts** (runScript, exec): JS executed in a codelib
  sandbox with direct cad_call access. All mutations batched into
  a single Yrs update, broadcast to connected browsers.

## Running

### Build (using `just`)

Three build steps, all automated via `justfile`:

```bash
just build       # WASM + frontend + server (full build)
just rebuild     # same as build (alias)
just build-wasm  # Rust -> WASM + wasm-bindgen
just build-web   # Vite bundles TS/React + WASM into dist/
just build-server # native server binary

just build-packed      # single binary with frontend baked in
just build-packed-full # same + examples/ baked in
just clean-dist        # remove stale hashed WASM from dist/
```

**IMPORTANT**: after changing cadview-web Rust code, you must run
both `just build-wasm` AND `just build-web`. The Vite build hashes
the WASM filename, so `wasm-bindgen` alone won't update the browser.

```bash
just run-floor   # start server with the ground floor DWG
just test        # run all crate tests
just dev         # Vite HMR for frontend dev (no server)
```

### Manual build (without just)

```bash
# 1. WASM (wasm-bindgen outputs to dist/)
cargo build -p cadview-web --target wasm32-unknown-unknown --release
wasm-bindgen target/wasm32-unknown-unknown/release/cadview-web.wasm --out-dir dist --web

# 2. Frontend (Vite bundles TS/React + WASM into dist/)
cd web && pnpm install && pnpm build && cd ..
```

### Web toolchain

The web build needs Node, pnpm, `just`, the `wasm32-unknown-unknown`
target (`rustup target add wasm32-unknown-unknown`) and the
`wasm-bindgen` CLI. The CLI version must equal the `wasm-bindgen` crate
version in `Cargo.lock`, or it refuses the wasm file; CI installs the
same version in `.github/workflows/`:

```bash
cargo install wasm-bindgen-cli --version <version from Cargo.lock>
```

The native crates (core, render, server) build and test without any of
this.

### Build caches and worktrees

- **One `target/` per checkout.** Never point two checkouts (the main
  one, a git worktree, a second clone) at the same `CARGO_TARGET_DIR` or
  `build.build-dir`. Cargo names workspace crates by their path relative
  to the workspace root and decides freshness by file time, so a checkout
  whose sources are older than the last build reuses the other checkout's
  compiled crates without recompiling. Tested on cargo 1.98.1: with a
  shared build directory, a test added only in a worktree ran, and
  failed, in the main checkout. `-Z checksum-freshness` would make
  sharing safe but is nightly-only; revisit when it is stable.
- **Keep the number of caches down instead.** A cache for this workspace
  is several GB (native debug, release and wasm32 each add their own).
  Build and test in one checkout where possible, one cargo invocation at
  a time. When a worktree needs its own build, build only the packages it
  changes (`-p`), and delete its `target/` when the work is merged.
- **Check free disk before a large or parallel build**, and ask before
  putting build output anywhere other than the checkout's own `target/`.
  A container or VM usually shares its host's disk, so both run out
  together.

### Script sandbox

Nothing to build: the JS engine ships inside the `codelib` crate
(git dependency, pinned by `Cargo.lock`). The first server start on a machine
compiles it to native code (~25 s), cached on disk after that.
`cad-client/cad-api-setup.js` is committed; rebuild it with
`just build-sandbox-api` only when `web/src/cad-api.ts` changes.

### Server mode (preferred)

```bash
# Single file
RUST_LOG=info cargo run -p cadview-server -- "path/to/file.dwg"

# Folder of DWGs
RUST_LOG=info cargo run -p cadview-server -- --dir ./drawings/

# No args = scan cwd
RUST_LOG=info cargo run -p cadview-server
```

Opens http://localhost:8765. Tab bar for multiple documents.
Drag-and-drop .dwg files onto the browser to open them.

CLI flags: `--dist <path>` overrides dist asset source (load from
disk instead of embedded). `--examples <path>` overrides examples
source. These work regardless of compile-time feature flags.

### Packed binary (single-file deploy)

```bash
just build-packed      # bake dist/ into the binary
just build-packed-full # bake dist/ + examples/ into the binary
```

Uses `rust-embed` behind feature flags `embedded-dist` and
`embedded-examples`. The resulting binary needs no dist/ directory.
`--dist <path>` still works to override at runtime.

### Releases

`just package [tag]` builds the packed binary for the host platform and
writes `release/codecad-<tag>-<target>.{tar.gz,zip}` plus a `.sha256`.
The archive holds the binary named `codecad`, README.md and LICENSE.

Pushing a `v*` tag runs `.github/workflows/release.yml`, which does the
same thing for five targets (Windows x64, Linux x64/arm64, macOS
x86_64/arm64) and attaches the archives to a GitHub release:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

One `assets` job builds dist/ once (it is not in git), then each
platform job downloads it and compiles
`cadview-server --features embedded-dist`. Examples are deliberately not
embedded; they ship with the Pages site. `workflow_dispatch` runs the
build without publishing, for testing workflow changes.

### Dev mode (Vite HMR for frontend, no server)

```bash
cd web && pnpm dev
```

Opens http://localhost:5173. WASM must be built first (step 1 above).

### Exporting example screenshots and DWGs

All example PNGs are 960x640. To re-export after changing an example:

```bash
# 1. Build everything and start server with examples
just build
RUST_LOG=info cargo run -p cadview-server --release -- ./examples/

# 2. Run the script on a clean document
curl -s -X POST http://localhost:8765/api/run \
  -H "Authorization: Bearer cadview-local-dev" \
  -H "Content-Type: application/json" \
  -d '{"program": "cad.clear()"}'
curl -s -X POST http://localhost:8765/api/run \
  -H "Authorization: Bearer cadview-local-dev" \
  -H "Content-Type: application/json" \
  -d '{"exec": "examples/01-flange-plate.js"}'

# 3. Save DWG
curl -s -X POST http://localhost:8765/api/run \
  -H "Authorization: Bearer cadview-local-dev" \
  -H "Content-Type: application/json" \
  -d '{"program": "return cad.saveDwg(\"examples/01-flange-plate.dwg\")"}'

# 4. Screenshot: render natively (GPU, else CPU), no browser needed
cargo run -p cadview-server --release -- examples/01-flange-plate.dwg \
  --render examples/01-flange-plate.png --size 960x640
```

The native render draws the same display list as the browser's Vello
view; against the committed PNGs (made by `examples/lib/render.mjs`,
headless Chromium on SwiftShader) it differs on at most 0.05 % of pixels
(anti-aliasing on curves). Use `render.mjs` when you need the embed
viewer's exact output, and `pngdiff` (see "Render to PNG") to compare.

## The ABI: `cad_call`

Single function bridging JS and the Rust Document:

```
cad_call(method: string, args_json: string) -> result_json: string
```

Targets whichever session `cad.useSession(id)` selected. Every return
value includes `_session` field confirming which session was targeted.

```js
cad.useSession("floor-plan.dwg")      // set target
cad.addLine([0, 0], [5, 0])           // draw on that session
cad.addCircle([2.5, 2], 1.5)
cad.describe()                         // scene summary
cad.entities()                         // all entities as array
await cad.save("output.json")          // save to server disk (RPC)
await cad.saveDwg("output.dwg")        // save as DWG (RPC)
```

### Session management API

```js
cad.sessions.create("my-drawing")      // new session + server sync
cad.sessions.destroy("my-drawing")     // close session
cad.sessions.list()                    // [{id, entity_count, ...}]
cad.sessions.loadDwgBytes(id, bytes)   // load DWG from ArrayBuffer

cad.useSession("my-drawing")           // set cad_call target
cad.currentSession()                   // get current target ID

cad.viewport.start("canvas1", "my-drawing")  // start renderer
cad.viewport.stop("canvas1")                 // stop renderer

cad.api.listDocuments()                // GET /api/documents
```

## Server-side script execution

JS scripts run in a codelib sandbox (JS engine in wasmtime) with
access to the document through codelib's `host.call`. Default limits:
30 s timeout, 512 MiB engine memory. All mutations are batched into a
single Yrs update, broadcast to connected browsers in real time.

### Entry points

```bash
# HTTP (curl, MCP, any HTTP client)
curl -X POST http://localhost:8765/api/run \
  -H "Authorization: Bearer cadview-local-dev" \
  -H "Content-Type: application/json" \
  -d '{"program": "cad.addLine([0,0],[100,0]); return cad.describe()"}'

# Execute a .js file from disk
curl -X POST http://localhost:8765/api/run \
  -d '{"exec": "design/electrical/place-electrical.js"}'

# From browser console (WebTransport RPC)
await cad.exec("design/electrical/place-electrical.js")
await cad.runScript("cad.addCircle([0,0], 50); return cad.describe()")
```

### Script environment

Scripts get `cad` on globalThis with the same API as the browser
(from `web/src/cad-api.ts`). Additional server-side capabilities:

| API | Description |
|-----|-------------|
| `cad.*` | All mutation/query methods (addLine, entities, etc.) |
| `cad.readFile(path)` | Read file from disk (relative to script dir) |
| `cad.use(name)` | Load a script library (an ES module, `lib/<name>.js` under the script dir) once per run; resolves to its exports. Also in the browser console. See `docs/api-surface.md`, "Script libraries" |
| `cad.save(path)` | Save document as JSON |
| `cad.saveDwg(path)` | Save document as DWG |
| `cad.savePdf(path, { colors, lineweights, skipLayers })` | Save a one-page PDF. `colors`: `"paper"` (default, readable on white), `"mono"`, `"screen"` (as stored). `lineweights: true` strokes entities at their own lineweight where one is set (default: uniform hairlines). `skipLayers`: layer names to leave out, on top of the layers the document hides (default: none) |
| `cad.exec(path)` | Execute another .js file |
| `cad.render(opts)` | Render the document to a PNG (GPU, else CPU) |
| `console.log(...)` | Captured in response `stdout` field |

### Architecture

```
  cad-api.ts (TypeScript, source of truth)
       │
       ├── web/src/cad.ts (browser: WASM cad_call + WebTransport RPC)
       │
       └── vite.sandbox.config.ts -> cad-api-setup.js (IIFE)
              │
              └── codelib prelude, after the bridge prelude in script.rs
                  (__cadCall/__rpcCall/__readFile/__loadLibrary -> host.call)
                     │
                     └── codelib sandbox, host calls answered by script.rs
                            │
                            ├── cad.<method> -> cadview_core::cad_call()
                            ├── rpc.<method> -> save/saveDwg/loadDwg/exec handlers
                            ├── readFile     -> fs::read (path-sandboxed)
                            └── lib.load     -> library.rs (cad.use: resolve +
                                                ES module transform)
```

The engine cannot load ES modules (`import()` traps the instance), so
`library.rs` turns a library's `import`/`export` declarations into an
async function that the loader in `cad-api.ts` evaluates.

`cad.exec(path)` runs the file as a nested codelib run on the same
document (moved into the nested run and handed back), up to 8 deep,
within what is left of the caller's time budget.

## Sync protocol

- **Transport**: WebTransport (QUIC) via wtransport crate. Self-signed
  cert generated at startup, SHA-256 hash injected into served HTML.
- **Auth**: bearer token injected into HTML, validated on HTTP API
  and WebTransport streams.
- **CRDT**: Yrs (Rust port of Yjs). Entities stored as bincode blobs
  in a YMap keyed by entity ID. Lossless roundtrip.
- **Stream classification**: each bidi stream starts with a first
  message. JSON with `method` field = RPC. JSON with
  `{type:"document",id}` = sync header (followed by binary SV).
  Raw binary = legacy sync (falls back to default document).
- **Per-document**: each document has its own sync stream and broadcast
  channel. Multiple sync streams coexist on one WebTransport connection.
- **Handshake**: client sends sync header + SV, server replies with
  update + SV, client sends its update. Then continuous bidi.
- **RPC**: per-call bidi streams. Request JSON includes `doc_id` for
  routing to the correct server-side document.

## Document store

S3-like flat key space. Keys are paths (e.g. `sub/floor-plan.dwg`).
Path prefixes act as virtual folders for UI grouping.

Two implementations:
- **SingleFileStore**: one CLI file, one key
- **FolderStore**: recursive directory scan, keys = relative paths

`DocumentRegistry` wraps the store with lazy loading: first sync
request for a key triggers DWG load. Unknown keys (new drawings from
clients) get empty slots created automatically.

## Rendering

Dual renderer behind `vello-renderer` feature flag (default on).
JS host auto-detects WebGPU via `navigator.gpu.requestAdapter()`.
Override with `?renderer=vello` or `?renderer=egui` query param.

What gets drawn is decided once, in `cadview_core::render`:
`build_display_list(&Document, &mut SceneCache, &View)` returns a
`DisplayList` (background, one world-to-pixel transform, and ordered
items, each an exact `BezPath` with a fill or stroke paint). Backends in
`cadview-render` only turn that list into pixels: the Vello scene
builder (browser and headless GPU) and the CPU rasterizer. They never
match on `Shape`.

- **One GPU stack**: vello 0.12, wgpu 30 and eframe/egui 0.37 move
  together, so the web bundle has exactly one wgpu (check with
  `cargo tree -p cadview-web --target wasm32-unknown-unknown -d`).
  Bump them together. The wasm-bindgen CLI version pinned in the
  workflows must equal the `wasm-bindgen` version in `Cargo.lock`.
- **Vello (default, WebGPU)**: GPU compute-based 2D renderer.
  `Shape::to_bezpath()` converts all entity types to kurbo BezPaths,
  Vello renders natively on GPU. No CPU tessellation. Intermediate
  Rgba8Unorm texture + `TextureBlitter` blit to Bgra8 surface
  (Windows). Demand-driven rAF: zero frames when idle.
- **One GPU device per page**: `cadview_render::context::GpuContext`
  owns the adapter, device, queue and `vello::Renderer` (Msaa16 only).
  The browser gets it from `shared_local()`: the first canvas creates
  it, canvases that start meanwhile wait for that result, and every
  canvas keeps only its own surface, surface config, intermediate
  texture and blitter. Each device creation logs
  `[CodeCAD] GPU device created (#N)` (browser console, or `log::info!`
  natively), so a headless browser can count devices. The egui
  fallback's eframe WebRunner sets up its own wgpu, separately.
- **egui (fallback)**: draws the same display list through the
  eframe/egui Painter. egui only draws polylines and triangles, so each
  path is flattened (fills triangulated) on the CPU and cached per path
  and zoom level in `RenderCache`. Only calls `request_repaint()` on
  interaction (zero idle frames).
- **Per-session canvas**: each visible session gets its own `<canvas>`.
  React manages lifecycle via `ViewportContainer`.
- **Screen-space LOD**: entities with screen-space bbox < 0.5px are
  culled. Between 0.5-8px, smoothstep alpha fade. Both renderers.
- **Frustum culling**: entities outside the viewport are skipped.
- **Dash LOD**: a linetype pattern under 4 px per repeat on screen, or
  past a budget of 100 000 dashes per frame, draws solid (Vello drops
  the whole frame when its dash buffers overflow). Decided in core, so
  the PNG renderer follows the same rule.
- **BezPath cache** (`SceneCache`, core): paths computed once per
  entity, reused every frame. Rebuilt when the entity count changes, and
  when a shell calls `invalidate()` because entities changed in place:
  the web shell bumps a per-session `doc_rev` on every mutation, sync
  update and file load, and both renderers compare against it.

### Render to PNG

The server renders natively, with no browser: GPU (wgpu + Vello, Msaa16,
the same scene the browser draws) and a CPU fallback. `backend: "auto"`
(the default) uses the GPU and falls back to the CPU with a logged
warning; `"gpu"` or `"cpu"` force one and fail if it is unavailable.
The GPU device (a `GpuContext`, the same setup the browser uses) is
created once per process (the first render pays for shader compilation). Build without the `gpu` feature
(`--no-default-features`) for a CPU-only server.

```bash
# A DWG straight to PNG
cadview-server examples/01-flange-plate.dwg --render out.png --size 960x640

# After a script
cadview-server --exec examples/01-flange-plate.js --render out.png --render-backend gpu
```

```js
// From a script, or the browser console (flushes sync first)
cad.render({ out: "renders/plate.png", size: [960, 640] })
// -> { ok, path, width, height, backend: "gpu" | "cpu", bytes, ms }
```

Options: `out` (default `render.png`, relative to the server's working
directory), `size` (default `[960, 640]`), `bounds: {min, max}` (default:
fit the whole drawing with a 10 % margin), `hiddenLayers`, `backend`,
`pixelRatio` (default 1; scales the size and the 1 px stroke width
together, like a high-DPI screen).

Compare two PNGs with
`cargo run -p cadview-render --example pngdiff -- a.png b.png [diff.png]`
(prints the fraction of pixels differing by more than 8/255 in any
channel).

### Render 3D to PNG

`cadview-render3d` renders a `cadview_core::scene3d::Scene3d` (triangle
meshes, placements, materials, a camera) headless, on the same
process-wide GPU device as the 2D renderer. Only the working quality
exists (shaded raster with edges and outlines, ambient occlusion, an
optional ground, clip planes for cutaways); `Quality::Presentation`
returns `Render3dError::NotAvailable`. No server or script API yet.

```bash
# Procedural meshes to PNGs: every working-render feature once, 500-part
# timings and contact-sheet.png (--fallback also tries wgpu's software
# adapter)
cargo run -p cadview-render3d --release --example shapes -- out/ --fallback
# Kernel-evaluated corpus parts, whole and cut, plus corpus-sheet.png (the
# harness is the only crate with a kernel)
cargo run -p solid-harness --release --example render_corpus -- out/
```

## Tests

```bash
cargo test -p cadview-core
cargo test -p cadview-render
cargo test -p cadview-render3d
cargo test -p cadview-web
cargo test -p cadview-server
cargo test -p solid-harness            # see "Solid harness" below
```

cadview-render's and cadview-render3d's GPU tests print `SKIPPED` and
pass when no GPU adapter is found.

cadview-core: 35+ tests (entity CRUD, cad_call dispatch, geometry).
cadview-web: 12 tests (session registry isolation, create/destroy/use).
cadview-server: 8 tests (store backends, registry, broadcast isolation).

### Solid harness (3D kernels)

The kernel adapters, the reference semantics and the evidence layers
live in `solid-harness` (docs/3d-test-harness.md). Core links no kernel.

```bash
# 3D0a gate: the prismatic corpus on arris, per-part table with timings
cargo test -p solid-harness --test corpus -- --nocapture
# Every case on every layer, for every kernel with a file in scoreboards/
# (arris.txt, cad-kernel.txt), compared with that file (ratchet)
cargo test -p solid-harness --release --test scoreboard -- --nocapture
# Regenerate the scoreboards after an accepted change (filter by test name,
# e.g. cad_kernel_scoreboard, to regenerate one)
SOLID_HARNESS_BLESS=1 cargo test -p solid-harness --release --test scoreboard
# Broken adapters: every mutant must be caught; prints which layer caught it
cargo test -p solid-harness --release --test mutants -- --nocapture
# Capability evidence per kernel (revolve, sweep, loft, rounds, offsets,
# sections, tessellation), each against a closed form
cargo test -p solid-harness --release --lib forward_look -- --nocapture
```

The scoreboard and mutant runs take a few minutes in a debug build; use
`--release`. The scoreboard test fails on any wrong case the committed
file does not record as wrong (fuzzer findings against a candidate
kernel are recorded and printed), on any case or layer that got worse
than the committed file, on a closed form that falls outside the
reference's certified volume enclosure, and on a scoreboard file that no
test runs.

The fuzzer is a development-time tool and never runs in `cargo test` or
CI. It generates seeded random recipes biased toward flush and coplanar
faces, tangent and breaking-out holes, near contacts (1e-12 to 1e-3 mm),
tiny tools and far-away parts, judges each without a closed form
(enclosure, shell test, mesh invariants, validity, and the two kernels
against each other), runs each batch in a child process so a hang costs
one case, and shrinks every distinct failure to a recipe to paste into
`src/cases.rs`:

```bash
cargo run -p solid-harness --release --example fuzz -- run --seed 1 --cases 1000 --kernels arris,cad-kernel
cargo run -p solid-harness --release --example fuzz -- one --seed 1 --index 34 --kernels arris,cad-kernel
# A planted bug must be found before a clean run is trusted:
cargo run -p solid-harness --release --example fuzz -- run --seed 101 --cases 300 --kernels broken-arris:hole
```

## Key design decisions

- **f64 plan space, f32 draw space**: geometry in f64 (kurbo), cast to
  f32 per-frame for GPU.
- **EntityId on every entity**: stable u64 IDs for mutations and sync.
- **Bincode serialization**: DrawEntity + Shape derive Serialize/
  Deserialize (kurbo serde feature). Stored as byte blobs in Yrs YMap.
- **Coordinates in DWG units (mm)**.
- **Layers auto-created**: `ensure_layer()` on every add operation.
- **`entities({expand: true})`**: flattens block inserts into
  world-coordinate sub-entities. Essential for querying geometry inside
  blocks (C_WINDOWS, furniture, etc.). Without expand, blocks appear
  as single points with no child lines.
- **cad_call is the universal ABI**: same method+JSON interface works
  via wasm-bindgen (browser), server-side Rust, and future WASM sandbox.
- **Active-pointer ABI**: `cad_call` targets `js_target` session.
  Console ergonomics preserved (no session_id on every call).
- **cadview-core has zero global state**: `cad_call` is pure
  `fn(&mut Document, method, args)`. All statefulness is in the shells.
- **Per-session everything**: camera, render cache, hidden layers,
  sync stream, broadcast channel. No cross-session leakage.
- **codelib sandbox for server-side scripts**: wasmtime isolation
  (memory limits, timeouts, crash containment) with the JS engine
  embedded, no guest build. One `host.call` channel for
  cad/rpc/readFile. Forward-compatible with multi-tenant cloud
  deployment.
- **Shared cad API (TypeScript)**: `web/src/cad-api.ts` is the single
  source of truth. Browser imports directly, sandbox gets a Vite-built
  IIFE. No method list duplication.
- **ByLayer style inheritance**: color, linetype, lineweight are
  `Option` on DrawEntity (`None` = inherit from layer). Transparency
  is entity-only (u8, 0=opaque). Named linetypes stored in
  `Document.linetypes` table. Standard patterns (Dashed, Center,
  Hidden, Phantom, Dot) pre-populated. Resolution happens at render
  time, not creation time.
  DWG AC1015 stores colors as ACI (AutoCAD Color Index), so RGB
  values are lossy through save/reload. Use ACI palette values
  in examples for lossless roundtrip.

## Required reading

Before drawing anything, read these in order:

1. **`SKILL.md`** - core creed, required workflow, fillet offset
   rules, involute gear patterns, polar copy technique.
2. **`docs/api-surface.md`** - JS API design, block system, mutation
   on selections, layer semantics, worked exercises.
3. **`docs/how-humans-do-cad.md`** - drafting techniques, construction
   geometry, snap modes, expert mental models.
