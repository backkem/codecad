// The play tower with slide and swings (docs/3d-examples.md, example 11),
// written once and used by two scripts, loaded with
//
//   const tower = await cad.use("playtower");
//   const m = tower.build();
//
// 11-play-tower.js checks it and renders it; 11-play-tower-partlist.js
// draws its part list. Everything a play set means lives here, in script:
// member names, sections, what is bought, the fixings each joint takes and
// the order of assembly. The core only sees parts and attributes.
//
// The design: four 90 x 90 posts carry a 1500 x 1500 platform 1450 above
// the ground. On it a cabin of board walls on three sides (a window at the
// back and on the right, a door to the stairs on the left) under a gabled
// timber roof, and an open front with a railing (a balcony side) and the
// opening for a wave slide. Stairs with closed risers and handrails climb
// the left side. Under the platform a sandbox framed in boards. A swing beam
// runs out to the right, from a head rail between the right-hand posts to
// an A-frame, with two swings. Bought: the slide and its foot, the swing
// seats, ropes and hangers, the sand, and the fixings.
//
// Every part carries `member` (what it is), `assembly`, `material` and,
// for sawn timber, `section` (thickness x width, mm). Bought parts carry
// `bought: true` and `item` (what to ask for), and `unit` where they are
// sold by length or volume. Angled members carry `cut`: their end cuts in
// words, with the angle off square (what the saw is set to).
//
// x = left to right (the swing beam runs out along +x), y = front to back
// (front at y = 0), z = up. Millimetres and degrees.

const sh = await cad.use("shapes");
const { box, member, keep } = sh;
const { PI, cos, sin, tan, atan2, hypot } = Math;
const rad = (d) => (d * PI) / 180;
const deg = (r) => (r * 180) / PI;
const r1 = (v) => Math.round(v * 10) / 10;

// ── Parameters (mm) ──────────────────────────────────────────────────
// The timber frame (posts, platform, swing beam, A-frame, stairs, roof).
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
const TIE = { t: 45, h: 95, z: 1050, half: 570 };   // 45 x 95, the stock of the plates and rafters
const STAIR = { width: 800, rises: 8, going: 200, nose: 20, stringer: [195, 45] };
const ROOF = { pitch: 35, plate: [45, 95], rafter: [95, 45], ridge: [45, 145],
  board: { t: 22, n: 7, gap: 5 }, cap: { t: 22, w: 85 }, overhang: 160, eave: 150 };

// The cabin: horizontal boards on the outside of the posts, from below the
// rim up to the posts' tops, butted with a gap; studs beside the openings.
const CLAD = { t: 22, w: 145, gap: 5, n: 7 };
const WINDOW = { from: 500, to: 1000, boards: [2, 4] };   // along the wall; boards left out
const STUD = [45, 95];                                     // depth into the cabin, width
// The front: a railing from the left wall to a rail post beside the slide.
const RAIL = { height: 600, rail: [95, 45], post: 90, slats: 4 };   // top above the deck
// The stairs: risers under each tread above the first, handrails on posts.
const RISER = { t: 22, h: 145 };
const HANDRAIL = { height: 700, section: [95, 45], post: 90 };      // top above the nosing line
// The sandbox: two rows of boards inside the posts, sand inside them.
const SANDBOX = { rows: 2, sand: 200 };
// The slide (bought): a moulded trough swept along a wave, its underside
// on the deck at the top and on a foot at the exit.
const SLIDE = {
  inner: 400, wall: 8, side: 160, lip: 30,   // trough: inside width, wall, side height, lips
  clear: 30,                                  // from the right-hand front post
  onDeck: 50, top: 150,                       // its flat top: on the deck, and in all
  steep: 40, crest: 15,                       // slope of the chute and of the wave's crest (deg)
  radii: [500, 900, 900, 700],                // the four bends, top to bottom
  chute: 400,                                 // the first straight run of the chute
  exit: 230, runout: 350,                     // the exit's underside above the ground, its flat run
  foot: [60, 40],                             // the foot: depth, set back from the end
};
// The swings (bought): seats on two ropes from hangers under the beam.
const SWING = { seat: [430, 170, 25], seatZ: 400, rope: 16, hanger: [60, 90, 10], between: 600, inset: 30 };

// Rules of thumb for the fixings, per joint (two parts that touch) or per
// part. A joint with `items: null` needs none (a notch, a bearing, a
// knot). Plain choices for a garden play set, not an engineering design.
const FIXINGS = [
  { a: "post", b: "rim", items: { "coach bolt M10 x 160": 2 } },
  { a: "joist", b: "rim", items: { "joist hanger 45 x 145": 1, "connector screw 5 x 40": 10 } },
  { a: "deck board", b: ["joist", "rim"], items: { "deck screw 5 x 60": 2 } },
  { a: "deck board", b: "post", items: null },
  { a: "head rail", b: "post", items: { "angle bracket 90 x 90": 2, "connector screw 5 x 40": 16 } },
  { a: "swing beam", b: "head rail", items: { "coach bolt M12 x 260": 2 } },
  { a: "swing beam", b: "leg", items: { "coach bolt M10 x 200": 2 } },
  { a: "swing beam", b: "wall board", items: null },
  { a: "tie", b: "leg", items: { "coach bolt M10 x 160": 2 } },
  { a: "stringer", b: "rim", items: { "coach bolt M10 x 120": 2 } },
  { a: "stringer", b: ["wall board", "deck board", "stud"], items: null },
  { a: "tread", b: "stringer", items: { "stair angle 40 x 40": 2, "screw 5 x 40": 8 } },
  { a: "riser", b: "tread", items: { "screw 4 x 50": 3 } },
  { a: "riser", b: "stringer", items: { "screw 4 x 50": 2 } },
  { a: "wall plate", b: "post", items: { "coach screw 10 x 160": 1 } },
  { a: "wall plate", b: ["wall board", "stud"], items: null },
  { a: "rafter", b: ["wall plate", "ridge"], items: { "screw 6 x 120": 2 } },
  { a: "roof board", b: "rafter", items: { "screw 5 x 50": 2 } },
  { a: "ridge cap", b: "roof board", items: { "screw 5 x 60": 6 } },
  { a: "ridge cap", b: "ridge cap", items: null },
  { a: "wall board", b: ["post", "stud", "rim", "head rail"], items: { "screw 4 x 40": 2 } },
  { a: "wall board", b: ["deck board", "wall board", "baluster"], items: null },
  { a: "stud", b: "deck board", items: { "screw 5 x 90": 4 } },
  { a: "stud", b: "head rail", items: { "screw 5 x 90": 2 } },
  { a: "top rail", b: ["post", "rail post"], items: { "screw 6 x 120": 2 } },
  { a: "rail post", b: "rim", items: { "coach bolt M10 x 160": 2 } },
  { a: "rail post", b: "deck board", items: null },
  { a: "baluster", b: ["rim", "top rail", "post"], items: { "screw 4 x 50": 2 } },
  { a: "baluster", b: "deck board", items: null },
  { a: "stair post", b: "stringer", items: { "coach bolt M10 x 160": 2 } },
  { a: "stair post", b: "wall board", items: null },
  { a: "handrail", b: "stair post", items: { "coach bolt M8 x 120": 2 } },
  { a: "handrail", b: "wall board", items: { "angle bracket 50 x 50": 1, "screw 4 x 40": 4 } },
  { a: "sandbox board", b: ["post", "sandbox board"], items: { "screw 5 x 70": 2 } },
  { a: "sand", b: "sandbox board", items: null },
  { a: "slide", b: "deck board", items: { "screw 6 x 60": 4 } },
  { a: "slide", b: "slide foot", items: null },
  { a: "hanger", b: "swing beam", items: { "coach bolt M10 x 200": 2 } },
  { a: "rope", b: ["hanger", "seat"], items: null },
  // Per part: a ground anchor under every foot that stands on the ground
  // and carries the frame.
  { each: ["post", "leg", "stair post"], items: { "ground anchor with post shoe": 1 } },
];

// The order of assembly: each step names the members it adds.
const STEPS = [
  { title: "Posts and platform frame", members: ["post", "rim", "joist"] },
  { title: "Deck", members: ["deck board"] },
  { title: "Swing frame", members: ["head rail", "swing beam", "leg", "tie"] },
  { title: "Stairs", members: ["stringer", "tread", "riser"] },
  { title: "Roof frame", members: ["wall plate", "rafter", "ridge"] },
  { title: "Roof boards", members: ["roof board", "ridge cap"] },
  { title: "Cabin walls", members: ["stud", "wall board"] },
  { title: "Railing", members: ["rail post", "top rail", "baluster"] },
  { title: "Stair handrails", members: ["stair post", "handrail"] },
  { title: "Sandbox", members: ["sandbox board", "sand"] },
  { title: "Slide", members: ["slide", "slide foot"] },
  { title: "Swings", members: ["hanger", "rope", "seat"] },
];

// Stock lengths the timber is sold in (mm), and the saw's kerf.
const STOCK = { lengths: [2400, 3000, 3600, 4200, 4800], kerf: 3 };

function build() {
  let ASM = "tower";
  const timber = (member, section, extra = {}) =>
    ({ name: member, attrs: { member, assembly: ASM, material: "timber", section, ...extra } });
  const bought = (member, item, material, extra = {}) =>
    ({ name: member, attrs: { member, assembly: ASM, material, bought: true, item, ...extra } });
  const mid = SIZE / 2;
  const half = { rotate: { angle: 180, about: [mid, mid, 0] } };
  const byMember = (m) => cad.parts({ attrs: { member: m } });
  // A copy of `part` whose box starts at `min`.
  const copyTo = (part, min) => cad.copy3d(part, { move: min.map((v, i) => v - part.bbox.min[i]) });

  // ── Posts ──────────────────────────────────────────────────────────
  const post = box([0, 0, 0], [POST, POST, POST_H], timber("post", "90x90"));
  cad.copy3d(post, { move: [SIZE - POST, 0, 0] });
  cad.copy3d(byMember("post"), { move: [0, SIZE - POST, 0] });
  const posts = byMember("post");

  // ── Platform frame: rims between the posts, joists between the rims ─
  const [jt, jd] = JOIST;
  const zj = DECK - BOARD_T - jd;        // underside of the frame
  const front = box([POST, 0, zj], [SIZE - POST, jt, zj + jd], timber("rim", "45x145"));
  cad.copy3d(front, { move: [0, SIZE - jt, 0] });
  const leftRim = box([0, POST, zj], [jt, SIZE - POST, zj + jd], timber("rim", "45x145"));
  cad.copy3d(leftRim, { move: [SIZE - jt, 0, 0] });
  const pitchJ = SIZE / (N_JOIST + 1);
  const joist = box([pitchJ - jt / 2, jt, zj], [pitchJ + jt / 2, SIZE - jt, zj + jd], timber("joist", "45x145"));
  cad.copy3d(joist, { move: [pitchJ, 0, 0], count: N_JOIST - 1 });

  // ── Deck boards across the joists ──────────────────────────────────
  const gap = (SIZE - N_BOARD * BOARD_W) / (N_BOARD - 1);
  const zd = DECK - BOARD_T;
  // Front board, notched round the two front posts; the back board is the
  // same board turned half a turn, so they share a definition.
  const edgeBoard = box([0, 0, zd], [SIZE, BOARD_W, DECK], timber("deck board", "28x130"));
  for (const p of posts.filter((p) => p.bbox.min[1] === 0)) cad.cut(edgeBoard, { part: p });
  cad.copy3d(edgeBoard, half);
  const board = box([0, BOARD_W + gap, zd], [SIZE, 2 * BOARD_W + gap, DECK], timber("deck board", "28x130"));
  cad.copy3d(board, { move: [0, BOARD_W + gap, 0], count: N_BOARD - 3 });

  // ── Swing beam on a head rail, out to the A-frame ──────────────────
  ASM = "swing frame";
  const [bw, bd] = BEAM;
  box([SIZE - POST, POST, SWING_H - bd], [SIZE, SIZE - POST, SWING_H], timber("head rail", "90x140"));
  const beam = box([SIZE - POST, mid - bw / 2, SWING_H], [BEAM_END, mid + bw / 2, SWING_H + bd], timber("swing beam", "90x140"));
  // A-frame legs from the feet to under the beam, cut level at both ends.
  const legTilt = deg(atan2(LEG_SPREAD - LEG_TOP, SWING_H));     // from plumb
  const foot = [A_X, mid - LEG_SPREAD, 0], head = [A_X, mid - LEG_TOP, SWING_H];
  const leg = member(foot, head, [LEG, LEG], { over: LEG, ...timber("leg", "90x90", { cut: `level ${r1(legTilt)}°, both ends` }) });
  keep(leg, [null, null, 0], [null, null, SWING_H]);
  cad.copy3d(leg, { rotate: { angle: 180, about: [A_X, mid, 0] } });
  // A tie across both legs, on their outer face.
  box([A_X + LEG / 2, mid - TIE.half, TIE.z], [A_X + LEG / 2 + TIE.t, mid + TIE.half, TIE.z + TIE.h], timber("tie", `${TIE.t}x${TIE.h}`));

  // ── Stairs up the left side ────────────────────────────────────────
  ASM = "stairs";
  const rise = DECK / STAIR.rises, g = STAIR.going, nose = STAIR.nose;
  const nTread = STAIR.rises - 1;
  const [sd, st] = STAIR.stringer;
  const y0 = mid - STAIR.width / 2, y1 = mid + STAIR.width / 2;
  // Tread k (1..7) runs from riser k (at x = -(8 - k) g) less the nosing to
  // riser k + 1, with its top at k rises.
  const riser = (k) => -(STAIR.rises - k) * g;
  const tread = box([riser(1) - nose, y0, rise - BOARD_T], [riser(2), y1, rise], timber("tread", "28x220"));
  cad.copy3d(tread, { move: [g, 0, rise], count: nTread - 1 });
  // Stringers along the line through the treads' centres, cut level on the
  // ground, plumb against the platform and level with the deck.
  const m = rise / g;                                     // slope
  const theta = deg(atan2(rise, g));                      // pitch
  const xc = riser(nTread) - nose + (g + nose) / 2;       // centre of the top tread
  const zc = nTread * rise - BOARD_T / 2;
  const line = (x) => zc + (x - xc) * m;
  const xFoot = xc - zc / m;                              // where the line meets the ground
  const halfBand = sd / 2 / cos(rad(theta));              // half its depth, measured vertically
  const stringer = member([xFoot, y0 - st / 2, 0], [0, y0 - st / 2, line(0)], [sd, st],
    { over: sd, ...timber("stringer", "45x195", { cut: `level ${r1(90 - theta)}°, plumb ${r1(theta)}°, level` }) });
  keep(stringer, [null, null, 0], [0, null, DECK]);
  cad.copy3d(stringer, { move: [0, STAIR.width + st, 0] });
  // Closed risers: a board under the nose of each tread above the first,
  // fixed to the tread with a drain gap at its foot.
  const riserBoard = box([riser(2) - RISER.t, y0, 2 * rise - BOARD_T - RISER.h], [riser(2), y1, 2 * rise - BOARD_T],
    timber("riser", "22x145"));
  cad.copy3d(riserBoard, { move: [g, 0, rise], count: nTread - 2 });

  // ── Roof: wall plates, rafters, a ridge board, roof boards, ridge caps
  // A gable over the platform, the ridge along x at y = mid, both slopes at
  // ROOF.pitch. Everything on the back slope is the front's copy turned half
  // a turn about the platform's centre, so it shares the front's definitions.
  ASM = "roof";
  const th = rad(ROOF.pitch), tn = tan(th), cs = cos(th), sn = sin(th);
  const [pw, ph] = ROOF.plate;
  const plateY = POST / 2 - pw / 2, zp = POST_H + ph;
  const X0 = -ROOF.overhang, X1 = SIZE + ROOF.overhang;
  const plate = box([X0, plateY, POST_H], [X1, plateY + pw, zp], timber("wall plate", "45x95"));
  cad.copy3d(plate, half);
  // Rafters: the underside runs through the plate's inner top edge; cut
  // plumb at the eave (ROOF.eave beyond the plate) and against the ridge
  // board; a birdsmouth seat cut with the plate itself.
  const [ra, rb] = ROOF.rafter, [rgT, rgD] = ROOF.ridge;
  const yIn = plateY + pw, yEave = plateY - ROOF.eave, yHead = mid - rgT / 2;
  const under = (y) => zp + (y - yIn) * tn;       // rafter underside
  const depthV = ra / cs;                          // rafter depth, measured vertically
  const top = (y) => under(y) + depthV;            // rafter top
  const rx0 = X0 + 10 + rb / 2;                    // first rafter's centre (x)
  const rPitch = (X1 - X0 - 20 - rb) / 3;          // four pairs, the outer ones at the plate ends
  const onAxis = (y) => under(y) + depthV / 2;
  const rafter = member([rx0, yEave, onAxis(yEave)], [rx0, yHead, onAxis(yHead)], [ra, rb],
    { over: ra, ...timber("rafter", "45x95", { cut: `plumb ${ROOF.pitch}°, both ends, birdsmouth` }) });
  keep(rafter, [null, yEave, null], [null, yHead, null]);
  cad.cut(rafter, { part: plate });
  cad.copy3d(rafter, { move: [rPitch, 0, 0], count: 3 });
  cad.copy3d(byMember("rafter"), half);
  // The ridge board between the rafter heads, its top 20 below the rafters'.
  const ridgeTop = top(yHead) - 20;
  const ridge = box([X0 + 10, yHead, ridgeTop - rgD], [X1 - 10, mid + rgT / 2, ridgeTop], timber("ridge", "45x145"));
  // Roof boards, butted with a gap, eave to ridge on the rafters' top face.
  // Stained green: the roof panels of the box picture.
  const RB = ROOF.board;
  const S = (yHead - yEave) / cs;                  // slope length of the rafters' top
  const bWid = (S - (RB.n - 1) * RB.gap) / RB.n;
  const BX0 = X0 - 10, BX1 = X1 + 10;
  const roofBoard = box([BX0, 0, 0], [BX1, bWid, RB.t], timber("roof board", `22x${Math.ceil(bWid)}`, { material: "green stained timber" }));
  cad.rotate3d(roofBoard, { angle: ROOF.pitch, axis: [1, 0, 0] });
  cad.move3d(roofBoard, [0, yEave, top(yEave)]);
  const up = RB.gap + bWid;
  cad.copy3d(roofBoard, { move: [0, up * cs, up * sn], count: RB.n - 1 });
  cad.copy3d(byMember("roof board"), half);
  // Ridge caps on the top boards, cut plumb where they meet at y = mid
  // (a bevel along one edge).
  const RC = ROOF.cap;
  const yE2 = yEave - RB.t * sn, zE2 = top(yEave) + RB.t * cs;   // eave edge of the boards' top face
  const s0 = S - RC.w;                                           // the cap's lower edge, up that face
  const sCut = (t) => (mid - yE2 + t * sn) / cs - s0;            // to the plumb cut, at t above the boards
  const capW = sCut(RC.t);                                       // its width, on the top face
  const cap = box([BX0, 0, 0], [BX1, capW + 20, RC.t],
    timber("ridge cap", "22x145", { material: "green stained timber", cut: `edge bevelled ${ROOF.pitch}°` }));
  cad.rotate3d(cap, { angle: ROOF.pitch, axis: [1, 0, 0] });
  cad.move3d(cap, [0, yE2 + s0 * cs, zE2 + s0 * sn]);
  keep(cap, [null, null, null], [null, mid, null]);
  cad.copy3d(cap, half);
  const roofTop = zE2 + (s0 + sCut(RC.t)) * sn + RC.t * cs;     // where the caps meet, on top

  // ── Cabin: board walls on the left, back and right ─────────────────
  ASM = "cabin";
  const WALL_Z0 = POST_H - (CLAD.n * CLAD.w + (CLAD.n - 1) * CLAD.gap);
  const boardZ = (k) => WALL_Z0 + k * (CLAD.w + CLAD.gap);
  const planks = new Map();       // one definition per direction and length
  // One wall board from a to b along `along` ("x" or "y"), row k, on the
  // face at `face` (the other plan coordinate), thickness toward `out`.
  function plank(along, a, b, k, face, out) {
    const c = out > 0 ? [face, face + CLAD.t] : [face - CLAD.t, face];
    const z = [boardZ(k), boardZ(k) + CLAD.w];
    const min = along === "x" ? [a, c[0], z[0]] : [c[0], a, z[0]];
    const max = along === "x" ? [b, c[1], z[1]] : [c[1], b, z[1]];
    const key = `${along}:${b - a}`;
    if (planks.has(key)) return copyTo(planks.get(key), min);
    const p = box(min, max, timber("wall board", `${CLAD.t}x${CLAD.w}`));
    planks.set(key, p);
    return p;
  }
  // A wall of CLAD.n rows from a to b, less its openings: {from, to, boards: [k0, k1]}.
  function wall(along, a, b, face, out, openings = []) {
    for (let k = 0; k < CLAD.n; k++) {
      let pieces = [[a, b]];
      for (const o of openings.filter((o) => k >= o.boards[0] && k <= o.boards[1])) {
        pieces = pieces.flatMap(([p, q]) => [[p, Math.min(q, o.from)], [Math.max(p, o.to), q]]).filter(([p, q]) => q > p);
      }
      for (const [p, q] of pieces) plank(along, p, q, k, face, out);
    }
  }
  const [studD, studW] = STUD;
  const stud = (min, max) => box(min, max, timber("stud", `${studD}x${studW}`));
  // Back wall (y = SIZE), a window in the middle, studs beside it.
  wall("x", 0, SIZE, SIZE, +1, [WINDOW]);
  const backStud = stud([WINDOW.from - studW, SIZE - studD, DECK], [WINDOW.from, SIZE, POST_H]);
  copyTo(backStud, [WINDOW.to, SIZE - studD, DECK]);
  // Right wall (x = SIZE), past the back wall's end, a window, studs up to
  // the head rail.
  wall("y", 0, SIZE + CLAD.t, SIZE, +1, [WINDOW]);
  const rightStud = stud([SIZE - studD, WINDOW.from - studW, DECK], [SIZE, WINDOW.from, SWING_H - bd]);
  copyTo(rightStud, [SIZE - studD, WINDOW.to, DECK]);
  // Left wall (x = 0): the door is the stair's whole width, stringers
  // included; a stud on each side of it.
  const door = [y0 - st, y1 + st];
  wall("y", 0, door[0], 0, -1);
  wall("y", door[1], SIZE + CLAD.t, 0, -1);
  const doorStud = stud([0, door[0] - studW, DECK], [studD, door[0], POST_H]);
  copyTo(doorStud, [0, door[1], DECK]);

  // ── Front: a railing from the left wall to a rail post at the slide ─
  ASM = "railing";
  const slideHalf = SLIDE.inner / 2 + SLIDE.wall + SLIDE.lip;          // the slide's half width
  const slideX = SIZE - POST - SLIDE.clear - slideHalf;               // its centre line
  const opening = [slideX - slideHalf - 40, SIZE - POST];             // the slide opening, along x
  const railTop = DECK + RAIL.height;
  const RP = RAIL.post;
  const railPost = box([opening[0] - RP, -RP, WALL_Z0], [opening[0], 0, railTop], timber("rail post", "90x90"));
  const [rd, rt] = RAIL.rail;
  // The top rail from the corner post along the rail post's back face.
  box([POST, 0, railTop - rd], [opening[0], rt, railTop], timber("top rail", "45x95"));
  // Slats on the outside, from the left wall's face to the rail post.
  const span = opening[0] - RP + CLAD.t;
  const slatGap = (span - RAIL.slats * CLAD.w) / RAIL.slats;
  const slat = box([-CLAD.t, -CLAD.t, WALL_Z0], [-CLAD.t + CLAD.w, 0, railTop], timber("baluster", `${CLAD.t}x${CLAD.w}`));
  cad.copy3d(slat, { move: [CLAD.w + slatGap, 0, 0], count: RAIL.slats - 1 });

  // ── Stair handrails on posts beside the stringers ──────────────────
  ASM = "stairs";
  const nosing = (x) => DECK + m * (x + nose);               // the line over the treads' front edges
  const HP = HANDRAIL.post, [hd, ht] = HANDRAIL.section;
  const hrTop = (x) => nosing(x) + HANDRAIL.height;
  const postY = [y0 - st - HP, y0 - st];                    // beside the near stringer
  const hrY = [postY[0] - ht, postY[0]];
  // A post at the foot, and one against the left wall.
  const xLow = xFoot + 150, xHigh = -CLAD.t - HP;
  const lowPost = box([xLow, postY[0], 0], [xLow + HP, postY[1], hrTop(xLow + HP) + 40], timber("stair post", "90x90"));
  const highPost = box([xHigh, postY[0], 0], [xHigh + HP, postY[1], hrTop(xHigh + HP) + 40], timber("stair post", "90x90"));
  // The handrail: its top on hrTop, from the low post's outer face to the
  // wall, cut plumb at both ends.
  const hrAxis = (x) => hrTop(x) - hd / 2 / cos(rad(theta));
  const yh = (hrY[0] + hrY[1]) / 2;
  const handrail = member([xLow, yh, hrAxis(xLow)], [-CLAD.t, yh, hrAxis(-CLAD.t)], [hd, ht],
    { over: hd, ...timber("handrail", "45x95", { cut: `plumb ${r1(theta)}°, both ends` }) });
  keep(handrail, [xLow, null, null], [-CLAD.t, null, null]);
  // The far side: the same parts moved across the stairs.
  const across = (y) => SIZE - y;   // mirror line y = mid, for a box's far edge
  for (const p of [lowPost, highPost]) cad.copy3d(p, { move: [0, across(postY[1]) - postY[0], 0] });
  cad.copy3d(handrail, { move: [0, across(hrY[1]) - hrY[0], 0] });

  // ── Sandbox under the platform ─────────────────────────────────────
  ASM = "sandbox";
  // Front and back boards behind the posts, the sides between them in line
  // with the posts' inner faces (set in from the stair posts: flush with the
  // posts' outer faces they left a 22 mm finger gap to the upper stair posts).
  const sbT = BOARD_T, sbH = BOARD_W;
  const sbFront = box([0, POST, 0], [SIZE, POST + sbT, sbH], timber("sandbox board", "28x130"));
  const sbSide = box([POST, POST + sbT, 0], [POST + sbT, SIZE - POST - sbT, sbH], timber("sandbox board", "28x130"));
  copyTo(sbFront, [0, SIZE - POST - sbT, 0]);
  copyTo(sbSide, [SIZE - POST - sbT, POST + sbT, 0]);
  cad.copy3d(byMember("sandbox board"), { move: [0, 0, sbH], count: SANDBOX.rows - 1 });
  const sand = box([POST + sbT, POST + sbT, 0], [SIZE - POST - sbT, SIZE - POST - sbT, SANDBOX.sand],
    bought("sand", "play sand", "sand", { unit: "m3" }));

  // ── Slide (bought): a trough swept along a wave ────────────────────
  ASM = "slide";
  // The path, in (s, h): s out from the slide's top end (along -y), h up.
  // Lines and tangent arcs, so the sweep's path has no corner. One piece:
  // as two sections the joint between two swept solids costs the kernel
  // seconds per query (see docs/3d-examples.md, example 11).
  function slidePath(chute2) {
    const segs = [];
    let s = 0, h = 0, phi = 0, len = 0;
    const lineTo = (L) => { s += L * cos(phi); h += L * sin(phi); len += L; segs.push({ to: [s, h] }); };
    const arcTo = (R, toDeg) => {
      const t = rad(toDeg), turn = t > phi ? 1 : -1;     // +1 turns up
      const cx = s - turn * R * sin(phi), cz = h + turn * R * cos(phi);
      const at = (a) => [cx + turn * R * sin(a), cz - turn * R * cos(a)];
      const via = at((phi + t) / 2);
      [s, h] = at(t);
      segs.push({ via, to: [s, h] });
      len += R * Math.abs(t - phi);
      phi = t;
    };
    const [R1, R2, R3, R4] = SLIDE.radii;
    lineTo(SLIDE.top);
    arcTo(R1, -SLIDE.steep);
    lineTo(SLIDE.chute);
    arcTo(R2, -SLIDE.crest);             // the wave's crest
    arcTo(R3, -SLIDE.steep);
    if (chute2 > 0) lineTo(chute2);
    arcTo(R4, 0);
    lineTo(SLIDE.runout);
    return { segs, s, h, len };
  }
  // The second straight run makes the drop: from the deck to the exit.
  const drop = DECK - SLIDE.exit;
  const trial = slidePath(0);
  const chute2 = (drop + trial.h) / sin(rad(SLIDE.steep));
  const path = slidePath(chute2);
  const ys = SLIDE.onDeck;                                   // the slide's top end (y)
  const world = ([s, h]) => [slideX, ys - s, DECK + h];
  const hw = SLIDE.inner / 2, sw = SLIDE.wall, sH = SLIDE.side, lip = SLIDE.lip;
  // The trough's section: its underside on the path, lips out at the top.
  const trough = [
    [-hw - sw, 0], [hw + sw, 0], [hw + sw, sH - sw], [hw + sw + lip, sH - sw], [hw + sw + lip, sH],
    [hw, sH], [hw, sw], [-hw, sw], [-hw, sH], [-hw - sw - lip, sH], [-hw - sw - lip, sH - sw], [-hw - sw, sH - sw],
  ];
  const slide = cad.sweep(trough, path.segs.map((sg) => (sg.via ? { via: world(sg.via), to: world(sg.to) } : { to: world(sg.to) })), {
    plane: { origin: [slideX, ys, DECK], x: [1, 0, 0], normal: [0, -1, 0] },
    ...bought("slide", `wave slide ${(path.len / 1000).toFixed(1)} m`, "yellow plastic"),
  });
  const yExit = ys - path.s;                                 // the slide's lower end (y)
  const [fd, fb] = SLIDE.foot;
  const slideFoot = box([slideX - hw - sw, yExit + fb, 0], [slideX + hw + sw, yExit + fb + fd, SLIDE.exit],
    bought("slide foot", "slide foot (with the slide)", "yellow plastic"));

  // ── Swings (bought): two seats on ropes from hangers under the beam ─
  // Between the cabin's right wall and the A-frame legs, the seats SWING.between apart.
  const [sw0, sd0, stk] = SWING.seat, [hx, hy, hz] = SWING.hanger;
  const inner = [SIZE + CLAD.t, A_X - LEG / 2];                     // free span along the beam
  const side = (inner[1] - inner[0] - 2 * sw0 - SWING.between) / 2; // seat to the frame, each end
  const seatX = [inner[0] + side, inner[0] + side + sw0 + SWING.between];
  const seatTop = SWING.seatZ + stk, hangerZ = SWING_H - hz;
  const swings = [];
  seatX.forEach((x0, i) => {
    ASM = `swing ${i + 1}`;
    const ropeX = [x0 + SWING.inset, x0 + sw0 - SWING.inset];
    let seat, ropes = [], hangers = [];
    if (i === 0) {
      seat = cad.extrude(sh.roundedRect(x0, mid - sd0 / 2, sw0, sd0, 25), {
        at: [0, 0, SWING.seatZ], distance: stk, ...bought("seat", `swing seat ${sw0} x ${sd0}`, "rubber"),
      });
      for (const x of ropeX) {
        const hg = hangers.length ? copyTo(hangers[0], [x - hx / 2, mid - hy / 2, hangerZ])
          : box([x - hx / 2, mid - hy / 2, hangerZ], [x + hx / 2, mid + hy / 2, SWING_H], bought("hanger", "swing hanger", "galvanised steel"));
        hangers.push(hg);
        const rope = ropes.length ? copyTo(ropes[0], [x - SWING.rope / 2, mid - SWING.rope / 2, seatTop])
          : cad.extrude({ center: [x, mid], radius: SWING.rope / 2 }, {
            at: [0, 0, seatTop], distance: hangerZ - seatTop, ...bought("rope", `swing rope Ø${SWING.rope}`, "rope", { unit: "m" }),
          });
        ropes.push(rope);
      }
    } else {
      // The second swing: the first one's parts, moved along the beam.
      const dx = x0 - seatX[0];
      const first = swings[0];
      const move = (p) => cad.copy3d(p, { move: [dx, 0, 0], attrs: { assembly: ASM } });
      seat = move(first.seat);
      ropes = first.ropes.map(move);
      hangers = first.hangers.map(move);
    }
    swings.push({
      n: i + 1, seat, ropes, hangers, x: [x0, x0 + sw0],
      pivot: [mid, hangerZ],                       // (y, z) of the rope tops
      ids: [seat, ...ropes, ...hangers].map((p) => p.id),
    });
  });

  // ── What the scripts need ──────────────────────────────────────────
  return {
    params: { SIZE, POST, POST_H, DECK, BOARD_T, BOARD_W, N_BOARD, JOIST, N_JOIST, BEAM, SWING_H, A_X, BEAM_END,
      LEG, LEG_SPREAD, LEG_TOP, TIE, STAIR, ROOF, CLAD, WINDOW, STUD, RAIL, RISER, HANDRAIL, SANDBOX, SLIDE, SWING },
    parts: { post, board, edgeBoard, beam, leg, stringer, tread, riserBoard, plate, rafter, ridge, roofBoard, cap,
      backStud, rightStud, doorStud, railPost, slat, lowPost, highPost, handrail, sbFront, sbSide, sand, slide, slideFoot },
    swings,
    // Sizes and lines the checks recompute from.
    frame: { mid, gap, zj, rise, going: g, nose, nTread, m, theta, xc, zc, line, xFoot, halfBand, legTilt },
    roof: { th, tn, cs, sn, pw, ph, plateY, zp, X0, X1, ra, rb, rgT, rgD, yIn, yEave, yHead, under, depthV, top,
      ridgeTop, S, bWid, BX0, BX1, yE2, zE2, s0, sCut, capW, roofTop },
    cabin: { WALL_Z0, boardZ, door },
    railing: { opening, railTop, slatGap, span },
    stairs: { nosing, hrTop, xLow, xHigh, postY, hrY },
    slide: { x: slideX, half: slideHalf, top: ys, exit: yExit, path, trough, chute2 },
    swing: { inner, side, seatX, seatTop, hangerZ },
    fixings: FIXINGS,
    steps: STEPS,
    stock: STOCK,
  };
}

return { build, FIXINGS, STEPS, STOCK };
