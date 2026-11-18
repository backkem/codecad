// 01 - Plate flange DN80 PN16 (EN 1092-1 type 01), two-view drawing.
//
// Front view with the bolt pattern on its pitch circle, and full section
// A-A (holes revolved into the cutting plane, as ISO 128 allows).
// Scale 1:2 on a 297 x 189 sheet.
//
// Exercises: shared drafting lib (cad.use), layers with ByLayer ACI
// colours / linetypes / lineweights, array (polar), regionClip (union /
// subtract) for the section profile, addHatch, measure with Ø labels,
// addFill arrowheads, leaders, title block.

const draft = await cad.use("drafting");
const { P, dim, leader, text, hatch, centerMark, arrow, polar } = draft;
draft.setup({ scale: 2 });

// ── Parameters (EN 1092-1, PN16, DN80, type 01) ─────────────────────
const D = 200;      // outside diameter
const K = 160;      // pitch circle diameter (PCD)
const nBolt = 8;    // bolt holes
const dHole = 18;   // hole diameter (M16 bolts)
const B = 91;       // bore (slip-on for 88.9 pipe)
const C = 18;       // plate thickness
const dRF = 138;    // raised face diameter
const fRF = 2;      // raised face height

const R = D / 2, rK = K / 2, rH = dHole / 2, rB = B / 2, rRF = dRF / 2;

// ── Sheet ───────────────────────────────────────────────────────────
draft.sheet({
  at: [0, 0], w: 297, h: 189,
  title: "PLATE FLANGE DN80 PN16",
  subtitle: "EN 1092-1 TYPE 01  -  S235JR",
  number: "CC-001", scale: "1:2",
});

// ── Front view ──────────────────────────────────────────────────────
const F = [P(88), P(100)];
cad.addCircle(F, R, { layer: "OUTLINE" });
cad.addCircle(F, rRF, { layer: "OUTLINE" });
cad.addCircle(F, rB, { layer: "OUTLINE" });
cad.addCircle(F, rK, { layer: "CENTER" });           // pitch circle
centerMark(F, R, { ext: 5 });

// One hole + its radial centreline, then polar-copy the master.
// Holes straddle the main centrelines (first hole at 360/n/2).
const half = 180 / nBolt;
const hc = polar(F, rK, half);
const hole = cad.addCircle(hc, rH, { layer: "OUTLINE" });
const holeCL = cad.addLine(polar(F, rK - rH - P(2), half), polar(F, rK + rH + P(2), half), { layer: "CENTER" });
cad.array([hole, holeCL], { polar: { center: F, count: nBolt } });

// Cutting plane A-A along the vertical centreline, viewed from the left.
const cpTop = [F[0], F[1] + R + P(8)], cpBot = [F[0], F[1] - R - P(8)];
for (const [end, inward] of [[cpTop, -1], [cpBot, 1]]) {
  cad.addLine(end, [end[0], end[1] + inward * P(6)], { layer: "OUTLINE", lineweight: 0.7 });
  const tip = [end[0] + P(8), end[1]];
  cad.addLine(end, tip, { layer: "OUTLINE" });
  arrow(tip, [1, 0], { layer: "OUTLINE", size: 3.5 });
  text("A", [tip[0] + P(2), end[1] - P(2.5)], { h: 5 });
}

// Hole callout (pattern note)
const hAng = 90 + half;                                  // hole at 112.5 deg
const hTip = polar(polar(F, rK, hAng), rH, 135);
leader(hTip, [F[0] - P(38), F[1] + P(62)], `${nBolt}x Ø${dHole} THRU\nEQ SP ON Ø${K} PCD`, { h: 3 });

// ── Section A-A ─────────────────────────────────────────────────────
// x across the thickness (back face at x0, raised face to the right),
// y vertical through the flange axis.
const x0 = P(178), yc = F[1];
const xC = x0 + C, xF = x0 + C + fRF;
const Y = (y) => yc + y;

// Section profile by region booleans: plate + raised face, minus the
// bore and the two bolt holes revolved into the cutting plane.
const box = (xa, ya, xb, yb) => [[xa, Y(ya)], [xb, Y(ya)], [xb, Y(yb)], [xa, Y(yb)]];
const body = cad.regionClip(box(x0, -R, xC, R), box(xC, -rRF, xF, rRF), "union").map((r) => r.outer);
const cuts = [box(x0 - 1, -rB, xF + 1, rB), box(x0 - 1, rK - rH, xC + 1, rK + rH), box(x0 - 1, -rK - rH, xC + 1, -rK + rH)];
const regions = cad.regionClip(body, cuts, "subtract");
for (const r of regions) {
  cad.addPolyline(r.outer, { closed: true, layer: "OUTLINE" });
  hatch(r.outer, { angle: 45, spacing: 2 });
}
// Bore silhouette: the far half's bore edges seen through the cut
cad.addLine([x0, Y(rB)], [x0, Y(-rB)], { layer: "OUTLINE" });
cad.addLine([xF, Y(rB)], [xF, Y(-rB)], { layer: "OUTLINE" });
// Axis + hole centrelines
cad.addLine([x0 - P(5), yc], [xF + P(5), yc], { layer: "CENTER" });
for (const s of [1, -1]) cad.addLine([x0 - P(3), Y(s * rK)], [xC + P(3), Y(s * rK)], { layer: "CENTER" });

text("SECTION A-A", [(x0 + xF) / 2, Y(R) + P(14)], { h: 5, align: "center" });

// Section dimensions: diameters stacked right, bore left, thickness below
dim([xF, Y(-rRF)], [xF, Y(rRF)], -8, { dir: "v", label: "Ø<>" });
dim([xC, Y(-rK)], [xC, Y(rK)], -17, { dir: "v", label: "Ø<>" });
dim([xC, Y(-R)], [xC, Y(R)], -25, { dir: "v", label: "Ø<>" });
dim([x0, Y(-rB)], [x0, Y(rB)], 8, { dir: "v", label: "Ø<>" });
dim([x0, Y(-R)], [xC, Y(-R)], -8, { dir: "h" });
dim([x0, Y(-R)], [xF, Y(-rRF)], -16, { dir: "h" });

// ── Notes ───────────────────────────────────────────────────────────
const nx = P(226), ny = P(150);
text("NOTES", [nx, ny], { h: 3.5 });
[
  "1. DIMENSIONS IN mm",
  "2. RAISED FACE Ra 3.2-6.3",
  "3. BREAK SHARP EDGES 0.5",
  "4. BOLTS M16 x 8",
].forEach((s, i) => text(s, [nx, ny - P(7 + i * 5.5)], { h: 2.5 }));

return cad.describe();
