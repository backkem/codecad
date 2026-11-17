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
  type: string;
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
  linetype?: string;
  lineweight?: number;
  transparency?: number;
  _session?: string;
}

export interface LayerInfo {
  name: string;
  color: [number, number, number];
}

type Point2 = [number, number];
/** Polyline vertex: [x, y] or [x, y, bulge] (bulge = tan(sweep/4), + = CCW arc). */
type Vertex = [number, number] | [number, number, number];
type Color3 = [number, number, number] | number; // [r,g,b] or ACI index
type Target = string | string[] | EntityJson | EntityJson[];
type EntityRef = string | EntityJson;
/** A line: two points, or a line entity (id or object). */
type LineRef = [Point2, Point2] | EntityRef;
/** Closed ring: points, a circle {center, radius}, or an entity (circle / closed polyline). */
type Ring = Point2[] | { center: Point2; radius: number } | EntityRef;

export interface StyleOpts {
  layer?: string;
  color?: Color3;
  linetype?: string;
  lineweight?: number;
  transparency?: number;
}

export interface DimOpts {
  /** Label template; "<>" is replaced by the measured value (e.g. "Ø<>", "R<>", "<> mm"). */
  label?: string;
  /** Fixed decimals; default rounds to 0.1 and drops a trailing ".0". */
  decimals?: number;
  text_height?: number;
  /** Arrowhead length (default 0.85 * text_height). */
  arrow?: number;
  layer?: string;
  color?: Color3;
}

export interface DimResult {
  value: number;
  /** Same as value (kept for older scripts). */
  distance: number;
  label: string;
  ids: string[];
  /** ID of the TEXT entity holding the label. */
  text: string | null;
}

export interface Region {
  outer: Point2[];
  holes: Point2[][];
  area: number;
}

export interface SavePdfOptions {
  /**
   * "paper" (default): readable on white, white becomes black, light
   * colours are darkened. "mono": everything black. "screen": colours
   * exactly as stored.
   */
  colors?: "paper" | "mono" | "screen";
  /**
   * true: stroke entities at their own lineweight (mm) where one is set
   * on the entity or its layer. Default false: uniform hairlines.
   */
  lineweights?: boolean;
}

function parseResult(raw: string): unknown {
  const result = JSON.parse(raw);
  if (result && typeof result === "object" && "error" in result)
    throw new Error(result.error as string);
  return result;
}

function ref(e: EntityRef): string {
  return typeof e === "string" ? e : e.id;
}

/** Entity refs inside lines/rings are sent as ids; point data passes through. */
function normalizeShapeRef<T>(v: T): T | string {
  if (v && typeof v === "object" && !Array.isArray(v) && "id" in v)
    return (v as unknown as EntityJson).id;
  return v;
}

function normalizeTarget(t: Target): string | string[] {
  if (typeof t === "string") return t;
  if (Array.isArray(t)) return t.map((x) => (typeof x === "string" ? x : x.id));
  if (t && typeof t === "object" && "id" in t) return t.id;
  return t as string;
}

/** Options for `cad.render`. */
export interface RenderOptions {
  /** Output path, relative to the server's working directory. Default "render.png". */
  out?: string;
  /** Image size in pixels (times pixelRatio). Default [960, 640]. */
  size?: [number, number];
  /** World region to fit, with a 10 % margin. Default: the whole drawing. */
  bounds?: { min: Point2; max: Point2 };
  /** Layers to leave out. */
  hiddenLayers?: string[];
  /** Default "auto": GPU if available, else CPU. */
  backend?: "auto" | "gpu" | "cpu";
  /** Device pixels per CSS pixel; scales size and stroke width. Default 1. */
  pixelRatio?: number;
  /** Samples per pixel for future 3D renders; ignored in 2D. */
  spp?: number;
}

export interface RenderResult {
  ok: boolean;
  path: string;
  width: number;
  height: number;
  backend: "gpu" | "cpu";
  bytes: number;
  ms: number;
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
    /** Intersection of two lines (infinite unless segments=true). null if parallel / no hit. */
    lineLineIntersection: (
      l1: LineRef,
      l2: LineRef,
      opts: { segments?: boolean } = {},
    ) =>
      call("lineLineIntersection", {
        l1: normalizeShapeRef(l1),
        l2: normalizeShapeRef(l2),
        ...opts,
      }) as Point2 | null,
    /** Lines tangent to two circles: [[pOnC1, pOnC2], ...] (0-2 lines). */
    tangentLines: (
      c1: Point2,
      r1: number,
      c2: Point2,
      r2: number,
      kind: "outer" | "inner" = "outer",
    ) => call("tangentLines", { c1, r1, c2, r2, kind }) as [Point2, Point2][],
    /** Tangent points on circle (c, r) seen from p (0, 1 or 2 points). */
    tangentFromPoint: (p: Point2, c: Point2, r: number) =>
      call("tangentFromPoint", {
        point: p,
        center: c,
        radius: r,
      }) as Point2[],
    /** Boolean on polygon regions (rings: points, circles or entity ids). */
    regionClip: (
      subject: Ring | Ring[],
      clip: Ring | Ring[],
      op: "intersect" | "union" | "subtract" | "xor" = "intersect",
    ) => {
      const n = (r: Ring | Ring[]) => {
        if (!Array.isArray(r)) return normalizeShapeRef(r);
        const first = r[0];
        // A single ring is a list of [x, y] points.
        if (Array.isArray(first) && typeof first[0] === "number") return r;
        return (r as Ring[]).map(normalizeShapeRef);
      };
      return call("regionClip", {
        subject: n(subject),
        clip: n(clip),
        op,
      }) as Region[];
    },

    // ── Add geometry ──────────────────────────────────────────────────
    addLayer: (
      name: string,
      opts: { color?: Color3; linetype?: string; lineweight?: number } = {},
    ) => call("addLayer", { name, ...opts }) as LayerInfo,
    addLine: (start: Point2, end: Point2, opts: StyleOpts = {}) =>
      call("addLine", { start, end, ...opts }) as EntityJson,
    addCircle: (center: Point2, radius: number, opts: StyleOpts = {}) =>
      call("addCircle", { center, radius, ...opts }) as EntityJson,
    addArc: (
      center: Point2,
      radius: number,
      opts: StyleOpts & {
        from?: number;
        to?: number;
        p1?: Point2;
        p2?: Point2;
        shortest?: boolean;
      } = {},
    ) => call("addArc", { center, radius, ...opts }) as EntityJson,
    /** Vertices may carry a bulge ([x, y, bulge]) to make that segment an arc. */
    addPolyline: (
      points: Vertex[],
      opts: StyleOpts & { closed?: boolean; bulges?: number[] } = {},
    ) => call("addPolyline", { points, ...opts }) as EntityJson,
    /** B-spline: control points, or fit: true to interpolate through the points. */
    addSpline: (
      points: Point2[],
      opts: StyleOpts & {
        fit?: boolean;
        degree?: number;
        closed?: boolean;
      } = {},
    ) => call("addSpline", { points, ...opts }) as EntityJson,
    addText: (
      text: string,
      at: Point2,
      opts: StyleOpts & {
        height?: number;
        /** Degrees, CCW. */
        rotation?: number;
        align?: "left" | "center" | "right";
        valign?: "baseline" | "middle" | "top";
      } = {},
    ) =>
      call("addText", { text, at, ...opts }) as {
        id: string;
        width: number;
        height: number;
        at: Point2;
        position: Point2;
        rotation: number;
      },
    /** Linear dimension. offset: signed distance of the dimension line. */
    measure: (
      from: Point2,
      to: Point2,
      opts: DimOpts & {
        offset?: number;
        dir?: "h" | "v" | "aligned";
        /** Move the text along the dimension line. */
        shift?: number;
      } = {},
    ) => call("measure", { from, to, ...opts }) as DimResult,
    /** Radius dimension of a circle/arc entity, or of {center, radius}. angle in degrees. */
    dimRadius: (
      target: EntityRef | { center: Point2; radius: number },
      opts: DimOpts & { angle?: number } = {},
    ) =>
      call("dimRadius", {
        ...(typeof target === "string" || "id" in target
          ? { id: ref(target as EntityRef) }
          : target),
        ...opts,
      }) as DimResult,
    dimDiameter: (
      target: EntityRef | { center: Point2; radius: number },
      opts: DimOpts & { angle?: number } = {},
    ) =>
      call("dimDiameter", {
        ...(typeof target === "string" || "id" in target
          ? { id: ref(target as EntityRef) }
          : target),
        ...opts,
      }) as DimResult,
    /** Angle dimension at center, CCW from ray(p1) to ray(p2). */
    dimAngle: (
      center: Point2,
      p1: Point2,
      p2: Point2,
      opts: DimOpts & { radius?: number } = {},
    ) => call("dimAngle", { center, p1, p2, ...opts }) as DimResult,
    /** Line hatch over boundary minus holes. pattern "solid" makes a fill. */
    addHatch: (
      boundary: Ring,
      opts: StyleOpts & {
        holes?: Ring[];
        angle?: number;
        spacing?: number;
        pattern?: "lines" | "cross" | "solid" | "ANSI31" | "ANSI37" | "SOLID";
      } = {},
    ) =>
      call("addHatch", {
        boundary: normalizeShapeRef(boundary),
        ...opts,
        ...(opts.holes ? { holes: opts.holes.map(normalizeShapeRef) } : {}),
      }) as { lines: number; ids: string[] } | EntityJson,
    /** Solid fill (SOLID hatch). Circle rings stay exact arcs. */
    addFill: (boundary: Ring, opts: StyleOpts & { holes?: Ring[] } = {}) =>
      call("addFill", {
        boundary: normalizeShapeRef(boundary),
        ...opts,
        ...(opts.holes ? { holes: opts.holes.map(normalizeShapeRef) } : {}),
      }) as EntityJson,

    // Blocks
    /**
     * Shapes: {type:"line",start,end} | {type:"circle",center,radius} |
     * {type:"arc",center,radius,from,to} | {type:"polyline",points,closed} |
     * {type:"text",text,at,height?,rotation?,align?,valign?}; each may set layer/color.
     */
    defineBlock: (
      name: string,
      shapes: Record<string, unknown>[],
      opts: { insert_point?: Point2; default_layer?: string } = {},
    ) => call("defineBlock", { name, shapes, ...opts }),
    place: (
      block: string,
      opts: {
        at: Point2;
        /** Degrees, CCW. */
        rotation?: number;
        scale?: number | [number, number];
        xScale?: number;
        yScale?: number;
        layer?: string;
      },
    ) => call("place", { block, ...opts }) as EntityJson,
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
    /** Trim a line/arc/circle at its intersections with boundary entities; keep the piece containing `keep`. Same id. */
    trimTo: (id: EntityRef, boundary: Target, keep: Point2) =>
      call("trimTo", {
        id: ref(id),
        boundary: normalizeTarget(boundary),
        keep,
      }) as EntityJson,
    /** Extend a line/arc to the boundary. Same id. */
    extendTo: (
      id: EntityRef,
      boundary: Target,
      opts: { end?: "start" | "end" | "nearest" } = {},
    ) =>
      call("extendTo", {
        id: ref(id),
        boundary: normalizeTarget(boundary),
        ...opts,
      }) as EntityJson,
    /**
     * Tangent arc between two lines/arcs/circles; trims or extends both (ids kept).
     * pick: points on the parts to keep (default: the ends away from the corner).
     * radius 0 makes a sharp corner (arc: null).
     */
    fillet: (
      a: EntityRef,
      b: EntityRef,
      radius: number,
      opts: { pick?: [Point2 | null, Point2 | null]; layer?: string } = {},
    ) =>
      call("fillet", { a: ref(a), b: ref(b), radius, ...opts }) as {
        arc: EntityJson | null;
        a: EntityJson;
        b: EntityJson;
      },
    /** Bevel two lines: d1 along a, d2 (default d1) along b. */
    chamfer: (
      a: EntityRef,
      b: EntityRef,
      d1: number,
      d2?: number,
      opts: { pick?: [Point2 | null, Point2 | null]; layer?: string } = {},
    ) =>
      call("chamfer", {
        a: ref(a),
        b: ref(b),
        d1,
        ...(d2 != null ? { d2 } : {}),
        ...opts,
      }) as { line: EntityJson | null; a: EntityJson; b: EntityJson },
    /** Polar or rectangular array. Counts include the original. Returns the new copies. */
    array: (
      t: Target,
      opts:
        | {
            polar: {
              center: Point2;
              count: number;
              /** Total angle in degrees (default 360). */
              angle?: number;
              /** Rotate copies (default true). */
              rotate?: boolean;
            };
          }
        | { rect: { rows: number; cols: number; dx: number; dy: number } },
    ) => call("array", { target: normalizeTarget(t), ...opts }) as EntityJson[],

    // ── Linetypes ──────────────────────────────────────────────────────
    addLinetype: (
      name: string,
      pattern: number[],
      opts?: { description?: string },
    ) => call("addLinetype", { name, pattern, ...opts }),
    linetypes: () =>
      call("linetypes", {}) as {
        name: string;
        description: string;
        pattern: number[];
      }[],

    // ── Introspection ──────────────────────────────────────────────────
    methods: () =>
      call("methods", {}) as { name: string; args: string; desc: string }[],
    aciColors: () =>
      call("aciColors", {}) as {
        index: number;
        color: [number, number, number];
      }[],

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
    /** Remove entities and layers. all: true also clears blocks and resets linetypes. */
    clear: (opts: { all?: boolean } = {}) => call("clear", opts),
    checkpoint: () => call("checkpoint"),
    undo: () => call("undo"),
    redo: () => call("redo"),

    // ── RPC (server operations) ───────────────────────────────────────
    save: (path = "cadview-output.json") => rpc("save", { path }),
    saveDwg: (path = "output.dwg") => rpc("saveDwg", { path }),
    savePdf: (path = "output.pdf", opts: SavePdfOptions = {}) =>
      rpc("savePdf", { ...opts, path }),
    /** Render the drawing to a PNG on the server (GPU, else CPU). */
    render: (opts: RenderOptions = {}) =>
      rpc("render", { out: "render.png", ...opts }) as
        | RenderResult
        | Promise<RenderResult>,
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
