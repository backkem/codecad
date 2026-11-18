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
const st = await cad.use("stool");
const ck = await cad.use("checks");
const run = ck.start("08-turned-stool");
const { PI, sin } = Math;

// The parts (the parameters and the recipe live in lib/stool.js, so the
// room of 09 places the same stool).
const {
  H, SEAT_D, SEAT_T, R_TOP, R_BOTTOM, LEG_AT, STRETCH, s, LU, legProfile, len,
  seat, leg, stretcher,
} = st.build();

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
