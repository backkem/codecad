// 09 - Room.
//
// The living room of the apartment in examples/07-floor-plan.js, built in
// 3D: its two external walls (300) with a window each, its two partitions
// (100), one with the door to the hall, a wooden floor and a ceiling. The
// bookshelf of 03 stands against the north wall, the stool of 08 in the
// sunlight from the west window; both are built by the same library code
// as their own examples (lib/bookshelf.js, lib/stool.js) and moved into
// place.
//
// The wall layout is 07's (the same millimetres: external walls 300,
// partitions 100, the living room 5000 x 3200 inside, windows W1 and W2,
// door D2), typed in here: a script under examples/3d can read files only
// below its own folder, so it cannot open 07's script or its DWG. Heights
// are this script's: walls 2600, sills 900, window and door heads 2100.
// The windows are open (no glass): sunlight would pass a pane (shadow rays
// cross glass), but the sky would then reach the room only by chance
// bounces again (see the portals below).
//
// The presentation render stands inside the room, in daylight: the sky
// with the sun low in the west. The two windows are named as portals, so
// the tracer samples the sky through them directly; without that an
// interior lit through windows converges very slowly.
//
// x = east, y = north, z = up; the plan's origin is the apartment's
// south-west outside corner.

const bs = await cad.use("bookshelf");
const st = await cad.use("stool");
const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("09-room");
const { box } = sh;

// ── Parameters (mm) ──────────────────────────────────────────────────
// From 07-floor-plan: the apartment is 9000 x 6000; external walls 300,
// partitions 100. The living room is bounded by the north and west
// external walls, the partition V1 (x 5300) and the partition H1 (y 2400).
const PLAN = { H: 6000, te: 300, ti: 100, v1: 5300, h1: 2400 };
const W1 = [1200, 3200];             // north window, x
const W2 = [3300, 4900];             // west window, y
const D2 = [3200, 4000];             // door to the hall, x
const WALL_H = 2600, SILL = 900, HEAD = 2100, DOOR_H = 2100;
const FLOOR_T = 20, CEIL_T = 200;
// Inside faces of the room.
const IN = { x0: PLAN.te, x1: PLAN.v1, y0: PLAN.h1 + PLAN.ti, y1: PLAN.H - PLAN.te };
// Toward the sun (world frame): low in the west, a little south.
const SUN = [-0.8, -0.25, 0.55];
// Where the furniture stands.
const SHELF_AT = 3700;               // the bookshelf's left side, against the north wall
const STOOL_AT = [2350, 4450];       // the stool's axis, in the sunlight

// ── Walls, floor, ceiling ────────────────────────────────────────────
const paint = (part) => ({ name: part, attrs: { part, material: "white paint" } });
const north = box([0, IN.y1, 0], [PLAN.v1 + PLAN.ti, PLAN.H, WALL_H], paint("north wall"));
cad.cut(north, sh.boxTool([W1[0], IN.y1 - 1, SILL], [W1[1], PLAN.H + 1, HEAD]));
const west = box([0, PLAN.h1, 0], [PLAN.te, IN.y1, WALL_H], paint("west wall"));
cad.cut(west, sh.boxTool([-1, W2[0], SILL], [PLAN.te + 1, W2[1], HEAD]));
const east = box([PLAN.v1, PLAN.h1, 0], [PLAN.v1 + PLAN.ti, IN.y1, WALL_H], paint("east wall"));
const south = box([IN.x0, PLAN.h1, 0], [IN.x1, IN.y0, WALL_H], paint("south wall"));
cad.cut(south, sh.boxTool([D2[0], PLAN.h1 - 1, -1], [D2[1], IN.y0 + 1, DOOR_H]));
const floor = box([IN.x0, IN.y0, 0], [IN.x1, IN.y1, FLOOR_T], {
  name: "floor", attrs: { part: "floor", material: "oak" },
});
const ceiling = box([0, PLAN.h1, WALL_H], [PLAN.v1 + PLAN.ti, PLAN.H, WALL_H + CEIL_T], paint("ceiling"));
const room = [north, west, east, south, floor, ceiling];

// ── Furniture, built by the libraries of 03 and 08 ───────────────────
const shelf = bs.build();
cad.move3d(shelf.parts, [SHELF_AT, IN.y1 - shelf.D, FLOOR_T]);
const stool = st.build();
// Its feet are square to the splayed legs, so their outer edges reach
// below the plane they stand on: lift it until the lowest edge touches
// the floor.
const partsOf = (list) => list.map((p) => cad.part(p.id));
const stoolLow = ck.boxOf(partsOf(stool.parts)).min[2];
cad.move3d(stool.parts, [STOOL_AT[0], STOOL_AT[1], FLOOR_T - stoolLow]);

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("25 parts: six for the room, twelve for the bookshelf, seven for the stool", parts.length, 25);
const vol = (p) => cad.part(p.id).volume;
const win = (a, b) => (b - a) * (HEAD - SILL);
run.near("north wall volume", vol(north), (PLAN.v1 + PLAN.ti) * PLAN.te * WALL_H - win(...W1) * PLAN.te);
run.near("west wall volume", vol(west), PLAN.te * (IN.y1 - PLAN.h1) * WALL_H - win(...W2) * PLAN.te);
run.near("east wall volume", vol(east), PLAN.ti * (IN.y1 - PLAN.h1) * WALL_H);
run.near("south wall volume", vol(south), (IN.x1 - IN.x0) * PLAN.ti * WALL_H - (D2[1] - D2[0]) * PLAN.ti * DOOR_H);
run.near("floor volume", vol(floor), (IN.x1 - IN.x0) * (IN.y1 - IN.y0) * FLOOR_T);
run.near("the room is 5000 x 3200 inside", (IN.x1 - IN.x0) * (IN.y1 - IN.y0), 5000 * 3200, 0, 1e-9);
// The bookshelf is the bookshelf of 03, unchanged, moved.
run.near("bookshelf shelf volume", vol(shelf.shelf), shelf.SHELF_W * shelf.SHELF_D * shelf.T);
run.box("bookshelf in place", ck.boxOf(partsOf(shelf.parts)), {
  min: [SHELF_AT, IN.y1 - shelf.D, FLOOR_T], max: [SHELF_AT + shelf.W, IN.y1, FLOOR_T + shelf.H],
});
run.ok("the bookshelf clears the north window", SHELF_AT > W1[1]);
// Nothing of the room overlaps the furniture, nor the bookshelf the
// stool. (The stool's own joints are not cut: its tenons overlap the seat
// and the legs, as in 08.)
const roomIds = room.map((p) => p.id);
const shelfIds = shelf.parts.map((p) => p.id);
const stoolIds = stool.parts.map((p) => p.id);
for (const [what, a, b] of [
  ["the room and the bookshelf", roomIds, shelfIds],
  ["the room and the stool", roomIds, stoolIds],
  ["the bookshelf and the stool", shelfIds, stoolIds],
  ["the room's own parts", roomIds, null],
]) {
  const c = cad.overlapping3d(a, b);
  run.ok(`${what} do not overlap`, c.pairs.length === 0 && c.skipped.length === 0, JSON.stringify(c));
}
const backOnWall = cad.pairsWithin3d(shelf.back, north, 1e-6);
run.equal("the bookshelf's back touches the north wall", backOnWall.pairs.length, 1);
const onFloor = cad.pairsWithin3d(floor, [...shelfIds, ...stoolIds], 1e-6);
run.ok("the bookshelf and all three stool legs touch the floor",
  onFloor.pairs.length >= 3 + 3 && partsOf(stool.parts).every((p) => p.bbox.min[2] >= FLOOR_T - 1e-9));
// The stool stands in the sun: the patch of floor the sun reaches through
// the west window is the window's inside opening carried along the sun's
// rays down to the floor (a parallelogram); the stool's axis lies in it.
const k = (HEAD - FLOOR_T) / SUN[2], k0 = (SILL - FLOOR_T) / SUN[2];
const patch = [
  [PLAN.te - SUN[0] * k0, W2[0] - SUN[1] * k0], [PLAN.te - SUN[0] * k0, W2[1] - SUN[1] * k0],
  [PLAN.te - SUN[0] * k, W2[1] - SUN[1] * k], [PLAN.te - SUN[0] * k, W2[0] - SUN[1] * k],
];
const inside = (p, poly) => {
  const side = poly.map((a, i) => {
    const b = poly[(i + 1) % poly.length];
    return Math.sign((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]));
  });
  return side.every((s) => s >= 0) || side.every((s) => s <= 0);
};
run.ok("the stool stands in the sunlight from the west window", inside(STOOL_AT, patch), JSON.stringify(patch));

// ── Renders ──────────────────────────────────────────────────────────
// The working picture: from above, as a dollhouse. The presentation
// render: inside, from the door's corner, looking north-west at both
// windows, in daylight through the two portals.
// The dollhouse cut: the ceiling, the south wall and the east wall taken
// away (clip planes remove the side their normal points into).
const DOLLHOUSE = [
  { point: [0, 0, WALL_H - 50], normal: [0, 0, 1] },
  { point: [0, IN.y0 + 50, 0], normal: [0, -1, 0] },
  { point: [IN.x1 - 50, 0, 0], normal: [1, 0, 0] },
];
const CAMERA = { eye: [4950, 2750, 1550], target: [1500, 5150, 1050], fov: 62 };
const portals = [
  { origin: [W1[0], PLAN.H, SILL], u: [W1[1] - W1[0], 0, 0], v: [0, 0, HEAD - SILL] },
  { origin: [0, W2[0], SILL], u: [0, W2[1] - W2[0], 0], v: [0, 0, HEAD - SILL] },
];
const DAYLIGHT = {
  environment: { preset: "sky", sun: { direction: SUN } },
  portals,
  exposure: 1.7,
  // 512 samples, denoised, look converged here; at the default 1024 the
  // script would pass the sandbox's 30 s limit.
  spp: globalThis.EXAMPLE_PRESENT_SPP ?? 512,
};
await run.render([
  { as: "iso", view: "iso", ground: true, clip: DOLLHOUSE },
  { as: "inside", camera: CAMERA, ground: { color: [120, 124, 108] }, present: DAYLIGHT },
]);

return run.summary({ sunPatch: patch.map((p) => p.map((v) => Math.round(v))) });
