// 11 - Play tower with slide and swing: the timber frame.
//
// The flagship of docs/3d-examples.md, here only its plain frame: four
// 90 x 90 posts carrying a 1500 x 1500 platform 1450 above the ground
// (rim boards and joists 45 x 145, eleven deck boards 130 x 28), a swing
// beam 90 x 140 whose underside is 2300 above the ground, running from a
// head rail between the tower's right-hand posts out to an A-frame of two
// 90 x 90 legs with a tie, and stairs up the left side (two 45 x 195
// stringers, seven 28 mm treads). 36 parts in 12 definitions: repeated
// members are copies of one definition.
//
// The deck boards at the front and back are notched round the posts with
// cut(board, {part: post}); the back one is the front one turned half a
// turn, so they share a definition. The angled members (legs, stringers)
// are built square to their length and cut level and plumb by
// intersecting them with a box (lib/shapes.js, member and keep).
//
// Not here yet (later rungs): the cabin, roof, railings, sandbox, slide,
// swings and fixings; the clearance rules (free space round the swings,
// head and finger traps, railing heights) and the check that no two parts
// overlap, which need interference and distance queries between parts.
//
// x = left to right (the swing beam runs out along +x), y = front to
// back (front at y = 0), z = up.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("11-play-tower-frame");
const { box, member, keep } = sh;
const { PI, cos, atan2, hypot } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const SIZE = 1500;             // platform, outside
const POST = 90;               // post section
const POST_H = 2300;           // post length (to the head rail's top)
const DECK = 1450;             // top of the deck above the ground
const BOARD_T = 28, BOARD_W = 130, N_BOARD = 11;
const JOIST = [45, 145];       // rims and joists: thickness, depth
const N_JOIST = 3;
const BEAM = [90, 140];        // swing beam and head rail: width, depth
const SWING_H = 2300;          // underside of the swing beam
const A_X = 3900;              // A-frame centre line (x)
const BEAM_END = 4000;
const LEG = 90, LEG_SPREAD = 960, LEG_TOP = 60;   // feet and heads off the centre (y)
const TIE = { t: 45, h: 100, z: 1050, half: 570 };
const STAIR = { width: 800, rises: 8, going: 200, nose: 20, stringer: [195, 45] };

const timber = (member, extra = {}) => ({ name: member, attrs: { member, ...extra } });
const mid = SIZE / 2;

// ── Posts ────────────────────────────────────────────────────────────
const post = box([0, 0, 0], [POST, POST, POST_H], timber("post", { section: "90x90" }));
cad.copy3d(post, { move: [SIZE - POST, 0, 0] });
cad.copy3d(cad.parts({ attrs: { member: "post" } }), { move: [0, SIZE - POST, 0] });
const posts = cad.parts({ attrs: { member: "post" } });

// ── Platform frame: rims between the posts, joists between the rims ──
const [jt, jd] = JOIST;
const zj = DECK - BOARD_T - jd;        // underside of the frame
const front = box([POST, 0, zj], [SIZE - POST, jt, zj + jd], timber("rim", { section: "45x145" }));
cad.copy3d(front, { move: [0, SIZE - jt, 0] });
const leftRim = box([0, POST, zj], [jt, SIZE - POST, zj + jd], timber("rim", { section: "45x145" }));
cad.copy3d(leftRim, { move: [SIZE - jt, 0, 0] });
const pitchJ = SIZE / (N_JOIST + 1);
const joist = box([pitchJ - jt / 2, jt, zj], [pitchJ + jt / 2, SIZE - jt, zj + jd], timber("joist", { section: "45x145" }));
cad.copy3d(joist, { move: [pitchJ, 0, 0], count: N_JOIST - 1 });

// ── Deck boards across the joists ────────────────────────────────────
const gap = (SIZE - N_BOARD * BOARD_W) / (N_BOARD - 1);
const zd = DECK - BOARD_T;
// Front board, notched round the two front posts.
const edgeBoard = box([0, 0, zd], [SIZE, BOARD_W, DECK], timber("deck", { section: "28x130" }));
for (const p of posts.filter((p) => p.bbox.min[1] === 0)) cad.cut(edgeBoard, { part: p });
// The back board is the same board turned half a turn.
cad.copy3d(edgeBoard, { rotate: { angle: 180, about: [mid, mid, 0] } });
// The boards between them share another definition.
const board = box([0, BOARD_W + gap, zd], [SIZE, 2 * BOARD_W + gap, DECK], timber("deck", { section: "28x130" }));
cad.copy3d(board, { move: [0, BOARD_W + gap, 0], count: N_BOARD - 3 });

// ── Swing beam on a head rail, out to the A-frame ────────────────────
const [bw, bd] = BEAM;
box([SIZE - POST, POST, SWING_H - bd], [SIZE, SIZE - POST, SWING_H], timber("head rail", { section: "90x140" }));
const beam = box([SIZE - POST, mid - bw / 2, SWING_H], [BEAM_END, mid + bw / 2, SWING_H + bd], timber("swing beam", { section: "90x140" }));

// A-frame legs: from the feet to under the beam, cut level at both ends.
const foot = [A_X, mid - LEG_SPREAD, 0], head = [A_X, mid - LEG_TOP, SWING_H];
const leg = member(foot, head, [LEG, LEG], { over: LEG, ...timber("leg", { section: "90x90" }) });
keep(leg, [null, null, 0], [null, null, SWING_H]);
cad.copy3d(leg, { rotate: { angle: 180, about: [A_X, mid, 0] } });
// A tie across both legs, on their outer face.
const tie = box([A_X + LEG / 2, mid - TIE.half, TIE.z], [A_X + LEG / 2 + TIE.t, mid + TIE.half, TIE.z + TIE.h], timber("tie", { section: "45x100" }));

// ── Stairs up the left side ──────────────────────────────────────────
const rise = DECK / STAIR.rises, g = STAIR.going, nose = STAIR.nose;
const nTread = STAIR.rises - 1;
const [sd, st] = STAIR.stringer;
const y0 = mid - STAIR.width / 2, y1 = mid + STAIR.width / 2;
// Tread k (1..7) runs from riser k (at x = -(8 - k) g) less the nosing to
// riser k + 1, with its top at k rises.
const riser = (k) => -(STAIR.rises - k) * g;
const tread = box([riser(1) - nose, y0, rise - BOARD_T], [riser(2), y1, rise], timber("tread", { section: "28x220" }));
cad.copy3d(tread, { move: [g, 0, rise], count: nTread - 1 });
// Stringers along the line through the treads' centres, cut level on the
// ground, plumb against the platform and level with the deck.
const m = rise / g;                                     // slope
const xc = riser(nTread) - nose + (g + nose) / 2;       // centre of the top tread
const zc = nTread * rise - BOARD_T / 2;
const line = (x) => zc + (x - xc) * m;
const xFoot = xc - zc / m;                              // where the line meets the ground
const sFoot = [xFoot, y0 - st / 2, 0], sHead = [0, y0 - st / 2, line(0)];
const stringer = member(sFoot, sHead, [sd, st], { over: sd, ...timber("stringer", { section: "45x195" }) });
keep(stringer, [null, null, 0], [0, null, DECK]);
cad.copy3d(stringer, { move: [0, STAIR.width + st, 0] });

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
const count = (member) => parts.filter((p) => p.attrs.member === member).length;
run.equal("36 parts", parts.length, 36);
run.equal("12 definitions", new Set(parts.map((p) => p.def)).size, 12);
const want = { post: 4, rim: 4, joist: 3, deck: 11, "head rail": 1, "swing beam": 1, leg: 2, tie: 1, stringer: 2, tread: 7 };
for (const [k, n] of Object.entries(want)) run.equal(`${n} x ${k}`, count(k), n);

const vol = (id) => cad.part(id).volume;
run.near("post volume", vol(post.id), POST * POST * POST_H);
run.near("deck board volume", vol(board.id), BOARD_W * BOARD_T * SIZE);
run.near("notched deck board volume", vol(edgeBoard.id), BOARD_W * BOARD_T * SIZE - 2 * POST * POST * BOARD_T);
run.near("swing beam volume", vol(beam.id), bw * bd * (BEAM_END - SIZE + POST));
// A prism cut by two planes that each cross it fully: its section times
// the length of its axis between them.
const legLen = hypot(LEG_SPREAD - LEG_TOP, SWING_H);
run.near("leg volume", vol(leg.id), LEG * LEG * legLen);
// The stringer: the same between the ground and the plumb cut, less the
// small triangle the level cut at deck height takes off its top corner.
const theta = atan2(rise, g);
const halfBand = sd / 2 / cos(theta);                    // half its depth, measured vertically
const over = line(0) + halfBand - DECK;                  // how far it would stand above the deck
run.ok("stringer reaches above the deck before its level cut", over > 0);
const sLen = hypot(0 - xFoot, line(0));
run.near("stringer volume", vol(stringer.id), sd * st * sLen - 0.5 * over * (over / m) * st);
run.near("tread volume", vol(tread.id), (g + nose) * STAIR.width * BOARD_T);
run.ok("treads lie within the stringers' depth",
  Math.abs((g + nose) / 2 * m) + BOARD_T / 2 < halfBand);

// Overall box: stringer foot to beam end, leg feet, beam top.
const legHalf = LEG / 2 / cos(atan2(LEG_SPREAD - LEG_TOP, SWING_H));
run.box("overall size", ck.boxOf(parts), {
  min: [xFoot - halfBand / m, mid - LEG_SPREAD - legHalf, 0],
  max: [BEAM_END, mid + LEG_SPREAD + legHalf, SWING_H + bd],
}, 1e-6);
run.near("deck height", ck.boxOf(cad.parts({ attrs: { member: "deck" } })).max[2], DECK);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso-sw", ground: true, colorBy: { attr: "member" } },
  { as: "exploded", view: "iso-sw", explode: 0.35, colorBy: { attr: "member" } },
  { as: "front", view: "front", colorBy: { attr: "member" } },
]);

return run.summary({ stairs: { rise, going: g, pitch_deg: Math.round(theta * 1800 / PI) / 10 } });
