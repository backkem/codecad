// Drawing sheets from projected views, loaded with
//
//   const vw = await cad.use("views");
//
// Plain script code over the public cad.* calls: cad.project draws a view
// of the parts as a block of exact 2D lines, arcs, circles and ellipses;
// this file arranges views in first- or third-angle order, maps 3D points
// onto a view (for dimensions and labels), and draws a sheet frame with a
// title block. Sheet units are millimetres on paper; `scale` is paper mm
// per model mm (0.5 for a 1:2 drawing).

const project = (opts) => cad.project(opts);
const viewPoints = (opts) => cad.viewPoints(opts);

// ACI colours, lossless through DWG.
const ACI = { red: 1, yellow: 2, green: 3, cyan: 4, blue: 5, white: 7, gray: 8, lightgray: 9 };

// The layers a sheet uses, with paper line weights (mm) and linetypes in
// paper mm. Names are this library's choice; cad.project is told them.
const LAYERS = {
  visible: { color: ACI.white, lineweight: 0.5 },
  hidden: { color: ACI.yellow, linetype: "VIEW_HIDDEN", lineweight: 0.25 },
  dims: { color: ACI.cyan, lineweight: 0.18 },
  text: { color: ACI.white, lineweight: 0.25 },
  frame: { color: ACI.lightgray, lineweight: 0.35 },
};

function setup() {
  cad.addLinetype("VIEW_HIDDEN", [3, 1.5], { description: "Hidden __ __ __" });
  for (const [name, o] of Object.entries(LAYERS)) cad.addLayer(name, o);
  return { visible: "visible", hidden: "hidden" };
}

// A 3D point on the sheet, in the view `v` that cad.project returned.
function toSheet(v, p) {
  const d = [p[0] - v.origin[0], p[1] - v.origin[1], p[2] - v.origin[2]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  return [v.at[0] + v.scale * dot(d, v.right), v.at[1] + v.scale * dot(d, v.up)];
}

// The world box around parts (cad.parts() objects).
function boxOf(parts) {
  const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
  for (const p of parts) {
    for (let i = 0; i < 3; i++) {
      min[i] = Math.min(min[i], p.bbox.min[i]);
      max[i] = Math.max(max[i], p.bbox.max[i]);
    }
  }
  return { min, max };
}

// The width and height (model mm) of a box seen in a named view.
function extent(view, box) {
  const corners = [];
  for (const x of [box.min[0], box.max[0]])
    for (const y of [box.min[1], box.max[1]])
      for (const z of [box.min[2], box.max[2]]) corners.push([x, y, z]);
  const pts = viewPoints({ view, origin: [0, 0, 0], points: corners });
  const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
  return [Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)];
}

// Where the principal views go around the front view, centred on `front`
// (sheet mm), `gap` paper mm apart. First angle (ISO): the top view below
// the front, the left view to its right, the right view to its left, the
// bottom view above. Third angle: the other way round. Every view is
// centred on the same 3D point (the box centre), so the views line up.
function arrange(box, { front = [0, 0], gap = 20, scale = 1, angle = "first" } = {}) {
  const s = angle === "third" ? -1 : 1;
  const [wf, hf] = extent("front", box).map((v) => v * scale);
  const [, ht] = extent("top", box).map((v) => v * scale);
  const [wl] = extent("left", box).map((v) => v * scale);
  const [, hb] = extent("bottom", box).map((v) => v * scale);
  const [wr] = extent("right", box).map((v) => v * scale);
  const [cx, cy] = front;
  return {
    front: [cx, cy],
    top: [cx, cy - s * (hf / 2 + gap + ht / 2)],
    bottom: [cx, cy + s * (hf / 2 + gap + hb / 2)],
    left: [cx + s * (wf / 2 + gap + wl / 2), cy],
    right: [cx - s * (wf / 2 + gap + wr / 2), cy],
  };
}

// Projects one view with this library's layers: `opts` are cad.project's.
function view(opts) {
  return project({ layers: { visible: "visible", hidden: "hidden" }, ...opts });
}

// A label centred under a view.
function label(v, text, { height = 3.5, below = 8 } = {}) {
  if (!v.bounds) return null;
  const x = 0.5 * (v.bounds.min[0] + v.bounds.max[0]);
  return cad.addText(text, [x, v.bounds.min[1] - below], { height, align: "center", layer: "text" });
}

// A sheet `size` paper mm (default A3 landscape) with a 10 mm border and a
// title block in the lower right corner: title, a note, the scale and the
// projection method.
function sheet({ size = [420, 297], title = "", scale = "1:1", angle = "first", note = "" } = {}) {
  const [w, h] = size;
  const m = 10;
  const rect = (x0, y0, x1, y1, layer) =>
    cad.addPolyline([[x0, y0], [x1, y0], [x1, y1], [x0, y1]], { closed: true, layer });
  rect(m, m, w - m, h - m, "frame");
  const tw = 160, th = 30;
  const x0 = w - m - tw, y0 = m;
  rect(x0, y0, w - m, y0 + th, "frame");
  cad.addLine([x0, y0 + 14], [w - m, y0 + 14], { layer: "frame" });
  cad.addLine([x0 + 100, y0], [x0 + 100, y0 + 14], { layer: "frame" });
  cad.addText(title, [x0 + 5, y0 + 19], { height: 6, layer: "text" });
  if (note) cad.addText(note, [x0 + 5, y0 + 5], { height: 3.5, layer: "text" });
  cad.addText(`${scale}  ${angle === "third" ? "third" : "first"} angle`, [x0 + 104, y0 + 5], {
    height: 3.5, layer: "text",
  });
  return { min: [m, m], max: [w - m, h - m] };
}

return { ACI, LAYERS, setup, project, viewPoints, toSheet, boxOf, extent, arrange, view, label, sheet };
