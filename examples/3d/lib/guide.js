// Construction guides: one drawing per assembly step, from a model's parts
// and an assembly order, loaded with
//
//   const guide = await cad.use("guide");
//   guide.setup();
//   const { view } = guide.chooseView(fresh, built, { prev });
//   guide.drawStep({ number, title, fresh, built, view, labels, joints, rows, instruction, details }, panel);
//
// Plain script code over the public cad.* calls, for any model made of
// parts. It knows nothing about what is built: the script hands it each
// step's parts, the label text, the fixings and where they go, the words.
//
// How a step is drawn:
//
// - The corner. Of the four isometric corners, the one that leaves the new
//   parts least covered: points in each new part's box, each covered when
//   the ray from it toward the eye passes through another built part's box
//   (chooseView). The previous step's corner is kept unless another is
//   clearly better.
// - The view. cad.project of every part built so far, read back with
//   cad.children and redrawn as plain entities: the new parts' visible
//   edges strong, the earlier parts' visible edges quiet, hidden edges only
//   for a new part that is mostly hidden. The parts are told apart by the
//   `partId` attribute every projected entity carries.
// - Labels. One per label the script gives, with a leader to a visible edge
//   of one of its parts. Placed on a grid of candidate positions inside the
//   drawing area: never on the drawing (the outline of every part on the
//   sheet, rasterised to a mask, plus the fixing markers and detail boxes),
//   never on another label; the cost is the leader's length plus penalties
//   for crossing another leader or label. Placed one by one, then each once
//   more with all the others fixed.
// - Fixings. A marker where each joint of the step sits (a 3D point the
//   script gives, mapped with cad.viewPoints), one marker shape per kind of
//   joint, and a table of the fixings under the view.
// - Details. Larger views of a few parts, cut to a window in script (lines
//   exactly, curves by their parameter), in free corners of the panel.
//
// Sheet units are paper millimetres; a view's `scale` is paper mm per
// model mm.

const sheet = await cad.use("sheet");
const { ACI } = sheet;
const { min, max, abs, hypot, ceil, floor, sqrt, PI, cos, sin } = Math;

// ── Layers ───────────────────────────────────────────────────────────
const LAYERS = {
  G_OLD: { color: 251, lineweight: 0.18 },                  // built before: quiet (dark grey)
  G_OLD_HIDDEN: { color: 251, lineweight: 0.13 },           // read back, never drawn
  G_NEW: { color: ACI.orange, lineweight: 0.5 },             // added in this step
  G_NEW_HIDDEN: { color: ACI.orange, lineweight: 0.25, linetype: "GUIDE_HIDDEN" },
  G_FIX: { color: ACI.cyan, lineweight: 0.25 },              // fixing markers
  G_LEADER: { color: ACI.white, lineweight: 0.18 },
  G_DETAIL: { color: ACI.lightgray, lineweight: 0.25 },
  G_NOTE: { color: ACI.lightgray, lineweight: 0.18 },
};

function setup() {
  sheet.setup();
  cad.addLinetype("GUIDE_HIDDEN", [1.2, 0.8], { description: "Hidden __ __ __" });
  for (const [name, o] of Object.entries(LAYERS)) cad.addLayer(name, o);
  return { layers: Object.keys(LAYERS) };
}

// ── Small geometry ───────────────────────────────────────────────────
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const corners = (b) => {
  const out = [];
  for (const x of [b.min[0], b.max[0]]) for (const y of [b.min[1], b.max[1]]) for (const z of [b.min[2], b.max[2]]) out.push([x, y, z]);
  return out;
};
const box2 = (pts) => {
  const b = { min: [Infinity, Infinity], max: [-Infinity, -Infinity] };
  for (const p of pts) for (let i = 0; i < 2; i++) { b.min[i] = min(b.min[i], p[i]); b.max[i] = max(b.max[i], p[i]); }
  return b;
};
const boxesMeet = (a, b, pad = 0) =>
  a.min[0] < b.max[0] + pad && a.max[0] > b.min[0] - pad && a.min[1] < b.max[1] + pad && a.max[1] > b.min[1] - pad;
const inBox = (p, b) => p[0] >= b.min[0] && p[0] <= b.max[0] && p[1] >= b.min[1] && p[1] <= b.max[1];

// Segments p-q and r-s cross (proper or touching).
function segsCross(p, q, r, s) {
  const o = (a, b, c) => Math.sign((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]));
  return o(p, q, r) !== o(p, q, s) && o(r, s, p) !== o(r, s, q);
}
// Segment p-q meets box b.
function segMeetsBox(p, q, b) {
  if (inBox(p, b) || inBox(q, b)) return true;
  const c = [[b.min[0], b.min[1]], [b.max[0], b.min[1]], [b.max[0], b.max[1]], [b.min[0], b.max[1]]];
  for (let i = 0; i < 4; i++) if (segsCross(p, q, c[i], c[(i + 1) % 4])) return true;
  return false;
}

// Convex hull (monotone chain) of 2D points.
function hull(pts) {
  const p = [...pts].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  if (p.length < 3) return p;
  const cr = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lo = [], up = [];
  for (const q of p) { while (lo.length >= 2 && cr(lo[lo.length - 2], lo[lo.length - 1], q) <= 0) lo.pop(); lo.push(q); }
  for (const q of p.reverse()) { while (up.length >= 2 && cr(up[up.length - 2], up[up.length - 1], q) <= 0) up.pop(); up.push(q); }
  return lo.slice(0, -1).concat(up.slice(0, -1));
}

// ── Views ────────────────────────────────────────────────────────────
// A named view's frame (right, up, toward the eye), from cad.viewPoints of
// the unit vectors: the view's own mapping, so any name cad.project knows
// works here.
const frames = new Map();
function frameOf(view) {
  if (!frames.has(view)) {
    const p = cad.viewPoints({ view, origin: [0, 0, 0], at: [0, 0], scale: 1, points: [[1, 0, 0], [0, 1, 0], [0, 0, 1]] });
    const right = [p[0][0], p[1][0], p[2][0]], up = [p[0][1], p[1][1], p[2][1]];
    frames.set(view, { right, up, from: cross(right, up) });
  }
  return frames.get(view);
}

// Does the ray from p toward the eye (direction d) pass through box b?
function rayHits(p, d, b) {
  let t0 = 1e-6, t1 = Infinity;
  for (let i = 0; i < 3; i++) {
    if (abs(d[i]) < 1e-12) {
      if (p[i] <= b.min[i] || p[i] >= b.max[i]) return false;
      continue;
    }
    let a = (b.min[i] - p[i]) / d[i], c = (b.max[i] - p[i]) / d[i];
    if (a > c) [a, c] = [c, a];
    t0 = max(t0, a);
    t1 = min(t1, c);
    if (t0 >= t1) return false;
  }
  return true;
}

// How much of the new parts' boxes the other built parts cover in a view:
// nine points in each new part's world box (its centre and a 2 x 2 x 2
// lattice), each covered when the ray from it toward the eye passes
// through another built part's box (shrunk 1 mm, so a neighbour that only
// touches does not count). The mean over the new parts, 0 (all in view)
// to 1 (all covered). Boxes, not solids: an angled member's box is larger
// than the member, so it covers more than the member does.
function coverage(fresh, built, view) {
  const { from } = frameOf(view);
  const lattice = [[0.5, 0.5, 0.5]];
  for (const fx of [0.2, 0.8]) for (const fy of [0.2, 0.8]) for (const fz of [0.2, 0.8]) lattice.push([fx, fy, fz]);
  const shrunk = built.map((q) => ({ id: q.id, min: q.bbox.min.map((v) => v + 1), max: q.bbox.max.map((v) => v - 1) }))
    .filter((b) => b.min.every((v, i) => v < b.max[i]));
  let sum = 0;
  for (const p of fresh) {
    const b = p.bbox;
    const others = shrunk.filter((q) => q.id !== p.id);
    let hit = 0, n = 0;
    for (const f of lattice) {
      const s = [0, 1, 2].map((i) => b.min[i] + f[i] * (b.max[i] - b.min[i]));
      n++;
      if (others.some((q) => rayHits(s, from, q))) hit++;
    }
    sum += hit / n;
  }
  return fresh.length ? sum / fresh.length : 0;
}

// The corner that shows the new parts best. Keeps the previous step's
// corner unless another is better by more than `keep` (so the reader is not
// turned round for a small gain).
function chooseView(fresh, built, { views = ["iso", "iso-sw", "iso-ne", "iso-nw"], prev = null, keep = 0.08 } = {}) {
  const scores = Object.fromEntries(views.map((v) => [v, coverage(fresh, built, v)]));
  let best = views.reduce((a, b) => (scores[b] < scores[a] - 1e-12 ? b : a));
  if (prev && scores[prev] !== undefined && scores[prev] <= scores[best] + keep) best = prev;
  return { view: best, scores };
}

// The extent of parts' boxes in a view (model mm, origin at 0).
function extentOf(parts, view) {
  const pts = cad.viewPoints({ view, origin: [0, 0, 0], at: [0, 0], scale: 1, points: parts.flatMap((p) => corners(p.bbox)) });
  return box2(pts);
}

const SCALES = [2, 2.5, 3, 4, 5, 6, 7.5, 8, 10, 12.5, 15, 20, 25, 30, 35, 40, 45, 50, 60, 70, 75, 80, 90, 100, 125, 150, 200];
// The largest 1:n from SCALES at which `ext` fits w x h (paper mm).
function fitScale(ext, w, h, { most = 1 / 2 } = {}) {
  const ew = ext.max[0] - ext.min[0], eh = ext.max[1] - ext.min[1];
  for (const n of SCALES) if (1 / n <= most && ew / n <= w && eh / n <= h) return { scale: 1 / n, n };
  const n = SCALES[SCALES.length - 1];
  return { scale: 1 / n, n };
}

// ── Redrawing projected entities ─────────────────────────────────────
function ellipsePoint(e, t) {
  const [mx, my] = e.edges.major_axis, r = e.edges.minor_ratio;
  return [e.center[0] + mx * cos(t) - r * my * sin(t), e.center[1] + my * cos(t) + r * mx * sin(t)];
}
function ellipseRange(e) {
  let { start: a, end: b } = e.edges;
  if (b <= a) b += 2 * PI;
  return [a, b];
}
// Points along a child entity (for masks, hulls and clipping).
function samplePoints(e, step = 1) {
  if (e.type === "line") {
    const n = max(1, ceil(hypot(e.end[0] - e.start[0], e.end[1] - e.start[1]) / step));
    return Array.from({ length: n + 1 }, (_, i) => [e.start[0] + ((e.end[0] - e.start[0]) * i) / n, e.start[1] + ((e.end[1] - e.start[1]) * i) / n]);
  }
  if (e.type === "circle" || e.type === "arc") {
    let a = e.type === "arc" ? (e.from * PI) / 180 : 0, b = e.type === "arc" ? (e.to * PI) / 180 : 2 * PI;
    if (b <= a) b += 2 * PI;
    const n = max(8, ceil(((b - a) * e.radius) / step));
    return Array.from({ length: n + 1 }, (_, i) => { const t = a + ((b - a) * i) / n; return [e.center[0] + e.radius * cos(t), e.center[1] + e.radius * sin(t)]; });
  }
  if (e.type === "ellipse") {
    const [a, b] = ellipseRange(e);
    const n = max(8, ceil(((b - a) * hypot(...e.edges.major_axis)) / step));
    return Array.from({ length: n + 1 }, (_, i) => ellipsePoint(e, a + ((b - a) * i) / n));
  }
  if (e.points) return e.points;
  return [];
}
const lengthOf = (e) => {
  if (e.type === "line") return hypot(e.end[0] - e.start[0], e.end[1] - e.start[1]);
  const p = samplePoints(e, 0.5);
  let s = 0;
  for (let i = 1; i < p.length; i++) s += hypot(p[i][0] - p[i - 1][0], p[i][1] - p[i - 1][1]);
  return s;
};

const midOf = (e) => {
  if (e.type === "line") return [(e.start[0] + e.end[0]) / 2, (e.start[1] + e.end[1]) / 2];
  const p = samplePoints(e, 0.5);
  return p[floor(p.length / 2)];
};

// Draw one child entity again as an ordinary entity on `layer`. Lines,
// arcs and circles stay exact. There is no call to add an ellipse, so an
// ellipse is drawn as a cubic spline fitted through 25 points of its arc
// (a finding: see docs/3d-examples.md, example 11).
function redraw(e, layer, attrs) {
  const o = { layer, ...(attrs ? { attrs } : {}) };
  if (e.type === "line") return cad.addLine(e.start, e.end, o);
  if (e.type === "circle") return cad.addCircle(e.center, e.radius, o);
  if (e.type === "arc") return cad.addArc(e.center, e.radius, { ...o, from: e.from, to: e.to });
  if (e.type === "ellipse") {
    const [a, b] = ellipseRange(e);
    const pts = Array.from({ length: 25 }, (_, i) => ellipsePoint(e, a + ((b - a) * i) / 24));
    return cad.addSpline(pts, { ...o, fit: true });
  }
  if (e.type === "spline") return cad.addSpline(e.points, { ...o, fit: false, degree: e.edges?.degree ?? 3 });
  return null;
}

// The pieces of a child entity inside rectangle r: [entity-like] pieces to
// redraw. Lines are clipped exactly (Liang-Barsky); a curve is split where
// its samples leave the rectangle and each inside run is redrawn (arcs as
// arcs, others as fitted splines).
function clip(e, r) {
  if (e.type === "line") {
    const [x0, y0] = e.start, dx = e.end[0] - x0, dy = e.end[1] - y0;
    let t0 = 0, t1 = 1;
    for (const [p, q] of [[-dx, x0 - r.min[0]], [dx, r.max[0] - x0], [-dy, y0 - r.min[1]], [dy, r.max[1] - y0]]) {
      if (abs(p) < 1e-12) { if (q < 0) return []; continue; }
      const t = q / p;
      if (p < 0) t0 = max(t0, t); else t1 = min(t1, t);
      if (t0 > t1) return [];
    }
    return [{ ...e, start: [x0 + t0 * dx, y0 + t0 * dy], end: [x0 + t1 * dx, y0 + t1 * dy] }];
  }
  const pts = samplePoints(e, 0.25);
  const runs = [];
  let cur = null;
  pts.forEach((p, i) => {
    if (inBox(p, r)) { if (!cur) { cur = { from: i, to: i }; runs.push(cur); } else cur.to = i; } else cur = null;
  });
  if (runs.length === 1 && runs[0].from === 0 && runs[0].to === pts.length - 1) return [e];
  const n = pts.length - 1;
  return runs.filter((u) => u.to > u.from).map((u) => {
    if (e.type === "arc" || e.type === "circle") {
      let a = e.type === "arc" ? e.from : 0, b = e.type === "arc" ? e.to : 360;
      if (b <= a) b += 360;
      return { type: "arc", center: e.center, radius: e.radius, from: a + ((b - a) * u.from) / n, to: a + ((b - a) * u.to) / n };
    }
    const run = pts.slice(u.from, u.to + 1);
    const k = max(2, min(24, run.length - 1));
    return { type: "spline-fit", points: Array.from({ length: k + 1 }, (_, i) => run[Math.round(((run.length - 1) * i) / k)]) };
  });
}
function redrawPiece(e, layer) {
  if (e.type === "spline-fit") return cad.addSpline(e.points, { layer, fit: true });
  return redraw(e, layer);
}

// ── A mask of the drawing on a grid ──────────────────────────────────
function makeGrid(rect, cell = 1) {
  const nx = ceil((rect.max[0] - rect.min[0]) / cell), ny = ceil((rect.max[1] - rect.min[1]) / cell);
  const a = new Uint8Array(nx * ny);
  const g = {
    rect, cell, nx, ny, a,
    idx: (p) => {
      const i = floor((p[0] - rect.min[0]) / cell), j = floor((p[1] - rect.min[1]) / cell);
      return i >= 0 && j >= 0 && i < nx && j < ny ? j * nx + i : -1;
    },
    mark(p) { const k = g.idx(p); if (k >= 0) a[k] = 1; },
    // Fill a convex polygon (cells whose centre is inside).
    fill(poly) {
      if (poly.length < 3) { for (const p of poly) g.mark(p); return; }
      const b = box2(poly);
      const n = poly.length;
      const i0 = max(0, floor((b.min[0] - rect.min[0]) / cell)), i1 = min(nx - 1, floor((b.max[0] - rect.min[0]) / cell));
      const j0 = max(0, floor((b.min[1] - rect.min[1]) / cell)), j1 = min(ny - 1, floor((b.max[1] - rect.min[1]) / cell));
      for (let j = j0; j <= j1; j++) {
        const py = rect.min[1] + (j + 0.5) * cell;
        for (let i = i0; i <= i1; i++) {
          const px = rect.min[0] + (i + 0.5) * cell;
          let ok = true;
          for (let k = 0; k < n && ok; k++) {
            const a0 = poly[k], a1 = poly[k + 1 === n ? 0 : k + 1];
            if ((a1[0] - a0[0]) * (py - a0[1]) - (a1[1] - a0[1]) * (px - a0[0]) < -1e-9) ok = false;
          }
          if (ok) a[j * nx + i] = 1;
        }
      }
      for (const p of poly) g.mark(p);
    },
    fillBox(b) {
      for (let y = b.min[1]; y <= b.max[1]; y += cell / 2) for (let x = b.min[0]; x <= b.max[0]; x += cell / 2) g.mark([x, y]);
    },
    // Summed-area table, for counting marked cells in a box at once.
    integral() {
      const s = new Float64Array((nx + 1) * (ny + 1));
      for (let j = 0; j < ny; j++) {
        let row = 0;
        for (let i = 0; i < nx; i++) {
          row += a[j * nx + i];
          s[(j + 1) * (nx + 1) + i + 1] = s[j * (nx + 1) + i + 1] + row;
        }
      }
      g.sum4 = (x0, y0, x1, y1) => {
        const i0 = max(0, floor((x0 - rect.min[0]) / cell)), j0 = max(0, floor((y0 - rect.min[1]) / cell));
        const i1 = min(nx, ceil((x1 - rect.min[0]) / cell)), j1 = min(ny, ceil((y1 - rect.min[1]) / cell));
        if (i1 <= i0 || j1 <= j0) return 0;
        const W = nx + 1;
        return s[j1 * W + i1] - s[j0 * W + i1] - s[j1 * W + i0] + s[j0 * W + i0];
      };
      g.sum = (b) => g.sum4(b.min[0], b.min[1], b.max[0], b.max[1]);
      return g;
    },
  };
  return g;
}

// ── Text blocks ──────────────────────────────────────────────────────
// A label of one or more lines [{text, h, layer}], drawn at (0, 0) and
// measured; moved into place later.
function makeLabel(lines, { gap = 0.6 } = {}) {
  let y = 0, w = 0;
  const ids = [];
  for (const l of lines) {
    y -= l.h;
    const t = cad.addText(l.text, [0, y], { height: l.h, layer: l.layer ?? "TEXT", ...(l.color != null ? { color: l.color } : {}) });
    ids.push(t.id);
    w = max(w, t.width);
    y -= gap;
  }
  const h = -y - gap;
  // Box relative to its top-left corner at (0, 0).
  return { ids, w, h, firstLine: lines[0].h, at: [0, 0] };
}
function moveLabel(lab, at) {
  cad.move(lab.ids, at[0] - lab.at[0], at[1] - lab.at[1]);
  lab.at = at;
  lab.box = { min: [at[0], at[1] - lab.h], max: [at[0] + lab.w, at[1]] };
}

// Text across at most `lines` lines of width w: words wrapped, the height
// shrunk down to `minH` if it still does not fit. Returns its box.
function para(str, at, { w, h = 2, minH = 1.5, lines = 2, layer = "TEXT", gap = 0.8, dry = false } = {}) {
  for (let hh = h; hh >= minH - 1e-9; hh -= 0.1) {
    const words = String(str).split(" ");
    const out = [];
    let cur = "";
    const ids = [];
    const width = (s) => { const t = cad.addText(s, [0, -1000], { height: hh, layer }); cad.remove(t.id); return t.width; };
    for (const wd of words) {
      const next = cur ? `${cur} ${wd}` : wd;
      if (cur && width(next) > w) { out.push(cur); cur = wd; } else cur = next;
    }
    if (cur) out.push(cur);
    if (out.length <= lines || hh - 0.1 < minH - 1e-9) {
      if (dry) return { lines: out.length, h: hh, height: out.length * (hh + gap) - gap };
      out.forEach((s, i) => ids.push(cad.addText(s, [at[0], at[1] - hh - i * (hh + gap)], { height: hh, layer }).id));
      return { ids, box: { min: [at[0], at[1] - out.length * (hh + gap) + gap], max: [at[0] + w, at[1]] }, lines: out.length, h: hh };
    }
  }
  return null;
}

// ── Markers for fixings ──────────────────────────────────────────────
// Outlines only: a 2D render paints solid fills as a light tint (a fixed
// alpha), so a filled marker would hardly show on the PNG.
const MARKERS = ["ring", "square", "triangle", "cross", "diamond", "plus"];
function marker(kind, p, r = 0.75, layer = "G_FIX") {
  const [x, y] = p;
  const poly = (pts) => [cad.addPolyline(pts, { closed: true, layer }).id];
  if (kind === "ring") return [cad.addCircle(p, r, { layer }).id];
  if (kind === "square") return poly([[x - r * 0.8, y - r * 0.8], [x + r * 0.8, y - r * 0.8], [x + r * 0.8, y + r * 0.8], [x - r * 0.8, y + r * 0.8]]);
  if (kind === "triangle") return poly([[x - r, y - r * 0.7], [x + r, y - r * 0.7], [x, y + r]]);
  if (kind === "diamond") return poly([[x - r, y], [x, y - r], [x + r, y], [x, y + r]]);
  if (kind === "plus") return [cad.addLine([x - r, y], [x + r, y], { layer }).id, cad.addLine([x, y - r], [x, y + r], { layer }).id];
  return [cad.addLine([x - r, y - r], [x + r, y + r], { layer }).id, cad.addLine([x - r, y + r], [x + r, y - r], { layer }).id];
}

// ── One projected view of a step ─────────────────────────────────────
// Projects `built` (cad.parts() objects) in `view`, fitted into `area` (a
// box on the sheet), with the parts whose ids are in `fresh` drawn strong.
// Options: `scale` (else the largest 1:n that fits), `margin` (paper mm
// kept free round the view for labels), `name` (the block), `hiddenWhen`
// (draw a new part's hidden edges when less than this share of its edge
// length is visible), `window` (a 3D box: only what lies inside its
// outline on the sheet is drawn, for a detail view).
//
// The block cad.project makes is read back (cad.children) and redrawn as
// plain entities, and its insert removed: the new parts' edges must be
// styled apart from the rest, and the projection has one layer per class of
// edge, not per part. (Drawing the new edges again over the block does not
// work: the block's grey copy of the same edges paints over them.)
function drawView({ built, fresh, view, area, scale = null, margin = [0, 0], name, hiddenWhen = 0.4, window = null, most = 1 / 2 }) {
  const freshSet = new Set(fresh);
  const ext = window ? box2(cad.viewPoints({ view, origin: [0, 0, 0], at: [0, 0], scale: 1, points: corners(window) })) : extentOf(built, view);
  const aw = area.max[0] - area.min[0] - 2 * margin[0], ah = area.max[1] - area.min[1] - 2 * margin[1];
  const fit = scale ? { scale, n: 1 / scale } : fitScale(ext, aw, ah, { most });
  const s = fit.scale;
  const centre = [(area.min[0] + area.max[0]) / 2, (area.min[1] + area.max[1]) / 2];
  const at = [centre[0] - s * (ext.min[0] + ext.max[0]) / 2, centre[1] - s * (ext.min[1] + ext.max[1]) / 2];
  const t0 = Date.now();
  const v = cad.project({
    view, parts: built.map((p) => p.id), origin: [0, 0, 0], at, scale: s, name,
    layers: { visible: "G_OLD", hidden: "G_OLD_HIDDEN" },
  });
  const ms = Date.now() - t0;
  const kids = cad.children(v.insert);
  const clipRect = window ? { ...ext, min: [at[0] + s * ext.min[0], at[1] + s * ext.min[1]], max: [at[0] + s * ext.max[0], at[1] + s * ext.max[1]] } : null;

  // Per part: its entities, visible and hidden length.
  const byPart = new Map();
  for (const e of kids) {
    const id = e.attrs?.partId;
    if (!byPart.has(id)) byPart.set(id, { vis: [], hid: [], visLen: 0, hidLen: 0 });
    const r = byPart.get(id);
    const len = lengthOf(e);
    if (e.attrs.edge === "hidden") { r.hid.push(e); r.hidLen += len; } else { r.vis.push(e); r.visLen += len; }
  }
  const drawn = [];          // entities drawn in sheet space: { e, layer }
  const add = (e, layer) => {
    const pieces = clipRect ? clip(e, clipRect) : [e];
    for (const pc of pieces) { const r = redrawPiece(pc, layer); if (r) drawn.push({ e: pc, layer, id: r.id, part: e.attrs?.partId }); }
  };
  const withHidden = new Set();
  for (const [id, r] of byPart) {
    const isNew = freshSet.has(id);
    for (const e of r.vis) add(e, isNew ? "G_NEW" : "G_OLD");
    if (isNew && r.visLen < hiddenWhen * (r.visLen + r.hidLen)) {
      withHidden.add(id);
      for (const e of r.hid) add(e, "G_NEW_HIDDEN");
    }
  }
  cad.remove(v.insert);

  // Anchors for leaders: midpoints of visible straight pieces of new parts
  // (inside the window, for a detail), longest first.
  const anchors = new Map();
  for (const id of fresh) {
    const r = byPart.get(id);
    if (!r) continue;
    const pieces = drawn.filter((d) => d.part === id && d.layer === "G_NEW").map((d) => d.e)
      .map((e) => ({ e, len: lengthOf(e), mid: midOf(e) }))
      .filter((x) => x.len >= 1.5)
      .sort((a, b) => b.len - a.len);
    const hid = r.hid.map((e) => ({ e, len: lengthOf(e), mid: midOf(e) })).filter((x) => x.len >= 1.5);
    anchors.set(id, pieces.length ? pieces : (withHidden.has(id) ? hid : []));
  }

  // The outline of every part on the sheet: the hull of its projected
  // edges (visible and hidden), for the label mask.
  const outlines = [];
  const ends = (e) => (e.type === "line" ? [e.start, e.end] : e.type === "spline-fit" ? e.points : samplePoints(e, 2));
  for (const [id, r] of byPart) {
    const pts = clipRect ? drawn.filter((d) => d.part === id).flatMap((d) => ends(d.e)) : [...r.vis, ...r.hid].flatMap(ends);
    if (pts.length) outlines.push({ id, poly: hull(pts) });
  }
  const bounds = clipRect ?? v.bounds;
  return {
    v, view, scale: s, n: fit.n, at, origin: [0, 0, 0], kids, byPart, anchors, outlines, withHidden, drawn, bounds, ms,
    toSheet: (p) => cad.viewPoints({ view, origin: [0, 0, 0], at, scale: s, points: [p] })[0],
    toSheetMany: (ps) => (ps.length ? cad.viewPoints({ view, origin: [0, 0, 0], at, scale: s, points: ps }) : []),
  };
}

// ── Label placement ──────────────────────────────────────────────────
// labels: [{ lines, tips: [[x, y], ...] }] (the tips the leader may go to,
// best first). area: the box labels must stay in. grid: the mask (marked
// cells are drawing). blocked: boxes no label or leader may meet. Returns
// the placed labels with their box, tip and leader.
function placeLabels(labels, { area, grid, blocked = [], step = 1.5, pad = 1.2 }) {
  grid.integral();
  const made = labels.map((l) => ({ ...l, lab: makeLabel(l.lines) }));
  const placed = new Map();
  const cost = (m, box, anchor, tip) => {
    const padded = { min: [box.min[0] - pad, box.min[1] - pad], max: [box.max[0] + pad, box.max[1] + pad] };
    let c = hypot(anchor[0] - tip[0], anchor[1] - tip[1]);
    c += 40 * grid.sum(padded);
    for (const b of blocked) if (boxesMeet(box, b, pad)) c += 1e5;
    for (const [other, o] of placed) {
      if (other === m) continue;
      if (boxesMeet(box, o.box, pad)) c += 1e5;
      if (segsCross(anchor, tip, o.anchor, o.tip)) c += 60;
      if (segMeetsBox(anchor, tip, o.box)) c += 400;
      if (segMeetsBox(o.anchor, o.tip, box)) c += 400;
    }
    for (const b of blocked) if (segMeetsBox(anchor, tip, b)) c += 400;
    return c;
  };
  // Where the leader leaves the label: the middle of the side facing the
  // tip, or under (over) the first line when the tip is below (above) it.
  const anchorOf = (box, firstLine, tip) => {
    if (tip[0] < box.min[0]) return [box.min[0] - 0.6, box.max[1] - firstLine / 2];
    if (tip[0] > box.max[0]) return [box.max[0] + 0.6, box.max[1] - firstLine / 2];
    const x = min(max(tip[0], box.min[0] + 1), box.max[0] - 1);
    return tip[1] < box.min[1] ? [x, box.min[1] - 0.6] : [x, box.max[1] + 0.6];
  };
  // Every grid position, cheapest first by a lower bound (the distance from
  // the box to the nearest tip plus the drawing under the box); the full
  // cost only while the bound can still beat the best so far.
  const best = (m) => {
    const { w, h } = m.lab;
    const tips = m.tips;
    // Lower bounds for every grid position, in flat arrays (no objects:
    // the sandbox interprets this loop).
    const xs = [], ys = [], lbs = [];
    let first = -1;
    for (let y = area.min[1] + h; y <= area.max[1]; y += step) {
      for (let x = area.min[0]; x + w <= area.max[0]; x += step) {
        let d = Infinity;
        for (let k = 0; k < tips.length; k++) {
          const t = tips[k];
          const dx = max(x - t[0], 0, t[0] - x - w), dy = max(y - h - t[1], 0, t[1] - y);
          const dd = dx * dx + dy * dy;
          if (dd < d) d = dd;
        }
        const lb = sqrt(d) + 40 * grid.sum4(x - pad, y - h - pad, x + w + pad, y + pad);
        if (first < 0 || lb < lbs[first]) first = lbs.length;
        xs.push(x); ys.push(y); lbs.push(lb);
      }
    }
    let top = null;
    const evaluate = (i) => {
      const box = { min: [xs[i], ys[i] - h], max: [xs[i] + w, ys[i]] };
      for (const tip of tips) {
        const anchor = anchorOf(box, m.lab.firstLine, tip);
        const c = cost(m, box, anchor, tip);
        if (!top || c < top.c) top = { c, at: [xs[i], ys[i]], box, anchor, tip };
      }
    };
    // The best bound first, then only the positions whose bound can still
    // beat it, cheapest first.
    evaluate(first);
    const rest = [];
    for (let i = 0; i < lbs.length; i++) if (i !== first && lbs[i] < top.c) rest.push(i);
    rest.sort((a, b) => lbs[a] - lbs[b]);
    for (const i of rest) {
      if (lbs[i] >= top.c) break;
      evaluate(i);
    }
    return top;
  };
  for (const m of made) placed.set(m, best(m));
  for (const m of made) { placed.delete(m); placed.set(m, best(m)); }
  return made.map((m) => {
    const p = placed.get(m);
    moveLabel(m.lab, p.at);
    const leader = cad.addLine(p.anchor, p.tip, { layer: "G_LEADER" });
    const dot = cad.addCircle(p.tip, 0.45, { layer: "G_LEADER" });
    return { ...m, box: m.lab.box, anchor: p.anchor, tip: p.tip, cost: p.c, under: grid.sum(m.lab.box), ids: [...m.lab.ids, leader.id, dot.id] };
  });
}

// ── Words ────────────────────────────────────────────────────────────
// Plural of a member or item name: the word before the first size or
// "with" ("coach bolt M10 x 160" -> "coach bolts M10 x 160").
function plural(n, name, { same = [] } = {}) {
  if (n === 1 || same.includes(name)) return name;
  const w = name.split(" ");
  let k = w.findIndex((x, i) => i > 0 && (/^[0-9MØ]/.test(x) || x === "with" || x === "(with"));
  if (k < 0) k = w.length;
  const last = w[k - 1];
  w[k - 1] = /(s|x|sh|ch)$/.test(last) ? `${last}es` : `${last}s`;
  return w.join(" ");
}

// ── Where a part meets the ground ────────────────────────────────────
// The middle of a part's lowest `depth` mm (along z): a unique copy of the
// part cut down to that slab with cad.intersect, its box read, the copy
// removed. For an angled leg that is its foot, where its box's middle is
// not on the leg at all.
function footOf(part, depth = 2) {
  const b = part.bbox;
  const copy = cad.copy3d(part.id, { unique: true });
  cad.intersect(copy.id, {
    extrude: { min: [b.min[0] - 1, b.min[1] - 1], max: [b.max[0] + 1, b.max[1] + 1] },
    plane: "xy", at: [0, 0, b.min[2] - 1], distance: depth + 1,
  });
  const f = cad.part(copy.id).bbox;
  cad.remove3d(copy.id);
  return [(f.min[0] + f.max[0]) / 2, (f.min[1] + f.max[1]) / 2, f.min[2]];
}

const textWidth = (s, h, layer = "HEAD") => {
  const t = cad.addText(s, [0, -1000], { height: h, layer });
  cad.remove(t.id);
  return t.width;
};

// ── One step in a panel ──────────────────────────────────────────────
// The panel: a header (number, title, a note), the view with its labels,
// fixing markers and details, then the fixings table and the instruction.
// `spec`:
//
//   number, title, note        the header
//   fresh, built               cad.parts() objects: added now, built so far
//   view                       the corner to draw from
//   labels                     [{ lines: [{text, h, layer}], parts: [ids] }]
//   joints                     [{ point: [x, y, z], kind, parts: [ids] }]
//                              (kind: a MARKERS name)
//   rows                       [{ kind, qty, item, where }] the fixings table
//   instruction                one or two lines of text
//   details                    [{ title, view, parts, window, size: [w, h],
//                              note, annotate(dv) }]
//   margin                     paper mm kept round the main view
//
// The main view takes the largest scale at which its parts' boxes fit the
// area and leave a free corner for each detail (predicted from the boxes'
// outlines before anything is projected). Returns what was drawn, for the
// script's checks.
function drawStep(spec, panel) {
  const { number, title, note = "", fresh, built, view, labels = [], joints = [], rows = [], instruction = "" } = spec;
  const ids0 = new Set(cad.entities().map((e) => e.id));
  const ms = {};
  let tk = Date.now();
  const lap = (k) => { const t = Date.now(); ms[k] = (ms[k] || 0) + t - tk; tk = t; };
  const x0 = panel.min[0], x1 = panel.max[0], y1 = panel.max[1];
  const W = x1 - x0;

  // Header.
  sheet.rect([x0, y1 - 9], [x0 + 9, y1], "HEAD", { lineweight: 0.5 });
  cad.addText(String(number), [x0 + 4.5, y1 - 4.5], { height: 5, align: "center", valign: "middle", layer: "HEAD" });
  sheet.text(title.toUpperCase(), [x0 + 11.5, y1 - 4.4], { h: 3.6, w: W - 12, layer: "HEAD" });

  // Footer: the instruction, then the fixings table above it.
  const instr = para(instruction, [x0, panel.min[1] + 8], { w: W, h: 2.1, minH: 1.6, lines: 2, layer: "TEXT" });
  cad.move(instr.ids, 0, -(8 - (instr.box.max[1] - instr.box.min[1])) / 2);
  const rowH = 2.9;
  const tableTop = panel.min[1] + 10 + rows.length * rowH + 4;
  sheet.text(rows.length ? "FIXINGS IN THIS STEP" : "NO FIXINGS IN THIS STEP", [x0, tableTop - 2.4], { h: 1.8, layer: "HEAD" });
  rows.forEach((r, i) => {
    const y = tableTop - 4 - (i + 1) * rowH + 0.6;
    if (r.kind) marker(r.kind, [x0 + 1.5, y + 0.9], 1);
    cad.addText(String(r.qty), [x0 + 10, y], { height: 1.8, align: "right", layer: "TEXT" });
    sheet.text(r.item, [x0 + 12, y], { h: 1.8, w: 50, layer: "TEXT" });
    sheet.text(r.where, [x0 + 64, y], { h: 1.6, w: W - 64, layer: "G_NOTE" });
  });
  const area = { min: [x0, tableTop + 1.5], max: [x1, y1 - 11] };

  // Details: each at its own scale, in a box sized to its drawing and its
  // title.
  const NOTE_H = 4.6;
  const dets = (spec.details ?? []).map((d) => {
    const [dw, dh] = d.size ?? [56, 46];
    const noteH = d.note ? NOTE_H : 0;
    const ext = box2(cad.viewPoints({ view: d.view, origin: [0, 0, 0], at: [0, 0], scale: 1, points: corners(d.window) }));
    const fit = fitScale(ext, dw - 4, dh - 9 - noteH, { most: 1 });
    const heading = `${d.title.toUpperCase()} (1:${fit.n})`;
    const w = max((ext.max[0] - ext.min[0]) * fit.scale + 4, textWidth(heading, 2) + 4);
    const noteOpts = { w: w - 3, h: 1.5, minH: 1.3, lines: 2, gap: 0.6, layer: "G_NOTE" };
    const nh = d.note ? para(d.note, [0, 0], { ...noteOpts, dry: true }).height + 1.6 : 0;
    return { ...d, fit, noteH: nh, noteOpts, heading, w, h: (ext.max[1] - ext.min[1]) * fit.scale + 9 + nh };
  });

  // Main view and the detail corners.
  const ext = extentOf(built, view);
  const margin = spec.margin ?? [6, 2];
  const aw = area.max[0] - area.min[0] - 2 * margin[0], ah = area.max[1] - area.min[1] - 2 * margin[1];
  const boxPts = dets.length ? cad.viewPoints({ view, origin: [0, 0, 0], at: [0, 0], scale: 1, points: built.flatMap((p) => corners(p.bbox)) }) : [];
  let choice = null;
  for (let k = SCALES.indexOf(fitScale(ext, aw, ah).n); k < SCALES.length && !choice; k++) {
    const s = 1 / SCALES[k];
    const c = [(area.min[0] + area.max[0]) / 2, (area.min[1] + area.max[1]) / 2];
    const at = [c[0] - s * (ext.min[0] + ext.max[0]) / 2, c[1] - s * (ext.min[1] + ext.max[1]) / 2];
    if (!dets.length) { choice = { s, at, boxes: [] }; break; }
    const g = makeGrid(area, 1);
    for (let i = 0; i < built.length; i++) g.fill(hull(boxPts.slice(8 * i, 8 * i + 8).map((p) => [at[0] + s * p[0], at[1] + s * p[1]])));
    g.integral();
    const boxes = [];
    for (const d of dets) {
      const cand = [
        { min: [area.min[0], area.min[1]], max: [area.min[0] + d.w, area.min[1] + d.h] },
        { min: [area.max[0] - d.w, area.min[1]], max: [area.max[0], area.min[1] + d.h] },
        { min: [area.min[0], area.max[1] - d.h], max: [area.min[0] + d.w, area.max[1]] },
        { min: [area.max[0] - d.w, area.max[1] - d.h], max: [area.max[0], area.max[1]] },
      ].filter((b) => boxes.every((o) => !boxesMeet(b, o, 1)))
        .map((b) => ({ b, n: g.sum(b) })).sort((a, b) => a.n - b.n);
      if (!cand.length || cand[0].n > 0.01 * d.w * d.h) break;
      boxes.push(cand[0].b);
    }
    if (boxes.length === dets.length) choice = { s, at, boxes };
  }
  lap("layout");
  const main = drawView({ built, fresh: fresh.map((p) => p.id), view, area, scale: choice.s, name: `guide-${number}` });
  lap("view");
  if (note) sheet.text(`${note}, 1:${main.n}`, [x0 + 11.5, y1 - 8.4], { h: 1.7, w: W - 12, layer: "G_NOTE" });

  // The mask for labels: every part's outline, the detail boxes, the
  // fixing markers.
  const g = makeGrid(area, 1);
  for (const o of main.outlines) g.fill(o.poly);
  const blocked = [];
  dets.forEach((d, i) => {
    const b = choice.boxes[i];
    const inner = { min: [b.min[0] + 2, b.min[1] + 2 + d.noteH], max: [b.max[0] - 2, b.max[1] - 7] };
    const inDet = new Set(d.parts.map((p) => p.id));
    const dv = drawView({
      built: d.parts, fresh: fresh.filter((f) => inDet.has(f.id)).map((p) => p.id),
      view: d.view, area: inner, scale: d.fit.scale, name: `guide-${number}-detail-${i + 1}`, window: d.window, hiddenWhen: 0,
    });
    sheet.rect(b.min, b.max, "G_DETAIL");
    cad.addText(d.heading, [b.min[0] + 2, b.max[1] - 4.2], { height: 2, layer: "HEAD" });
    if (d.note) para(d.note, [b.min[0] + 1.5, b.min[1] + d.noteH], d.noteOpts);
    // The step's fixing markers between the detail's own parts.
    const dj = joints.filter((j) => j.parts && j.parts.every((id) => inDet.has(id)));
    const dp = dv.toSheetMany(dj.map((j) => j.point));
    dj.forEach((j, k) => { if (inBox(dp[k], dv.bounds)) marker(j.kind, dp[k], 0.9); });
    if (d.annotate) d.annotate(dv);
    d.v = dv;
    d.box = b;
    blocked.push(b);
    g.fillBox(b);
  });
  lap("details");

  // Fixing markers on the main view.
  const pts = main.toSheetMany(joints.map((j) => j.point));
  joints.forEach((j, i) => {
    marker(j.kind, pts[i], 0.9);
    g.fillBox({ min: [pts[i][0] - 0.9, pts[i][1] - 0.9], max: [pts[i][0] + 0.9, pts[i][1] + 0.9] });
  });
  lap("markers");

  // Labels, each to a visible piece of one of its parts; a part with no
  // visible piece gets its centre (cad.viewPoints).
  const centreOf = new Map(fresh.map((p) => [p.id, p.centroid ?? p.bbox.min.map((v, i) => (v + p.bbox.max[i]) / 2)]));
  const placed = placeLabels(labels.map((l) => {
    let tips = l.parts.flatMap((id) => (main.anchors.get(id) ?? []).slice(0, 3)).sort((a, b) => b.len - a.len).slice(0, 6).map((a) => a.mid);
    if (!tips.length) tips = main.toSheetMany(l.parts.slice(0, 3).map((id) => centreOf.get(id)));
    return { ...l, tips };
  }), { area, grid: g, blocked });
  lap("labels");

  const mine = cad.entities().filter((e) => !ids0.has(e.id));
  return { main, details: dets, labels: placed, area, entities: mine.map((e) => e.id), joints: joints.length, ms };
}

return {
  LAYERS, MARKERS, SCALES,
  setup, frameOf, coverage, chooseView, extentOf, fitScale, drawView, drawStep, placeLabels, makeLabel, moveLabel, para, marker,
  makeGrid, hull, box2, boxesMeet, inBox, segsCross, corners, plural, redraw, clip, footOf,
};
