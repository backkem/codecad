// 12 - Site plan, scale 1:500.
//
// The boundary is plotted from a surveyor's traverse (whole-circle
// bearings + distances from corner A); the closing leg D-A is computed
// and reported. The building line is the boundary offset inward by the
// setbacks, with corners closed by zero-radius fillets. The dwelling
// footprint and the parking bays are set square to the street boundary
// (bays: one stall, rect array, rotated into place). Existing ground is
// shown with spline contours at 0.5 m. Site area and coverage come from
// region areas.
//
// Exercises: offset, fillet (radius 0) to close corners, regionClip area,
// addSpline (fit), array (rect) + rotate, addText rotation, addHatch,
// measure (aligned), table, title block.

const draft = await cad.use("drafting");
const { P, dim, text, polar, add, sub, mul, unit, perp } = draft;
const { ACI } = draft;
draft.setup({
  scale: 500,
  layers: {
    BOUNDARY: { color: ACI.red, linetype: "PHAN", lineweight: 0.5 },
    BLDG_LINE: { color: ACI.magenta, linetype: "HID", lineweight: 0.25 },
    BUILDING: { color: ACI.white, lineweight: 0.5 },
    PAVING: { color: ACI.lightgray, lineweight: 0.25 },
    CONTOUR: { color: ACI.orange, lineweight: 0.18 },
    TREE: { color: ACI.green, lineweight: 0.18 },
    ROAD: { color: ACI.gray, lineweight: 0.25 },
  },
});

draft.sheet({
  at: [-P(15), -P(140)], w: 297, h: 189,
  title: "SITE PLAN - LOT 14",
  subtitle: "HIGH STREET, PROPOSED DWELLING",
  number: "CC-012", scale: "1:500",
});

// ── Traverse: whole-circle bearings (deg clockwise from north), metres
const dms = (deg) => {
  const d = Math.floor(deg), m = Math.round((deg - d) * 60);
  return `${d}°${String(m).padStart(2, "0")}'`;
};
const quad = (b) => {               // quadrant bearing text, e.g. N 85°30' E
  if (b <= 90) return `N ${dms(b)} E`;
  if (b <= 180) return `S ${dms(180 - b)} E`;
  if (b <= 270) return `S ${dms(b - 180)} W`;
  return `N ${dms(360 - b)} W`;
};
const legs = [[85.5, 87.40], [167.75, 54.20], [268 + 40 / 60, 81.30]];
const pts = [[0, 0]];
for (const [b, d] of legs) {
  const r = (b * Math.PI) / 180, last = pts[pts.length - 1];
  pts.push([last[0] + d * 1000 * Math.sin(r), last[1] + d * 1000 * Math.cos(r)]);
}
const [A, B, C, D] = pts;
const close = sub(A, D);
const closeLen = Math.hypot(...close) / 1000;
let closeBrg = (Math.atan2(close[0], close[1]) * 180) / Math.PI;
if (closeBrg < 0) closeBrg += 360;
const allLegs = [...legs, [closeBrg, closeLen]];
const corners = [A, B, C, D];

// Boundary as four line entities (clockwise, so "inside" is to the right)
const bLines = corners.map((p, i) => cad.addLine(p, corners[(i + 1) % 4], { layer: "BOUNDARY" }));
corners.forEach((p, i) => {
  cad.addCircle(p, P(0.8), { layer: "BUILDING" });
  const out = mul(unit(sub(p, cad.midpoint(A, C))), P(4));
  text("ABCD"[i], add(p, out), { h: 3, align: "center", valign: "middle" });
});
// Leg annotation: bearing + distance, along the leg, outside the site
allLegs.forEach(([b, d], i) => {
  const p = corners[i], q = corners[(i + 1) % 4];
  const dir = unit(sub(q, p));
  let ang = (Math.atan2(dir[1], dir[0]) * 180) / Math.PI;
  if (ang > 90) ang -= 180;
  if (ang < -90) ang += 180;
  const mid = cad.midpoint(p, q);
  const off = mul(unit(sub(mid, cad.midpoint(A, C))), P(3.5));
  text(`${quad(b)}  ${d.toFixed(2)} m`, add(mid, off), { h: 2.3, align: "center", valign: "middle", rot: ang, layer: "BOUNDARY" });
});

// ── Street ──────────────────────────────────────────────────────────
const u = unit(sub(B, A)), v = perp(u).map((x) => -x);   // v points into the site
const streetOff = (d) => [add(A, add(mul(u, -3500), mul(v, -d))), add(B, add(mul(u, 14000), mul(v, -d)))];
for (const d of [2500, 9500]) { const [p, q] = streetOff(d); cad.addLine(p, q, { layer: "ROAD" }); }
{ const [p, q] = streetOff(6000); cad.addLine(p, q, { layer: "CENTER" }); }
const sAng = (Math.atan2(u[1], u[0]) * 180) / Math.PI;
text("HIGH STREET", add(cad.midpoint(A, B), mul(v, -7500)), { h: 3.5, align: "center", valign: "middle", rot: sAng, layer: "ROAD" });

// ── Building line: boundary offset by the setbacks, corners closed by
// zero-radius fillets.
const setbacks = [6000, 3000, 3000, 3000];
const bl = bLines.map((ln, i) => cad.offset(ln, -setbacks[i]));
for (let i = 0; i < 4; i++) cad.fillet(bl[i], bl[(i + 1) % 4], 0);
// cad.offset keeps the source layer, so redraw the closed result on BLDG_LINE.
const blPts = bl.map((ln) => cad.entity(ln.id).start);
cad.remove(bl);
cad.addPolyline(blPts, { closed: true, layer: "BLDG_LINE" });

// ── Dwelling footprint (L-shape), square to the street ──────────────
const O = add(A, add(mul(u, 14000), mul(v, 10000)));
const L = (s, t) => add(O, add(mul(u, s), mul(v, t)));
const fp = [L(0, 0), L(24000, 0), L(24000, 12000), L(10000, 12000), L(10000, 19000), L(0, 19000)];
cad.addPolyline(fp, { closed: true, layer: "BUILDING", lineweight: 0.7 });
draft.hatch(fp, { angle: 45 + sAng, spacing: 1.6, layer: "HATCH" });
const fpArea = cad.regionClip(fp, fp, "intersect")[0].area / 1e6;
const tc = L(12000, 5600);
text("PROPOSED DWELLING", tc, { h: 2.6, align: "center", valign: "middle", rot: sAng });
text("2 STOREYS  FFL +12.40", add(tc, mul(v, -P(-4))), { h: 2, align: "center", valign: "middle", rot: sAng });

// Setback dimensions in metres, measured square to the boundary
const setDim = (p, line, off) => {
  const q = cad.projectOnto(p, line);
  dim(p, q, off, { h: 2.3, label: `${(cad.distance(p, q) / 1000).toFixed(1)}` });
};
setDim(L(4000, 0), [A, B], 0);
setDim(L(0, 15000), [D, A], 0);

// ── Driveway + parking: one stall, rect array, rotated square to street
const Q = L(30000, 4000);
const stall = [
  cad.addLine(Q, add(Q, [0, -5000]), { layer: "PAVING" }),
  cad.addLine(add(Q, [0, -5000]), add(Q, [2500, -5000]), { layer: "PAVING" }),
];
const copies = cad.array(stall, { rect: { rows: 1, cols: 6, dx: 2500, dy: 0 } });
const endLine = cad.addLine(add(Q, [15000, 0]), add(Q, [15000, -5000]), { layer: "PAVING" });
cad.rotate([...stall, ...copies, endLine], Q, sAng);
const lot = [L(28000, -8000), L(47000, -8000), L(47000, 9000), L(28000, 9000)];
cad.addPolyline(lot, { closed: true, layer: "PAVING" });
cad.addLine(L(35000, -8000), add(A, mul(u, 49000)), { layer: "PAVING" });
cad.addLine(L(39500, -8000), add(A, mul(u, 53500)), { layer: "PAVING" });
text("6 BAYS 2.5 x 5.0", L(37500, -2000), { h: 2.2, align: "center", valign: "middle", rot: sAng });

// ── Existing contours (0.5 m) as fitted splines, inside the site ────
const inside = (p) => cad.regionClip([[p[0] - 1, p[1] - 1], [p[0] + 1, p[1] - 1], [p[0] + 1, p[1] + 1], [p[0] - 1, p[1] + 1]], corners, "intersect").length > 0;
const xs = Array.from({ length: 36 }, (_, i) => -2000 + i * 3000);
for (let k = 0; k < 7; k++) {
  const level = 10.0 + k * 0.5;
  const pts2 = xs.map((x) => [x, -44000 + k * 7600 + 3000 * Math.sin(x / 19000) + 1100 * Math.sin(x / 7000 + k)]).filter(inside);
  if (pts2.length < 3) continue;
  cad.addSpline(pts2, { fit: true, layer: "CONTOUR" });
  const lp = pts2.reduce((best, p) => (Math.abs(p[0] - 68000) < Math.abs(best[0] - 68000) ? p : best));
  text(level.toFixed(1), add(lp, [0, P(1)]), { h: 2, layer: "CONTOUR" });
}

// Existing trees
for (const [x, y, r] of [[22000, -41000, 3500], [32000, -42000, 3000], [78000, -38000, 4000], [88000, -14000, 3000]]) {
  cad.addCircle([x, y], r, { layer: "TREE" });
  cad.addLine([x - 600, y - 600], [x + 600, y + 600], { layer: "TREE" });
  cad.addLine([x - 600, y + 600], [x + 600, y - 600], { layer: "TREE" });
}

// ── North arrow + scale bar ─────────────────────────────────────────
const NA = [P(262 - 15), P(160 - 140)], nr = P(7);
cad.addCircle(NA, nr, { layer: "THIN" });
const tri = [[NA[0], NA[1] + nr], [NA[0] - nr * 0.35, NA[1] - nr * 0.6], [NA[0], NA[1] - nr * 0.3]];
cad.addFill(tri, { layer: "TEXT" }); cad.addPolyline(tri, { closed: true, layer: "TEXT" });
cad.addPolyline([[NA[0], NA[1] + nr], [NA[0] + nr * 0.35, NA[1] - nr * 0.6], [NA[0], NA[1] - nr * 0.3]], { layer: "TEXT" });
text("N", [NA[0], NA[1] + nr + P(2)], { h: 4, align: "center" });
const sb = [P(14 - 15), P(14 - 140)];
for (let i = 0; i <= 4; i++) {
  const x = sb[0] + i * 5000;
  cad.addLine([x, sb[1]], [x, sb[1] + P(2.5)], { layer: "TEXT" });
  text(String(i * 5), [x, sb[1] + P(3.5)], { h: 2, align: "center" });
  if (i < 4 && i % 2 === 0) draft.hatch([[x, sb[1]], [x + 5000, sb[1]], [x + 5000, sb[1] + P(1.5)], [x, sb[1] + P(1.5)]], { angle: 0, spacing: 0.25, layer: "TEXT" });
}
cad.addPolyline([[sb[0], sb[1]], [sb[0] + 20000, sb[1]], [sb[0] + 20000, sb[1] + P(1.5)], [sb[0], sb[1] + P(1.5)]], { closed: true, layer: "TEXT" });
text("m", [sb[0] + 20000 + P(3), sb[1] + P(3.5)], { h: 2 });

// ── Site data ───────────────────────────────────────────────────────
const siteArea = cad.regionClip(corners, corners, "intersect")[0].area / 1e6;
draft.table([P(215 - 15), P(132 - 140)], [
  ["SITE DATA", ""],
  ["SITE AREA", `${siteArea.toFixed(0)} m²`],
  ["FOOTPRINT", `${fpArea.toFixed(0)} m²`],
  ["COVERAGE", `${((100 * fpArea) / siteArea).toFixed(1)} %`],
  ["PARKING", "6 BAYS"],
  ["SETBACKS", "FRONT 6, SIDE 3 m"],
  ["CLOSING LEG D-A", `${quad(closeBrg)}`],
  ["", `${closeLen.toFixed(2)} m`],
], [32, 40], { h: 2.2, rowH: 5, align: [null, "right"] });
// Line legend
[["BOUNDARY", "BOUNDARY"], ["BLDG_LINE", "BUILDING LINE (SETBACKS)"], ["CONTOUR", "EXISTING CONTOUR 0.5 m"]].forEach(([layer, label], i) => {
  const y = P(84 - 140) - i * P(5.5), x = P(215 - 15);
  cad.addLine([x, y], [x + P(12), y], { layer });
  text(label, [x + P(15), y - P(1)], { h: 2.1 });
});

return { siteArea, fpArea, closeBrg, closeLen };
