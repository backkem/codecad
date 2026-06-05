// CAD client: typed wrapper around the cad_call wasm-bindgen export.
//
// Multi-session aware. Each open document is a session.
// cad.useSession(id) sets which session cad.* calls target.
// cad.sessions.* manages session lifecycle.
// cad.viewport.* manages per-session renderers (canvas elements).

import init, {
  cad_call,
  session_create,
  session_current,
  session_destroy,
  session_list,
  session_load_dwg,
  session_use,
  start_renderer,
  stop_renderer,
  yrs_apply_update,
  yrs_encode_update,
  yrs_pending_update,
  yrs_state_vector,
} from "cadview-wasm";
import type { CadApi, EntityJson, LayerInfo } from "./cad-api";
import { buildCadApi } from "./cad-api";

export type { CadApi, EntityJson, LayerInfo };

// ── Types ────────────────────────────────────────────────────────────

export interface SessionInfo {
  id: string;
  entity_count: number;
  js_target: boolean;
  visible: boolean;
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
    const result = JSON.parse(
      start_renderer(canvas, sessionId, _rendererType),
    );
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

// ── Build CAD API from shared definition ────────────────────────────

const cadCore = buildCadApi({
  call: (method, argsJson) => {
    const raw = cad_call(method, argsJson);
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
});

// ── CAD API object (shared core + browser-specific) ─────────────────

export const cad = {
  ...cadCore,

  // Browser-only: session management
  sessions,
  viewport,

  useSession(id: string): void {
    const result = JSON.parse(session_use(id));
    if (result.error) throw new Error(result.error);
  },

  currentSession(): string | null {
    const raw = session_current();
    return JSON.parse(raw) as string | null;
  },

  // Override save/saveDwg to flush sync + rename tabs
  save: async (path = "cadview-output.json") => {
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
  },
  saveDwg: async (path = "output.dwg") => {
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

export function getInitialSessionId(): string {
  return _initialSessionId;
}

/** Lightweight init for embed viewers: WASM + renderer detection only.
 *  No server detection, no session creation, no WebTransport. */
export async function initCadEmbed(): Promise<void> {
  await init();
  _rendererType = await detectRenderer();
}

export async function initCad(): Promise<void> {
  await init();

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
    if (docs.length > 0) {
      const loaded = docs.find((d) => d.loaded) ?? docs[0];
      initialDocId = loaded.id;
    }
  } catch {
    // No server or API unavailable
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
