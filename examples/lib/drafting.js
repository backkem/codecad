// Shared drafting helpers for the CodeCAD examples. An ES module, loaded
// with cad.use (server-side exec, or the browser console):
//
//   const draft = await cad.use("drafting");   // this file: lib/drafting.js
//   draft.setup({ scale: 2 });   // 1 paper mm = 2 model units (1:2)
//
// Only what the cad API doesn't do natively lives here: paper-scaled
// layers and linetypes, a sheet border with title block, leaders and
// centre marks. Dimensions, text alignment, hatching, fills, fillets,
// intersections and region booleans are native cad.* calls; the thin
// wrappers below only convert PAPER millimetres (text heights,
// dimension offsets, hatch spacing) to model units, so a 1:50 truss and
// a 1:1 flange get the same on-sheet proportions.

// ACI (AutoCAD Color Index) values: lossless through DWG save/reload.
export const ACI = {
  red: 1, yellow: 2, green: 3, cyan: 4, blue: 5, magenta: 6, white: 7,
  gray: 8, lightgray: 9, orange: 30, amber: 40,
};

let S = 1; // model units per paper mm
export { S as scale };
export const P = (v) => v * S;

// ── Setup: paper-scaled linetypes + standard layers ───────────────
// Linetype dash lengths are in model units, so they are scaled with
// the drawing (the API has no LTSCALE).
export function setup(opts = {}) {
  S = opts.scale || 1;
  cad.addLinetype("CL", [8, 2, 1.5, 2].map(P), { description: "Centre ____ . ____" });
  cad.addLinetype("HID", [3, 1.5].map(P), { description: "Hidden __ __ __" });
  cad.addLinetype("PHAN", [10, 2, 1.5, 2, 1.5, 2].map(P), { description: "Phantom ____ . . ____" });
  const L = {
    OUTLINE: { color: ACI.white, lineweight: 0.5 },
    THIN: { color: ACI.lightgray, lineweight: 0.25 },
    HIDDEN: { color: ACI.yellow, linetype: "HID", lineweight: 0.25 },
    CENTER: { color: ACI.red, linetype: "CL", lineweight: 0.18 },
    PHANTOM: { color: ACI.lightgray, linetype: "PHAN", lineweight: 0.18 },
    HATCH: { color: ACI.gray, lineweight: 0.18 },
    DIM: { color: ACI.cyan, lineweight: 0.18 },
    TEXT: { color: ACI.white, lineweight: 0.25 },
    FRAME: { color: ACI.lightgray, lineweight: 0.35 },
    ...(opts.layers || {}),
  };
  for (const [name, o] of Object.entries(L)) cad.addLayer(name, o);
  return L;
}

// ── Vector helpers ────────────────────────────────────────────────
export const add = (a, b) => [a[0] + b[0], a[1] + b[1]];
export const sub = (a, b) => [a[0] - b[0], a[1] - b[1]];
export const mul = (a, k) => [a[0] * k, a[1] * k];
export const len = (a) => Math.hypot(a[0], a[1]);
export const unit = (a) => mul(a, 1 / len(a));
export const perp = (a) => [-a[1], a[0]];
export const polar = (c, r, deg) => {
  const t = (deg * Math.PI) / 180;
  return [c[0] + r * Math.cos(t), c[1] + r * Math.sin(t)];
};
// Parallel copy of a two-point line (positive = left). cad.offset needs
// an entity; this is for construction lines that are never drawn.
export const offsetLine = ([a, b], d) => {
  const n = perp(unit(sub(b, a)));
  return [add(a, mul(n, d)), add(b, mul(n, d))];
};

// ── Paper-scaled wrappers around native calls ─────────────────────
export const text = (str, at, o = {}) =>
  cad.addText(String(str), at, {
    height: P(o.h || 3.5), layer: o.layer || "TEXT",
    ...(o.align ? { align: o.align } : {}), ...(o.valign ? { valign: o.valign } : {}),
    ...(o.rot ? { rotation: o.rot } : {}), ...(o.color != null ? { color: o.color } : {}),
  });

// Workaround: the Vello renderer paints SOLID fills at ~14% alpha, so
// filled arrowheads nearly vanish on screen. Trace each fill's boundary
// with a polyline so it reads as solid (the DWG keeps the real SOLID).
export function outlineFills(ids) {
  for (const id of ids) {
    const e = cad.entity(id);
    if (e.type !== "solid_fill" || !e.edges?.boundary?.every((g) => g.type === "line")) continue;
    cad.addPolyline(e.edges.boundary.map((g) => g.to), { closed: true, layer: e.layer });
  }
}
export const withOutlines = (r) => { outlineFills(r.ids); return r; };

// Linear dimension; off = signed paper-mm offset (+ = left of a->b).
export const dim = (a, b, off, o = {}) => withOutlines(
  cad.measure(a, b, {
    offset: P(off), text_height: P(o.h || 3.5), layer: o.layer || "DIM",
    ...(o.dir ? { dir: o.dir } : {}), ...(o.label ? { label: o.label } : {}),
    ...(o.shift ? { shift: P(o.shift) } : {}), ...(o.decimals != null ? { decimals: o.decimals } : {}),
  }));

export const hatch = (boundary, o = {}) =>
  cad.addHatch(boundary, {
    angle: o.angle != null ? o.angle : 45, spacing: P(o.spacing || 2.5),
    layer: o.layer || "HATCH", ...(o.holes ? { holes: o.holes } : {}),
    ...(o.pattern ? { pattern: o.pattern } : {}),
  });

// ── Still missing natively: arrowheads, leaders, centre marks ─────
export function arrow(tip, dir, o = {}) {
  const L = P(o.size || 3), w = L / 6, d = unit(dir), n = perp(d);
  const back = sub(tip, mul(d, L));
  const tri = [tip, add(back, mul(n, w)), sub(back, mul(n, w))];
  cad.addFill(tri, { layer: o.layer || "DIM" });
  cad.addPolyline(tri, { closed: true, layer: o.layer || "DIM" });
}

// Leader: arrow (or dot) at tip, line to knee, horizontal shelf with text.
export function leader(tip, knee, label, o = {}) {
  const layer = o.layer || "DIM", h = o.h || 3.5;
  const lines = String(label).split("\n");
  const right = o.side ? o.side === "right" : knee[0] >= tip[0];
  const ts = lines.map((s, i) =>
    text(s, [knee[0] + (right ? P(1) : -P(1)), knee[1] + P(1) - i * P(h * 1.6)],
      { h, layer, align: right ? "left" : "right" }));
  const w = Math.max(...ts.map((t) => t.width));
  cad.addLine(tip, knee, { layer });
  cad.addLine(knee, add(knee, [right ? w + P(2) : -(w + P(2)), 0]), { layer });
  if (o.arrow === false) cad.addFill({ center: tip, radius: P(0.6) }, { layer });
  else arrow(tip, sub(tip, knee), { layer });
}

// Centre lines through c, overshooting radius r by ext paper mm.
export function centerMark(c, r, o = {}) {
  const e = r + P(o.ext != null ? o.ext : 3), layer = o.layer || "CENTER", rot = o.rot || 0;
  if (o.h !== false) cad.addLine(polar(c, e, rot + 180), polar(c, e, rot), { layer });
  if (o.v !== false) cad.addLine(polar(c, e, rot - 90), polar(c, e, rot + 90), { layer });
}

// ── Table: rows of cells, top-left at `at`, column widths in paper mm ─
export function table(at, rows, colsPaper, o = {}) {
  const rowH = P(o.rowH || 6), h = o.h || 2.5;
  const cols = colsPaper.map(P), tw = cols.reduce((s, v) => s + v, 0);
  const [tx, ty] = at, bot = ty - rows.length * rowH;
  if (o.title) text(o.title, [tx, ty + P(2.5)], { h: o.titleH || 3.5 });
  cad.addPolyline([[tx, ty], [tx + tw, ty], [tx + tw, bot], [tx, bot]], { closed: true, layer: "FRAME" });
  rows.forEach((r, i) => {
    const y = ty - (i + 1) * rowH;
    if (i < rows.length - 1) cad.addLine([tx, y], [tx + tw, y], { layer: "FRAME" });
    let x = tx;
    r.forEach((cell, j) => {
      const right = o.align && o.align[j] === "right";
      text(cell, [right ? x + cols[j] - P(1.5) : x + P(1.5), y + (rowH - P(h)) / 2],
        { h, layer: i === 0 && o.header !== false ? "FRAME" : "TEXT", ...(right ? { align: "right" } : {}) });
      x += cols[j];
    });
  });
  let x = tx;
  cols.slice(0, -1).forEach((w) => { x += w; cad.addLine([x, ty], [x, bot], { layer: "FRAME" }); });
  return { min: [tx, bot], max: [tx + tw, ty] };
}

// ── Sheet border + title block ────────────────────────────────────
// at: lower-left corner (model). w, h: sheet size in paper mm.
export function sheet(o) {
  const at = o.at || [0, 0];
  const W = P(o.w || 297), H = P(o.h || 189), m = P(5);
  const x0 = at[0], y0 = at[1], x1 = x0 + W, y1 = y0 + H;
  const rect = (a, b, layer, extra = {}) =>
    cad.addPolyline([a, [b[0], a[1]], b, [a[0], b[1]]], { closed: true, layer, ...extra });
  rect([x0, y0], [x1, y1], "FRAME", { lineweight: 0.18 });
  const fx0 = x0 + m, fy0 = y0 + m, fx1 = x1 - m, fy1 = y1 - m;
  rect([fx0, fy0], [fx1, fy1], "FRAME", { lineweight: 0.5 });

  // Title block: bottom-right, 120 x 18 paper mm, two rows.
  const tw = P(120), th = P(18), rh = P(7), brandW = P(26);
  const tx0 = fx1 - tw, ty1 = fy0 + th;
  rect([tx0, fy0], [fx1, ty1], "FRAME");
  cad.addLine([tx0, fy0 + rh], [fx1, fy0 + rh], { layer: "FRAME" });
  cad.addLine([fx1 - brandW, fy0 + rh], [fx1 - brandW, ty1], { layer: "FRAME" });
  const fields = [["DWG NO.", o.number || "-", 22], ["SCALE", o.scale || "1:1", 26], ["DATE", o.date || "2026-10-13", 30], ["UNITS", o.units || "mm", 42]];
  let fx = tx0;
  fields.forEach(([label, value, wPaper], i) => {
    if (i > 0) cad.addLine([fx, fy0], [fx, fy0 + rh], { layer: "FRAME" });
    text(label, [fx + P(1.2), fy0 + rh - P(2.4)], { h: 1.6, layer: "FRAME" });
    text(value, [fx + P(1.2), fy0 + P(1.2)], { h: 2.5 });
    fx += P(wPaper);
  });
  text(o.title || "UNTITLED", [tx0 + P(2), ty1 - P(5.6)], { h: 4 });
  if (o.subtitle) text(o.subtitle, [tx0 + P(2), fy0 + rh + P(1.4)], { h: 2, layer: "FRAME" });
  text("CodeCAD", [fx1 - brandW / 2, fy0 + rh + (th - rh) / 2], { h: 3.2, align: "center", valign: "middle", color: ACI.amber });
  return { frame: { min: [fx0, fy0], max: [fx1, fy1] }, titleBlock: { min: [tx0, fy0], max: [fx1, ty1] } };
}
