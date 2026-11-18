// 03 - Bookshelf, 800 x 1800 x 300, the worked example of
// docs/3d-design.md section 10.1.
//
// Two 18 mm sides with 6 mm dados for five shelves, a top that sits on the
// sides, and a 6 mm back panel in a 6 x 6 rabbet. Three rails complete the
// twelve parts: a plinth under the bottom shelf at the front, a low rail
// behind it at the back, and a hanging rail under the top for fixing to
// the wall. The front edges of the sides and the top are rounded to R3.
//
// The joints are cut with the parts themselves: cut(side, {part: shelf})
// takes the shelf as it is placed now, so a dado is exactly where its
// shelf sits. The five shelves share one definition.
//
// x = width (left to right), y = depth (front at y = 0), z = up.
//
// The checks at the end ask the kernel: no two parts share volume, every
// shelf touches both sides (it fills its dados), and the gap between two
// shelves is the opening.

const bs = await cad.use("bookshelf");
const ck = await cad.use("checks");
const run = ck.start("03-bookshelf");

// The parts (the parameters and the recipe live in lib/bookshelf.js, so
// the room of 09 places the same bookshelf).
const {
  W, H, D, T, BACK, DADO, N_SHELF, R, HS, SHELF_W, SHELF_D, opening,
  shelf, shelves, back, left, right, top,
} = bs.build();

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("twelve parts", parts.length, 12);
run.equal("five shelves share one definition", new Set(shelves.map((p) => p.def)).size, 1);
run.box("overall size", ck.boxOf(parts), { min: [0, 0, 0], max: [W, D, H] });

const round = (1 - Math.PI / 4) * R * R;   // section removed by a roundover
const vol = (id) => cad.part(id).volume;
run.near("shelf volume", vol(shelf.id), SHELF_W * SHELF_D * T);
const sideVol = T * D * HS
  - N_SHELF * DADO * T * SHELF_D   // dados
  - DADO * BACK * HS               // rabbet
  - round * HS;                    // roundover
run.near("left side volume", vol(left.id), sideVol);
run.near("right side volume", vol(right.id), sideVol);
run.near("top volume", vol(top.id), W * D * T - round * W);
run.near("back volume", vol(back.id), SHELF_W * BACK * HS);
for (const p of shelves) {
  // Each dado is as deep as the shelf runs into it, and as long and wide.
  run.ok(`shelf ${p.id} sits in both dados`,
    p.bbox.min[0] === T - DADO && p.bbox.max[0] === W - T + DADO);
}
run.near("opening between shelves", opening, 326.4, 1e-12);

// Between the parts, on the exact solids.
const clash = cad.overlapping3d(null);
run.ok("no two parts overlap", clash.pairs.length === 0 && clash.skipped.length === 0,
  JSON.stringify(clash));
const sides = { attrs: { part: "side" } };
const held = cad.pairsWithin3d({ attrs: { part: "shelf" } }, sides, 1e-6);
run.equal("every shelf touches both sides", held.pairs.length, 2 * N_SHELF);
const gap = cad.distance3d(shelves[0], shelves[1]);
run.near("gap between the two lowest shelves", gap.distance, opening);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "exploded", view: "iso", explode: 0.6, ground: true },
  { as: "front", view: "front" },
]);

return run.summary({ opening });
