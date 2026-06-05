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
  and sync to cadview-core.
- **cadview-sandbox**: Wasmtime WASM component sandbox for running
  JS scripts server-side. Custom WIT interface (cad-call, rpc-call,
  read-file) gives scripts direct access to the document model.
  Epoch-based timeouts, memory isolation, stdout/stderr capture.

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
    cadview-web/              # wasm-bindgen target (browser renderer)
      src/main.rs             # SessionRegistry, egui renderer, wasm-bindgen
                              #   exports (session_*, cad_call, yrs_*, start_renderer)
      src/vello_render.rs     # Vello GPU renderer: wgpu surface, Scene builder,
                              #   demand-driven rAF, pointer/wheel input, BezPath cache
    cadview-server/           # native Rust server
      src/main.rs             # entry: HTTP (axum) + WebTransport (wtransport)
      src/store.rs            # DocumentStore trait (SingleFileStore, FolderStore)
      src/registry.rs         # DocumentRegistry: lazy-loading, per-doc SyncDoc + broadcast
      src/assets.rs           # AssetProvider trait: embedded (rust-embed) or disk
      src/http.rs             # static file serving, /api/documents, /api/run, auth
      src/transport.rs        # WebTransport: stream classification, per-doc sync, RPC
      src/script.rs           # server-side JS execution: run_script, exec_file
      src/sync.rs             # re-exports cadview_core::sync::SyncDoc
    cadview-sandbox/          # Wasmtime WASM component sandbox
      src/lib.rs              # Sandbox struct, component caching, epoch timeouts
      src/host.rs             # WasiView + custom cad::Host trait (closures)
      src/error.rs            # SandboxError
      cadview-sandbox.wasm    # pre-built StarlingMonkey JS component (~12MB)
  wit/                        # WIT interface definitions
    cadview-runtime.wit       # custom world: cad-call, rpc-call, read-file imports
    deps/                     # WASI wit deps (cli, io, clocks, http, etc.)
  sandbox-component/          # JS component build (jco componentize)
    runtime-wrapper.js        # WIT imports -> globalThis bridge, exports run()
    package.json              # pnpm: @bytecodealliance/jco + componentize-js
    build.sh                  # Vite lib build + jco componentize pipeline
  cad-client/                 # build output for sandbox
    cad-api-setup.js          # Vite-built IIFE (from cad-api.ts), embedded by Rust
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
 │ Sandbox (Wasmtime)  │                           │                       │
 │  JS scripts via     │   POST /api/run           │ React: TabBar, Picker │
 │  cad-call WIT import│◄─── (HTTP, curl, MCP) ───│  ViewportContainer    │
 │  read-file, rpc-call│                           │                       │
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
- **Server-side scripts** (runScript, exec): JS executed in Wasmtime
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

### Sandbox component (only when WIT or runtime-wrapper.js changes)

```bash
cd sandbox-component && bash build.sh
```

Runs Vite lib build (TS -> JS for cad API setup) then jco componentize
(JS -> WASM component). Output: `crates/cadview-sandbox/cadview-sandbox.wasm`
(~12MB, committed). First Wasmtime compile takes ~2min, cached after that.

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

### Dev mode (Vite HMR for frontend, no server)

```bash
cd web && pnpm dev
```

Opens http://localhost:5173. WASM must be built first (step 1 above).

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

JS scripts run in a Wasmtime WASM sandbox with direct access to the
document via custom WIT imports (cad-call, rpc-call, read-file).
All mutations are batched into a single Yrs update, broadcast to
connected browsers in real time.

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
| `cad.save(path)` | Save document as JSON |
| `cad.saveDwg(path)` | Save document as DWG |
| `cad.exec(path)` | Execute another .js file |
| `console.log(...)` | Captured in response `stdout` field |

### Architecture

```
  cad-api.ts (TypeScript, source of truth)
       │
       ├── web/src/cad.ts (browser: WASM cad_call + WebTransport RPC)
       │
       └── vite.sandbox.config.ts -> cad-api-setup.js (IIFE)
              │
              └── Rust host prepends to user script
                     │
                     └── runtime-wrapper.js (WIT imports -> globalThis bridge)
                            │
                            └── Wasmtime sandbox (cadview-sandbox crate)
                                   │
                                   ├── cad-call -> cadview_core::cad_call()
                                   ├── rpc-call -> save/saveDwg/exec handlers
                                   └── read-file -> fs::read (path-sandboxed)
```

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

- **Vello (default, WebGPU)**: GPU compute-based 2D renderer.
  `Shape::to_bezpath()` converts all entity types to kurbo BezPaths,
  Vello renders natively on GPU. No CPU tessellation. Intermediate
  Rgba8Unorm texture + `TextureBlitter` blit to Bgra8 surface
  (Windows). Demand-driven rAF: zero frames when idle.
- **egui (fallback)**: CPU tessellation via eframe/egui Painter.
  `RenderCache` caches arcs, curves, fills per entity per zoom level.
  Only calls `request_repaint()` on interaction (zero idle frames).
- **Per-session canvas**: each visible session gets its own `<canvas>`.
  React manages lifecycle via `ViewportContainer`.
- **Screen-space LOD**: entities with screen-space bbox < 0.5px are
  culled. Between 0.5-8px, smoothstep alpha fade. Both renderers.
- **Frustum culling**: entities outside the viewport are skipped.
- **BezPath cache** (Vello): paths computed once per entity, reused
  every frame. Invalidated on entity count change.

## Tests

```bash
cargo test -p cadview-core
cargo test -p cadview-web
cargo test -p cadview-server
```

cadview-core: 35+ tests (entity CRUD, cad_call dispatch, geometry).
cadview-web: 12 tests (session registry isolation, create/destroy/use).
cadview-server: 8 tests (store backends, registry, broadcast isolation).

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
- **Wasmtime sandbox for server-side scripts**: real process isolation
  (memory limits, epoch timeouts, crash containment). Custom WIT
  imports for cad-call/rpc-call/read-file. Forward-compatible with
  multi-tenant cloud deployment.
- **Shared cad API (TypeScript)**: `web/src/cad-api.ts` is the single
  source of truth. Browser imports directly, sandbox gets a Vite-built
  IIFE. No method list duplication.

## Required reading

Before drawing anything, read these in order:

1. **`SKILL.md`** - core creed, required workflow, fillet offset
   rules, involute gear patterns, polar copy technique.
2. **`docs/api-surface.md`** - JS API design, block system, mutation
   on selections, layer semantics, worked exercises.
3. **`docs/how-humans-do-cad.md`** - drafting techniques, construction
   geometry, snap modes, expert mental models.
