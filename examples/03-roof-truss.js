// 03 - Howe roof truss, 12 m span, 3 m rise (pitch 26.6 deg), 6 panels.
//
// Timber members are drawn at their real depth around the setting-out
// lines: chords 47x150, webs 47x100 (C24). Chord ends and the apex cut
// come from line intersections of the offset member edges; every web is
// the member strip clipped to the opening between the chords (region
// intersect), and the diagonals are cut back to the vertical faces
// (region subtract). Nail plates, wall bearings, chained dimensions, the
// pitch angle and a member schedule complete the sheet. Scale 1:50.
//
// Exercises: lineLineIntersection, regionClip (intersect / subtract),
// addHatch, measure (chained + overall), dimAngle, mirror, custom
// scaled linetypes, shared title block.

const draft = await cad.use("drafting");
const { P, dim, text, offsetLine, add, sub, mul, unit, perp } = draft;
draft.setup({
  scale: 50,
  layers: {
    TIMBER: { color: draft.ACI.white, lineweight: 0.5 },
    SETOUT: { color: draft.ACI.gray, linetype: "CL", lineweight: 0.13 },
    PLATE: { color: draft.ACI.gray, lineweight: 0.18 },
    WALL: { color: draft.ACI.lightgray, lineweight: 0.35 },
  },
});

// ── Parameters (mm) ─────────────────────────────────────────────────
const L = 12000, H = 3000, nPanels = 6;
const pw = L / nPanels;            // panel width
const ch = 150 / 2;                // chord half-depth
const wh = 100 / 2;                // web half-depth
const yTop = (x) => H * (1 - Math.abs(x - L / 2) / (L / 2)); // setting-out of top chord

// ── Sheet: truss setting-out origin at paper (28, 82) ───────────────
draft.sheet({
  at: [-P(28), -P(82)], w: 297, h: 189,
  title: "ROOF TRUSS T1 - HOWE",
  subtitle: "12.0 m SPAN, 26.6 DEG PITCH, C24 TIMBER",
  number: "CC-003", scale: "1:50",
});

// ── Setting-out lines (member centrelines) ──────────────────────────
const nodesB = Array.from({ length: nPanels + 1 }, (_, i) => [i * pw, 0]);
const nodesT = nodesB.map(([x]) => [x, yTop(x)]);
cad.addLine([-P(4), 0], [L + P(4), 0], { layer: "SETOUT" });
cad.addLine([0, 0], [L / 2, H], { layer: "SETOUT" });
cad.addLine([L / 2, H], [L, 0], { layer: "SETOUT" });

// ── Chords from offset edges + intersections ────────────────────────
const tcL = [[0, 0], [L / 2, H]];
const upL = offsetLine(tcL, ch), loL = offsetLine(tcL, -ch);
const seat = [[-1000, ch], [L + 1000, ch]];          // top face of bottom chord
const plumb = [[L / 2, -1000], [L / 2, H + 1000]];    // apex joint
const P3 = cad.lineLineIntersection(upL, seat);       // heel: top chord outer corner
const P2 = cad.lineLineIntersection(loL, seat);       // heel: inner corner
const apexU = cad.lineLineIntersection(upL, plumb);
const apexL = cad.lineLineIntersection(loL, plumb);
const mirrorX = (p) => [L - p[0], p[1]];

const leftChord = cad.addPolyline([P3, P2, apexL, apexU], { closed: true, layer: "TIMBER" });
cad.mirror(cad.copy(leftChord, 0, 0), [L / 2, 0], [L / 2, 1]);
cad.addPolyline([[P3[0], -ch], [L - P3[0], -ch], [L - P3[0], ch], [P3[0], ch]], { closed: true, layer: "TIMBER" });

// Opening between the chords: every web is clipped to it.
const opening = [P2, mirrorX(P2), apexL];

// ── Webs ────────────────────────────────────────────────────────────
// Member strip around a setting-out segment, overshooting both ends.
function strip(a, b, half) {
  const d = unit(sub(b, a));
  const a2 = sub(a, mul(d, 600)), b2 = add(b, mul(d, 600));
  const n = mul(perp(d), half);
  return [add(a2, n), add(b2, n), sub(b2, n), sub(a2, n)];
}
const drawRegions = (regions) => regions.forEach((r) => cad.addPolyline(r.outer, { closed: true, layer: "TIMBER" }));

const verticals = nodesB.slice(1, -1).map((n, i) => strip(n, nodesT[i + 1], wh));
verticals.forEach((v) => drawRegions(cad.regionClip(v, opening, "intersect")));
nodesB.slice(1, -1).forEach((n, i) => cad.addLine(n, nodesT[i + 1], { layer: "SETOUT" }));

// Howe diagonals rise toward midspan: bottom node i -> top node i+1 (left half)
const diagonals = [];
for (let i = 1; i < nPanels / 2; i++) {
  diagonals.push([nodesB[i], nodesT[i + 1]]);
  diagonals.push([mirrorX(nodesB[i]), mirrorX(nodesT[i + 1])]);
}
for (const [a, b] of diagonals) {
  const inside = cad.regionClip(strip(a, b, wh), opening, "intersect").map((r) => r.outer);
  drawRegions(cad.regionClip(inside, verticals, "subtract"));
  cad.addLine(a, b, { layer: "SETOUT" });
}

// ── Nail plates (both faces), centred on the joints ─────────────────
function plate(c, w, h, rotDeg = 0) {
  const t = (rotDeg * Math.PI) / 180, u = [Math.cos(t), Math.sin(t)], v = perp(u);
  const pts = [[-w / 2, -h / 2], [w / 2, -h / 2], [w / 2, h / 2], [-w / 2, h / 2]]
    .map(([x, y]) => add(c, add(mul(u, x), mul(v, y))));
  cad.addPolyline(pts, { closed: true, layer: "PLATE" });
}
const slope = (Math.atan2(H, L / 2) * 180) / Math.PI;
nodesB.slice(1, -1).forEach((n) => plate(add(n, [0, 60]), 260, 260));
[1, 2].forEach((i) => {
  plate(add(nodesT[i], mul(perp(unit([L / 2, H])), -40)), 300, 230, slope);
  plate(mirrorX(add(nodesT[i], mul(perp(unit([L / 2, H])), -40))), 300, 230, -slope);
});
plate([L / 2, H - 230], 380, 300);
for (const hx of [0, L]) plate([hx + (hx ? -1 : 1) * 260, 70], 600, 260);

// ── Bearings: wall plate on masonry, with break line ────────────────
for (const x of [0, L]) {
  const wp = [[x - 50, -ch - 50], [x + 50, -ch - 50], [x + 50, -ch], [x - 50, -ch]];
  cad.addPolyline(wp, { closed: true, layer: "TIMBER" });
  cad.addLine(wp[0], wp[2], { layer: "THIN" });          // timber end-grain cross
  cad.addLine(wp[1], wp[3], { layer: "THIN" });
  const top = -ch - 50, bot = top - 600, hw = 107.5;
  const wall = [[x - hw, bot], [x - hw, top], [x + hw, top], [x + hw, bot],
    [x + 40, bot], [x + 15, bot + 60], [x - 15, bot - 60], [x - 40, bot]];
  cad.addPolyline(wall, { closed: true, layer: "WALL" });
  draft.hatch(wall, { angle: 45, spacing: 1.6, layer: "HATCH" });
}

// ── Dimensions ──────────────────────────────────────────────────────
const below = -(675 + P(9)) / P(1);   // clear of the walls
nodesB.slice(0, -1).forEach((n, i) => dim(n, nodesB[i + 1], below, { dir: "h", h: 3 }));
dim(nodesB[0], nodesB[nPanels], below - 9, { dir: "h", h: 3.5 });
dim([L, 0], [L / 2, H], -12, { dir: "v", h: 3 });
const ang = cad.dimAngle([0, 0], [1, 0], [L / 2, H], { radius: P(22), text_height: P(3), layer: "DIM", label: "<>°" });
draft.outlineFills(ang.ids);

// ── Member schedule + notes ─────────────────────────────────────────
draft.table([-P(14), P(92)], [
  ["MEMBER", "SIZE", "GRADE"],
  ["TOP CHORD", "47 x 150", "C24"],
  ["BOTTOM CHORD", "47 x 150", "C24"],
  ["WEBS", "47 x 100", "C24"],
], [32, 22, 14], { title: "MEMBER SCHEDULE" });

const nx = -P(14), ny = -P(45);
text("NOTES", [nx, ny], { h: 3 });
[
  "1. DIMENSIONS TO SETTING-OUT LINES, IN mm",
  "2. TRUSSES AT 600 CTS, BEARING ON 100 x 50 WALL PLATE",
  "3. NAIL PLATES BOTH FACES, TO MANUFACTURER'S DESIGN",
].forEach((s, i) => text(s, [nx, ny - P(5 + i * 4.5)], { h: 2.3 }));

return cad.describe();
