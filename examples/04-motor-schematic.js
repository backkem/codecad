// 04 - DOL (direct-on-line) motor starter, IEC 60617 style schematic.
//
// Power circuit: 3-phase supply -> motor protective circuit breaker Q1
// -> contactor KM1 main contacts -> thermal overload F1 -> motor M1.
// Control circuit (230 V): fuse F2 -> F1 trip contact 95-96 -> STOP S1
// (NC) -> START S2 (NO) with KM1 13-14 seal-in -> coil KM1, plus run
// lamp H1. Symbols are a small block library placed with cad.place();
// terminal numbers, device tags (IEC 81346 "-Q1") and a parts list
// complete the sheet. Scale 1:1.
//
// Exercises: defineBlock (incl. text), place, lineCircleIntersection
// (motor entries), addFill (junction dots), dashed mechanical linkage
// via ByLayer linetype, table, title block.

const draft = await cad.use("drafting");
const { P, text } = draft;
draft.setup({
  scale: 1,
  layers: {
    SYM: { color: draft.ACI.white, lineweight: 0.35 },
    WIRE: { color: draft.ACI.white, lineweight: 0.25 },
    PE: { color: draft.ACI.green, lineweight: 0.25 },
    LINK: { color: draft.ACI.lightgray, linetype: "HID", lineweight: 0.18 },
    TAG: { color: draft.ACI.yellow, lineweight: 0.25 },
    TERM: { color: draft.ACI.lightgray, lineweight: 0.18 },
  },
});

draft.sheet({
  w: 297, h: 189,
  title: "DOL MOTOR STARTER",
  subtitle: "POWER + CONTROL CIRCUIT, 400 V 3~ / 230 V",
  number: "CC-004", scale: "NTS",
});

// ── Symbol library (insert point = top terminal, 15 mm tall) ────────
const lead = [{ type: "line", start: [0, 0], end: [0, -5] }, { type: "line", start: [0, -10], end: [0, -15] }];
const blade = (dx) => ({ type: "line", start: [0, -10], end: [dx, -4.4] });
cad.defineBlock("NO", [...lead, blade(-3.4)], { default_layer: "SYM" });
cad.defineBlock("NC", [...lead, { type: "line", start: [0, -5], end: [2.8, -5] }, blade(3.4)], { default_layer: "SYM" });
cad.defineBlock("KM_NO", [...lead, blade(-3.4), { type: "arc", center: [0, -5.9], radius: 0.9, from: 90, to: 270 }], { default_layer: "SYM" });
cad.defineBlock("CB", [...lead, blade(-3.4),
  { type: "line", start: [-1, -4], end: [1, -6] }, { type: "line", start: [-1, -6], end: [1, -4] }], { default_layer: "SYM" });
cad.defineBlock("OL", [
  { type: "line", start: [0, 0], end: [0, -4.5] }, { type: "line", start: [0, -10.5], end: [0, -15] },
  { type: "polyline", points: [[-2.5, -4.5], [2.5, -4.5], [2.5, -10.5], [-2.5, -10.5]], closed: true },
  { type: "polyline", points: [[0, -4.5], [0, -6], [-1.4, -6], [-1.4, -9], [0, -9], [0, -10.5]] },
], { default_layer: "SYM" });
cad.defineBlock("FUSE", [
  { type: "line", start: [0, 0], end: [0, -15] },
  { type: "polyline", points: [[-1.5, -4], [1.5, -4], [1.5, -11], [-1.5, -11]], closed: true },
], { default_layer: "SYM" });
cad.defineBlock("COIL", [
  { type: "line", start: [0, 0], end: [0, -5] }, { type: "line", start: [0, -10], end: [0, -15] },
  { type: "polyline", points: [[-4, -5], [4, -5], [4, -10], [-4, -10]], closed: true },
], { default_layer: "SYM" });
cad.defineBlock("LAMP", [
  { type: "line", start: [0, 0], end: [0, -4] }, { type: "line", start: [0, -11], end: [0, -15] },
  { type: "circle", center: [0, -7.5], radius: 3.5 },
  { type: "line", start: [-2.47, -5.03], end: [2.47, -9.97] }, { type: "line", start: [-2.47, -9.97], end: [2.47, -5.03] },
], { default_layer: "SYM" });
// Push-button actuator (drawn left of a contact): dashed stem + button
cad.defineBlock("PUSH", [
  { type: "line", start: [-1.7, -7.2], end: [-6, -7.2], layer: "LINK" },
  { type: "polyline", points: [[-5, -5.6], [-6, -5.6], [-6, -8.8], [-5, -8.8]] },
], { default_layer: "SYM" });

// Junction dot: SOLID fill (DWG) + concentric rings so it also reads solid
// in the Vello viewer, which paints fills nearly transparent.
const dot = (p) => {
  cad.addFill({ center: p, radius: 0.8 }, { layer: "WIRE" });
  [0.8, 0.55, 0.3].forEach((r) => cad.addCircle(p, r, { layer: "WIRE" }));
};
const wire = (...pts) => cad.addPolyline(pts, { layer: "WIRE" });
const tag = (s, p, sub) => {
  text(s, p, { h: 3, layer: "TAG", align: "right" });
  if (sub) text(sub, [p[0], p[1] - 4], { h: 2, layer: "TERM", align: "right" });
};
const terms = (x, yTop, a, b) => {
  text(a, [x + 1.2, yTop - 3.2], { h: 1.8, layer: "TERM" });
  text(b, [x + 1.2, yTop - 14.2], { h: 1.8, layer: "TERM" });
};

// ── Power circuit ───────────────────────────────────────────────────
const phases = [32, 47, 62];
const xPE = 77, xN = 92, yBus = 172;
["L1", "L2", "L3"].forEach((n, i) => text(n, [phases[i], yBus + 2], { h: 3, align: "center" }));
text("PE", [xPE, yBus + 2], { h: 3, align: "center", layer: "PE" });
text("N", [xN, yBus + 2], { h: 3, align: "center" });
text("3/N/PE ~ 50 Hz 400/230 V", [phases[0] - 4, yBus + 8], { h: 2.5 });

function threePole(block, yTop, tagName, sub, nums) {
  phases.forEach((x, i) => {
    cad.place(block, { at: [x, yTop] });
    if (nums) terms(x, yTop, nums[2 * i], nums[2 * i + 1]);
  });
  if (block !== "OL") cad.addLine([phases[0] - 1.7, yTop - 7.2], [phases[2] + 1, yTop - 7.2], { layer: "LINK" });
  tag(tagName, [phases[0] - 6, yTop - 6.5], sub);
}

const yQ1 = 160, yKM = 132, yF1 = 104;
phases.forEach((x) => wire([x, yBus], [x, yQ1]));
threePole("CB", yQ1, "-Q1", "6.3-10 A", ["1", "2", "3", "4", "5", "6"]);
phases.forEach((x) => wire([x, yQ1 - 15], [x, yKM]));
threePole("KM_NO", yKM, "-KM1", "AC-3 9 A", ["1", "2", "3", "4", "5", "6"]);
phases.forEach((x) => wire([x, yKM - 15], [x, yF1]));
threePole("OL", yF1, "-F1", "7-10 A", ["1", "2", "3", "4", "5", "6"]);

// Motor: phases drop vertically and enter where they meet the circle
const mC = [47, 60], mR = 18;
phases.forEach((x, i) => {
  const hit = cad.lineCircleIntersection([[x, yF1 - 15], [x, 0]], mC, mR).sort((a, b) => b[1] - a[1])[0];
  wire([x, yF1 - 15], hit);
  text(["U1", "V1", "W1"][i], [x + 1.2, hit[1] + 2], { h: 1.8, layer: "TERM" });
});
cad.addCircle(mC, mR, { layer: "SYM" });
text("M", [mC[0], mC[1] + 1], { h: 6, align: "center" });
text("3~", [mC[0], mC[1] - 6], { h: 3.5, align: "center" });
tag("-M1", [phases[0] - 6, mC[1] + 4], "4 kW 8.2 A");
// PE to motor frame
const pe = cad.lineCircleIntersection([[200, mC[1] - 9], mC.map((v, i) => (i ? mC[1] - 9 : v))], mC, mR).sort((a, b) => b[0] - a[0])[0];
cad.addPolyline([[xPE, yBus], [xPE, pe[1]], pe], { layer: "PE" });
text("PE", [pe[0] + 2, pe[1] + 1.2], { h: 1.8, layer: "PE" });

// ── Control circuit ─────────────────────────────────────────────────
const xc1 = 122, xc2 = 142, xc3 = 166, yTap = 167, yN = 52;
dot([phases[0], yTap]);
wire([phases[0], yTap], [xc1, yTap], [xc1, 162]);
cad.place("FUSE", { at: [xc1, 162] });
tag("-F2", [xc1 - 5, 156], "2 A gG");
terms(xc1, 162, "1", "2");
cad.place("NC", { at: [xc1, 142] }); wire([xc1, 147], [xc1, 142]);
tag("-F1", [xc1 - 5, 136]);
terms(xc1, 142, "95", "96");
wire([xc1, 127], [xc1, 122]);
cad.place("NC", { at: [xc1, 122] }); cad.place("PUSH", { at: [xc1, 122] });
tag("-S1", [xc1 - 9, 116], "STOP");
terms(xc1, 122, "11", "12");
wire([xc1, 107], [xc1, 100]);
dot([xc1, 100]);
wire([xc1, 100], [xc2, 100], [xc2, 95]);
wire([xc1, 100], [xc1, 95]);
cad.place("NO", { at: [xc1, 95] }); cad.place("PUSH", { at: [xc1, 95] });
tag("-S2", [xc1 - 9, 89], "START");
terms(xc1, 95, "13", "14");
cad.place("KM_NO", { at: [xc2, 95] });
text("-KM1", [xc2 + 4, 87.5], { h: 3, layer: "TAG" });
terms(xc2, 95, "13", "14");
wire([xc1, 80], [xc1, 75]);
wire([xc2, 80], [xc2, 75], [xc1, 75]);
dot([xc1, 75]);
wire([xc1, 75], [xc1, 70]);
cad.place("COIL", { at: [xc1, 70] });
tag("-KM1", [xc1 - 6, 63]);
text("A1", [xc1 + 1.2, 66.8], { h: 1.8, layer: "TERM" });
text("A2", [xc1 + 1.2, 55.8], { h: 1.8, layer: "TERM" });
wire([xc1, 55], [xc1, yN]);
// Run lamp in parallel with the coil
wire([xc2, 75], [xc3, 75], [xc3, 70]);
dot([xc2, 75]);
cad.place("LAMP", { at: [xc3, 70] });
text("-H1", [xc3 + 5, 63], { h: 3, layer: "TAG" });
text("RUN", [xc3 + 5, 59], { h: 2, layer: "TERM" });
wire([xc3, 55], [xc3, yN]);
// Neutral return
cad.addLine([xN, yBus], [xN, yN], { layer: "WIRE" });
cad.addLine([xN, yN], [xc3, yN], { layer: "WIRE" });
dot([xc1, yN]);

// ── Parts list ──────────────────────────────────────────────────────
draft.table([190, 172], [
  ["TAG", "DESCRIPTION"],
  ["-Q1", "MOTOR CB 3P 6.3-10 A"],
  ["-KM1", "CONTACTOR 9 A, 230 V COIL"],
  ["-F1", "THERMAL OVERLOAD 7-10 A"],
  ["-F2", "FUSE 2 A gG"],
  ["-S1", "PUSHBUTTON NC, RED"],
  ["-S2", "PUSHBUTTON NO, GREEN"],
  ["-H1", "PILOT LAMP 230 V, GREEN"],
  ["-M1", "MOTOR 4 kW 400 V 8.2 A"],
], [14, 76], { title: "PARTS LIST", h: 2.3, rowH: 5.5 });

text("SEQUENCE OF OPERATION", [190, 112], { h: 3.5 });
[
  "1. CLOSE -Q1. PRESS -S2: -KM1 ENERGISES",
  "   AND SEALS IN THROUGH -KM1 13-14.",
  "2. -H1 SHOWS RUN WHILE -KM1 IS ON.",
  "3. PRESS -S1, OR OVERLOAD -F1 TRIPS",
  "   (95-96 OPENS): -KM1 DROPS OUT.",
  "4. -Q1 GIVES SHORT-CIRCUIT PROTECTION.",
].forEach((l, i) => text(l, [190, 104 - i * 5], { h: 2.3 }));

text("CONTROL 230 V AC", [xc1 - 6, 34], { h: 2.5 });
text("POWER 400 V AC", [phases[0] - 4, 34], { h: 2.5 });

return cad.describe();
