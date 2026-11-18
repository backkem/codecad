// The turned stool of 08-turned-stool as parts, shared by 08 and 09-room:
//
//   const st = await cad.use("stool");
//   const s = st.build();     // the seven parts and the sizes the checks use
//
// A round seat Ø320 x 30, 450 high, its top edge rounded R10 and its
// bottom edge R3; three turned legs splayed 8°; three stretchers, each a
// ruled loft of three circles. Built standing on z = 0, the stool's axis
// on z. Plain script code over cad.*.

const { PI, sin, cos } = Math;

function build() {
  // ── Parameters (mm) ────────────────────────────────────────────────
  const H = 450, SEAT_D = 320, SEAT_T = 30;
  const R_TOP = 10, R_BOTTOM = 3;      // seat edge rounds
  const SPLAY = 8;                     // degrees from vertical
  const LEG_AT = 110;                  // leg axis meets the seat's underside at this radius
  const TENON = [12, 20];              // radius, length
  const STRETCH = { z: 180, mid: 12, end: 9 };   // height on the leg axes: mid cove

  const zu = H - SEAT_T;                        // seat underside
  const s = (SPLAY * PI) / 180;
  const LU = zu / cos(s);                       // leg length, foot to shoulder

  // ── Seat ───────────────────────────────────────────────────────────
  const seat = cad.extrude({ center: [0, 0], radius: SEAT_D / 2 }, {
    at: [0, 0, zu], distance: SEAT_T, name: "seat", attrs: { part: "seat", material: "oak" },
  });
  // Edges picked at +y, away from the circle's seam at +x.
  cad.fillet3d(seat, { edges: { near: [[0, SEAT_D / 2, H]] }, radius: R_TOP });
  cad.fillet3d(seat, { edges: { near: [[0, SEAT_D / 2, zu]] }, radius: R_BOTTOM });

  // ── Leg: one revolve (u = radius, v = along the leg, foot at 0) ────
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
  const legCopies = cad.copy3d(leg, { rotate: { angle: 120, axis: [0, 0, 1] }, count: 2 });

  // ── Stretchers between neighbouring legs, on the leg axes ──────────
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
  const stretcherCopies = cad.copy3d(stretcher, { rotate: { angle: 120, axis: [0, 0, 1] }, count: 2 });

  return {
    H, SEAT_D, SEAT_T, R_TOP, R_BOTTOM, SPLAY, LEG_AT, TENON, STRETCH,
    zu, s, LU, legProfile, len,
    seat, leg, stretcher,
    parts: [seat, leg, ...[legCopies].flat(), stretcher, ...[stretcherCopies].flat()],
  };
}

return { build };
