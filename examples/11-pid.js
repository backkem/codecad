// 11 - P&ID: feed tank with level control and pumped transfer with
// flow control (ISA 5.1 symbols).
//
// Feed from unit 100 enters T-101 through level control valve LCV-101
// (LT-101 -> LIC-101). Pump P-101 draws from the tank bottom and
// discharges through a check valve and isolation valve, past PI-103 and
// flow transmitter FT-102, to flow control valve FCV-102 (FIC-102) and
// on to exchanger E-201. Instrument bubbles are one template block with
// placeholder text, cloned per instrument with replaceText; panel-mounted
// instruments get the horizontal bar. Signals are dashed, process lines
// carry flow arrows and line numbers.
//
// Exercises: defineBlock (text placeholders), clone + replaceText, place
// (rotation), addFill arrows, ByLayer linetypes, legend, title block.

const draft = await cad.use("drafting");
const { text, arrow } = draft;
const { ACI } = draft;
draft.setup({
  scale: 1,
  layers: {
    PROC: { color: ACI.white, lineweight: 0.5 },
    EQUIP: { color: ACI.white, lineweight: 0.5 },
    VALVE: { color: ACI.white, lineweight: 0.35 },
    INST: { color: ACI.cyan, lineweight: 0.25 },
    SIGNAL: { color: ACI.cyan, linetype: "HID", lineweight: 0.18 },
    LINENO: { color: ACI.yellow, lineweight: 0.18 },
  },
});

draft.sheet({
  w: 297, h: 189,
  title: "P&ID - FEED SYSTEM",
  subtitle: "T-101 / P-101, UNIT 150",
  number: "CC-011", scale: "NTS",
});

// ── Symbol library ──────────────────────────────────────────────────
// Valves: insert point at the centre, pipe along x.
const bow = [{ type: "polyline", points: [[-4, -2.5], [4, 2.5], [4, -2.5], [-4, 2.5]], closed: true }];
cad.defineBlock("GATE", bow, { default_layer: "VALVE" });
// Check valve: bowtie with the downstream triangle filled (hatched with
// close lines, since block shapes can't carry a solid fill).
const fillTri = [];
for (let x = 0.4; x < 4; x += 0.4) fillTri.push({ type: "line", start: [x, (-2.5 * x) / 4], end: [x, (2.5 * x) / 4] });
cad.defineBlock("CHECK", [...bow, ...fillTri], { default_layer: "VALVE" });
cad.defineBlock("CTRLV", [...bow,
  { type: "line", start: [0, 0], end: [0, 6] },
  { type: "arc", center: [0, 6], radius: 3.2, from: 0, to: 180 },
  { type: "line", start: [-3.2, 6], end: [3.2, 6] }], { default_layer: "VALVE" });
cad.defineBlock("PUMP", [
  { type: "circle", center: [0, 0], radius: 6 },
  { type: "line", start: [0, 6], end: [8, 6] },
  { type: "polyline", points: [[-5, -7], [5, -7], [3.4, -4.4], [-3.4, -4.4]], closed: true },
], { default_layer: "EQUIP" });
// Instrument bubble templates: "XXX" = function letters, "000" = loop no.
const bubble = (panel) => [
  { type: "circle", center: [0, 0], radius: 5.5 },
  ...(panel ? [{ type: "line", start: [-5.5, 0], end: [5.5, 0] }] : []),
  { type: "text", text: "XXX", at: [0, 0.9], height: 2.2, align: "center" },
  { type: "text", text: "000", at: [0, -3.1], height: 2.2, align: "center" },
];
cad.defineBlock("BUBBLE_FIELD", bubble(false), { default_layer: "INST" });
cad.defineBlock("BUBBLE_PANEL", bubble(true), { default_layer: "INST" });
function instrument(tag, at, panel = false) {
  const [fn, no] = tag.split("-");
  cad.clone(panel ? "BUBBLE_PANEL" : "BUBBLE_FIELD", tag, { replaceText: { XXX: fn, "000": no } });
  cad.place(tag, { at });
  return at;
}

const pipe = (...pts) => cad.addPolyline(pts, { layer: "PROC" });
const sig = (...pts) => cad.addPolyline(pts, { layer: "SIGNAL" });
const flow = (p, dir) => arrow(p, dir, { layer: "PROC", size: 3.5 });
const lineNo = (s, at, rot = 0) => text(s, at, { h: 2, layer: "LINENO", rot });
function offPage(label, sub, at, dirRight) {
  const [x, y] = at, w = 26, hh = 4;
  const pts = dirRight
    ? [[x, y - hh], [x + w, y - hh], [x + w + 5, y], [x + w, y + hh], [x, y + hh]]
    : [[x, y - hh], [x + w, y - hh], [x + w, y + hh], [x, y + hh], [x - 5, y]];
  cad.addPolyline(pts, { closed: true, layer: "PROC" });
  text(label, [x + w / 2 + (dirRight ? 0 : -1), y - 1.1], { h: 2.3, align: "center" });
  text(sub, [x + w / 2, y - hh - 4], { h: 1.9, align: "center", layer: "FRAME" });
}

// ── Tank T-101 ──────────────────────────────────────────────────────
const tx0 = 62, tx1 = 92, ty0 = 62, ty1 = 122, txc = (tx0 + tx1) / 2;
cad.addLine([tx0, ty0], [tx0, ty1], { layer: "EQUIP" });
cad.addLine([tx1, ty0], [tx1, ty1], { layer: "EQUIP" });
cad.addArc([txc, ty1 - 14], 21.2, { p1: [tx1, ty1], p2: [tx0, ty1], layer: "EQUIP" });   // dished heads
cad.addArc([txc, ty0 + 14], 21.2, { p1: [tx0, ty0], p2: [tx1, ty0], layer: "EQUIP" });
cad.addLine([tx0, ty1 - 6], [tx1, ty1 - 6], { layer: "THIN" });                         // max level
text("T-101", [txc, 96], { h: 3.5, align: "center" });
text("FEED TANK", [txc, 91], { h: 2.2, align: "center" });
text("5 m³", [txc, 87], { h: 2.2, align: "center" });
// Vent
pipe([txc + 6, ty1 + 6.5], [txc + 6, ty1 + 14], [txc + 10, ty1 + 14], [txc + 10, ty1 + 11]);
text("VENT", [txc + 12, ty1 + 12], { h: 2 });

// ── Feed in through LCV-101 ─────────────────────────────────────────
const yIn = 150;
offPage("FROM U-100", "FEED 10 m³/h", [14, yIn], true);
pipe([45, yIn], [txc - 6, yIn], [txc - 6, ty1 + 6.5]);
cad.place("GATE", { at: [52, yIn] });
cad.place("CTRLV", { at: [66, yIn] });
flow([txc - 6, ty1 + 9], [0, -1]);
lineNo('3"-P-150-01-CS1', [44, yIn + 2], 0);
const lic = instrument("LIC-101", [66, 172], true);
sig([66, 166.5], [66, yIn + 9.2]);
const lt = instrument("LT-101", [108, 108]);
pipe([tx1, 112], [102.5, 112]);
pipe([tx1, 78], [102.5, 78], [102.5, 112]);
sig([108, 113.5], [108, 172], [71.5, 172]);
text("LCV-101", [72, yIn + 9], { h: 2, layer: "INST" });

// ── Pump suction and P-101 ──────────────────────────────────────────
const yS = 40, pc = [130, yS];
pipe([txc, ty0 - 6.5], [txc, yS], [pc[0] - 6, yS]);
cad.place("GATE", { at: [100, yS] });
flow([116, yS], [1, 0]);
lineNo('4"-P-150-02-CS1', [80, yS + 2]);
cad.place("PUMP", { at: pc });
text("P-101", [pc[0], pc[1] - 13], { h: 3.5, align: "center" });
text("10 m³/h @ 32 m", [pc[0], pc[1] - 17.5], { h: 2, align: "center" });
// Drain
pipe([txc, 48], [txc - 12, 48], [txc - 12, 42]);
cad.place("GATE", { at: [txc - 12, 46], rotation: 90 });
text("DRAIN", [txc - 22, 38], { h: 2 });

// ── Discharge: check, isolation, PI, FT, FCV ────────────────────────
const yD = 80;
pipe([pc[0] + 8, yS + 6], [pc[0] + 14, yS + 6], [pc[0] + 14, yD], [258, yD]);
flow([pc[0] + 14, 66], [0, 1]);
cad.place("CHECK", { at: [156, yD] });
cad.place("GATE", { at: [168, yD] });
lineNo('3"-P-150-03-CS1', [pc[0] + 12.5, yS + 9], 90);
instrument("PI-103", [145, 100]);
pipe([145, yD], [145, 94.5]);
// FT-102 with orifice plate (two short bars across the line)
cad.addLine([196, yD - 4], [196, yD + 4], { layer: "VALVE" });
cad.addLine([197.4, yD - 4], [197.4, yD + 4], { layer: "VALVE" });
instrument("FT-102", [196.7, 100]);
cad.addLine([196.7, yD + 4], [196.7, 94.5], { layer: "INST" });
instrument("FIC-102", [226, 122], true);
sig([202.2, 100], [210, 100], [210, 122], [220.5, 122]);
cad.place("CTRLV", { at: [226, yD] });
sig([226, 116.5], [226, yD + 9.2]);
text("FCV-102", [232, yD + 9], { h: 2, layer: "INST" });
flow([246, yD], [1, 0]);
offPage("TO E-201", "EXCHANGER", [258, yD], true);

// ── Legend ──────────────────────────────────────────────────────────
const lx = 178, ly = 172;
text("LEGEND", [lx, ly + 5], { h: 3 });
[["CTRLV", "CONTROL VALVE"], ["GATE", "GATE VALVE"], ["CHECK", "CHECK VALVE"]].forEach(([b, l], i) => {
  cad.place(b, { at: [lx + 4, ly - 9 - i * 10] });
  text(l, [lx + 12, ly - 10 - i * 10], { h: 2.1 });
});
cad.addLine([lx + 50, ly - 4], [lx + 60, ly - 4], { layer: "PROC" });
text("PROCESS", [lx + 63, ly - 5], { h: 2.1 });
cad.addLine([lx + 50, ly - 13], [lx + 60, ly - 13], { layer: "SIGNAL" });
text("ELECTRIC SIGNAL", [lx + 63, ly - 14], { h: 2.1 });
cad.addCircle([lx + 55, ly - 22], 3, { layer: "INST" });
cad.addLine([lx + 52, ly - 22], [lx + 58, ly - 22], { layer: "INST" });
text("PANEL INSTRUMENT", [lx + 63, ly - 23], { h: 2.1 });

return cad.describe();
