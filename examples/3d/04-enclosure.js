// 04 - Electronics enclosure, 100 x 60 x 25, two parts.
//
// A base and a lid with 2 mm walls and R3 outside corners, split 17 mm
// above the floor. Four corner bosses (Ø7) in each half carry the lid
// screws: Ø2.5 pilot holes in the base, Ø3 clearance holes in the lid. A
// 90 x 50 x 1.6 board sits on four Ø5 standoffs, with corners notched to
// clear the bosses, and carries its larger components as blocks: a chip,
// two capacitors, a pin header, a 9 x 3.5 connector at the front wall and
// a Ø8 jack at the right wall. Both walls have cut-outs to match.
//
// The pockets are cuts of a rounded rectangle (exact arcs), the bosses
// and standoffs unions, the holes cad.hole. Ties to docs/pcb-direction.md:
// the board is where a PCB would come in.
//
// x = length, y = width (front at y = 0), z = up.
//
// Not yet: interference and clearance checks (later rungs) would confirm
// the board clears the bosses (1 mm by construction) and the parts touch
// only where they should.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("04-enclosure");
const { box, roundedRect, roundedRectArea, segmentArea } = sh;
const { PI } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const L = 100, W = 60, H = 25;   // outside size
const WALL = 2;                  // wall, floor and lid thickness
const R_OUT = 3;                 // outside corner radius
const R_IN = R_OUT - WALL;       // inside corner radius
const SPLIT = 17;                // base height; the lid is the rest
const BOSS_R = 3.5, BOSS_AT = 5; // corner bosses: radius, inset of the centre
const PILOT = 2.5, CLEAR = 3;    // screw holes in base and lid
const PILOT_DEPTH = 10;
const BOARD = [90, 50, 1.6];     // board size
const BOARD_AT = [5, 5, 6];      // board corner (on the standoffs)
const NOTCH = 4.5;               // board corner notches
const STANDOFF_R = 2.5;
const STANDOFFS = [[15, 15], [85, 15], [15, 45], [85, 45]];
const JACK = { y: 30, dia: 8, hole: 8.4 };
const PLUG = { x: 50, w: 9, h: 3.5, cut: [10, 4.5], r: 1 };

const boardTop = BOARD_AT[2] + BOARD[2];
const corners = [[BOSS_AT, BOSS_AT], [L - BOSS_AT, BOSS_AT], [BOSS_AT, W - BOSS_AT], [L - BOSS_AT, W - BOSS_AT]];
const named = (part, extra = {}) => ({ name: part, attrs: { part, ...extra } });

// ── Base ─────────────────────────────────────────────────────────────
const base = cad.extrude(roundedRect(0, 0, L, W, R_OUT), { distance: SPLIT, ...named("base") });
// The pocket, from the floor up and out through the open top.
cad.cut(base, { extrude: roundedRect(WALL, WALL, L - 2 * WALL, W - 2 * WALL, R_IN), at: [0, 0, WALL], distance: SPLIT });
// Corner bosses, from inside the floor up to the split, with pilot holes.
for (const [x, y] of corners) {
  cad.union(base, { extrude: { center: [x, y], radius: BOSS_R }, at: [0, 0, WALL - 1], distance: SPLIT - WALL + 1 });
  cad.hole(base, { at: [x, y, SPLIT], dia: PILOT, depth: PILOT_DEPTH });
}
// Standoffs for the board, with pilot holes into the floor.
for (const [x, y] of STANDOFFS) {
  cad.union(base, { extrude: { center: [x, y], radius: STANDOFF_R }, at: [0, 0, WALL - 1], distance: BOARD_AT[2] - WALL + 1 });
  cad.hole(base, { at: [x, y, BOARD_AT[2]], dia: PILOT, depth: BOARD_AT[2] - WALL + 1 });
}
// Cut-out for the connector in the front wall (sketched on the xz plane,
// cut toward -y through the wall).
const plugZ = boardTop + PLUG.h / 2;
const [cw, chh] = PLUG.cut;
cad.cut(base, {
  extrude: roundedRect(PLUG.x - cw / 2, plugZ - chh / 2, cw, chh, PLUG.r),
  plane: "xz", at: [0, WALL + 1, 0], distance: WALL + 2,
});
// Hole for the jack in the right wall.
const jackZ = boardTop + JACK.dia / 2;
cad.hole(base, { at: [L, JACK.y, jackZ], dir: [-1, 0, 0], dia: JACK.hole, depth: WALL + 1 });

// ── Lid ──────────────────────────────────────────────────────────────
const LID = H - SPLIT;
const lid = cad.extrude(roundedRect(0, 0, L, W, R_OUT), { at: [0, 0, SPLIT], distance: LID, ...named("lid") });
cad.cut(lid, { extrude: roundedRect(WALL, WALL, L - 2 * WALL, W - 2 * WALL, R_IN), at: [0, 0, SPLIT - 1], distance: LID - WALL + 1 });
for (const [x, y] of corners) {
  cad.union(lid, { extrude: { center: [x, y], radius: BOSS_R }, at: [0, 0, SPLIT], distance: LID - WALL + 0.5 });
  cad.hole(lid, { at: [x, y, H], dia: CLEAR });
}

// ── Board ────────────────────────────────────────────────────────────
const [bx, by, bz] = BOARD_AT;
const board = box(BOARD_AT, [bx + BOARD[0], by + BOARD[1], boardTop], named("board"));
for (const [x, y] of [[bx, by], [bx + BOARD[0] - NOTCH, by], [bx, by + BOARD[1] - NOTCH], [bx + BOARD[0] - NOTCH, by + BOARD[1] - NOTCH]]) {
  cad.cut(board, sh.boxTool([x, y, bz - 1], [x + NOTCH, y + NOTCH, boardTop + 1]));
}
for (const [x, y] of STANDOFFS) cad.hole(board, { at: [x, y, boardTop], dia: CLEAR });

// ── Components on the board (as blocks) ──────────────────────────────
const comp = (kind) => named(kind, { component: true });
box([43, 23, boardTop], [57, 37, boardTop + 1.6], comp("chip"));
const cap = cad.extrude({ center: [25, 40], radius: 3 }, { at: [0, 0, boardTop], distance: 9, ...comp("capacitor") });
cad.copy3d(cap, { move: [8, 0, 0] });
box([75, 20, boardTop], [77.5, 40, boardTop + 8.5], comp("header"));
box([PLUG.x - PLUG.w / 2, WALL + 0.5, boardTop], [PLUG.x + PLUG.w / 2, 10, boardTop + PLUG.h], comp("connector"));
const jack = box([84, JACK.y - 4.5, boardTop], [L - WALL - 1, JACK.y + 4.5, boardTop + JACK.dia], comp("jack"));
cad.union(jack, { extrude: { center: [JACK.y, jackZ], radius: JACK.dia / 2 }, plane: "yz", at: [L - WALL - 1, 0, 0], distance: 2.5 });

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("nine parts", parts.length, 9);
run.box("overall size", ck.boxOf(parts), { min: [0, 0, 0], max: [L, W, H] });

const outer = roundedRectArea(L, W, R_OUT), inner = roundedRectArea(L - 2 * WALL, W - 2 * WALL, R_IN);
// A boss joins two walls: its circle less the two segments inside them.
const bossArea = PI * BOSS_R ** 2 - 2 * segmentArea(BOSS_R, BOSS_AT - WALL);
const disc = (d) => (PI * d * d) / 4;
const baseVol = outer * SPLIT - inner * (SPLIT - WALL)
  + 4 * bossArea * (SPLIT - WALL) - 4 * disc(PILOT) * PILOT_DEPTH
  + 4 * disc(2 * STANDOFF_R) * (bz - WALL) - 4 * disc(PILOT) * (bz - WALL + 1)
  - roundedRectArea(cw, chh, PLUG.r) * WALL
  - disc(JACK.hole) * WALL;
run.near("base volume", cad.part(base.id).volume, baseVol);
const lidVol = outer * LID - inner * (LID - WALL) + 4 * bossArea * (LID - WALL) - 4 * disc(CLEAR) * LID;
run.near("lid volume", cad.part(lid.id).volume, lidVol);
run.near("board volume", cad.part(board.id).volume,
  (BOARD[0] * BOARD[1] - 4 * NOTCH * NOTCH - 4 * disc(CLEAR)) * BOARD[2]);
run.near("jack volume", cad.part(jack.id).volume,
  (L - WALL - 1 - 84) * 9 * JACK.dia + disc(JACK.dia) * 2.5);
// Everything inside fits under the lid.
const inside = cad.parts({ has: ["component"] });
run.ok("components below the lid's inner face", ck.boxOf(inside).max[2] < H - WALL);
run.ok("board clears the bosses",
  Math.hypot(bx + NOTCH - BOSS_AT, by - BOSS_AT) > BOSS_R && bx + NOTCH - BOSS_AT - BOSS_R >= 1 - 1e-12);

// ── Renders ──────────────────────────────────────────────────────────
const lift = 30;
const notLid = cad.parts().filter((p) => p.id !== lid.id).map((p) => p.id);
await run.render([{ as: "iso", view: "iso", ground: true }]);
cad.move3d(lid, [0, 0, lift]);
await run.render([{ as: "lid-lifted", view: "iso", ground: true }]);
cad.move3d(lid, [0, 0, -lift]);
await run.render([{ as: "top", view: "top", select: notLid }]);

return run.summary();
