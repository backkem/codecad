// The bookshelf of 03-bookshelf as parts, shared by 03 and 09-room:
//
//   const bs = await cad.use("bookshelf");
//   const b = bs.build();     // the twelve parts and the sizes the checks use
//
// 800 x 1800 x 300: two 18 mm sides with 6 mm dados for five shelves, a
// top that sits on the sides, a 6 mm back panel in a 6 x 6 rabbet, a
// plinth, a low rail and a hanging rail. The front edges of the sides and
// the top are rounded to R3. Built at the origin: x = width (left to
// right), y = depth (front at y = 0), z = up. Plain script code over cad.*.

const sh = await cad.use("shapes");
const { box } = sh;

function build() {
  // ── Parameters (mm) ────────────────────────────────────────────────
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

  // ── Shelves: one definition, five copies ───────────────────────────
  const shelf = box([T - DADO, 0, PLINTH], [W - T + DADO, SHELF_D, PLINTH + T], oak("shelf"));
  cad.copy3d(shelf, { move: [0, 0, pitch], count: N_SHELF - 1 });
  const shelves = cad.parts({ attrs: { part: "shelf" } });

  // ── Back panel (birch ply) ─────────────────────────────────────────
  const back = box([T - DADO, D - BACK, 0], [W - T + DADO, D, HS], {
    name: "back", attrs: { part: "back", material: "ply" },
  });

  // ── Sides: mirror images, so each has its own definition ───────────
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

  // ── Top, sitting on the sides, front edge rounded ──────────────────
  const top = box([0, 0, HS], [W, D, H], oak("top"));
  cad.fillet3d(top, { edges: { near: [[W / 2, 0, H]] }, radius: R });

  // ── Rails between the sides ────────────────────────────────────────
  const rails = [
    box([T, SETBACK, 0], [W - T, SETBACK + T, PLINTH], oak("plinth")),
    box([T, D - BACK - T, 0], [W - T, D - BACK, PLINTH], oak("low rail")),
    box([T, D - BACK - T, HS - RAIL], [W - T, D - BACK, HS], oak("hanging rail")),
  ];

  return {
    W, H, D, T, BACK, DADO, N_SHELF, PLINTH, SETBACK, RAIL, R,
    HS, SHELF_W, SHELF_D, opening, pitch,
    shelf, shelves, back, left, right, top, rails,
    parts: [...shelves, back, left, right, top, ...rails],
  };
}

return { build };
