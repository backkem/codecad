// 08 - Distribution board DB-1, single-line diagram (IEC 60617 symbols).
//
// Supply 3/N/PE 400/230 V -> energy meter -> 63 A 4-pole main switch ->
// main busbar. Two residual-current devices each feed a sub-busbar of
// miniature circuit breakers (one 300 mA general group, one 30 mA group
// for the wet rooms), plus a type 2 surge protective device. Every
// outgoing circuit carries its number, MCB rating, phase and cable.
// The symbols are one small block library, reused for the legend.
//
// Exercises: defineBlock (with text + per-shape layers), place (scale,
// rotation), addText rotation, ByLayer dashed linkage, enclosure in
// PHANTOM linetype, title block.

const draft = await cad.use("drafting");
const { text } = draft;
const { ACI } = draft;
draft.setup({
  scale: 1,
  layers: {
    SYM: { color: ACI.white, lineweight: 0.35 },
    WIRE: { color: ACI.white, lineweight: 0.25 },
    BUS: { color: ACI.white, lineweight: 0.7 },
    LINK: { color: ACI.lightgray, linetype: "HID", lineweight: 0.18 },
    TAG: { color: ACI.yellow, lineweight: 0.25 },
    CIRC: { color: ACI.cyan, lineweight: 0.18 },
    PE: { color: ACI.green, lineweight: 0.25 },
  },
});

draft.sheet({
  w: 297, h: 189,
  title: "DB-1 SINGLE-LINE DIAGRAM",
  subtitle: "APARTMENT A1 - TN-S 3/N/PE 400/230 V",
  number: "CC-008", scale: "NTS",
});

// ── Symbol library: insert point = top terminal, 20 mm tall ─────────
const leads = [{ type: "line", start: [0, 0], end: [0, -6] }, { type: "line", start: [0, -14], end: [0, -20] }];
const blade = { type: "line", start: [0, -14], end: [-3.6, -6.6] };
const cross = [{ type: "line", start: [-1, -5], end: [1, -7] }, { type: "line", start: [-1, -7], end: [1, -5] }];
const oval = (cy) => ({ type: "polyline", closed: true, points: Array.from({ length: 16 }, (_, i) => [3.2 * Math.cos((i * Math.PI) / 8), cy + 1.1 * Math.sin((i * Math.PI) / 8)]) });
cad.defineBlock("MCB", [...leads, blade, ...cross], { default_layer: "SYM" });
cad.defineBlock("RCD", [...leads, blade, ...cross, oval(-17),
  { type: "polyline", points: [[3.2, -17], [4.5, -17], [4.5, -10.3], [-1.9, -10.3]], layer: "LINK" }], { default_layer: "SYM" });
cad.defineBlock("ISOLATOR", [...leads, blade, { type: "line", start: [-1.6, -6], end: [1.6, -6] }], { default_layer: "SYM" });
cad.defineBlock("METER", [
  { type: "line", start: [0, 0], end: [0, -5] }, { type: "line", start: [0, -13], end: [0, -20] },
  { type: "polyline", points: [[-6, -5], [6, -5], [6, -13], [-6, -13]], closed: true },
  { type: "text", text: "kWh", at: [0, -9], height: 2.5, align: "center", valign: "middle" },
], { default_layer: "SYM" });
cad.defineBlock("SPD", [
  { type: "line", start: [0, 0], end: [0, -6] }, { type: "line", start: [0, -14], end: [0, -20] },
  { type: "polyline", points: [[-2.5, -6], [2.5, -6], [2.5, -14], [-2.5, -14]], closed: true },
  { type: "polyline", points: [[0, -6.8], [0, -10.6], [-1, -9.4], [1, -10.6], [0, -13.2]] },
], { default_layer: "SYM" });
cad.defineBlock("EARTH", [
  { type: "line", start: [0, 0], end: [0, -3] },
  { type: "line", start: [-3, -3], end: [3, -3] }, { type: "line", start: [-2, -4.2], end: [2, -4.2] }, { type: "line", start: [-1, -5.4], end: [1, -5.4] },
], { default_layer: "PE" });

const wire = (...pts) => cad.addPolyline(pts, { layer: "WIRE" });
const tag = (s, p, sub) => {
  text(s, p, { h: 3, layer: "TAG" });
  if (sub) sub.split("\n").forEach((l, i) => text(l, [p[0], p[1] - 4 - i * 3.4], { h: 2.1, layer: "CIRC" }));
};
// Conductor count marks (n short slashes across a vertical wire)
const slashes = (x, y, n) => {
  for (let i = 0; i < n; i++) {
    const yy = y + (i - (n - 1) / 2) * 1.4;
    cad.addLine([x - 1.6, yy - 1.2], [x + 1.6, yy + 1.2], { layer: "WIRE" });
  }
};

// ── Incoming supply, meter, main switch, busbar ─────────────────────
const xIn = 30, yBus = 126;
text("SUPPLY", [xIn - 8, 181], { h: 3 });
text("3/N/PE 400/230 V 50 Hz", [xIn - 8, 177], { h: 2.2, layer: "CIRC" });
wire([xIn, 174], [xIn, 168]);
slashes(xIn, 171, 4);
cad.place("METER", { at: [xIn, 168] });
tag("-P1", [xIn + 9, 161], "ENERGY METER");
wire([xIn, 148], [xIn, 146]);
cad.place("ISOLATOR", { at: [xIn, 146] });
tag("-Q0", [xIn + 9, 139], "63 A 4P\nMAIN SWITCH");
cad.addLine([xIn - 3.6, 136.2], [xIn - 3.6, 133], { layer: "LINK" });
wire([xIn, 126], [xIn, yBus]);
cad.addLine([xIn, yBus], [262, yBus], { layer: "BUS" });
text("L1 L2 L3 N", [236, yBus + 2], { h: 2.2, layer: "CIRC" });

// ── Circuits ────────────────────────────────────────────────────────
const groups = [
  {
    rcd: "-F1", rcdSub: "40 A 300 mA\nTYPE A", x0: 44,
    circuits: [
      ["C1", "B10", "L1", "3G1.5", "LIGHTS LIVING/HALL"],
      ["C2", "B10", "L2", "3G1.5", "LIGHTS BED/STUDY"],
      ["C3", "B16", "L3", "3G2.5", "SOCKETS LIVING"],
      ["C4", "B16", "L1", "3G2.5", "SOCKETS BED/STUDY"],
      ["C5", "B20", "L2", "3G2.5", "KITCHEN WORKTOP"],
      ["C6", "B32", "L3", "3G6", "COOKER"],
    ],
  },
  {
    rcd: "-F2", rcdSub: "40 A 30 mA\nTYPE A", x0: 150,
    circuits: [
      ["C7", "B16", "L1", "3G2.5", "BATH SOCKETS"],
      ["C8", "B16", "L2", "3G2.5", "WASHING MACHINE"],
      ["C9", "B16", "L3", "3G2.5", "TOWEL HEATER"],
      ["C10", "B16", "-", "-", "SPARE"],
    ],
  },
];
const dx = 15, yRcd = 122, ySub = 96, yMcb = 92, yOut = 62;
for (const g of groups) {
  const xs = g.circuits.map((_, i) => g.x0 + i * dx);
  const xr = (xs[0] + xs[xs.length - 1]) / 2;
  wire([xr, yBus], [xr, yRcd]);
  cad.place("RCD", { at: [xr, yRcd] });
  tag(g.rcd, [xr + 7, yRcd - 6], g.rcdSub);
  wire([xr, yRcd - 20], [xr, ySub]);
  cad.addLine([xs[0], ySub], [xs[xs.length - 1], ySub], { layer: "BUS", lineweight: 0.5 });
  g.circuits.forEach(([num, rating, phase, cable, desc], i) => {
    const x = xs[i];
    wire([x, ySub], [x, yMcb]);
    cad.place("MCB", { at: [x, yMcb] });
    wire([x, yMcb - 20], [x, yOut]);
    if (cable !== "-") slashes(x, yOut + 3, 3);
    text(num, [x, yOut - 4], { h: 2.6, align: "center", layer: "TAG" });
    text(phase === "-" ? rating : `${rating} ${phase}`, [x, yOut - 8], { h: 2, align: "center", layer: "CIRC" });
    text(cable, [x, yOut - 11.5], { h: 2, align: "center", layer: "CIRC" });
    text(desc, [x + 1, yOut - 15], { h: 2.1, rot: 90, align: "right" });
  });
}

// ── Surge protection to the earth bar ───────────────────────────────
const xs = 240;
wire([xs, yBus], [xs, 122]);
cad.place("SPD", { at: [xs, 122] });
tag("-F3", [xs + 5, 116], "SPD TYPE 2\n40 kA");
cad.addLine([xs, 102], [xs, 96], { layer: "PE" });
cad.place("EARTH", { at: [xs, 96] });
text("MET", [xs + 4, 91], { h: 2.1, layer: "PE" });

// ── Enclosure ───────────────────────────────────────────────────────
cad.addPolyline([[20, 68], [272, 68], [272, 152], [20, 152]], { closed: true, layer: "PHANTOM" });
text("DB-1  IP40  24 MODULES", [214, 154], { h: 2.5, layer: "FRAME" });

// ── Legend: the same blocks, laid on their side ─────────────────────
const lx = 152, ly = 182;
text("LEGEND", [lx, ly - 2], { h: 3 });
[["MCB", "CIRCUIT BREAKER"], ["RCD", "RESIDUAL CURRENT DEVICE"], ["ISOLATOR", "SWITCH-DISCONNECTOR"], ["SPD", "SURGE PROTECTIVE DEVICE"]]
  .forEach(([b, label], i) => {
    const y = ly - 9 - i * 6.5, col = i % 2, row = Math.floor(i / 2);
    const x = lx + col * 66, yy = ly - 10 - row * 9;
    cad.place(b, { at: [x, yy], rotation: -90, scale: 0.6 });
    text(label, [x + 15, yy - 1], { h: 2 });
  });

return cad.describe();
