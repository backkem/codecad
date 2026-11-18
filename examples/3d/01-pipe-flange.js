// 01 - Weld-neck pipe flange DN50, the 3D twin of 01-flange-plate.
//
// A disc Ø165 x 18 with a raised face Ø102 x 2 on its front, and a hub
// tapering from Ø74 at the disc to Ø60.3 at the weld end, 45 above the
// disc. Bore Ø52.5, the inside of a Ø60.3 x 3.9 pipe (the target list
// says Ø60.3 for the bore too, which would leave no wall at the weld end).
// Four Ø18 bolt holes on a Ø125 pitch circle, straddling the centre lines.
// Both outside edges of the disc are chamfered 1 mm.
//
// The body is one revolve of exact lines; the chamfers and holes are
// features on it. The recipe lives in lib/flange.js, so the pipe spool
// (07) places the same flange. It stands on its raised face, axis along z.

const fl = await cad.use("flange");
const ck = await cad.use("checks");
const run = ck.start("01-pipe-flange");

const { part, dims: d, volume } = fl.weldNeck();

// ── Checks ───────────────────────────────────────────────────────────
const p = cad.part(part.id);
run.ok("bolt holes clear the raised face and the hub",
  d.pcd / 2 - d.hole / 2 > d.dRF / 2 && d.pcd / 2 - d.hole / 2 > d.hubBase / 2);
run.ok("the chamfers stop short of the holes", d.D / 2 - d.ch > d.pcd / 2 + d.hole / 2);
run.near("flange volume", p.volume, volume);
run.box("overall size", p.bbox, { min: [-d.D / 2, -d.D / 2, 0], max: [d.D / 2, d.D / 2, d.height] });
run.near("centroid on the axis", Math.hypot(p.centroid[0], p.centroid[1]), 0, 0, 1e-9);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  // Half cutaway through the axis: the bore and the tapered hub.
  { as: "cutaway", view: "iso", ground: true, clip: { point: [0, 0, 0], normal: [0, -1, 0] } },
]);

return run.summary({ mass_kg: Math.round(p.volume * 7.85e-6 * 1000) / 1000 });
