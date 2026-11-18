// CAD client: typed wrapper around the cad_call wasm-bindgen export.
//
// Multi-session aware. Each open document is a session.
// cad.useSession(id) sets which session cad.* calls target.
// cad.sessions.* manages session lifecycle.
// cad.viewport.* manages per-session renderers (canvas elements).

import init, {
  cad_call,
  export_dwg_bytes,
  export_pdf_bytes,
  gpu_devices_created,
  kernel_expect,
  kernel_status,
  session_create,
  session_current,
  session_destroy,
  session_list,
  session_load_bake,
  session_load_dwg,
  session_use,
  start_renderer,
  start_view3d,
  stop_renderer,
  view3d_command,
  view3d_stats,
  yrs_apply_update,
  yrs_encode_update,
  yrs_pending_update,
  yrs_state_vector,
} from "cadview-wasm";
import type {
  CadApi,
  EntityJson,
  LayerInfo,
  RenderOptions,
  RenderResult,
  SavePdfOptions,
} from "./cad-api";
import { buildCadApi } from "./cad-api";
import { KERNEL_HINT, type KernelInfo, loadKernel } from "./kernel";

export type { CadApi, EntityJson, LayerInfo };

// ── Types ────────────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  entity_count: number;
  /** Solid parts in the document. */
  part_count: number;
  /** A prebaked 3D scene is loaded. */
  baked: boolean;
  js_target: boolean;
  visible: boolean;
}

/** Options of a 3D view: the 3D `cad.render` options plus its own. */
export interface View3dOptions extends RenderOptions {
  quality?: "working" | "presentation";
}

/** A change to a running 3D view. */
export interface View3dCommand {
  quality?: "working" | "presentation";
  view?: RenderOptions["view"];
  projection?: "orthographic" | "perspective";
  /** Re-fit the camera to the parts. */
  fit?: boolean;
  /** Turn the camera: [yaw, pitch] in radians. */
  orbit?: [number, number];
  /** Presentation: fixed sample target (null: adapt to the GPU). */
  spp?: number | null;
  /** Presentation: 1 full resolution, 2 half. */
  traceScale?: 1 | 2;
  /** Presentation: the denoiser (default on, as in `cad.render`). */
  denoise?: boolean;
  /** Render options to change (colours, materials, light, ...). */
  options?: RenderOptions;
}

/** What `cad.view3d.stats(key)` reports. */
export interface View3dStats {
  key: string;
  session: string;
  quality: "working" | "presentation";
  /** Frames drawn (raster frames and trace batches). */
  frames: number;
  rasterFrames: number;
  traceBatches: number;
  /** Animation-frame callbacks run. */
  ticks: number;
  firstFrameMs: number | null;
  rasterCpuMs: number;
  rasterGpuMs: number;
  /** Presentation: samples accumulated, and the target. */
  samples: number;
  target: number;
  /** Time from the start of the accumulation to the target (ms). */
  traceDoneMs: number | null;
  /** GPU time of one sample at the current resolution (ms). */
  sampleMs: number;
  traceScale: number;
  denoise: boolean;
  batch: number;
  resets: number;
  sceneBuilds: number;
  parts: number;
  triangles: number;
  width: number;
  height: number;
  adapter: string;
  camera: {
    yaw: number;
    pitch: number;
    distance: number;
    target: [number, number, number];
    projection: string;
  } | null;
  error: string | null;
}

export interface DocumentInfo {
  id: string;
  filename: string;
  prefix: string;
  loaded: boolean;
  entity_count?: number;
  layer_count?: number;
}

// ── Per-session sync state ──────────────────────────────────────────

interface SessionSync {
  writer: WritableStreamDefaultWriter<Uint8Array>;
  reader: FrameReader;
  /** Set to true when the sync stream is being torn down. */
  closing: boolean;
}

/** Per-session sync streams. Key = session ID. */
const _syncStreams = new Map<string, SessionSync>();

/** Callback for renaming tabs (set by App.tsx). */
let _onTabRenamed: ((sessionId: string, newLabel: string) => void) | null =
  null;

/** Register a callback for tab rename events. */
export function onTabRenamed(
  cb: (sessionId: string, newLabel: string) => void,
): void {
  _onTabRenamed = cb;
}

// ── Internal helpers ─────────────────────────────────────────────────

let _syncScheduled = false;
function schedulePendingSync(): void {
  if (_syncScheduled) return;
  _syncScheduled = true;
  queueMicrotask(flushPendingSync);
}

function flushPendingSync(): void {
  _syncScheduled = false;
  const sid = session_current();
  if (!sid || sid === "null") return;
  const sessionId = JSON.parse(sid) as string;
  if (!sessionId) return;

  const sync = _syncStreams.get(sessionId);
  if (!sync || sync.closing) return;

  const update = yrs_pending_update(sessionId);
  if (update.byteLength > 0) {
    writeMessage(sync.writer, update).catch((e) =>
      console.warn(`[CodeCAD] Failed to send Yrs update for ${sessionId}:`, e),
    );
  }
}

async function flushAndWaitSync(): Promise<void> {
  _syncScheduled = false;
  const sid = session_current();
  if (!sid || sid === "null") return;
  const sessionId = JSON.parse(sid) as string;
  if (!sessionId) return;

  const sync = _syncStreams.get(sessionId);
  if (!sync || sync.closing) return;

  const update = yrs_pending_update(sessionId);
  if (update.byteLength > 0) {
    await writeMessage(sync.writer, update);
    await new Promise((r) => setTimeout(r, 50));
  }
}

// ── RPC over WebTransport ────────────────────────────────────────────

let _transport: WebTransport | null = null;

async function rpcCall(
  method: string,
  args: Record<string, unknown> = {},
  docId?: string,
): Promise<unknown> {
  if (!_transport) throw new Error("No WebTransport connection");

  const bidi = await _transport.createBidirectionalStream();
  const writer = bidi.writable.getWriter();
  const reader = new FrameReader(bidi.readable.getReader());

  const msg = JSON.stringify({
    method,
    args,
    ...(docId ? { doc_id: docId } : {}),
  });
  await writeMessage(writer, new TextEncoder().encode(msg));

  const response = await reader.readMessage();
  const result = JSON.parse(new TextDecoder().decode(response));

  writer.close().catch(() => {});
  return result;
}

// ── Session management ──────────────────────────────────────────────

const sessions = {
  create(id: string): void {
    const result = JSON.parse(session_create(id));
    if (result.error) throw new Error(result.error);
    if (_transport) {
      runYrsSyncForSession(_transport, id).catch((e: Error) => {
        console.warn(`[CodeCAD] Sync failed for '${id}':`, e);
      });
    }
  },

  destroy(id: string): void {
    const sync = _syncStreams.get(id);
    if (sync) {
      sync.closing = true;
      sync.writer.close().catch(() => {});
      _syncStreams.delete(id);
    }
    const result = JSON.parse(session_destroy(id));
    if (result.error) throw new Error(result.error);
  },

  list(): SessionInfo[] {
    return JSON.parse(session_list()) as SessionInfo[];
  },

  loadDwgBytes(
    id: string,
    data: Uint8Array,
  ): { ok: boolean; entities: number } {
    const result = JSON.parse(session_load_dwg(id, data));
    if (result.error) throw new Error(result.error);
    return result;
  },

  /**
   * Load a prebaked 3D scene (`cad.bake3d`'s file). `adopt`: the session's
   * parts become the bake's (a viewer); otherwise the bake only supplies
   * geometry for the parts the session already has.
   */
  loadBake(
    id: string,
    data: Uint8Array,
    adopt: boolean,
  ): { ok: boolean; parts: number; solids: number; triangles: number } {
    const result = JSON.parse(session_load_bake(id, data, adopt));
    if (result.error) throw new Error(result.error);
    return result;
  },
};

// ── 3D view ─────────────────────────────────────────────────────────

/** Is there a WebGPU adapter? The 3D view needs one (no fallback). */
export async function hasWebGpu(): Promise<boolean> {
  if (typeof navigator === "undefined" || !("gpu" in navigator)) return false;
  try {
    const adapter = await (navigator as any).gpu.requestAdapter();
    return !!adapter;
  } catch {
    return false;
  }
}

/** A message for pages without WebGPU. */
export const NO_WEBGPU_3D =
  "The 3D view needs WebGPU, which this browser does not offer (try a current Chrome or Edge). The 2D view still works.";

const view3d = {
  /** Show a session's parts on `canvas`. Returns a key for the calls below. */
  start(
    canvas: HTMLCanvasElement,
    sessionId: string,
    opts: View3dOptions = {},
  ): string {
    const r = JSON.parse(start_view3d(canvas, sessionId, JSON.stringify(opts)));
    if (r.error) throw new Error(r.error);
    return r.key;
  },
  stop(key: string): void {
    stop_renderer(key);
  },
  command(key: string, cmd: View3dCommand): void {
    const r = JSON.parse(view3d_command(key, JSON.stringify(cmd)));
    if (r.error) throw new Error(r.error);
  },
  stats(key: string): View3dStats {
    const r = JSON.parse(view3d_stats(key));
    if (r.error && !r.key) throw new Error(r.error);
    return r as View3dStats;
  },
  /** GPU devices this page created (one is expected). */
  devices(): number {
    return gpu_devices_created();
  },
};

// ── Renderer selection ─────────────────────────────────────────────────

let _rendererType: "vello" | "egui" = "egui";

async function detectRenderer(): Promise<"vello" | "egui"> {
  if (typeof navigator === "undefined" || !("gpu" in navigator)) return "egui";
  try {
    const adapter = await (navigator as any).gpu.requestAdapter();
    return adapter ? "vello" : "egui";
  } catch {
    return "egui";
  }
}

const viewport = {
  /** Attach a renderer to a canvas for the given session. Returns an opaque key for stop(). */
  start(canvas: HTMLCanvasElement, sessionId: string): string {
    const result = JSON.parse(start_renderer(canvas, sessionId, _rendererType));
    if (result.error) throw new Error(result.error);
    return result.key;
  },

  stop(rendererKey: string): void {
    const result = JSON.parse(stop_renderer(rendererKey));
    if (result.error) throw new Error(result.error);
  },

  get rendererType(): "vello" | "egui" {
    return _rendererType;
  },

  /** Switch renderer for future viewport.start() calls. Existing viewports are unaffected. */
  setRendererType(type_: "vello" | "egui"): void {
    _rendererType = type_;
    console.log(`[CodeCAD] Renderer set to: ${type_}`);
  },
};

// ── Helpers ─────────────────────────────────────────────────────────

function downloadBlob(
  content: string | ArrayBuffer | Uint8Array,
  filename: string,
  mime: string,
) {
  const blob = new Blob([content as BlobPart], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

// ── Build CAD API from shared definition ────────────────────────────

const cadCore = buildCadApi({
  call: (method, argsJson) => {
    const raw = cad_call(method, argsJson);
    // A call that needed geometry before the kernel was loaded: start
    // fetching it now (once), so the call works when tried again.
    if (raw.includes("solid kernel is not loaded"))
      loadKernel().catch(() => {});
    schedulePendingSync();
    return raw;
  },
  rpc: (method, argsJson) => {
    const sid = cad.currentSession();
    return rpcCall(method, JSON.parse(argsJson), sid ?? undefined).then((r) =>
      JSON.stringify(r),
    );
  },
  readFile: null,
  // cad.use: the server reads the library (from its working directory,
  // like inline scripts); cad-api.ts runs it here.
  library: (name) =>
    rpcCall("loadLibrary", { name }).then((r) => JSON.stringify(r)),
});

// ── CAD API object (shared core + browser-specific) ─────────────────

export const cad = {
  ...cadCore,

  // Browser-only: session management
  sessions,
  viewport,
  view3d,

  /**
   * Load the solid kernel (a separate WebAssembly module, fetched once).
   * Until then, calls that need geometry fail with a message saying so.
   */
  loadKernel: (): Promise<KernelInfo> => loadKernel(),
  kernel: {
    load: (): Promise<KernelInfo> => loadKernel(),
    status: (): {
      state: "none" | "not-loaded" | "loaded";
      kernel: string;
      evaluations: number;
    } => JSON.parse(kernel_status()),
  },

  useSession(id: string): void {
    const result = JSON.parse(session_use(id));
    if (result.error) throw new Error(result.error);
  },

  currentSession(): string | null {
    const raw = session_current();
    return JSON.parse(raw) as string | null;
  },

  // Override save/saveDwg to flush sync + rename tabs.
  // Falls back to browser download when no server is connected.
  save: async (path = "cadview-output.json") => {
    if (_serverAvailable) {
      await flushAndWaitSync();
      const sid = cad.currentSession();
      const result = await rpcCall("save", { path }, sid ?? undefined);
      if (sid && _onTabRenamed) {
        const name =
          path
            .replace(/\.[^.]+$/, "")
            .split("/")
            .pop() || path;
        _onTabRenamed(sid, name);
      }
      return result;
    }
    // Standalone: download JSON via browser
    const entities = cad.entities();
    const json = JSON.stringify(entities, null, 2);
    downloadBlob(json, path, "application/json");
    return { ok: true, path, entities: entities.length };
  },
  saveDwg: async (path = "output.dwg") => {
    if (_serverAvailable) {
      await flushAndWaitSync();
      const sid = cad.currentSession();
      const result = await rpcCall("saveDwg", { path }, sid ?? undefined);
      if (sid && _onTabRenamed) {
        const name =
          path
            .replace(/\.[^.]+$/, "")
            .split("/")
            .pop() || path;
        _onTabRenamed(sid, name);
      }
      return result;
    }
    // Standalone: export DWG bytes via WASM and trigger browser download
    const sid = cad.currentSession();
    if (!sid) return { ok: false, error: "no session" };
    const bytes = export_dwg_bytes(sid);
    downloadBlob(bytes, path, "application/octet-stream");
    return { ok: true, path };
  },
  // The server writes its copy of the document, so pending edits go first.
  saveJson: async (path = "document.json") => {
    await flushAndWaitSync();
    return rpcCall("saveJson", { path }, cad.currentSession() ?? undefined);
  },
  savePdf: async (path = "output.pdf", opts: SavePdfOptions = {}) => {
    if (_serverAvailable) {
      await flushAndWaitSync();
      const sid = cad.currentSession();
      return rpcCall("savePdf", { ...opts, path }, sid ?? undefined);
    }
    const sid = cad.currentSession();
    if (!sid) return { ok: false, error: "no session" };
    const bytes = export_pdf_bytes(sid, JSON.stringify(opts));
    downloadBlob(bytes, path, "application/pdf");
    return { ok: true, path };
  },
  // Server-side PNG render. Flush pending sync first so the server sees
  // the latest edits.
  render: async (opts: RenderOptions = {}): Promise<RenderResult> => {
    if (!_serverAvailable) {
      throw new Error("cad.render needs the CodeCAD server");
    }
    await flushAndWaitSync();
    const sid = cad.currentSession();
    const result = (await rpcCall(
      "render",
      { out: "render.png", ...opts },
      sid ?? undefined,
    )) as RenderResult & { error?: string };
    if (result.error) throw new Error(result.error);
    return result;
  },

  // HTTP API
  api: {
    async listDocuments(): Promise<DocumentInfo[]> {
      const token = window.__CADVIEW_TOKEN;
      const headers: Record<string, string> = {};
      if (token) headers.Authorization = `Bearer ${token}`;
      const resp = await fetch("/api/documents", { headers });
      if (!resp.ok) throw new Error(`GET /api/documents: ${resp.status}`);
      return resp.json();
    },
    /**
     * The server's prebaked 3D scene of a document (its kernel evaluates
     * the parts): the interim geometry source of the app's 3D view while
     * the page has no kernel of its own. Pending edits are flushed first.
     */
    async fetchBake(
      docId: string,
      opts: RenderOptions = {},
    ): Promise<Uint8Array> {
      await flushAndWaitSync();
      const token = window.__CADVIEW_TOKEN;
      const headers: Record<string, string> = {};
      if (token) headers.Authorization = `Bearer ${token}`;
      const q = new URLSearchParams({ doc: docId });
      if (Object.keys(opts).length) q.set("options", JSON.stringify(opts));
      const resp = await fetch(`/api/bake3d?${q}`, { headers });
      if (!resp.ok) {
        throw new Error(`GET /api/bake3d: ${resp.status} ${await resp.text()}`);
      }
      return new Uint8Array(await resp.arrayBuffer());
    },
  },
};

// ── WebTransport with auto-reconnect ─────────────────────────────────

let _reconnectDelay = 1000;
const MAX_RECONNECT_DELAY = 30_000;
let _reconnectTimer: ReturnType<typeof setTimeout> | null = null;

declare global {
  interface Window {
    __CADVIEW_WT_PORT?: number;
    __CADVIEW_CERT_HASH?: Uint8Array;
    __CADVIEW_TOKEN?: string;
  }
}

async function connectWebTransport(): Promise<WebTransport | null> {
  const port = window.__CADVIEW_WT_PORT;
  const certHash = window.__CADVIEW_CERT_HASH;
  if (!port || !certHash) return null;

  const url = `https://localhost:${port}`;
  console.log(`[CodeCAD] Connecting WebTransport to ${url}...`);

  try {
    const transport = new WebTransport(url, {
      serverCertificateHashes: [
        { algorithm: "sha-256", value: certHash.buffer as ArrayBuffer },
      ],
    });
    await transport.ready;
    console.log("[CodeCAD] WebTransport connected");
    _transport = transport;
    _reconnectDelay = 1000;

    transport.closed
      .then((info) => {
        console.log("[CodeCAD] WebTransport closed:", info?.reason || "");
        scheduleReconnect();
      })
      .catch((e: Error) => {
        console.warn("[CodeCAD] WebTransport closed with error:", e.message);
        scheduleReconnect();
      });

    return transport;
  } catch (e) {
    console.warn(`[CodeCAD] WebTransport failed: ${(e as Error).message}`);
    return null;
  }
}

function scheduleReconnect(): void {
  _transport = null;
  for (const [, sync] of _syncStreams) {
    sync.closing = true;
  }
  _syncStreams.clear();
  if (_reconnectTimer) return;
  console.log(`[CodeCAD] Reconnecting in ${_reconnectDelay}ms...`);
  _reconnectTimer = setTimeout(async () => {
    _reconnectTimer = null;
    const transport = await connectWebTransport();
    if (transport) {
      runYrsSyncForSession(transport, _initialSessionId).catch((e: Error) => {
        console.error("[CodeCAD] Re-sync failed:", e);
        scheduleReconnect();
      });
    } else {
      _reconnectDelay = Math.min(_reconnectDelay * 2, MAX_RECONNECT_DELAY);
      scheduleReconnect();
    }
  }, _reconnectDelay);
}

// ── Length-prefixed message framing ──────────────────────────────────

async function writeMessage(
  writer: WritableStreamDefaultWriter,
  data: Uint8Array,
): Promise<void> {
  const frame = new Uint8Array(4 + data.byteLength);
  new DataView(frame.buffer).setUint32(0, data.byteLength, false);
  frame.set(data, 4);
  await writer.write(frame);
}

class FrameReader {
  private reader: ReadableStreamDefaultReader<Uint8Array>;
  private buffer = new Uint8Array(0);

  constructor(reader: ReadableStreamDefaultReader<Uint8Array>) {
    this.reader = reader;
  }

  private async fill(needed: number): Promise<void> {
    while (this.buffer.byteLength < needed) {
      const { value, done } = await this.reader.read();
      if (done) throw new Error("stream closed");
      const chunk = new Uint8Array(
        (value as Uint8Array).buffer ?? (value as Uint8Array),
      );
      const combined = new Uint8Array(
        this.buffer.byteLength + chunk.byteLength,
      );
      combined.set(this.buffer);
      combined.set(chunk, this.buffer.byteLength);
      this.buffer = combined;
    }
  }

  async readMessage(): Promise<Uint8Array> {
    await this.fill(4);
    const len = new DataView(
      this.buffer.buffer,
      this.buffer.byteOffset,
    ).getUint32(0, false);
    await this.fill(4 + len);
    const msg = this.buffer.slice(4, 4 + len);
    this.buffer = this.buffer.slice(4 + len);
    return msg;
  }
}

// ── Yrs sync over WebTransport ──────────────────────────────────────

async function runYrsSyncForSession(
  transport: WebTransport,
  sessionId: string,
): Promise<void> {
  const bidi = await transport.createBidirectionalStream();
  const writer = bidi.writable.getWriter();
  const frameReader = new FrameReader(bidi.readable.getReader());

  const header = JSON.stringify({ type: "document", id: sessionId });
  await writeMessage(writer, new TextEncoder().encode(header));

  const sv = yrs_state_vector(sessionId);
  await writeMessage(writer, sv);

  const serverUpdate = await frameReader.readMessage();
  yrs_apply_update(sessionId, serverUpdate);

  const serverSv = await frameReader.readMessage();

  const ourUpdate = yrs_encode_update(sessionId, serverSv);
  await writeMessage(writer, ourUpdate);

  console.log(`[CodeCAD] Yrs sync complete for session '${sessionId}'`);

  const syncState: SessionSync = {
    writer,
    reader: frameReader,
    closing: false,
  };
  _syncStreams.set(sessionId, syncState);

  listenForServerUpdates(sessionId, frameReader);
}

async function listenForServerUpdates(
  sessionId: string,
  frameReader: FrameReader,
): Promise<void> {
  for (;;) {
    try {
      const update = await frameReader.readMessage();
      if (update.byteLength > 0) yrs_apply_update(sessionId, update);
    } catch {
      _syncStreams.delete(sessionId);
      break;
    }
  }
}

// ── Init ─────────────────────────────────────────────────────────────

let _initialSessionId = "default";
let _serverAvailable = false;

export function getInitialSessionId(): string {
  return _initialSessionId;
}

export function isServerAvailable(): boolean {
  return _serverAvailable;
}

/** Intercept fetch to track WASM download progress. Returns cleanup fn. */
function trackWasmProgress(onProgress: (pct: number) => void): () => void {
  const original = window.fetch;
  const tracked: typeof fetch = async (input, init) => {
    const response = await original(input, init);
    const url = input instanceof Request ? input.url : input.toString();
    if (!url.endsWith(".wasm")) return response;

    const total = Number.parseInt(
      response.headers.get("content-length") || "0",
      10,
    );
    if (!total || !response.body) return response;

    const reader = response.body.getReader();
    let received = 0;
    const stream = new ReadableStream({
      async pull(controller) {
        const { done, value } = await reader.read();
        if (done) {
          controller.close();
          return;
        }
        received += value.length;
        onProgress(Math.round((received / total) * 100));
        controller.enqueue(value);
      },
    });
    return new Response(stream, {
      headers: response.headers,
      status: response.status,
      statusText: response.statusText,
    });
  };
  window.fetch = tracked;
  return () => {
    window.fetch = original;
  };
}

/** Set the loader percentage text (works for both app and embed). */
let _onLoadProgress: ((pct: number) => void) | null = null;

export function setLoadProgressCallback(cb: (pct: number) => void): void {
  _onLoadProgress = cb;
}

/** Lightweight init for embed viewers: WASM + renderer detection only.
 *  No server detection, no session creation, no WebTransport. */
export async function initCadEmbed(): Promise<void> {
  const cleanup = trackWasmProgress((pct) => _onLoadProgress?.(pct));
  await init();
  kernel_expect(KERNEL_HINT);
  cleanup();
  _rendererType = await detectRenderer();
}

export async function initCad(): Promise<void> {
  const cleanup = trackWasmProgress((pct) => _onLoadProgress?.(pct));
  await init();
  kernel_expect(KERNEL_HINT);
  cleanup();

  const params = new URLSearchParams(window.location.search);
  const forced = params.get("renderer");
  if (forced === "vello" || forced === "egui") {
    _rendererType = forced;
  } else {
    _rendererType = await detectRenderer();
  }
  console.log(
    `[CodeCAD] Renderer: ${_rendererType}${forced ? " (forced)" : ""}`,
  );

  (globalThis as Record<string, unknown>).cad = cad;
  (globalThis as Record<string, unknown>).cad_call = cad_call;

  let initialDocId = "default";
  try {
    const docs = await cad.api.listDocuments();
    _serverAvailable = true;
    if (docs.length > 0) {
      const loaded = docs.find((d) => d.loaded) ?? docs[0];
      initialDocId = loaded.id;
    }
  } catch {
    _serverAvailable = false;
  }

  _initialSessionId = initialDocId;

  try {
    cad.sessions.create(initialDocId);
  } catch {
    // Already exists
  }
  cad.useSession(initialDocId);

  const transport = await connectWebTransport();
  if (transport) {
    console.log(`[CodeCAD] Server mode: syncing '${initialDocId}'`);
    runYrsSyncForSession(transport, initialDocId).catch((e: Error) => {
      console.error("[CodeCAD] Initial sync failed:", e);
      scheduleReconnect();
    });
  } else if (window.__CADVIEW_WT_PORT) {
    scheduleReconnect();
  } else {
    console.log("[CodeCAD] Standalone mode: local WASM only");
  }
}
