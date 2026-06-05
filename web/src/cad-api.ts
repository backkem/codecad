// cad-api.ts - Typed CAD API surface (source of truth).
//
// Used by both:
//   - Browser (cad.ts): Vite imports directly
//   - Server sandbox: esbuild emits .js, host prepends to user scripts
//
// The factory takes backend functions and returns the full typed cad object.
// Backends may be sync (browser WASM, sandbox WIT) or async (browser RPC).

export interface EntityJson {
  id: string;
  entity_type: string;
  layer: string;
  color: [number, number, number];
  bounds: { min: [number, number]; max: [number, number] };
  start?: [number, number];
  end?: [number, number];
  center?: [number, number];
  radius?: number;
  points?: [number, number][];
  closed?: boolean;
  from?: number;
  to?: number;
  block_name?: string;
  rotation?: number;
  children?: string[];
  text?: string;
  height?: number;
  _session?: string;
}

export interface LayerInfo {
  name: string;
  color: [number, number, number];
}

type Point2 = [number, number];
type Color3 = [number, number, number];
type Target = string | string[] | EntityJson | EntityJson[];

function parseResult(raw: string): unknown {
  const result = JSON.parse(raw);
  if (result && typeof result === "object" && "error" in result)
    throw new Error(result.error as string);
  return result;
}

function normalizeTarget(t: Target): string | string[] {
  if (typeof t === "string") return t;
  if (Array.isArray(t)) return t.map((x) => (typeof x === "string" ? x : x.id));
  if (t && typeof t === "object" && "id" in t) return t.id;
  return t as string;
}

export interface CadApiBackends {
  /** Dispatch to cad_call. Returns JSON result string. */
  call: (method: string, argsJson: string) => string;
  /** RPC dispatch (save, loadDwg, exec). Returns JSON result string or Promise. */
  rpc: (method: string, argsJson: string) => string | Promise<string>;
  /** Read file contents. Null if unavailable. */
  readFile: ((path: string) => string) | null;
}

export function buildCadApi({
  call: rawCall,
  rpc: rawRpc,
  readFile: rawReadFile,
}: CadApiBackends) {
  function call(method: string, args: Record<string, unknown> = {}): unknown {
    return parseResult(rawCall(method, JSON.stringify(args)));
  }

  function rpc(method: string, args: Record<string, unknown> = {}): unknown {
    const raw = rawRpc(method, JSON.stringify(args));
    if (typeof raw === "string") return parseResult(raw);
    return (raw as Promise<string>).then(parseResult);
  }

  return {
    // ── Observe ───────────────────────────────────────────────────────
    describe: () => call("describe") as { entities: number; layers: number },
    entities: (opts?: { expand?: boolean; layer?: string }) =>
      call("entities", opts || {}) as EntityJson[],
    entity: (id: string) => call("entity", { id }) as EntityJson,
    children: (id: string | EntityJson) =>
      call("children", {
        id: typeof id === "string" ? id : id.id,
      }) as EntityJson[],

    // Spatial queries
    near: (pt: Point2, r: number) =>
      call("near", { point: pt, radius: r }) as EntityJson[],
    inBounds: (rect: { min: Point2; max: Point2 }, mode?: string) =>
      call("inBounds", { rect, mode }) as EntityJson[],

    // Topology
    connectedTo: (id: string | EntityJson, tolerance?: number) =>
      call("connectedTo", {
        id: typeof id === "string" ? id : id.id,
        ...(tolerance != null ? { tolerance } : {}),
      }) as EntityJson[],

    // Measurements
    distance: (a: Point2, b: Point2) => call("distance", { a, b }) as number,
    midpoint: (a: Point2, b: Point2) => call("midpoint", { a, b }) as Point2,
    direction: (a: Point2, b: Point2) => call("direction", { a, b }) as number,

    // Geometry helpers
    lineCircleIntersection: (
      line: [Point2, Point2],
      center: Point2,
      radius: number,
    ) => call("lineCircleIntersection", { line, center, radius }),
    circleCircleIntersection: (
      c1: Point2,
      r1: number,
      c2: Point2,
      r2: number,
    ) => call("circleCircleIntersection", { c1, r1, c2, r2 }),
    projectOntoCircle: (point: Point2, center: Point2, radius: number) =>
      call("projectOntoCircle", { point, center, radius }),
    projectOnto: (point: Point2, line: [Point2, Point2]) =>
      call("projectOnto", { point, line }),
    angleOf: (point: Point2, center: Point2) =>
      call("angleOf", { point, center }) as number,

    // ── Add geometry ──────────────────────────────────────────────────
    addLayer: (name: string, opts: { color?: Color3 } = {}) =>
      call("addLayer", { name, ...opts }) as LayerInfo,
    addLine: (
      start: Point2,
      end: Point2,
      opts: { layer?: string; color?: Color3 } = {},
    ) => call("addLine", { start, end, ...opts }) as EntityJson,
    addCircle: (
      center: Point2,
      radius: number,
      opts: { layer?: string; color?: Color3 } = {},
    ) => call("addCircle", { center, radius, ...opts }) as EntityJson,
    addArc: (
      center: Point2,
      radius: number,
      opts: Record<string, unknown> = {},
    ) => call("addArc", { center, radius, ...opts }) as EntityJson,
    addPolyline: (
      points: Point2[],
      opts: { closed?: boolean; layer?: string; color?: Color3 } = {},
    ) => call("addPolyline", { points, ...opts }) as EntityJson,
    addText: (
      text: string,
      at: Point2,
      opts: { height?: number; layer?: string; color?: Color3 } = {},
    ) => call("addText", { text, at, ...opts }),
    measure: (
      from: Point2,
      to: Point2,
      opts: { offset?: number; layer?: string } = {},
    ) => call("measure", { from, to, ...opts }),
    addHatch: (
      boundary: Point2[],
      opts: { angle?: number; spacing?: number; layer?: string } = {},
    ) => call("addHatch", { boundary, ...opts }),

    // Blocks
    defineBlock: (
      name: string,
      shapes: unknown[],
      opts: Record<string, unknown> = {},
    ) => call("defineBlock", { name, shapes, ...opts }),
    place: (block: string, opts: Record<string, unknown> = {}) =>
      call("place", { block, ...opts }) as EntityJson,
    clone: (
      source: string,
      name: string,
      opts: { replaceText?: Record<string, string> } = {},
    ) => call("clone", { source, name, ...opts }),

    // ── Mutate ────────────────────────────────────────────────────────
    remove: (t: Target) => call("remove", { target: normalizeTarget(t) }),
    move: (t: Target, dx: number, dy: number) =>
      call("move", { target: normalizeTarget(t), dx, dy }),
    copy: (t: Target, dx: number, dy: number) =>
      call("copy", { target: normalizeTarget(t), dx, dy }) as EntityJson[],
    rotate: (t: Target, center: Point2, angle: number) =>
      call("rotate", { target: normalizeTarget(t), center, angle }),
    mirror: (t: Target, p1: Point2, p2: Point2) =>
      call("mirror", { target: normalizeTarget(t), p1, p2 }),
    offset: (id: string | EntityJson, distance: number) =>
      call("offset", {
        id: typeof id === "string" ? id : id.id,
        distance,
      }) as EntityJson,
    trim: (id: string | EntityJson, cutPoint: Point2, keep: string) =>
      call("trim", {
        id: typeof id === "string" ? id : id.id,
        cut: cutPoint,
        keep,
      }) as EntityJson,

    // ── Layers ────────────────────────────────────────────────────────
    setLayerVisible: (name: string, visible: boolean) =>
      call("setLayerVisible", { name, visible }),

    // ── View ──────────────────────────────────────────────────────────
    fitView: () => call("fitView"),
    zoomTo: (opts: unknown) => {
      if (
        opts &&
        typeof opts === "object" &&
        "bounds" in (opts as Record<string, unknown>) &&
        "id" in (opts as Record<string, unknown>)
      )
        return call("zoomTo", { id: (opts as EntityJson).id });
      if (Array.isArray(opts) && opts.length > 0) {
        let minX = Infinity,
          minY = Infinity,
          maxX = -Infinity,
          maxY = -Infinity;
        for (const e of opts as EntityJson[]) {
          if (e.bounds?.min) {
            minX = Math.min(minX, e.bounds.min[0]);
            minY = Math.min(minY, e.bounds.min[1]);
          }
          if (e.bounds?.max) {
            maxX = Math.max(maxX, e.bounds.max[0]);
            maxY = Math.max(maxY, e.bounds.max[1]);
          }
        }
        const pad = Math.max(maxX - minX, maxY - minY) * 0.1 || 10;
        return call("zoomTo", {
          bounds: {
            min: [minX - pad, minY - pad],
            max: [maxX + pad, maxY + pad],
          },
        });
      }
      return call("zoomTo", opts as Record<string, unknown>);
    },
    getView: () => call("getView") as { center: Point2; zoom: number },

    // ── Undo/redo ─────────────────────────────────────────────────────
    clear: () => call("clear"),
    checkpoint: () => call("checkpoint"),
    undo: () => call("undo"),
    redo: () => call("redo"),

    // ── RPC (server operations) ───────────────────────────────────────
    save: (path = "cadview-output.json") => rpc("save", { path }),
    saveDwg: (path = "output.dwg") => rpc("saveDwg", { path }),
    savePdf: (path = "output.pdf") => rpc("savePdf", { path }),
    loadDwg: (path: string) => rpc("loadDwg", { path }),
    loadElmt: (
      path: string,
      opts: { name?: string; defaultLayer?: string } = {},
    ) => rpc("loadElmt", { path, ...opts }),
    loadElmtDir: (path: string, opts: { defaultLayer?: string } = {}) =>
      rpc("loadElmtDir", { path, ...opts }),
    exec: (path: string, opts: Record<string, unknown> = {}) =>
      rpc("exec", { path, ...opts }),
    runScript: (program: string, opts: Record<string, unknown> = {}) =>
      rpc("runScript", { program, ...opts }),

    // ── File I/O ──────────────────────────────────────────────────────
    readFile: rawReadFile
      ? (path: string) => rawReadFile(path)
      : () => {
          throw new Error("readFile not available in this environment");
        },
  };
}

export type CadApi = ReturnType<typeof buildCadApi>;
