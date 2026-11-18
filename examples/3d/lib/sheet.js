// A drawing sheet for the 3D examples' 2D output (part lists, schedules),
// loaded with
//
//   const sheet = await cad.use("sheet");
//   sheet.setup();
//
// The same conventions as examples/lib/drafting.js (paper millimetres,
// ACI colours that survive DWG, a frame with a title block), which the 3D
// examples cannot load: a library loads from the lib/ folder next to the
// script. Only what a tabular sheet needs: layers, text that fits its
// cell, tables, a title block and plain rectangles.

const ACI = { red: 1, yellow: 2, green: 3, cyan: 4, blue: 5, magenta: 6, white: 7, gray: 8, lightgray: 9, orange: 30, amber: 40 };

function setup(layers = {}) {
  const L = {
    TEXT: { color: ACI.white, lineweight: 0.25 },
    HEAD: { color: ACI.amber, lineweight: 0.35 },
    FRAME: { color: ACI.lightgray, lineweight: 0.35 },
    GRID: { color: ACI.gray, lineweight: 0.18 },
    NOTE: { color: ACI.cyan, lineweight: 0.18 },
    ...layers,
  };
  for (const [name, o] of Object.entries(L)) cad.addLayer(name, o);
  return L;
}

// Text that fits: at most `w` wide (shrinks down to `min` height, then
// stops shrinking). Returns the text entity (with its width).
function text(str, at, { h = 2.5, w = Infinity, min = 1.4, layer = "TEXT", align, valign, color } = {}) {
  const opts = { height: h, layer, ...(align ? { align } : {}), ...(valign ? { valign } : {}), ...(color != null ? { color } : {}) };
  let t = cad.addText(String(str), at, opts);
  if (t.width > w && h > min) {
    const h2 = Math.max(min, (h * w) / t.width);
    cad.remove(t.id);
    t = cad.addText(String(str), at, { ...opts, height: h2 });
  }
  return t;
}

const rect = (a, b, layer = "FRAME", extra = {}) =>
  cad.addPolyline([a, [b[0], a[1]], b, [a[0], b[1]]], { closed: true, layer, ...extra });

// A table: `cols` [{title, w, align}] (paper mm), `rows` arrays of cells,
// top-left at `at`. Header row on HEAD; rule lines on GRID. Returns its box.
function table(at, cols, rows, { rowH = 4, h = 2, title, titleH = 3 } = {}) {
  const [x0, y0] = at;
  const tw = cols.reduce((s, c) => s + c.w, 0);
  if (title) text(title, [x0, y0 + 2], { h: titleH, layer: "HEAD" });
  const all = [cols.map((c) => c.title), ...rows];
  const y1 = y0 - all.length * rowH;
  rect([x0, y1], [x0 + tw, y0]);
  all.forEach((r, i) => {
    const y = y0 - (i + 1) * rowH;
    if (i < all.length - 1) cad.addLine([x0, y], [x0 + tw, y], { layer: i === 0 ? "FRAME" : "GRID" });
    let x = x0;
    r.forEach((cell, j) => {
      const c = cols[j], right = c.align === "right";
      if (cell !== "" && cell != null) {
        text(cell, [right ? x + c.w - 1.2 : x + 1.2, y + rowH / 2], {
          h: i === 0 ? h * 0.9 : h, w: c.w - 2.4, valign: "middle", layer: i === 0 ? "HEAD" : "TEXT", ...(right ? { align: "right" } : {}),
        });
      }
      x += c.w;
    });
  });
  let x = x0;
  cols.slice(0, -1).forEach((c) => { x += c.w; cad.addLine([x, y0], [x, y1], { layer: "GRID" }); });
  return { min: [x0, y1], max: [x0 + tw, y0] };
}

// The sheet: an outer edge, a frame 5 inside, and a title block at the
// frame's bottom right. Returns the frame and the title block's boxes.
function frame({ w = 297, h = 198, title, subtitle, number = "-", scale = "-", date = "-", units = "mm" }) {
  rect([0, 0], [w, h], "FRAME", { lineweight: 0.18 });
  const f = { min: [5, 5], max: [w - 5, h - 5] };
  rect(f.min, f.max, "FRAME", { lineweight: 0.5 });
  const tw = 120, th = 18, rh = 7, brandW = 26;
  const tx0 = f.max[0] - tw, ty1 = f.min[1] + th;
  rect([tx0, f.min[1]], [f.max[0], ty1]);
  cad.addLine([tx0, f.min[1] + rh], [f.max[0], f.min[1] + rh], { layer: "FRAME" });
  cad.addLine([f.max[0] - brandW, f.min[1] + rh], [f.max[0] - brandW, ty1], { layer: "FRAME" });
  let fx = tx0;
  for (const [label, value, fw] of [["DWG NO.", number, 22], ["SCALE", scale, 26], ["DATE", date, 30], ["UNITS", units, 42]]) {
    if (fx > tx0) cad.addLine([fx, f.min[1]], [fx, f.min[1] + rh], { layer: "FRAME" });
    text(label, [fx + 1.2, f.min[1] + rh - 2.4], { h: 1.6, layer: "FRAME" });
    text(value, [fx + 1.2, f.min[1] + 1.2], { h: 2.5, w: fw - 2.4 });
    fx += fw;
  }
  text(title, [tx0 + 2, ty1 - 5.6], { h: 4, w: tw - brandW - 4 });
  if (subtitle) text(subtitle, [tx0 + 2, f.min[1] + rh + 1.4], { h: 2, w: tw - brandW - 4, layer: "FRAME" });
  text("CodeCAD", [f.max[0] - brandW / 2, f.min[1] + rh + (th - rh) / 2], { h: 3.2, align: "center", valign: "middle", color: ACI.amber });
  return { frame: f, titleBlock: { min: [tx0, f.min[1]], max: [f.max[0], ty1] } };
}

return { ACI, setup, text, rect, table, frame };
