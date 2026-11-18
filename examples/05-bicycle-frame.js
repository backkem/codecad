// 05 - Road bike frame geometry (size 56), side elevation, scale 1:10.
//
// Set out from the bottom bracket the way frame builders do: stack and
// reach locate the head tube, the head and seat angles fix the tube axes,
// BB drop + chainstay length place the rear axle (line/circle
// intersection), and the fork rake places the front axle (offset steering
// axis meets the axle line). Tubes are strips around their axes, fused
// into one welded silhouette with a region union. Wheels, seat post,
// saddle and bars are reference geometry; a geometry chart lists the
// numbers a fitter asks for.
//
// Exercises: lineCircleIntersection, lineLineIntersection, regionClip
// (union), addSpline (saddle, drop bar), dimAngle, measure (h / v /
// aligned), table, title block.

const draft = await cad.use("drafting");
const { P, dim, text, polar, add, sub, mul, unit, perp, offsetLine } = draft;
draft.setup({
  scale: 8,
  layers: {
    FRAME_OUT: { color: draft.ACI.white, lineweight: 0.5 },
    AXIS: { color: draft.ACI.red, linetype: "CL", lineweight: 0.18 },
    WHEEL: { color: draft.ACI.gray, lineweight: 0.25 },
    REF: { color: draft.ACI.lightgray, linetype: "PHAN", lineweight: 0.18 },
  },
});

// ── Geometry inputs (mm, degrees) ───────────────────────────────────
const stack = 565, reach = 385;
const headAngle = 73, seatAngle = 73.5;
const headTube = 160, bbDrop = 70, chainstay = 410, rake = 45;
const tyreR = 672 / 2, rimR = 630 / 2;   // 700x25C

draft.sheet({
  at: [-P(107), -P(72)], w: 297, h: 189,
  title: "ROAD FRAME - SIZE 56",
  subtitle: "GEOMETRY / SETTING-OUT DRAWING",
  number: "CC-005", scale: "1:8",
});

// ── Setting out ─────────────────────────────────────────────────────
const BB = [0, 0];
const axleLine = [[-2000, bbDrop], [2000, bbDrop]];
const rearAxle = cad.lineCircleIntersection(axleLine, BB, chainstay).sort((a, b) => a[0] - b[0])[0];

const htTop = [reach, stack];
const steerDir = [Math.cos((-headAngle * Math.PI) / 180), Math.sin((-headAngle * Math.PI) / 180)]; // down the steerer
const htBot = add(htTop, mul(steerDir, headTube));
const steerAxis = [add(htTop, mul(steerDir, -200)), add(htTop, mul(steerDir, 900))];
// Fork rake: the axle sits on a line parallel to the steering axis, `rake` in front.
const forkLine = offsetLine(steerAxis, rake);  // left of downward direction = forward
const frontAxle = cad.lineLineIntersection(forkLine, axleLine);

const seatDir = polar([0, 0], 1, 180 - seatAngle);
const seatAxis = [BB, mul(seatDir, 900)];
// Horizontal top tube meets the head tube 25 mm below its top
const ttHead = add(htTop, mul(steerDir, 25));
const ttLine = [[-500, ttHead[1]], [1500, ttHead[1]]];
const ttSeat = cad.lineLineIntersection(seatAxis, ttLine);
const stTop = cad.lineLineIntersection(seatAxis, [[-500, ttHead[1] + 20], [1500, ttHead[1] + 20]]);
const dtHead = add(htBot, mul(steerDir, -35));
const ssSeat = add(ttSeat, mul(seatDir, -30));

// Axes (centre lines) of every member
for (const [a, b] of [[BB, stTop], [ttSeat, ttHead], [BB, dtHead], [BB, rearAxle], [rearAxle, ssSeat]])
  cad.addLine(a, b, { layer: "AXIS" });
cad.addLine(steerAxis[0], add(htTop, mul(steerDir, 560)), { layer: "AXIS" });
cad.addLine([rearAxle[0] - tyreR - 40, bbDrop], [frontAxle[0] + tyreR + 40, bbDrop], { layer: "AXIS" });

// ── Tubes: strips around the axes, fused into one silhouette ────────
function strip(a, b, ra, rb = ra, ext = 0) {
  const d = unit(sub(b, a)), n = perp(d);
  const a2 = sub(a, mul(d, ext)), b2 = add(b, mul(d, ext));
  return [add(a2, mul(n, ra)), add(b2, mul(n, rb)), sub(b2, mul(n, rb)), sub(a2, mul(n, ra))];
}
const crown = add(htBot, mul(steerDir, 30));
const tubes = [
  strip(htTop, htBot, 22, 22, 0),                // head tube 44
  strip(ttSeat, ttHead, 15.9),                   // top tube 31.8
  strip(BB, dtHead, 19),                         // down tube 38
  strip(BB, stTop, 15.9),                        // seat tube 31.8
  strip(BB, rearAxle, 11, 7),                    // chainstay, tapered
  strip(ssSeat, rearAxle, 8, 6),                 // seat stay
  strip(crown, frontAxle, 15, 7),                // fork blade, tapered
  strip(htBot, crown, 24, 24, 4),                // fork crown
];
const bbShell = { center: BB, radius: 21 };
const frame = cad.regionClip(tubes, [bbShell], "union");
for (const r of frame) {
  cad.addPolyline(r.outer, { closed: true, layer: "FRAME_OUT" });
  r.holes.forEach((h) => cad.addPolyline(h, { closed: true, layer: "FRAME_OUT" }));
}
// Dropouts and BB shell face
for (const ax of [rearAxle, frontAxle]) cad.addCircle(ax, 6, { layer: "FRAME_OUT" });
cad.addCircle(BB, 12, { layer: "THIN" });

// ── Wheels ──────────────────────────────────────────────────────────
for (const ax of [rearAxle, frontAxle]) {
  cad.addCircle(ax, tyreR, { layer: "WHEEL" });
  cad.addCircle(ax, rimR, { layer: "WHEEL" });
  cad.addCircle(ax, rimR - 25, { layer: "WHEEL" });
  cad.addCircle(ax, 28, { layer: "WHEEL" });
}
const ground = bbDrop - tyreR;
cad.addLine([rearAxle[0] - tyreR - 60, ground], [frontAxle[0] + tyreR + 60, ground], { layer: "THIN" });

// ── Reference: seat post, saddle, stem and drop bar ─────────────────
const saddleH = 740;                        // BB to saddle top along the seat axis
const sTop = mul(seatDir, saddleH - 20);
cad.addPolyline(strip(stTop, sTop, 13.6), { closed: true, layer: "REF" });
const sc = mul(seatDir, saddleH);
cad.addSpline([add(sc, [-150, -10]), add(sc, [-110, 8]), add(sc, [-20, 2]), add(sc, [60, -2]), add(sc, [120, -12])], { fit: true, layer: "REF" });
cad.addSpline([add(sc, [-150, -10]), add(sc, [-60, -30]), add(sc, [60, -24]), add(sc, [120, -12])], { fit: true, layer: "REF" });
const spacer = add(htTop, mul(steerDir, -30));
cad.addPolyline(strip(htTop, spacer, 16), { closed: true, layer: "REF" });
const stemEnd = add(spacer, polar([0, 0], 100, -6));
cad.addPolyline(strip(add(spacer, mul(steerDir, 10)), stemEnd, 14), { closed: true, layer: "REF" });
cad.addSpline([stemEnd, add(stemEnd, [55, 4]), add(stemEnd, [80, -30]), add(stemEnd, [60, -100]), add(stemEnd, [10, -125]), add(stemEnd, [-30, -140])], { fit: true, layer: "REF" });
cad.addCircle(stemEnd, 15.9, { layer: "REF" });

// ── Dimensions ──────────────────────────────────────────────────────
const d0 = { decimals: 0, h: 3 };
dim(rearAxle, frontAxle, -(bbDrop - ground) / P(1) - 10, { ...d0, dir: "h", label: "WHEELBASE <>" });
dim(BB, htTop, -7, { ...d0, dir: "v", label: "STACK <>" });
dim(BB, htTop, 12, { ...d0, dir: "h", label: "REACH <>" });
dim(ttSeat, ttHead, -7, { ...d0, dir: "h", label: "ETT <>" });
dim(rearAxle, BB, -9, d0);
dim(BB, [BB[0], bbDrop], -12, { ...d0, dir: "v", h: 2.5 });
// Angles are measured CCW from the first ray to the second.
cad.addLine(htTop, add(htTop, [-P(11), 0]), { layer: "DIM" });
const angOpts = { text_height: P(2.8), layer: "DIM", label: "<>°" };
draft.outlineFills(cad.dimAngle(BB, mul(seatDir, 100), [-100, 0], { ...angOpts, radius: P(15) }).ids);
draft.outlineFills(cad.dimAngle(htTop, sub(htTop, steerDir), add(htTop, [-1, 0]), { ...angOpts, radius: P(9) }).ids);

// ── Geometry chart ──────────────────────────────────────────────────
const ett = ttHead[0] - ttSeat[0];
const wb = frontAxle[0] - rearAxle[0];
const trail = (frontAxle[1] - ground) / Math.tan((headAngle * Math.PI) / 180) - rake / Math.sin((headAngle * Math.PI) / 180);
const r0 = (v) => String(Math.round(v));
draft.table([P(225 - 107), P(165 - 72)], [
  ["GEOMETRY", "SIZE 56"],
  ["STACK", r0(stack)], ["REACH", r0(reach)],
  ["HEAD ANGLE", `${headAngle}°`], ["SEAT ANGLE", `${seatAngle}°`],
  ["EFF. TOP TUBE", r0(ett)], ["HEAD TUBE", r0(headTube)],
  ["CHAINSTAY", r0(chainstay)], ["BB DROP", r0(bbDrop)],
  ["FORK RAKE", r0(rake)], ["TRAIL", r0(trail)],
  ["WHEELBASE", r0(wb)],
], [34, 20], { h: 2.3, rowH: 5, align: [null, "right"] });

return { ett, wb, trail, rearAxle, frontAxle };
