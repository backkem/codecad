// 11 - Play tower with slide and swing: the timber frame.
//
// The flagship of docs/3d-examples.md, here only its plain frame: four
// 90 x 90 posts carrying a 1500 x 1500 platform 1450 above the ground
// (rim boards and joists 45 x 145, eleven deck boards 130 x 28), a swing
// beam 90 x 140 whose underside is 2300 above the ground, running from a
// head rail between the tower's right-hand posts out to an A-frame of two
// 90 x 90 legs with a tie, and stairs up the left side (two 45 x 195
// stringers, seven 28 mm treads). Over the platform a gabled timber roof,
// pitch 35 degrees, ridge along x: two 45 x 95 wall plates on the posts
// (160 past them at the gable ends), four pairs of 45 x 95 rafters (150
// past the plates at the eaves), a 45 x 145 ridge board, seven 22 mm roof
// boards a side butted with 5 mm gaps, and two ridge caps. 63 parts in 17
// definitions: repeated members are copies of one definition.
//
// The deck boards at the front and back are notched round the posts with
// cut(board, {part: post}); the back one is the front one turned half a
// turn, so they share a definition, and so does the back half of the roof.
// The angled members (legs, stringers, rafters) are built square to their
// length and cut level and plumb by intersecting them with a box
// (lib/shapes.js, member and keep); a rafter's seat on its plate is cut
// with the plate itself. The roof boards are butted, not lapped, so that
// every contact is a face the overlap check reads as a touch (lapped
// boards would touch each other and the rafters along lines).
//
// The checks ask the kernel that no two parts overlap, that the swing beam
// rests on the head rail and both A-frame legs, that every rafter rests on
// a plate and the ridge, and how far the roof clears the swing beam. Not
// here yet: the cabin, railings, sandbox, slide, swings and fixings, and
// the clearance rules (free space round the swings, head and finger traps,
// railing heights), which are script rules over the same queries.
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

const timber = (member, extra = {}) => ({ name: member, attrs: { member, material: "timber", ...extra } });
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

// ── Roof: wall plates, rafters, a ridge board, roof boards, ridge caps ─
// A gable over the platform, the ridge along x at y = mid, both slopes at
// ROOF.pitch. Everything on the back slope is the front's copy turned half
// a turn about the platform's centre, so it shares the front's definitions.
const ROOF = { pitch: 35, plate: [45, 95], rafter: [95, 45], ridge: [45, 145],
  board: { t: 22, n: 7, gap: 5 }, cap: { t: 22, w: 100 }, overhang: 160, eave: 150 };
const th = ROOF.pitch * PI / 180, tn = Math.tan(th), cs = cos(th), sn = Math.sin(th);
const half = { rotate: { angle: 180, about: [mid, mid, 0] } };
// Wall plates on edge, centred on the posts, overhanging the posts at the
// gable ends.
const [pw, ph] = ROOF.plate;
const plateY = POST / 2 - pw / 2, zp = POST_H + ph;
const X0 = -ROOF.overhang, X1 = SIZE + ROOF.overhang;
const plate = box([X0, plateY, POST_H], [X1, plateY + pw, zp], timber("wall plate", { section: "45x95" }));
cad.copy3d(plate, half);
// Rafters: the underside runs through the plate's inner top edge; cut plumb
// at the eave (ROOF.eave beyond the plate) and against the ridge board; a
// birdsmouth seat cut with the plate itself.
const [ra, rb] = ROOF.rafter, [rgT, rgD] = ROOF.ridge;
const yIn = plateY + pw, yEave = plateY - ROOF.eave, yHead = mid - rgT / 2;
const under = (y) => zp + (y - yIn) * tn;       // rafter underside
const depthV = ra / cs;                          // rafter depth, measured vertically
const top = (y) => under(y) + depthV;            // rafter top
const rx0 = X0 + 10 + rb / 2;                    // first rafter's centre (x)
const rPitch = (X1 - X0 - 20 - rb) / 3;          // four pairs, the outer ones at the plate ends
const onAxis = (y) => under(y) + depthV / 2;
const rafter = member([rx0, yEave, onAxis(yEave)], [rx0, yHead, onAxis(yHead)], [ra, rb],
  { over: ra, ...timber("rafter", { section: "45x95" }) });
keep(rafter, [null, yEave, null], [null, yHead, null]);
cad.cut(rafter, { part: plate });
cad.copy3d(rafter, { move: [rPitch, 0, 0], count: 3 });
cad.copy3d(cad.parts({ attrs: { member: "rafter" } }), half);
// The ridge board between the rafter heads, its top 20 below the rafters'.
const ridgeTop = top(yHead) - 20;
const ridge = box([X0 + 10, yHead, ridgeTop - rgD], [X1 - 10, mid + rgT / 2, ridgeTop], timber("ridge", { section: "45x145" }));
// Roof boards, butted with a gap, eave to ridge on the rafters' top face.
const RB = ROOF.board;
const S = (yHead - yEave) / cs;                  // slope length of the rafters' top
const bWid = (S - (RB.n - 1) * RB.gap) / RB.n;
const BX0 = X0 - 10, BX1 = X1 + 10;
const roofBoard = box([BX0, 0, 0], [BX1, bWid, RB.t], timber("roof board", { section: `22x${Math.round(bWid)}` }));
cad.rotate3d(roofBoard, { angle: ROOF.pitch, axis: [1, 0, 0] });
cad.move3d(roofBoard, [0, yEave, top(yEave)]);
const up = RB.gap + bWid;
cad.copy3d(roofBoard, { move: [0, up * cs, up * sn], count: RB.n - 1 });
cad.copy3d(cad.parts({ attrs: { member: "roof board" } }), half);
// Ridge caps on the top boards, cut plumb where they meet at y = mid.
const RC = ROOF.cap;
const yE2 = yEave - RB.t * sn, zE2 = top(yEave) + RB.t * cs;   // eave edge of the boards' top face
const s0 = S - RC.w;                                           // the cap's lower edge, up that face
const sCut = (t) => (mid - yE2 + t * sn) / cs - s0;            // to the plumb cut, at t above the boards
const cap = box([BX0, 0, 0], [BX1, sCut(RC.t) + 20, RC.t], timber("ridge cap", { section: "22x100" }));
cad.rotate3d(cap, { angle: ROOF.pitch, axis: [1, 0, 0] });
cad.move3d(cap, [0, yE2 + s0 * cs, zE2 + s0 * sn]);
keep(cap, [null, null, null], [null, mid, null]);
cad.copy3d(cap, half);
const roofTop = zE2 + (s0 + sCut(RC.t)) * sn + RC.t * cs;     // where the caps meet, on top

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
const count = (member) => parts.filter((p) => p.attrs.member === member).length;
run.equal("63 parts", parts.length, 63);
run.equal("17 definitions", new Set(parts.map((p) => p.def)).size, 17);
const want = { post: 4, rim: 4, joist: 3, deck: 11, "head rail": 1, "swing beam": 1, leg: 2, tie: 1, stringer: 2, tread: 7,
  "wall plate": 2, rafter: 8, ridge: 1, "roof board": 14, "ridge cap": 2 };
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

// The roof: plates and ridge are boxes; a rafter is its section along its
// axis between the two plumb cuts, less the birdsmouth (a right triangle,
// the plate's width by that width times the slope, across the rafter); a
// roof board is a box; a ridge cap is its length times the area between
// its lower edge and the plumb cut, integrated over its thickness.
run.near("wall plate volume", vol(plate.id), pw * ph * (X1 - X0));
run.near("rafter volume", vol(rafter.id), ra * rb * (yHead - yEave) / cs - 0.5 * pw * (pw * tn) * rb);
run.near("ridge volume", vol(ridge.id), rgT * rgD * (X1 - X0 - 20));
run.near("roof board volume", vol(roofBoard.id), RB.t * bWid * (BX1 - BX0));
run.near("ridge cap volume", vol(cap.id),
  (BX1 - BX0) * (RC.t * (mid - yE2) / cs - RC.t * s0 + tn * RC.t * RC.t / 2));

// Overall box: stringer foot to beam end, leg feet, the caps on top.
const legHalf = LEG / 2 / cos(atan2(LEG_SPREAD - LEG_TOP, SWING_H));
run.box("overall size", ck.boxOf(parts), {
  min: [xFoot - halfBand / m, mid - LEG_SPREAD - legHalf, 0],
  max: [BEAM_END, mid + LEG_SPREAD + legHalf, roofTop],
}, 1e-6);
run.near("deck height", ck.boxOf(cad.parts({ attrs: { member: "deck" } })).max[2], DECK);

// Between the parts, on the exact solids. A rule that fails names them.
const clash = cad.overlapping3d(null, undefined, { attrs: true });
run.ok("no two parts overlap", clash.pairs.length === 0 && clash.skipped.length === 0,
  clash.pairs.map((p) => `${p.aAttrs.member} ${p.a} and ${p.bAttrs.member} ${p.b}: ${p.volume}`)
    .concat(clash.skipped.map((s) => `${s.a} and ${s.b}: ${s.reason}`)).join("; "));
const rests = cad.pairsWithin3d(beam, { has: ["member"] }, 1e-6, { attrs: true });
run.equal("the swing beam rests on the head rail and both legs",
  rests.pairs.map((p) => p.bAttrs.member).sort().join(", "), "head rail, leg, leg");
// Every rafter rests on a wall plate and against the ridge board.
const supports = cad.parts().filter((p) => ["wall plate", "ridge"].includes(p.attrs.member)).map((p) => p.id);
const seats = cad.pairsWithin3d({ attrs: { member: "rafter" } }, supports, 1e-6, { attrs: true });
const byRafter = {};
for (const p of seats.pairs) (byRafter[p.a] ??= []).push(p.bAttrs.member);
run.ok("every rafter rests on a wall plate and the ridge",
  Object.keys(byRafter).length === 8 && Object.values(byRafter).every((s) => s.sort().join() === "ridge,wall plate"),
  JSON.stringify(byRafter));
// The roof clears the swing beam under it. Nearest are the rafters over
// it: from the beam's top edge (y = mid - 45) square to their sloping
// underside, the vertical gap times cos(pitch).
const roofIds = cad.parts().filter((p) => ["wall plate", "rafter", "ridge", "roof board", "ridge cap"]
  .includes(p.attrs.member)).map((p) => p.id);
const near = cad.pairsWithin3d(beam, roofIds, 1000);
const headroom = Math.min(...near.pairs.map((p) => p.distance));
run.near("the swing beam's clearance to the roof", headroom,
  Math.min((under(mid - bw / 2) - (SWING_H + bd)) * cs, ridgeTop - rgD - (SWING_H + bd)));
run.ok("the swing beam clears the roof by more than 300", headroom > 300);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso-sw", ground: true, colorBy: { attr: "member" } },
  { as: "exploded", view: "iso-sw", explode: 0.35, colorBy: { attr: "member" } },
  { as: "front", view: "front", colorBy: { attr: "member" } },
]);

return run.summary({
  stairs: { rise, going: g, pitch_deg: Math.round(theta * 1800 / PI) / 10 },
  roof: { pitch_deg: ROOF.pitch, parts: roofIds.length, headroom: Math.round(headroom * 10) / 10 },
});
