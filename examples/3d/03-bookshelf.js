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

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("03-bookshelf");
const { box } = sh;

// ── Parameters (mm) ──────────────────────────────────────────────────
const W = 800, H = 1800, D = 300;   // outside width, height, depth
const T = 18;                       // board thickness
const BACK = 6;                     // back panel thickness (= rabbet width)
const DADO = 6;                     // dado and rabbet depth
const N_SHELF = 5;                  // shelves, the lowest one included
const PLINTH = 60;                  // height of the bottom shelf above the floor
const SETBACK = 20;                 // plinth set back from the front
const RAIL = 100;                   // hanging rail height
const R = 3;                        // roundover on the front edges

const HS = H - T;                   // side height (the top sits on the sides)
const SHELF_W = W - 2 * T + 2 * DADO;  // shelves run into both dados
const SHELF_D = D - BACK;              // and stop at the back panel
// Equal openings between the bottom shelf and the top.
const opening = (HS - (PLINTH + T) - (N_SHELF - 1) * T) / N_SHELF;
const pitch = opening + T;

const oak = (part) => ({ name: part, attrs: { part, material: "oak" } });

// ── Shelves: one definition, five copies ─────────────────────────────
const shelf = box([T - DADO, 0, PLINTH], [W - T + DADO, SHELF_D, PLINTH + T], oak("shelf"));
cad.copy3d(shelf, { move: [0, 0, pitch], count: N_SHELF - 1 });
const shelves = cad.parts({ attrs: { part: "shelf" } });

// ── Back panel (birch ply) ───────────────────────────────────────────
const back = box([T - DADO, D - BACK, 0], [W - T + DADO, D, HS], {
  name: "back", attrs: { part: "back", material: "ply" },
});

// ── Sides: mirror images, so each has its own definition ─────────────
// x0 is the side's left face; `outer` is the x of its outside face.
function side(x0, outer) {
  const s = box([x0, 0, 0], [x0 + T, D, HS], oak("side"));
  // Round the outside front edge before the joints are cut.
  cad.fillet3d(s, { edges: { near: [[outer, 0, HS / 2]] }, radius: R });
  for (const p of shelves) cad.cut(s, { part: p });   // five dados
  cad.cut(s, { part: back });                         // the rabbet
  return s;
}
const left = side(0, 0);
const right = side(W - T, W);

// ── Top, sitting on the sides, front edge rounded ────────────────────
const top = box([0, 0, HS], [W, D, H], oak("top"));
cad.fillet3d(top, { edges: { near: [[W / 2, 0, H]] }, radius: R });

// ── Rails between the sides ──────────────────────────────────────────
box([T, SETBACK, 0], [W - T, SETBACK + T, PLINTH], oak("plinth"));
box([T, D - BACK - T, 0], [W - T, D - BACK, PLINTH], oak("low rail"));
box([T, D - BACK - T, HS - RAIL], [W - T, D - BACK, HS], oak("hanging rail"));

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
