// 06 - Piston and connecting rod, half-section assembly (scale 1:1) with
// DETAIL A of the ring grooves at 3:1. The rod shank is shown broken
// (conventional break) so the 145 mm centre distance fits the sheet.
//
// Left of the axis the piston is cut (hatched; the hollow gudgeon pin is
// cut across, hatched the other way); right of the axis is the outside
// view with the ring grooves and the pin end. The con-rod is not
// sectioned (ISO 128-50: rods/shafts are not cut lengthwise). Its shank
// is filleted into both eyes with true tangent arcs, then the parts that
// are hidden inside the piston are trimmed away. Hidden detail is
// omitted in the half-section, per convention.
//
// Exercises: addHatch, regionClip, fillet (line-circle), trimTo (circle + lines), mirror,
// dimDiameter, measure, detail view clipped with trimTo, table.

const draft = await cad.use("drafting");
const { P, dim, text, leader, hatch, add, sub, mul } = draft;
draft.setup({ scale: 1, layers: { ROD: { color: draft.ACI.white, lineweight: 0.5 } } });

draft.sheet({
  at: [-P(92), -P(141)], w: 297, h: 189,
  title: "PISTON + CONNECTING ROD",
  subtitle: "86 mm BORE - HALF SECTION",
  number: "CC-006", scale: "1:1",
});

// ── Piston parameters (mm), pin centre at the origin ─────────────────
const R = 43, top = 32, bottom = -38;           // bore 86, compression height 32
const root = 39.4;                              // ring groove root radius
const grooves = [[26, 24.8], [20.8, 19.3], [15.8, 12.8]]; // [top, bottom] y of each groove
const pinR = 11, pinBore = 6.5, eyeR = 15, bushR = 12.5;
const cc = 145, bigR = 36, shellR = 26, journalR = 24;

// ── Piston: left half in section ────────────────────────────────────
// Crown + ring belt (the oil groove is drilled through to the inside,
// so it separates the ring belt from the skirt in this section).
// (Bulge vertices would make the fillet a true polyline arc, but bulged
// polylines currently hang the browser renderer, so the R6 fillet is an
// addArc and the hatch boundary samples it.)
const fC = [-30, 18], fR = 6;                     // crown/wall fillet
const filletPts = Array.from({ length: 9 }, (_, i) => draft.polar(fC, fR, 180 - (90 * i) / 8));
const beltEdge = [                                // outline, axis edge left open
  [0, 24], [0, top], [-R + 0.5, top], [-R, top - 0.5],
  [-R, grooves[0][0]], [-root, grooves[0][0]], [-root, grooves[0][1]], [-R, grooves[0][1]],
  [-R, grooves[1][0]], [-root, grooves[1][0]], [-root, grooves[1][1]], [-R, grooves[1][1]],
  [-R, grooves[2][0]], [-36, grooves[2][0]], [-36, 18],
];
const belt = [...beltEdge.slice(1), ...filletPts.slice(1), [0, 24]];   // closed region for hatching
const skirt = [[-R, grooves[2][1]], [-36, grooves[2][1]], [-36, 10], [-40, 6], [-40, bottom], [-R, bottom]];
cad.addPolyline(beltEdge.slice(1), { layer: "OUTLINE" });
cad.addArc(fC, fR, { from: 90, to: 180, layer: "OUTLINE" });
cad.addLine([-30, 24], [0, 24], { layer: "OUTLINE" });
cad.addPolyline(skirt, { closed: true, layer: "OUTLINE" });
hatch(belt, { angle: 45, spacing: 1.6 });
hatch(skirt, { angle: 45, spacing: 1.6 });

// ── Piston: right half, outside view ────────────────────────────────
const sil = [[0, top], [R - 0.5, top], [R, top - 0.5], [R, grooves[0][0]]];
cad.addPolyline(sil, { layer: "OUTLINE" });
cad.addLine([0, top - 0.5], [R, top - 0.5], { layer: "OUTLINE" });          // chamfer edge
grooves.forEach(([yt, yb], i) => {
  cad.addLine([0, yt], [R, yt], { layer: "OUTLINE" });
  cad.addLine([0, yb], [R, yb], { layer: "OUTLINE" });
  cad.addLine([root, yt], [root, yb], { layer: "OUTLINE" });
  const next = i < 2 ? grooves[i + 1][0] : bottom;
  cad.addLine([R, yb], [R, next], { layer: "OUTLINE" });
});
cad.addLine([0, bottom], [R, bottom], { layer: "OUTLINE" });
// Pin end, seen through the pin bore in the skirt
cad.addArc([0, 0], pinR, { from: -90, to: 90, layer: "OUTLINE" });
cad.addArc([0, 0], pinBore, { from: -90, to: 90, layer: "OUTLINE" });

// ── Gudgeon pin: cut across in the section half ─────────────────────
const leftHalf = [[-50, -50], [0, -50], [0, 50], [-50, 50]];
const pinCut = cad.regionClip(cad.regionClip({ center: [0, 0], radius: pinR }, { center: [0, 0], radius: pinBore }, "subtract").map((r) => r.outer), leftHalf, "intersect");
cad.addArc([0, 0], pinR, { from: 90, to: 270, layer: "OUTLINE" });
cad.addArc([0, 0], pinBore, { from: 90, to: 270, layer: "OUTLINE" });
pinCut.forEach((r) => hatch(r.outer, { angle: 135, spacing: 1.2 }));

// ── Connecting rod: eyes + shank, true fillets ──────────────────────
const eyeS = cad.addCircle([0, 0], eyeR, { layer: "ROD" });
const eyeB = cad.addCircle([0, -cc], bigR, { layer: "ROD" });
const shankL = cad.addLine([-8.5, 0], [-15, -cc], { layer: "ROD" });
const shankR = cad.addLine([8.5, 0], [15, -cc], { layer: "ROD" });
const fSL = cad.fillet(shankL, eyeS, 6).arc;
const fSR = cad.fillet(shankR, eyeS, 6).arc;
const fBL = cad.fillet(shankL, eyeB, 10).arc;
const fBR = cad.fillet(shankR, eyeB, 10).arc;
// Keep the outer parts of the eyes, between the fillets.
const eyeSArc = cad.trimTo(eyeS, [fSL, fSR], [0, eyeR]);
cad.trimTo(eyeB, [fBL, fBR], [0, -cc - bigR]);
// Inside the piston the rod is only visible in the cut (left) half.
const axis = cad.addLine([0, 60], [0, -60], { layer: "CENTER" });
const skirtEdge = cad.addLine([0, bottom], [R, bottom], { layer: "CENTER" });
cad.trimTo(eyeSArc, axis, [-eyeR, 0]);
cad.remove(fSR);
cad.trimTo(shankR, skirtEdge, [15, -cc + 20]);
cad.remove([axis, skirtEdge]);
cad.addArc([0, 0], bushR, { from: 90, to: 270, layer: "ROD" });
// H-section recess (bulge ends), below the skirt
const xAt = (y) => 8.5 + (6.5 * -y) / cc - 4;
const [ra, rb] = [xAt(-46), xAt(-108)];
const recL = cad.addLine([-ra, -46], [-rb, -108], { layer: "THIN" });
const recR = cad.addLine([ra, -46], [rb, -108], { layer: "THIN" });
cad.addArc([0, -108], rb, { from: 180, to: 360, layer: "THIN" });
cad.addArc([0, -46], ra, { from: 0, to: 180, layer: "THIN" });
// Big end: bearing shells, cap joint, cap bolts (hidden)
cad.addCircle([0, -cc], shellR, { layer: "ROD" });
const journal = cad.addCircle([0, -cc], journalR, { layer: "THIN" });
for (const s of [-1, 1]) {
  cad.addLine([s * shellR, -cc], [s * bigR, -cc], { layer: "ROD" });
  for (const x of [27.5, 34]) {
    const h = Math.sqrt(bigR * bigR - x * x) - 1;
    cad.addLine([s * x, -cc + h], [s * x, -cc - h], { layer: "HIDDEN" });
  }
  cad.addLine([s * 30.75, -cc + 20], [s * 30.75, -cc - 20], { layer: "CENTER" });
}

// ── Conventional break in the shank ─────────────────────────────────
// Cut every member crossing the band between yA and yB, draw wavy break
// lines, then slide the lower part up by the gap.
const yA = -47, yB = -103, gap = yA - yB;
const cutA = cad.addLine([-60, yA], [60, yA], { layer: "THIN" });
const cutB = cad.addLine([-60, yB], [60, yB], { layer: "THIN" });
for (const ln of [shankL, shankR, recL, recR]) {
  const lower = cad.copy(ln, 0, 0)[0];
  const e = cad.entity(ln.id);   // cad.entity() takes an id string, not an entity
  cad.trimTo(ln, cutA, e.start);          // upper piece ends at yA
  cad.trimTo(lower, cutB, e.end);         // lower piece starts at yB
}
cad.remove([cutA, cutB]);
const halfW = (y) => 8.5 + (6.5 * -y) / cc;
const wavy = (y, w, dy) => cad.addSpline([[-w, y + dy], [-w / 2, y + dy + 1.6], [0, y + dy], [w / 2, y + dy - 1.6], [w, y + dy]], { fit: true, layer: "THIN" });
const rodLayers = ["ROD", "THIN", "HIDDEN", "CENTER"];
const below = cad.entities().filter((e) => rodLayers.includes(e.layer) && e.bounds.max[1] <= yB + 1e-6 && e.bounds.min[1] > -cc - bigR - 1);
const shift = gap - 8;     // keep an 8 mm gap between the break lines
cad.move(below, 0, shift);
wavy(yA, halfW(yA), 0);
wavy(yB, halfW(yB), shift);
const ccDraw = cc - shift;   // drawn centre distance

// ── Centre lines ────────────────────────────────────────────────────
cad.addLine([0, top + P(6)], [0, -ccDraw - bigR - P(5)], { layer: "CENTER" });
cad.addLine([-eyeR - P(4), 0], [eyeR + P(4), 0], { layer: "CENTER" });
cad.addLine([-bigR - P(4), -ccDraw], [bigR + P(4), -ccDraw], { layer: "CENTER" });

// ── Dimensions + callouts ───────────────────────────────────────────
dim([-R, top], [R, top], 5, { dir: "h", label: "Ø<>" });
dim([0, 0], [R, top], -8, { dir: "v" });
dim([R, bottom], [R, top], -17, { dir: "v" });
dim([0, -ccDraw], [0, 0], -(R / P(1) + 32), { dir: "v", label: String(cc) });
draft.withOutlines(cad.dimDiameter(journal, { angle: -40, label: "Ø<>", text_height: P(3), layer: "DIM" }));
leader([-41.5, -28], [-R - P(12), -P(20)], "PISTON", { h: 3 });
leader([-pinR + 2, -4], [-R - P(12), P(4)], "PIN", { h: 3 });
leader([-halfW(-40) - 0.5, -44], [-R - P(10), -P(40)], "CON-ROD", { h: 3 });

// Detail marker on the ring belt
const Ac = [-40, 21], Ar = 11.6;
cad.addCircle(Ac, Ar, { layer: "THIN" });
text("A", add(Ac, [-Ar - P(6), Ar - P(5)]), { h: 5 });

// ── DETAIL A (2:1 on the sheet = 4x the main view) ──────────────────
const k = 3, Dc = [P(136), P(-9)];
const T = (p) => add(Dc, mul(sub(p, Ac), k));
const frame = cad.addCircle(Dc, Ar * k, { layer: "THIN" });
const inside = (p) => Math.hypot(p[0] - Dc[0], p[1] - Dc[1]) < Ar * k - 1e-6;
// Section material of the ring belt, clipped to the detail circle
const beltD = cad.regionClip(belt.map((p) => T(p)), { center: Dc, radius: Ar * k }, "intersect");
const skirtD = cad.regionClip(skirt.map((p) => T(p)), { center: Dc, radius: Ar * k }, "intersect");
[...beltD, ...skirtD].forEach((r) => hatch(r.outer, { angle: 45, spacing: 2.5 }));
// Visible edges: profile segments, trimmed to the detail circle
for (const ring of [belt, skirt]) {
  ring.forEach((p, i) => {
    const q = ring[(i + 1) % ring.length];
    const a = T(p), b = T(q);
    if ((p[0] === 0 && q[0] === 0) || (ring === belt && i >= beltEdge.length - 2 && i < beltEdge.length - 2 + filletPts.length - 1)) return; // axis, fillet
    const inA = inside(a), inB = inside(b);
    if (!inA && !inB && !cad.lineCircleIntersection([a, b], Dc, Ar * k).length) return;
    const ln = cad.addLine(a, b, { layer: "OUTLINE" });
    if (inA && inB) return;
    try { cad.trimTo(ln, frame, inA ? a : inB ? b : cad.midpoint(a, b)); } catch (e) { cad.remove(ln); }
  });
}
cad.addArc(T(fC), fR * k, { from: 90, to: 180, layer: "OUTLINE" });
const dd = (p, q, off, label, o = {}) => dim(T(p), T(q), off, { label, h: 3, ...o });
dd([-R, grooves[0][1]], [-R, grooves[0][0]], 8, "1.2", { dir: "v", h: 2.5 });
dd([-R, grooves[1][1]], [-R, grooves[1][0]], 8, "1.5", { dir: "v", h: 2.5 });
dd([-R, grooves[2][1]], [-R, grooves[2][0]], 8, "3", { dir: "v", h: 2.5 });
dd([-R, grooves[0][0]], [-R, top], 17, "6", { dir: "v", h: 2.5 });
dd([-R, grooves[0][1]], [-root, grooves[0][1]], -5, "3.6", { dir: "h", h: 2.5 });
text("DETAIL A", [Dc[0], Dc[1] - Ar * k - P(8)], { h: 4, align: "center" });
text("SCALE 3:1", [Dc[0], Dc[1] - Ar * k - P(13)], { h: 2.5, align: "center" });

// ── Data table ──────────────────────────────────────────────────────
draft.table([P(203 - 92), P(72 - 141)], [
  ["ITEM", "DATA"],
  ["PISTON", "AlSi12CuNi, Ø86 x 70"],
  ["COMPRESSION HEIGHT", "32"],
  ["RINGS", "1.2 / 1.5 / 3.0"],
  ["GUDGEON PIN", "Ø22 x 60, BORE Ø13"],
  ["CON-ROD", "42CrMo4, 145 C-C"],
  ["BIG END", "Ø48 PIN, SHELLS"],
], [40, 44], { title: "PART DATA", h: 2.3, rowH: 5.5 });

return cad.describe();
