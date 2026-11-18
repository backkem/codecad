// 02 - Mounting bracket, an L from 6 mm plate.
//
// A base leg 80 long and an upright 60 high, both 60 wide, bent with an
// inside radius of 6 (outside 12). A triangular gusset stiffens the bend
// in the middle. The base has two Ø9 holes counterbored Ø15 x 3, the
// upright two 9 x 20 slots, and the four free corners are rounded R3.
//
// The L is one extrude of a side profile with exact bend arcs (bulges).
// The slots are cuts of slot profiles: lines and half circles.
//
// x = width, y = along the base leg (the upright at y = 0), z = up.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("02-mounting-bracket");
const { PI } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const T = 6;                 // plate thickness
const WIDTH = 60;            // across the bracket (x)
const BASE = 80, UP = 60;    // leg lengths, outside
const RI = 6, RO = RI + T;   // bend radii, inside and outside
const R_CORNER = 3;          // free corners
const GUSSET = { t: 6, leg: 41, overlap: 1 };   // triangle legs from the inner faces
const HOLE = 9, CBORE = [15, 3];
const HOLES_Y = 60, HOLES_X = [15, 45];
const SLOT = { len: 20, w: 9, z: 40 };           // slot centre height
const SLOTS_X = [15, 45];

// ── The L: side profile on the yz plane, extruded across x ───────────
const q = -Math.tan(PI / 8);   // bulge of a clockwise quarter arc
const side = [
  [RO, 0], [BASE, 0], [BASE, T], [RO, T, q],      // base leg, inner bend arc
  [T, RO], [T, UP], [0, UP], [0, RO, -q],          // upright, outer bend arc
];
const bracket = cad.extrude(side, {
  plane: "yz", distance: WIDTH, name: "bracket", attrs: { part: "bracket", material: "steel" },
});

// Round the four free corners (edges across the ends of both legs).
cad.fillet3d(bracket, {
  edges: { near: [[0, BASE, T / 2], [WIDTH, BASE, T / 2], [0, T / 2, UP], [WIDTH, T / 2, UP]] },
  radius: R_CORNER,
});

// ── Gusset: a triangle in the bend, sunk 1 mm into both legs ─────────
const g0 = T - GUSSET.overlap, gx = (WIDTH - GUSSET.t) / 2;
const tri = [[g0, g0], [g0 + GUSSET.leg, g0], [g0, g0 + GUSSET.leg]];
cad.union(bracket, { extrude: tri, plane: "yz", at: [gx, 0, 0], distance: GUSSET.t });

// ── Holes and slots ──────────────────────────────────────────────────
for (const x of HOLES_X) cad.hole(bracket, { at: [x, HOLES_Y, T], dia: HOLE, cbore: CBORE });
for (const x of SLOTS_X) {
  // Sketched on the xz plane (normal -y) at the upright's inner face.
  cad.cut(bracket, { extrude: sh.slot(x, SLOT.z, SLOT.len, SLOT.w), plane: "xz", at: [0, T + 1, 0], distance: T + 2 });
}

// ── Checks ───────────────────────────────────────────────────────────
const p = cad.part(bracket.id);
// Side profile: the two straight legs beyond the bend, plus the quarter
// ring of the bend between RI and RO.
const strips = (BASE - RO) * T + (UP - RO) * T;
const ring = (PI / 4) * (RO * RO - RI * RI);
const profileArea = strips + ring;
const roundCorner = (1 - PI / 4) * R_CORNER * R_CORNER * T;
// Gusset: the triangle less what lies inside the plate: the two 1 mm
// strips along the legs (their 1 x 1 corner counted once) and the corner
// of the bend between the inner faces and the inner arc.
const leg = GUSSET.leg, o = GUSSET.overlap;
const strip = leg * o - (o * o) / 2;   // a 1 mm strip of the triangle along a leg
const inPlate = 2 * strip - o * o + RI * RI * (1 - PI / 4);
const gusset = (leg * leg / 2 - inPlate) * GUSSET.t;
const holes = HOLES_X.length * ((PI * HOLE * HOLE / 4) * (T - CBORE[1]) + (PI * CBORE[0] ** 2 / 4) * CBORE[1]);
const slots = SLOTS_X.length * (SLOT.w * (SLOT.len - SLOT.w) + PI * SLOT.w * SLOT.w / 4) * T;
run.near("bracket volume", p.volume, profileArea * WIDTH - 4 * roundCorner + gusset - holes - slots);
run.box("overall size", p.bbox, { min: [0, 0, 0], max: [WIDTH, BASE, UP] });
run.ok("slots clear the bend and the top", SLOT.z - SLOT.len / 2 > RO && SLOT.z + SLOT.len / 2 < UP - R_CORNER);
run.ok("counterbores clear the gusset", HOLES_Y - CBORE[0] / 2 > g0 + leg - T);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "front", view: "front" },
]);

return run.summary({ mass_kg: Math.round(p.volume * 7.85e-6 * 1000) / 1000 });
