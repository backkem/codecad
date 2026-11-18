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
  /** The entity's attributes; absent when it has none. */
  attrs?: Attrs;
  /** For a child of a block insert (expand, children): its index in the block. */
  child?: number;
  _session?: string;
}

/** An attribute value: a string, a finite number or a boolean. */
export type AttrValue = string | number | boolean;
/**
 * Free-form attributes: the script's own facts about an entity (a net, a
 * part number, a room). The core stores, syncs and filters on them and never
 * interprets them. Keys: 1-64 ASCII letters, digits and `_ - . : /`.
 */
export type Attrs = Record<string, AttrValue>;

/**
 * Which entities a call works on. Every condition that is set must hold.
 * Works for `entities()`, every mutation target and the set queries.
 */
export interface Selector {
  ids?: string[];
  /** Layer name, or any of several. */
  layer?: string | string[];
  /** Entity type as `entities()` reports it ("line", "arc", "block_insert", ...). */
  type?: string | string[];
  /** Equal values per key (typed: "1" is not 1); null means the key is absent. */
  attrs?: Record<string, AttrValue | null>;
  /** Keys that must be present, whatever their value. */
  has?: string[];
  /** The entity's bounding box overlaps this rectangle. */
  bounds?: { min: Point2; max: Point2 };
  /**
   * Block inserts as their child shapes, each matched on its own layer and
   * attributes (block definition, then shape, then insert; a later one wins).
   */
  expand?: boolean;
}

/** One shape a set query could not use exactly (spline, ellipse, text, ...). */
export interface SkippedShape {
  id: string;
  child?: number;
  type: string;
}

/** A pair from a set query: an entity of a and one of b. */
export interface PairBase {
  a: string;
  b: string;
  /** Child index in a block insert (expanded, or the child nearest the contact). */
  aChild?: number;
  bChild?: number;
  /** With `attrs: true`: each entity's (effective) attributes. */
  aAttrs?: Attrs;
  bAttrs?: Attrs;
}

export interface ClosePair extends PairBase {
  distance: number;
  /** The closest point on a, and on b. */
  pa: Point2;
  pb: Point2;
}

export interface ContactPair extends PairBase {
  /** Where the outlines meet (and the ends of any shared stretch). */
  points: Point2[];
  /** The outlines coincide along a stretch (collinear lines, one circle). */
  shared: boolean;
  /** With `areas`: the outlines do not meet but (part of) b lies inside a ("a") or a inside b ("b"). */
  contains?: "a" | "b";
}

export interface InsidePair {
  a: string;
  /** The containing entity; null for a rectangle or ring region. */
  b: string | null;
  aChild?: number;
  bChild?: number;
  aAttrs?: Attrs;
  bAttrs?: Attrs;
  /** a touches the region's boundary. */
  touches: boolean;
}

export interface SetResult<P> {
  pairs: P[];
  /** Shapes left out: no rule passes silently on geometry it did not see. */
  skipped: SkippedShape[];
  /** More pairs than `limit` (default 100 000). */
  truncated?: boolean;
}

export interface PairOpts {
  /** Leave out pairs whose values for all these keys are equal (missing counts as a value). */
  differ?: string[];
  /** Include both entities' attributes in each pair. */
  attrs?: boolean;
  /** Most pairs returned (default 100 000). */
  limit?: number;
}

export interface LayerInfo {
  name: string;
  color: [number, number, number];
}

type Point2 = [number, number];
/** Polyline vertex: [x, y] or [x, y, bulge] (bulge = tan(sweep/4), + = CCW arc). */
type Vertex = [number, number] | [number, number, number];
type Color3 = [number, number, number] | number; // [r,g,b] or ACI index
/** Ids, entity objects, or a selector object (see `Selector`). */
type Target = string | string[] | EntityJson | EntityJson[] | Selector;
/** A target, or null for every entity. */
type Selection = Target | null;
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
  /** Attributes for the new entity (or entities). */
  attrs?: Attrs;
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
  /**
   * Layers to leave out, by exact name, on top of the layers the document
   * marks invisible. Default: none.
   */
  skipLayers?: string[];
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

function normalizeTarget(t: Target): string | string[] | Selector {
  if (typeof t === "string") return t;
  if (Array.isArray(t)) return t.map((x) => (typeof x === "string" ? x : x.id));
  if (t && typeof t === "object" && "id" in t) return (t as EntityJson).id;
  return t as Selector;
}

function normalizeSelection(t: Selection | undefined) {
  return t == null ? null : normalizeTarget(t);
}

/** A region for `inside`: a rectangle, a ring of points, or a selection. */
type InsideRegion = { min: Point2; max: Point2 } | Point2[] | Target;

function normalizeRegion(r: InsideRegion) {
  if (Array.isArray(r) && Array.isArray(r[0])) return r;
  if (r && typeof r === "object" && "min" in r && "max" in r) return r;
  return normalizeTarget(r as Target);
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
  /**
   * 3D presentation renders: samples per pixel (1 to 65536). Default 1024;
   * with the denoiser most scenes look converged at 256.
   */
  spp?: number;
  /** 3D presentation renders: seed of the random numbers. Default 0; the same scene, spp and seed give the same bytes. */
  seed?: number;
  /**
   * 3D presentation renders: filter the image with the edge-aware denoiser
   * (guided by the first surface's albedo, normal and position). Default
   * true; false shows the raw estimate.
   */
  denoise?: boolean;

  // ── 3D: the document's solid parts (server with a kernel and a GPU) ──
  /**
   * "3d" draws the parts, "2d" the drawing. Default: 3D when `view` is
   * given, or when the document has parts and no 2D entities.
   */
  mode?: "2d" | "3d";
  /**
   * Named view. Default "iso", the south-east isometric corner (also
   * "iso-se"); "iso-sw", "iso-ne" and "iso-nw" are the other three.
   */
  view?:
    | "iso"
    | "isometric"
    | "iso-se"
    | "iso-sw"
    | "iso-ne"
    | "iso-nw"
    | "front"
    | "back"
    | "left"
    | "right"
    | "top"
    | "bottom";
  /** Default "orthographic" ("perspective" with a `camera`). */
  projection?: "orthographic" | "perspective";
  /**
   * A camera placed by hand instead of a named view: eye and target in mm.
   * Perspective with `fov`, the vertical field of view in degrees (default
   * 30); orthographic with `height`, the view's height in mm (or with
   * `projection: "orthographic"`). `up` is the world direction shown as up
   * (default [0, 0, 1]). Used as given unless `fit` is true.
   */
  camera?: {
    eye: Point3;
    target: Point3;
    fov?: number;
    height?: number;
    up?: Point3;
  };
  /** Fit the camera to the parts: default true for a named view, false for a `camera`. */
  fit?: boolean;
  /** Which parts to draw (default all). */
  select?: PartTarget | null;
  /**
   * Colours: "part" (default, one palette colour per part), "def" (one per
   * definition), or {attr: key} (one per value of that attribute; a value
   * written "#rrggbb" is used as the colour). A render option only: the
   * core gives the attribute no other meaning.
   */
  colorBy?: "part" | "def" | { attr: string };
  /** Sharp edges and silhouettes over the shading. Default true. */
  edges?: boolean;
  /** Ambient occlusion: true (default), false, or a strength 0..1. */
  occlusion?: boolean | number;
  /**
   * A ground: true, or {height?, color?}. Default none. The working render
   * shows its contact shadow; the presentation render makes it an endless
   * floor (colour = albedo, default the background's) that takes shadows
   * and bounce light.
   */
  ground?:
    | boolean
    | {
        height?: number;
        color?: [number, number, number] | string;
        /**
         * Presentation renders: a material like a part's, over `color`
         * (e.g. a lawn: a rough green with a "mottle" pattern). The working
         * render draws its colour.
         */
        material?: MaterialSpec;
      };
  /** [r, g, b], "#rrggbb", "light" (default) or "viewer" (the 2D viewer's dark). */
  background?: [number, number, number] | string;
  /** Cutaway: up to 4 planes; each removes the side its normal points into. */
  clip?: ClipPlane | ClipPlane[];
  /**
   * With several clip planes: "any" (default) removes what lies beyond any
   * plane; "all" removes only what lies beyond every plane (a quarter
   * cutaway from two planes).
   */
  clipMode?: "any" | "all";
  /** Exploded view: a factor, or {factor, axes: "xyz"}. */
  explode?: number | { factor?: number; axes?: string };
  /** Tessellation chord in mm. Default: a quarter pixel, as a power of two. */
  chord?: number;
  /**
   * "working" (default): shaded raster with edges, milliseconds.
   * "presentation": path traced (global illumination, soft shadows,
   * materials), the same scene, camera and options; `spp` sets its quality.
   * It draws no edge lines or ambient occlusion (the light does that).
   */
  quality?: "working" | "presentation";
  /**
   * Physical materials by attribute value, like `colorBy`: parts whose
   * attribute `attr` has a value in `map` get that material, others get
   * `default` (if given) over their `colorBy` colour. The names are the
   * script's (`examples/3d/lib/materials.js` has presets). The working
   * render uses only `color`, `metallic` and `roughness`.
   */
  materials?: {
    attr?: string;
    map?: Record<string, MaterialSpec>;
    default?: MaterialSpec;
  };
  /**
   * Presentation renders: the light. "studio" (default: a grey dome and a
   * soft key light that follows the camera) or "sky" (daylight, a sharp
   * sun), or an object to adjust one. Colours are sRGB.
   */
  environment?:
    | "studio"
    | "sky"
    | {
        preset?: "studio" | "sky";
        zenith?: [number, number, number] | string;
        horizon?: [number, number, number] | string;
        /** Below the horizon (a `ground` covers it). */
        ground?: [number, number, number] | string;
        /** Scales dome and sun together. Default 1. */
        intensity?: number;
        /**
         * A studio backdrop: what is seen just above the horizon blends up
         * from the lit ground into the dome, so a level view shows no line
         * where the floor meets it. Default true for "studio", false for "sky".
         */
        sweep?: boolean;
        /**
         * Aerial haze (mm): the ground seen at distance t blends toward the
         * horizon colour by 1 - exp(-t / haze), so it meets the sky softly.
         * Default 150000 for "sky", 0 (none) for "studio".
         */
        haze?: number;
        /** A distant disc light, or false for none. */
        sun?:
          | false
          | {
              /** Toward the light; a world direction unless `frame` is "camera" (x right, y up, z toward the viewer). */
              direction?: Point3;
              frame?: "world" | "camera";
              /** Angular diameter in degrees: 0.53 the real sun, 10 to 30 a soft box. */
              angle?: number;
              /** Irradiance on a surface facing the light. */
              irradiance?: number;
              color?: [number, number, number] | string;
            };
      };
  /** Presentation renders: exposure in stops (+1 doubles the light). Default 0. */
  exposure?: number;
  /**
   * Presentation renders: up to 4 openings (windows, doors) through which
   * the dome lights an interior, each a rectangle: a corner and two
   * perpendicular edges (mm). The tracer samples the sky through them
   * directly; the image converges faster, to the same result.
   */
  portals?: { origin: Point3; u: Point3; v: Point3 }[];
}

/**
 * Physical parameters of one material. Every field is optional; a missing
 * `color` keeps the part's `colorBy` colour.
 */
export interface MaterialSpec {
  /** sRGB base colour. */
  color?: [number, number, number] | string;
  /** 0 dielectric, 1 metal. */
  metallic?: number;
  /** 0 mirror to 1 fully rough. Default 0.5. */
  roughness?: number;
  /** 0 to 1: light passing through as through glass. */
  transmission?: number;
  /** Index of refraction, 1 to 3. Default 1.5 (1 has no specular reflection). */
  ior?: number;
  /** Emitted light: a strength (white), or a colour and strength. */
  emission?:
    | number
    | { color?: [number, number, number] | string; strength?: number };
  /** A clear coat (lacquer, gloss paint), 0 to 1, and its roughness. */
  clearcoat?: number;
  clearcoatRoughness?: number;
  /**
   * A pattern without texture coordinates, in the part's own frame.
   * "bands": rings about an axis, seen as grain along it (wood).
   * "brushed": streaks along the axis, or in circles about it with
   * `circular` (turned metal), and highlights stretched across them.
   * "mottle": soft patches about `scale` mm across (default 500), up to
   * `strength` (default 0.5) of the way toward `color` (default darker),
   * so an even surface such as a lawn is not flat.
   * `axis`: "long" (default, the part's longest direction), "short",
   * "symmetry" (a turned part's axis) or a vector.
   */
  pattern?: {
    kind: "flat" | "bands" | "brushed" | "mottle";
    axis?: "long" | "short" | "symmetry" | Point3;
    /** bands: mm between rings (default 4); ring colour; 0..1 strength; mm of wander; mm from the part's centre to the ring axis. */
    spacing?: number;
    color?: [number, number, number] | string;
    strength?: number;
    warp?: number;
    distance?: number;
    /** brushed: 0..1 stretch of highlights; 0..1 streak contrast; mm streak width; circles about the axis. */
    anisotropy?: number;
    streaks?: number;
    scale?: number;
    circular?: boolean;
  };
}

export interface ClipPlane {
  point: Point3;
  /** Points into the side that is removed. */
  normal: Point3;
}

export interface RenderResult {
  ok: boolean;
  path: string;
  width: number;
  height: number;
  backend: "gpu" | "cpu";
  bytes: number;
  ms: number;
  /** 3D renders only. */
  mode?: "3d";
  quality?: "working" | "presentation";
  parts?: number;
  triangles?: number;
  chord?: number;
  kernel?: string;
  /** Presentation renders: samples per pixel reached, and where the time went (ms). */
  samples?: number;
  /** Presentation renders: whether the image was denoised. */
  denoised?: boolean;
  trace?: {
    samples: number;
    passes: number;
    instances: number;
    /** Triangles traced (each shared mesh once). */
    triangles: number;
    meshesBuilt: number;
    bvhMs: number;
    uploadMs: number;
    traceMs: number;
    pngMs: number;
  };
}

/** Options for `cad.bake3d`: the 3D `cad.render` options a viewer starts from, and the file. */
export interface Bake3dOptions
  extends Omit<RenderOptions, "size" | "pixelRatio" | "backend" | "mode"> {
  /** Output path, relative to the server's working directory. Default "scene.bake3d". */
  out?: string;
}

export interface Bake3dResult {
  ok: boolean;
  path: string;
  bytes: number;
  parts: number;
  defs: number;
  /** Distinct solids (definitions with equal features count once). */
  solids: number;
  triangles: number;
  chord: number;
  /** Solids whose mesh the kernel would only make at a coarser chord. */
  coarser: number;
  kernel: string;
  /** The file format version. */
  format: number;
  ms: number;
}

// ── Solid parts ──────────────────────────────────────────────────────

/** A 3D point or vector [x, y, z] in mm (Z up). */
export type Point3 = [number, number, number];

/**
 * One closed profile loop, kept exact (lines and arcs, never flattened):
 * vertices ([x, y] or [x, y, bulge]), a circle, a rectangle, or a closed 2D
 * entity (closed polyline, circle, full ellipse, closed spline).
 */
export type ProfileLoop =
  | Vertex[]
  | { center: Point2; radius: number }
  | { min: Point2; max: Point2 }
  | EntityRef;

/** A profile: one loop, or an outer loop with holes (the shape regionClip returns). */
export type Profile =
  | ProfileLoop
  | { outer: ProfileLoop; holes?: ProfileLoop[] };

/**
 * A sketch plane. "xy": u along X, normal +Z. "xz": u along X, v along Z,
 * normal -Y. "yz": u along Y, v along Z, normal +X. Or a general plane.
 */
export type Plane =
  | "xy"
  | "xz"
  | "yz"
  | { origin?: Point3; x: Point3; normal: Point3 };

export interface SketchOpts {
  plane?: Plane;
  /** Moves the plane's origin here. */
  at?: Point3;
}

/** A path segment: a line to `to`, or an arc through `via` to `to`. */
export type PathSeg = { to: Point3 } | { via: Point3; to: Point3 };

export interface LoftSection extends SketchOpts {
  profile: Profile;
}

export interface NewPartOpts {
  /** The part's attributes (the script's own facts; never interpreted). */
  attrs?: Attrs;
  /** A name for the definition, for messages and reports. */
  name?: string;
}

/** A placed part as cad.parts() reports it. */
export interface PartJson {
  /** "p_12": parts share the id counter with entities ("e_12") but not the prefix. */
  id: string;
  /** "d_3": the definition (feature tree) this part places. */
  def: string;
  name: string;
  /** Definition frame to world: rotation as a unit quaternion [w, x, y, z]. */
  placement: {
    translation: Point3;
    rotation: [number, number, number, number];
  };
  features: { id: number; kind: string }[];
  /** How many parts share the definition. */
  instances: number;
  attrs?: Attrs;
  /** With a kernel: tight box in the world and in the part's own frame. */
  bbox?: { min: Point3; max: Point3 };
  localBbox?: { min: Point3; max: Point3 };
  /**
   * With a kernel: 0 when both boxes are exact; otherwise each side lies at
   * most this far (mm) outside the solid (where an edge has no exact box,
   * as where two cylinders meet).
   */
  bboxTol?: number;
  /** With a kernel: exact volume (mm^3) and centroid (world). */
  volume?: number;
  centroid?: Point3 | null;
  _session?: string;
}

/** Which parts a call works on: the entity selector's shape, for parts. */
export interface PartSelector {
  ids?: string[];
  /** Parts of this definition. */
  def?: string;
  attrs?: Record<string, AttrValue | null>;
  has?: string[];
  /** Box overlap: 3 coordinates, or 2 for the plan (every height). Needs a kernel. */
  bounds?: { min: number[]; max: number[] };
}

type PartRef = string | PartJson;
/** A part, several, or a part selector. */
export type PartTarget = PartRef | PartRef[] | PartSelector;

/** A body for cut, union and intersect: a feature, or another part as placed now. */
export type Tool =
  | (SketchOpts & { extrude: Profile; distance: number; taper?: number })
  | (SketchOpts & { revolve: Profile; axis: [Point2, Point2]; angle?: number })
  | (SketchOpts & { sweep: Profile; path: PathSeg[] })
  | { loft: LoftSection[]; ruled?: boolean }
  | { part: PartRef };

/** Edges by meaning, never by kernel topology. `feature` defaults to the first. */
export type EdgeSel =
  | "all"
  | { cap: "start" | "end" | "both"; feature?: number; segs?: number[] }
  | { lateral: number[]; feature?: number }
  | { near: Point3[] };

/** Positions in world coordinates (default) or in the part's own frame. */
export interface FrameOpts {
  frame?: "world" | "local";
}

/** Options of the pair queries between parts, as in 2D. */
export type PartPairOpts = PairOpts;

/** A pair of parts, as the 3D set queries report it. */
export interface PartPairBase {
  a: string;
  b: string;
  aAttrs?: Attrs;
  bAttrs?: Attrs;
}

/** Two parts that share volume (or touch, with `touching: true`). */
export interface OverlapPair extends PartPairBase {
  /** The shared volume (mm^3), 0 for a touch. */
  volume: number;
  /** The shared volume over the largest face of the box both parts' boxes share: the penetration depth for box-shaped overlaps. */
  depth?: number;
  /** a lies wholly in b; b wholly in a. */
  aInB?: boolean;
  bInA?: boolean;
  /** The pair touches and shares no volume. */
  touching?: boolean;
  /** A point of the contact. */
  at?: Point3;
}

/** Two parts and the closest points between them. */
export interface GapPair extends PartPairBase {
  /** 0 when they touch or overlap. */
  distance: number;
  pa: Point3;
  pb: Point3;
}

export interface PartInsidePair {
  a: string;
  /** The part whose solid holds a; null for a box region. */
  b: string | null;
  aAttrs?: Attrs;
  bAttrs?: Attrs;
  /** Box regions: a touches the box's boundary. */
  touches?: boolean;
}

export interface PartSetResult<P> {
  pairs: P[];
  /** Pairs the kernel could not answer, with the reason: no rule passes silently on them. */
  skipped: { a: string; b: string; reason: string }[];
  /** More pairs than `limit` (default 100 000). */
  truncated?: boolean;
}

/** A named view, as `cad.render` names them. */
export type ViewName =
  | "front"
  | "back"
  | "left"
  | "right"
  | "top"
  | "bottom"
  | "iso"
  | "isometric"
  | "iso-se"
  | "iso-sw"
  | "iso-ne"
  | "iso-nw";

/** Which view, which parts, and where the view goes on the sheet. */
export interface ViewOpts {
  /** A named view; or give `from` instead. */
  view?: ViewName;
  /** The direction toward the eye (the viewer looks along -from). */
  from?: Point3;
  /** With `from`: which way is up on the sheet (default world Z, or Y when looking along Z). */
  up?: Point3;
  /** Which parts (default all). */
  parts?: PartTarget | null;
  /** The 3D point drawn at `at` (default the centre of the parts' box). */
  origin?: Point3;
  /** Where the view's origin goes on the sheet. Default [0, 0]. */
  at?: Point2;
  /** Sheet units per mm. Default 1. */
  scale?: number;
}

/** Options for `cad.project`. */
export interface ProjectOpts extends ViewOpts {
  /** Block name. Default "view-<view>" ("view-view" for a direction). Projecting again replaces it. */
  name?: string;
  /** Draw hidden edges. Default true. */
  hidden?: boolean;
  /** Draw silhouettes of curved faces. Default true. */
  silhouettes?: boolean;
  /** Draw edges where faces meet smoothly (round boundaries) even off the outline. Default false. */
  smooth?: boolean;
  /** Layers per class of curve; silhouettes default to the visible and hidden layers. */
  layers?: {
    visible?: string;
    hidden?: string;
    silhouette?: string;
    hiddenSilhouette?: string;
  };
  /** Copy each part's attributes onto its entities too. Default false. */
  partAttrs?: boolean;
  /** Place an insert of the block (or move the existing one). Default true. */
  place?: boolean;
  /** Ask the kernel's exact ray hits where the mesh oracle is unsure. Default true. */
  exact?: boolean;
  /** Chord (mm) of the oracle's mesh. Default max(0.01, 1e-5 x the parts' size). */
  chord?: number;
}

/** What `cad.project` made. */
export interface ProjectResult {
  block: string;
  /** The insert's entity id, or null with place: false. */
  insert: string | null;
  view: string;
  /** Unit vectors: toward the eye, sheet right, sheet up. */
  from: Point3;
  right: Point3;
  up: Point3;
  origin: Point3;
  at: Point2;
  scale: number;
  parts: number;
  entities: number;
  /** By type (line, arc, circle, ellipse, spline) and by class (visible, hidden). */
  counts: Record<string, number>;
  /** The view's extent on the sheet, or null when it is empty. */
  bounds: { min: Point2; max: Point2 } | null;
  /** Faces whose silhouette has no closed form here, and parts without an oracle mesh. */
  open: { part: string; face?: number; reason: string }[];
  stats: Record<string, number>;
  oracle: Record<string, number>;
  ms: Record<string, number>;
  /** A 3D point's position on the sheet in this view. */
  toSheet: (p: Point3) => Point2;
}

/** `toSheet` for a projection result: plain arithmetic on its frame. */
function withToSheet(v: Omit<ProjectResult, "toSheet">): ProjectResult {
  const dot = (a: number[], b: number[]) =>
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  return {
    ...v,
    toSheet: (p: Point3) => {
      const d = [p[0] - v.origin[0], p[1] - v.origin[1], p[2] - v.origin[2]];
      return [
        v.at[0] + v.scale * dot(d, v.right),
        v.at[1] + v.scale * dot(d, v.up),
      ];
    },
  };
}

function normalizeParts(
  t: PartTarget | null | undefined,
): string | string[] | PartSelector | null {
  return t == null ? null : normalizePartTarget(t);
}

function partRef(p: PartRef): string {
  return typeof p === "string" ? p : p.id;
}

function normalizePartTarget(t: PartTarget): string | string[] | PartSelector {
  if (typeof t === "string") return t;
  if (Array.isArray(t)) return t.map(partRef);
  if (t && typeof t === "object" && "id" in t) return (t as PartJson).id;
  return t as PartSelector;
}

/** Entity objects inside profiles are sent as ids. */
function normalizeLoop(l: ProfileLoop): ProfileLoop {
  return l && typeof l === "object" && !Array.isArray(l) && "id" in l
    ? (l as EntityJson).id
    : l;
}

function normalizeProfile(p: Profile): Profile {
  if (p && typeof p === "object" && !Array.isArray(p) && "outer" in p)
    return {
      outer: normalizeLoop(p.outer),
      ...(p.holes ? { holes: p.holes.map(normalizeLoop) } : {}),
    };
  return normalizeLoop(p as ProfileLoop);
}

function normalizeTool(t: Tool): Tool {
  if ("part" in t) return { part: partRef(t.part) };
  if ("extrude" in t) return { ...t, extrude: normalizeProfile(t.extrude) };
  if ("revolve" in t) return { ...t, revolve: normalizeProfile(t.revolve) };
  if ("sweep" in t) return { ...t, sweep: normalizeProfile(t.sweep) };
  return {
    ...t,
    loft: t.loft.map((s) => ({ ...s, profile: normalizeProfile(s.profile) })),
  };
}

export interface CadApiBackends {
  /** Dispatch to cad_call. Returns JSON result string. */
  call: (method: string, argsJson: string) => string;
  /** RPC dispatch (save, loadDwg, exec). Returns JSON result string or Promise. */
  rpc: (method: string, argsJson: string) => string | Promise<string>;
  /** Read file contents. Null if unavailable. */
  readFile: ((path: string) => string) | null;
  /**
   * The host side of `cad.use`: for a library name, the JSON text
   * `{id, code}` (the file `lib/<name>.js` and its text) or `{error}`.
   * Null or absent if unavailable.
   */
  library?: LibraryBackend | null;
}

export type LibraryBackend = (name: string) => string | Promise<string>;

/** `cad.use`: a promise of what the library's script returns. */
export type CadUse = <T = Record<string, any>>(name: string) => Promise<T>;

/**
 * `cad.use(name)`: run the script `lib/<name>.js` once and give back what it
 * returns. A library is a script like any other (an async function body), so
 * it ends with `return { ... }` and loads other libraries with `cad.use`.
 * Every later call for the same name gets the same result.
 */
export function createLibraryLoader(load: LibraryBackend): CadUse {
  // Indirect eval: the library sees the global scope, never this one.
  // biome-ignore lint/security/noGlobalEval: running library code is the point of cad.use
  const indirectEval = eval;
  const loaded = new Map<string, Promise<unknown>>();

  async function run(name: string): Promise<unknown> {
    const reply = JSON.parse(await load(name)) as {
      id: string;
      code: string;
      error?: string;
    };
    if (reply.error) throw new Error(`cad.use: ${reply.error}`);
    // The wrapper shares the library's first line, so line numbers in
    // errors are the file's own; sourceURL puts its name in stack traces.
    const body = indirectEval(
      `(async () => {"use strict"; ${reply.code}\n})\n//# sourceURL=${reply.id}`,
    ) as () => Promise<unknown>;
    return body();
  }

  return ((name: string) => {
    let library = loaded.get(name);
    if (!library) {
      library = run(name);
      loaded.set(name, library);
    }
    return library;
  }) as CadUse;
}

function unavailableUse<T = Record<string, any>>(_name: string): Promise<T> {
  return Promise.reject(new Error("cad.use needs the CodeCAD server"));
}

export function buildCadApi({
  call: rawCall,
  rpc: rawRpc,
  readFile: rawReadFile,
  library,
}: CadApiBackends) {
  function call(method: string, args: Record<string, unknown> = {}): unknown {
    return parseResult(rawCall(method, JSON.stringify(args)));
  }

  function rpc(method: string, args: Record<string, unknown> = {}): unknown {
    const raw = rawRpc(method, JSON.stringify(args));
    if (typeof raw === "string") return parseResult(raw);
    return (raw as Promise<string>).then(parseResult);
  }

  const use: CadUse = library ? createLibraryLoader(library) : unavailableUse;

  return {
    // ── Observe ───────────────────────────────────────────────────────
    describe: () =>
      call("describe") as {
        entities: number;
        layers: string[];
        /** Attribute keys in use, with how many entities carry each. */
        attrs: Record<string, number>;
        /** Placed solid parts and part definitions. */
        parts: number;
        partDefs: number;
        /** Attribute keys in use on parts, with how many parts carry each. */
        partAttrs: Record<string, number>;
        /** The kernel that answers part geometry ("arris 0.11.1"), or null. */
        solidKernel: string | null;
      },
    /** Entities matching a selector (all without one); expand: block children too. */
    entities: (opts?: Selector) =>
      call("entities", { ...opts }) as EntityJson[],
    entity: (id: string) => call("entity", { id }) as EntityJson,
    children: (id: string | EntityJson) =>
      call("children", {
        id: typeof id === "string" ? id : id.id,
      }) as EntityJson[],

    // Spatial queries
    /**
     * Entities within `r` of a point, nearest first, each with `distance`
     * (exact for lines, arcs, circles, polylines and fills; `approx: true`
     * where a bounding box stood in, e.g. text).
     */
    near: (pt: Point2, r: number, opts: { select?: Selection } = {}) =>
      call("near", {
        point: pt,
        radius: r,
        ...(opts.select !== undefined
          ? { select: normalizeSelection(opts.select) }
          : {}),
      }) as (EntityJson & { distance: number; approx?: boolean })[],
    /** Entities whose bounding box overlaps ("intersect") or lies within ("contain") the rectangle. */
    inBounds: (
      rect: { min: Point2; max: Point2 },
      mode?: "intersect" | "contain",
      opts: { select?: Selection } = {},
    ) =>
      call("inBounds", {
        rect,
        mode,
        ...(opts.select !== undefined
          ? { select: normalizeSelection(opts.select) }
          : {}),
      }) as EntityJson[],

    // Set queries: whole selections in one call, exact geometry
    /**
     * Every pair of an entity of a and one of b (b null: pairs within a)
     * strictly closer than `distance`, with the distance and closest points.
     */
    pairsWithin: (
      a: Selection,
      b: Selection | undefined,
      distance: number,
      opts: PairOpts = {},
    ) =>
      call("pairsWithin", {
        a: normalizeSelection(a),
        b: normalizeSelection(b),
        distance,
        ...opts,
      }) as SetResult<ClosePair>,
    /**
     * Every pair whose outlines meet, with the meeting points. areas: closed
     * shapes count as filled, so a shape inside another is a pair too.
     */
    overlapping: (
      a: Selection,
      b?: Selection,
      opts: PairOpts & { areas?: boolean } = {},
    ) =>
      call("overlapping", {
        a: normalizeSelection(a),
        b: normalizeSelection(b),
        ...opts,
      }) as SetResult<ContactPair>,
    /**
     * Entities of a that lie inside a region: a rectangle, a ring of points,
     * or each closed entity of a selection (circles, closed polylines,
     * fills). Touching the boundary counts unless strict.
     */
    inside: (
      a: Selection,
      region: InsideRegion,
      opts: PairOpts & { strict?: boolean } = {},
    ) =>
      call("inside", {
        a: normalizeSelection(a),
        region: normalizeRegion(region),
        ...opts,
      }) as SetResult<InsidePair>,
    /** For each entity of a, its k nearest in b (b null: within a), optionally only within a distance. */
    nearest: (
      a: Selection,
      b?: Selection,
      opts: PairOpts & { k?: number; within?: number } = {},
    ) =>
      call("nearest", {
        a: normalizeSelection(a),
        b: normalizeSelection(b),
        ...opts,
      }) as SetResult<ClosePair>,

    // Attributes
    /** Merge attributes into the targets (null removes a key; replace: start empty). All or nothing. */
    setAttrs: (
      t: Target,
      attrs: Record<string, AttrValue | null>,
      opts: { replace?: boolean } = {},
    ) =>
      call("setAttrs", { target: normalizeTarget(t), attrs, ...opts }) as {
        updated: string[];
      },
    removeAttrs: (t: Target, keys: string[]) =>
      call("removeAttrs", { target: normalizeTarget(t), keys }) as {
        updated: string[];
      },
    /** One entity's attributes ({} when none). */
    attrs: (id: EntityRef) => call("attrs", { id: ref(id) }) as Attrs,
    /** Attributes of a block definition, or (shape: i) of one of its shapes. Returns the new set. */
    setBlockAttrs: (
      name: string,
      attrs: Record<string, AttrValue | null>,
      opts: { replace?: boolean; shape?: number } = {},
    ) => call("setBlockAttrs", { name, attrs, ...opts }) as Attrs,
    blockAttrs: (name: string) =>
      call("blockAttrs", { name }) as { attrs: Attrs; shapes: Attrs[] },

    // ── Solid parts (docs/api-surface.md, "Solid parts") ──────────────
    // A part places a definition (a feature tree: shape only) in the world
    // and carries attributes. Mm, degrees, Z up. Modifying a part changes
    // its definition, so every part sharing it changes too. Where a kernel
    // runs (server scripts) every change is evaluated first and a refusal
    // throws "<call>: part p_3 ("name"), feature 4 (hole): <reason>" and
    // changes nothing; elsewhere geometry queries throw "this build has no
    // solid kernel".
    /** New part: `profile` on its plane, extruded `distance` along the normal (negative: against it). */
    extrude: (
      profile: Profile,
      opts: SketchOpts & NewPartOpts & { distance: number; taper?: number },
    ) =>
      call("extrude", {
        ...opts,
        profile: normalizeProfile(profile),
      }) as PartJson,
    /** New part: `profile` turned about `axis` (two points in profile coordinates) by `angle` (default 360). */
    revolve: (
      profile: Profile,
      opts: SketchOpts &
        NewPartOpts & { axis: [Point2, Point2]; angle?: number },
    ) =>
      call("revolve", {
        ...opts,
        profile: normalizeProfile(profile),
      }) as PartJson,
    /** New part: `profile` carried along `path`, which starts at the plane's origin along its normal. */
    sweep: (
      profile: Profile,
      path: PathSeg[],
      opts: SketchOpts & NewPartOpts = {},
    ) =>
      call("sweep", {
        ...opts,
        profile: normalizeProfile(profile),
        path,
      }) as PartJson,
    /** New part: a solid through the sections in order (ruled: straight between them). */
    loft: (
      sections: LoftSection[],
      opts: NewPartOpts & { ruled?: boolean } = {},
    ) =>
      call("loft", {
        ...opts,
        sections: sections.map((s) => ({
          ...s,
          profile: normalizeProfile(s.profile),
        })),
      }) as PartJson,
    /** Subtract a tool body (or another part, as placed now) from a part. */
    cut: (part: PartRef, tool: Tool, opts: FrameOpts = {}) =>
      call("cut", {
        target: partRef(part),
        tool: normalizeTool(tool),
        ...opts,
      }) as PartJson,
    /** Add a tool body (or another part) to a part. */
    union: (part: PartRef, tool: Tool, opts: FrameOpts = {}) =>
      call("union", {
        target: partRef(part),
        tool: normalizeTool(tool),
        ...opts,
      }) as PartJson,
    /** Keep only what a part and a tool body share. */
    intersect: (part: PartRef, tool: Tool, opts: FrameOpts = {}) =>
      call("intersect", {
        target: partRef(part),
        tool: normalizeTool(tool),
        ...opts,
      }) as PartJson,
    /** A hole from `at` along `dir` (default [0, 0, -1]): depth in mm or "through" (default); cbore: [dia, depth]. */
    hole: (
      part: PartRef,
      opts: FrameOpts & {
        at: Point3;
        dir?: Point3;
        dia: number;
        depth?: number | "through";
        cbore?: [number, number];
      },
    ) => call("hole", { target: partRef(part), ...opts }) as PartJson,
    /** Round edges (exact); fallback "visual" is recorded for renderers, the B-rep stays exact or the call fails. */
    fillet3d: (
      part: PartRef,
      opts: FrameOpts & {
        edges: EdgeSel;
        radius: number;
        fallback?: "error" | "visual";
      },
    ) => call("fillet3d", { target: partRef(part), ...opts }) as PartJson,
    /** Bevel edges by d (d2: the other face; kernels may only do d2 = d). */
    chamfer3d: (
      part: PartRef,
      opts: FrameOpts & { edges: EdgeSel; d: number; d2?: number },
    ) => call("chamfer3d", { target: partRef(part), ...opts }) as PartJson,
    /** Move parts by [dx, dy, dz]. */
    move3d: (t: PartTarget, by: Point3) =>
      call("move3d", { target: normalizePartTarget(t), by }) as {
        moved: string[];
      },
    /** Rotate parts by `angle` degrees about the line through `about` (default origin) along `axis` (default Z). */
    rotate3d: (
      t: PartTarget,
      opts: { angle: number; axis?: Point3; about?: Point3 },
    ) =>
      call("rotate3d", { target: normalizePartTarget(t), ...opts }) as {
        rotated: string[];
      },
    /**
     * Placed copies that share the definition (no re-evaluation). Each copy
     * is the previous one rotated, then moved; `count` copies in a row make
     * an array. unique: each copy gets a definition of its own. attrs are
     * merged into the source's. One part in: one part out; else an array.
     */
    copy3d: (
      t: PartTarget,
      opts: {
        move?: Point3;
        rotate?: { angle: number; axis?: Point3; about?: Point3 };
        count?: number;
        attrs?: Record<string, AttrValue | null>;
        unique?: boolean;
      } = {},
    ) =>
      call("copy3d", { target: normalizePartTarget(t), ...opts }) as
        | PartJson
        | PartJson[],
    /**
     * Mirror parts across the plane through `point` (default the origin)
     * with `normal`. A mirror image is not a rigid motion, so a mirrored part
     * places a mirrored definition, made once per definition and shared by
     * every part mirrored from it (a copy, not a live link: later edits of
     * the original do not reach it). In place by default ({mirrored,
     * defsMade, defsRemoved}); copy: keep the originals and return the
     * images (one part in: one part out).
     */
    mirror3d: (
      t: PartTarget,
      opts: {
        point?: Point3;
        normal: Point3;
        copy?: boolean;
        attrs?: Record<string, AttrValue | null>;
      },
    ) =>
      call("mirror3d", { target: normalizePartTarget(t), ...opts }) as
        | PartJson
        | PartJson[]
        | { mirrored: string[]; defsMade: string[]; defsRemoved: string[] },
    /** Remove parts; definitions no part uses any more (directly or as a tool) go too. */
    remove3d: (t: PartTarget) =>
      call("remove3d", { target: normalizePartTarget(t) }) as {
        removed: string[];
        defsRemoved: string[];
      },
    /** Merge attributes into parts (null removes a key; replace: start empty). All or nothing. */
    setPartAttrs: (
      t: PartTarget,
      attrs: Record<string, AttrValue | null>,
      opts: { replace?: boolean } = {},
    ) =>
      call("setPartAttrs", {
        target: normalizePartTarget(t),
        attrs,
        ...opts,
      }) as { updated: string[] },
    removePartAttrs: (t: PartTarget, keys: string[]) =>
      call("removePartAttrs", { target: normalizePartTarget(t), keys }) as {
        updated: string[];
      },
    /**
     * Parts matching a selector (all without one), with placement,
     * attributes and (geometry, default true; needs a kernel) the tight box
     * in the world and in the part's frame, the volume and the centroid.
     */
    parts: (sel: PartSelector & { geometry?: boolean } = {}) =>
      call("parts", { ...sel }) as PartJson[],
    part: (id: PartRef, opts: { geometry?: boolean } = {}) =>
      call("part", { id: partRef(id), ...opts }) as PartJson,
    /** A definition's whole feature tree (by "d_3", or a part's). */
    partDef: (def: string | PartJson) =>
      call("partDef", { def: typeof def === "string" ? def : def.def }) as {
        id: string;
        name: string;
        features: unknown[];
        parts: string[];
        tools: string[];
      },

    // Set queries between parts: the 2D set queries' names plus "3d", whole
    // selections in one call, on the exact solids (needs a kernel). A pair
    // the kernel cannot answer is listed in `skipped`.
    /**
     * Every pair of a part of a and one of b (b null: pairs within a) that
     * shares volume, with the shared volume (interference). A shared solid
     * thinner than `tolerance` (mm, default 1e-6) is a touch; touching: also
     * list touching pairs (volume 0).
     */
    overlapping3d: (
      a: PartTarget | null,
      b?: PartTarget | null,
      opts: PartPairOpts & { tolerance?: number; touching?: boolean } = {},
    ) =>
      call("overlapping3d", {
        a: normalizeParts(a),
        b: normalizeParts(b),
        ...opts,
      }) as PartSetResult<OverlapPair>,
    /** Every pair strictly closer than `distance` (clearance), with the distance and closest points. */
    pairsWithin3d: (
      a: PartTarget | null,
      b: PartTarget | null | undefined,
      distance: number,
      opts: PartPairOpts = {},
    ) =>
      call("pairsWithin3d", {
        a: normalizeParts(a),
        b: normalizeParts(b),
        distance,
        ...opts,
      }) as PartSetResult<GapPair>,
    /**
     * The parts of a inside a box {min, max} (touching its boundary counts
     * unless strict), or inside the solid of each part of a selection.
     */
    inside3d: (
      a: PartTarget | null,
      region: { min: Point3; max: Point3 } | PartTarget,
      opts: PartPairOpts & { strict?: boolean } = {},
    ) =>
      call("inside3d", {
        a: normalizeParts(a),
        region:
          region &&
          typeof region === "object" &&
          "min" in region &&
          "max" in region
            ? region
            : normalizeParts(region as PartTarget),
        ...opts,
      }) as PartSetResult<PartInsidePair>,
    /** The distance between two parts and the closest points (0 when they touch or overlap). */
    distance3d: (a: PartRef, b: PartRef) =>
      call("distance3d", { a: partRef(a), b: partRef(b) }) as {
        a: string;
        b: string;
        distance: number;
        pa: Point3;
        pb: Point3;
      },

    // Projected views (needs a kernel): a block of exact 2D lines, arcs,
    // circles and ellipses per view, visible and hidden, with an insert.
    /**
     * Projects the parts in a view: one block (true size) and one insert
     * at `at` scaled by `scale`. Each entity carries partId, edge
     * ("visible" | "hidden") and curve ("edge" | "silhouette").
     */
    project: (opts: ProjectOpts) =>
      withToSheet(
        call("project", {
          ...opts,
          ...(opts.parts !== undefined
            ? { parts: normalizeParts(opts.parts) }
            : {}),
        }) as Omit<ProjectResult, "toSheet">,
      ),
    /** Where 3D points land on the sheet in the view these options describe. */
    viewPoints: (opts: ViewOpts & { points: Point3[] }) =>
      call("viewPoints", {
        ...opts,
        ...(opts.parts !== undefined
          ? { parts: normalizeParts(opts.parts) }
          : {}),
      }) as Point2[],

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
     * {type:"text",text,at,height?,rotation?,align?,valign?}; each may set
     * layer/color and attrs (the shape's own attributes). opts.attrs: the
     * definition's attributes. A placed child carries definition, then
     * shape, then insert attributes (a later one wins).
     */
    defineBlock: (
      name: string,
      shapes: Record<string, unknown>[],
      opts: {
        insert_point?: Point2;
        default_layer?: string;
        attrs?: Attrs;
      } = {},
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
        /** Attributes of the insert. */
        attrs?: Attrs;
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
    /** Export the entities() view as JSON (one way; see saveJson for a file that loads back). */
    save: (path = "cadview-output.json") => rpc("save", { path }),
    /**
     * Save as DWG. Entity attributes are kept as extended data; the result's
     * `attrs` says what was kept and what DWG cannot hold (block definition
     * attributes).
     */
    saveDwg: (path = "output.dwg") => rpc("saveDwg", { path }),
    /** The whole document as lossless JSON (attributes, blocks, ids, styles). */
    saveJson: (path = "document.json") => rpc("saveJson", { path }),
    /** Replace the document with one saved by saveJson. */
    loadJson: (path: string) => rpc("loadJson", { path }),
    savePdf: (path = "output.pdf", opts: SavePdfOptions = {}) =>
      rpc("savePdf", { ...opts, path }),
    /**
     * Render to a PNG on the server: the drawing (GPU, else CPU), or with
     * `view` (or mode: "3d") the document's solid parts, shaded (GPU).
     */
    render: (opts: RenderOptions = {}) =>
      rpc("render", { out: "render.png", ...opts }) as
        | RenderResult
        | Promise<RenderResult>,
    /**
     * Write the document's parts as a prebaked 3D scene (server, needs the
     * kernel): meshes, exact boxes and volumes, placements, attributes and
     * the given render options (view, colours, materials, light), for a
     * viewer that has no kernel. `chord` defaults to a quarter pixel at
     * 1280 px.
     */
    bake3d: (opts: Bake3dOptions = {}) =>
      rpc("bake3d", { out: "scene.bake3d", ...opts }) as
        | Bake3dResult
        | Promise<Bake3dResult>,
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

    // ── Script libraries ──────────────────────────────────────────────
    /**
     * Load a script library once and return its exports:
     * `const draft = await cad.use("drafting")` loads `lib/drafting.js`.
     * A relative path (`"./parts.js"`) is relative to the script's folder.
     */
    use,
  };
}

export type CadApi = ReturnType<typeof buildCadApi>;
