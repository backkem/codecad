// 02 - Fibonacci spiral and the golden rectangle, as a construction sheet.
//
// Left: a 144 x 89 rectangle tiled with Fibonacci squares (89, 55, 34 ...
// 1, 1), cut in the order left, top, right, bottom. A quarter arc in each
// square forms the spiral; the two construction diagonals cross at its
// pole. Right: the classical compass construction of the golden
// rectangle from a square (Euclid VI.30).
//
// Exercises: shared drafting lib, addArc (p1/p2 tangent mode),
// lineCircleIntersection, lineLineIntersection, procedural subdivision, ISO dimensions,
// PHANTOM/THIN layers, text sized to each square.

const draft = await cad.use("drafting");
const { P, dim, text } = draft;
draft.setup({
  scale: 1,
  layers: {
    SPIRAL: { color: draft.ACI.amber, lineweight: 0.7 },
    POINTS: { color: draft.ACI.white, lineweight: 0.25 },
  },
});

draft.sheet({
  w: 297, h: 189,
  title: "FIBONACCI SPIRAL",
  subtitle: "GOLDEN RECTANGLE CONSTRUCTION",
  number: "CC-002", scale: "1:1",
});

// ── Fibonacci tiling ────────────────────────────────────────────────
const fib = [1, 1];
while (fib[fib.length - 1] < 144) fib.push(fib[fib.length - 1] + fib[fib.length - 2]);
const W = fib[fib.length - 1]; // 144
const H = fib[fib.length - 2]; // 89
const X0 = 24, Y0 = 50;

cad.addPolyline([[X0, Y0], [X0 + W, Y0], [X0 + W, Y0 + H], [X0, Y0 + H]], { closed: true, layer: "OUTLINE" });

// Remaining rectangle, cut a square off one side each step.
let rect = { x: X0, y: Y0, w: W, h: H };
let entry = [X0, Y0]; // spiral enters the first square at its lower-left corner
const sides = ["left", "top", "right", "bottom"];
const squares = [];
for (let i = 0; rect.w > 1e-9 && rect.h > 1e-9; i++) {
  const s = Math.min(rect.w, rect.h);
  const side = sides[i % 4];
  let sq, cut; // cut: the edge shared with what remains, as [axis, value]
  if (side === "left") { sq = [rect.x, rect.y]; cut = ["x", rect.x + s]; rect = { x: rect.x + s, y: rect.y, w: rect.w - s, h: rect.h }; }
  if (side === "top") { sq = [rect.x, rect.y + rect.h - s]; cut = ["y", rect.y + rect.h - s]; rect = { x: rect.x, y: rect.y, w: rect.w, h: rect.h - s }; }
  if (side === "right") { sq = [rect.x + rect.w - s, rect.y]; cut = ["x", rect.x + rect.w - s]; rect = { x: rect.x, y: rect.y, w: rect.w - s, h: rect.h }; }
  if (side === "bottom") { sq = [rect.x, rect.y]; cut = ["y", rect.y + s]; rect = { x: rect.x, y: rect.y + s, w: rect.w, h: rect.h - s }; }
  squares.push({ x: sq[0], y: sq[1], s });

  // Square outline (the dividing edge is all that is new inside the big rectangle)
  cad.addPolyline([sq, [sq[0] + s, sq[1]], [sq[0] + s, sq[1] + s], [sq[0], sq[1] + s]], { closed: true, layer: "THIN" });

  // Spiral quarter arc: from the entry corner to the diagonally opposite
  // corner, centred on the corner that lies on the cut edge.
  const corners = [[sq[0], sq[1]], [sq[0] + s, sq[1]], [sq[0] + s, sq[1] + s], [sq[0], sq[1] + s]];
  const k = corners.findIndex((c) => Math.abs(c[0] - entry[0]) < 1e-9 && Math.abs(c[1] - entry[1]) < 1e-9);
  const exit = corners[(k + 2) % 4];
  const cands = [corners[(k + 1) % 4], corners[(k + 3) % 4]];
  const ax = cut[0] === "x" ? 0 : 1;
  const center = cands.find((c) => Math.abs(c[ax] - cut[1]) < 1e-9);
  cad.addArc(center, s, { p1: entry, p2: exit, layer: "SPIRAL" });
  entry = exit;

  // Size label, scaled to the square
  if (s >= 5) {
    const h = Math.min(9, Math.max(2, s * 0.13));
    text(String(s), [sq[0] + s / 2, sq[1] + s / 2], { h, align: "center", valign: "middle", layer: "TEXT" });
  }
}

// ── Pole: intersection of the two construction diagonals ────────────
// Diagonal of the whole rectangle and of the rectangle left after the
// first cut; every later square shares this pole.
const s1 = squares[1];
const dA = [[X0, Y0 + H], [X0 + W, Y0]];
const dB2 = [[X0 + H, Y0], [s1.x + s1.s, s1.y + s1.s]];
const pole = cad.lineLineIntersection(dA, dB2);
cad.addLine(dA[0], dA[1], { layer: "PHANTOM" });
cad.addLine(dB2[0], dB2[1], { layer: "PHANTOM" });
cad.addCircle(pole, 0.8, { layer: "POINTS" });
draft.leader(pole, [pole[0] + 9, pole[1] + 14], "POLE", { h: 2.5, arrow: false });

dim([X0, Y0], [X0 + W, Y0], -9, { dir: "h" });
dim([X0, Y0], [X0, Y0 + H], 9, { dir: "v" });
text(`FIBONACCI SQUARES  ${fib.slice(0, -1).join(" ")}`, [X0, Y0 + H + 8], { h: 3 });

// ── Construction of the golden rectangle from a square ──────────────
const a = 36;                       // side of the generating square
const A = [192, 72], B = [A[0] + a, A[1]], C = [A[0] + a, A[1] + a], D = [A[0], A[1] + a];
cad.addPolyline([A, B, C, D], { closed: true, layer: "OUTLINE" });
const M = cad.midpoint(A, B);
const rMC = cad.distance(M, C);
// E: where the arc about M through C meets the extended base line
const hits = cad.lineCircleIntersection([A, [A[0] + 3 * a, A[1]]], M, rMC);
const E = hits.sort((p, q) => q[0] - p[0])[0];
const F = [E[0], D[1]];
cad.addLine(B, E, { layer: "OUTLINE" });
cad.addLine(E, F, { layer: "OUTLINE" });
cad.addLine(C, F, { layer: "OUTLINE" });
cad.addLine(M, C, { layer: "PHANTOM" });
cad.addArc(M, rMC, { p1: C, p2: E, layer: "THIN" });

for (const [name, p, dx, dy] of [["A", A, -3.5, -4], ["B", B, -1, -4.5], ["C", C, -1, 1.5], ["D", D, -3.5, 1.5], ["M", M, -1, -4.5], ["E", E, 0.8, -4.5], ["F", F, 0.8, 1.5]]) {
  cad.addCircle(p, 0.6, { layer: "POINTS" });
  text(name, [p[0] + dx, p[1] + dy], { h: 3 });
}
dim(A, B, -12, { dir: "h", label: "1" });
dim(A, E, -20, { dir: "h", label: "1.618" });

const tx = 186;
text("GOLDEN RATIO", [tx, 150], { h: 4 });
[
  "1. SQUARE ABCD, SIDE 1",
  "2. M = MIDPOINT OF AB",
  "3. ARC CENTRE M THROUGH C",
  "   MEETS AB EXTENDED AT E",
  "4. AEFD IS A GOLDEN RECTANGLE",
].forEach((s, i) => text(s, [tx, 141 - i * 4.6], { h: 2.5 }));
// (Greek/maths glyphs such as phi and the root sign don't survive the
// AC1015 DWG text encoding, so the formula is spelled out.)
text("PHI = (1 + SQRT 5) / 2 = 1.6180", [tx, 40], { h: 3 });
text(`${W} / ${H} = ${(W / H).toFixed(4)}  (F13 / F12)`, [tx, 34], { h: 3 });

return cad.describe();
