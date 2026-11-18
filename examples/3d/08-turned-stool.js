// 08 - Turned stool.
//
// A round seat Ø320 x 30, 450 high, its top edge rounded R10 and its
// bottom edge R3. Three turned legs splayed 8°, each one revolve of lines
// and arcs: a tapered foot, a bead, a cove, a second small bead and a
// tenon into the seat. Three stretchers between the legs, each a ruled
// loft of three circles: Ø24 in the middle tapering to Ø18 at the ends.
//
// The legs share one definition, placed three times by turning a copy
// 120° about the stool's axis; so do the stretchers.
//
// Simplified: the joints are not cut. The tenons sit in the seat and the
// stretcher ends in the legs, overlapping them, and the legs' shoulders
// and feet stay square to the leg (8° off level). Cutting a mortise with
// a tilted leg, or levelling its foot, meets its round faces at an angle
// (elliptic edges); not tried here, as the interference check that would
// confirm the fit is a later rung.
//
// z = up, the stool's axis on z.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("08-turned-stool");
const { PI, sin, cos } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const H = 450, SEAT_D = 320, SEAT_T = 30;
const R_TOP = 10, R_BOTTOM = 3;      // seat edge rounds
const SPLAY = 8;                     // degrees from vertical
const LEG_AT = 110;                  // leg axis meets the seat's underside at this radius
const TENON = [12, 20];              // radius, length
const STRETCH = { z: 180, mid: 12, end: 9 };   // height on the leg axes: mid cove

const zu = H - SEAT_T;                        // seat underside
const s = (SPLAY * PI) / 180;
const LU = zu / cos(s);                       // leg length, foot to shoulder

// ── Seat ─────────────────────────────────────────────────────────────
const seat = cad.extrude({ center: [0, 0], radius: SEAT_D / 2 }, {
  at: [0, 0, zu], distance: SEAT_T, name: "seat", attrs: { part: "seat", material: "oak" },
});
// Edges picked at +y, away from the circle's seam at +x.
cad.fillet3d(seat, { edges: { near: [[0, SEAT_D / 2, H]] }, radius: R_TOP });
cad.fillet3d(seat, { edges: { near: [[0, SEAT_D / 2, zu]] }, radius: R_BOTTOM });

// ── Leg: one revolve (u = radius, v = along the leg, foot at 0) ──────
// Beads are half circles: a flatter bead (a larger circle) would cross
// the leg's axis, which the kernel refuses ("an arc's circle crosses the
// revolve axis: a spindle torus") even though the arc itself stays clear.
const legProfile = [
  [0, 0], [14, 0],
  [18, 140, 1],       // taper up to the bead; bead: a half circle R6
  [18, 152],
  [18, 160, -0.2],    // cove: 40 long, 4 deep
  [18, 200],
  [18, 330, 1],       // small bead: a half circle R4
  [18, 338],
  [18, LU], [TENON[0], LU], [TENON[0], LU + TENON[1]], [0, LU + TENON[1]],
];
const leg = cad.revolve(legProfile, {
  plane: "xz", axis: [[0, 0], [0, 1]], name: "leg", attrs: { part: "leg", material: "oak" },
});
// Lean it 8° toward the axis, its shoulder at the seat's underside, on +x.
cad.rotate3d(leg, { angle: -SPLAY, axis: [0, 1, 0] });
const d = [-sin(s), 0, cos(s)];                       // leg direction, foot to top
cad.move3d(leg, [LEG_AT - LU * d[0], 0, zu - LU * d[2]]);
cad.copy3d(leg, { rotate: { angle: 120, axis: [0, 0, 1] }, count: 2 });

// ── Stretchers between neighbouring legs, on the leg axes ────────────
const onAxis = (phi, z) => {
  // The leg axis at height z, for the leg at angle phi.
  const r = LEG_AT + (zu - z) * Math.tan(s);
  return [r * cos(phi), r * sin(phi), z];
};
const a = onAxis(0, STRETCH.z), b = onAxis((2 * PI) / 3, STRETCH.z);
const mid = a.map((v, i) => (v + b[i]) / 2);
const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
const dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len, 0];
const section = (origin, radius) => ({
  profile: { center: [0, 0], radius }, plane: { origin, x: [0, 0, 1], normal: dir },
});
const stretcher = cad.loft(
  [section(a, STRETCH.end), section(mid, STRETCH.mid), section(b, STRETCH.end)],
  { ruled: true, name: "stretcher", attrs: { part: "stretcher", material: "oak" } },
);
cad.copy3d(stretcher, { rotate: { angle: 120, axis: [0, 0, 1] }, count: 2 });

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("seven parts", parts.length, 7);
run.equal("three definitions", new Set(parts.map((p) => p.def)).size, 3);
run.near("seat volume", cad.part(seat.id).volume,
  PI * (SEAT_D / 2) ** 2 * SEAT_T - sh.roundRing(SEAT_D / 2, R_TOP) - sh.roundRing(SEAT_D / 2, R_BOTTOM));
// Pappus on the exact profile (lines and arcs).
run.near("leg volume", cad.part(leg.id).volume, 2 * PI * sh.loopMoments(legProfile).moment);
const frustum = (h, r1, r2) => (PI * h / 3) * (r1 * r1 + r1 * r2 + r2 * r2);
run.near("stretcher volume", cad.part(stretcher.id).volume, 2 * frustum(len / 2, STRETCH.end, STRETCH.mid));
// The legs stand on the ground at the same height and are spread wider
// than the seat.
const legs = cad.parts({ attrs: { part: "leg" } });
for (const p of legs) run.near(`leg ${p.id} foot level`, p.bbox.min[2], legs[0].bbox.min[2], 0, 1e-9);
const footR = LEG_AT + LU * sin(s);
run.ok("feet spread wider than the seat", footR > SEAT_D / 2);
run.near("seat height", ck.boxOf(parts).max[2], H, 0, 1e-9);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "front", view: "front" },
]);

return run.summary({ footRadius: Math.round(footR * 10) / 10 });
