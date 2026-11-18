// 11 - Play tower with slide and swings.
//
// The flagship of docs/3d-examples.md: a garden play set of our own design,
// built by lib/playtower.js (so the part list script builds the same model)
// and checked here by rules written in script. Four 90 x 90 posts carry a
// 1500 x 1500 platform 1450 above the ground; on it a cabin with board walls
// on three sides (windows at the back and on the right, a door to the
// stairs) under a gabled timber roof, and an open front with a railing and
// the opening for a wave slide. Stairs with closed risers and handrails
// climb the left side, a sandbox sits under the platform, and a swing beam
// runs out to an A-frame with two swings. The slide, the swings and the sand
// are bought, and say so (`bought: true`).
//
// The rules (lib/rules.js, over overlapping3d and pairsWithin3d):
//   - no two parts overlap, and every part rests on what carries it;
//   - every post, leg and stair post stands on the ground;
//   - free space: nothing but the swing itself within reach of each seat
//     along its arc, and a clear run-out zone beyond the slide's exit;
//   - no gap that can trap a head among the parts a child reaches, and none
//     that can trap a finger where hands go;
//   - every platform edge above the fall height has a guard, except at the
//     stair and slide openings.
// The limits are below. They are illustrative, chosen to be plausible for a
// garden play set; they are not taken from a safety standard, and passing
// them says nothing about one.
//
// x = left to right (the swing beam runs out along +x), y = front to back
// (front at y = 0), z = up.

const tower = await cad.use("playtower");
const rules = await cad.use("rules");
const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const mat = await cad.use("materials");
const run = ck.start("11-play-tower");
const { PI, cos, sin, tan, atan2, hypot } = Math;

// ── Limits (mm; illustrative, not a safety standard) ─────────────────
const FALL = 1000;                         // a floor higher than this needs a guard
const GUARD = 600;                         // ... at least this high above the floor
const HEAD = { name: "head", min: 89, max: 230 };     // a gap in this range can trap a head
const FINGER = { name: "finger", min: 8, max: 25 };   // ... a finger, where hands go
const FREE = {
  // Each swing: past the seat along its arc, front and back; beside the
  // seat; above its top (the child); below its underside.
  swing: { along: 1800, side: 250, above: 1100, below: 100 },
  // The slide: a run-out zone beyond its exit.
  slide: { ahead: 2000, side: 500, height: 1800 },
};

// ── The model ────────────────────────────────────────────────────────
const m = tower.build();
const t0 = Date.now();
const P = m.params, F = m.frame, R = m.roof;
const { SIZE, POST, POST_H, DECK, BOARD_T, BOARD_W, BEAM, SWING_H, BEAM_END, LEG, LEG_SPREAD, LEG_TOP, STAIR, ROOF } = P;
const parts = cad.parts();
const ofMember = (...names) => parts.filter((p) => names.includes(p.attrs.member));

// ── Checks: counts, volumes, the overall box ─────────────────────────
const want = {
  post: 4, rim: 4, joist: 3, "deck board": 11, "head rail": 1, "swing beam": 1, leg: 2, tie: 1,
  stringer: 2, tread: 7, riser: 6, "stair post": 4, handrail: 2,
  "wall plate": 2, rafter: 8, ridge: 1, "roof board": 14, "ridge cap": 2,
  "wall board": 34, stud: 6, "rail post": 1, "top rail": 1, baluster: 4,
  "sandbox board": 8, sand: 1, slide: 1, "slide foot": 1, hanger: 4, rope: 4, seat: 2,
};
const total = Object.values(want).reduce((a, b) => a + b, 0);
run.equal(`${total} parts`, parts.length, total);
for (const [k, n] of Object.entries(want)) run.equal(`${n} x ${k}`, ofMember(k).length, n);
const defs = new Set(parts.map((p) => p.def)).size;
run.equal("42 definitions: repeated members share one", defs, 42);
run.ok("every part has a member, an assembly and a material",
  parts.every((p) => p.attrs.member && p.attrs.assembly && p.attrs.material));
run.ok("every part is sawn timber with a section or bought",
  parts.every((p) => (p.attrs.material.includes("timber") && /^\d+x\d+$/.test(p.attrs.section)) || p.attrs.bought === true));

const vol = (p) => cad.part(p.id).volume;
const Q = m.parts;
// The frame, as in the frame's first version.
run.near("post volume", vol(Q.post), POST * POST * POST_H);
run.near("deck board volume", vol(Q.board), BOARD_W * BOARD_T * SIZE);
run.near("notched deck board volume", vol(Q.edgeBoard), BOARD_W * BOARD_T * SIZE - 2 * POST * POST * BOARD_T);
run.near("swing beam volume", vol(Q.beam), BEAM[0] * BEAM[1] * (BEAM_END - SIZE + POST));
// A prism cut by two planes that each cross it fully: its section times
// the length of its axis between them.
run.near("leg volume", vol(Q.leg), LEG * LEG * hypot(LEG_SPREAD - LEG_TOP, SWING_H));
// The stringer: the same between the ground and the plumb cut, less the
// small triangle the level cut at deck height takes off its top corner.
const [sd, st] = STAIR.stringer;
const over = F.line(0) + F.halfBand - DECK;
run.ok("stringer reaches above the deck before its level cut", over > 0);
run.near("stringer volume", vol(Q.stringer), sd * st * hypot(F.xFoot, F.line(0)) - 0.5 * over * (over / F.m) * st);
run.near("tread volume", vol(Q.tread), (F.going + F.nose) * STAIR.width * BOARD_T);
run.ok("treads lie within the stringers' depth", Math.abs((F.going + F.nose) / 2 * F.m) + BOARD_T / 2 < F.halfBand);
// The roof: plates and ridge are boxes; a rafter is its section along its
// axis between the two plumb cuts, less the birdsmouth; a roof board is a
// box; a ridge cap is its length times the area between its lower edge and
// the plumb cut, integrated over its thickness.
const RC = ROOF.cap, RB = ROOF.board;
run.near("wall plate volume", vol(Q.plate), R.pw * R.ph * (R.X1 - R.X0));
run.near("rafter volume", vol(Q.rafter), R.ra * R.rb * (R.yHead - R.yEave) / R.cs - 0.5 * R.pw * (R.pw * R.tn) * R.rb);
run.near("ridge volume", vol(Q.ridge), R.rgT * R.rgD * (R.X1 - R.X0 - 20));
run.near("roof board volume", vol(Q.roofBoard), RB.t * R.bWid * (R.BX1 - R.BX0));
run.near("ridge cap volume", vol(Q.cap),
  (R.BX1 - R.BX0) * (RC.t * (F.mid - R.yE2) / R.cs - RC.t * R.s0 + R.tn * RC.t * RC.t / 2));
// The new members: boxes, the handrail like a leg, the slide its section
// times its path (the path starts and ends level, so the bends add and take
// off the same), a seat a rounded rectangle, a rope a cylinder.
const { CLAD, STUD, RISER, HANDRAIL, SLIDE, SWING } = P;
run.near("wall board volume", vol(ofMember("wall board").find((p) => p.localBbox.max[0] - p.localBbox.min[0] === SIZE)),
  CLAD.t * CLAD.w * SIZE);
run.near("stud volume", vol(Q.backStud), STUD[0] * STUD[1] * (POST_H - DECK));
run.near("riser volume", vol(Q.riserBoard), RISER.t * RISER.h * STAIR.width);
run.near("baluster volume", vol(Q.slat), CLAD.t * CLAD.w * (m.railing.railTop - m.cabin.WALL_Z0));
const hrRun = -CLAD.t - m.stairs.xLow;
run.near("handrail volume", vol(Q.handrail), HANDRAIL.section[0] * HANDRAIL.section[1] * hrRun / cos(F.theta * PI / 180));
run.near("slide volume", vol(Q.slide), sh.loopArea(m.slide.trough) * m.slide.path.len, 1e-7);
run.near("seat volume", vol(m.swings[0].seat), sh.roundedRectArea(SWING.seat[0], SWING.seat[1], 25) * SWING.seat[2]);
run.near("rope volume", vol(m.swings[0].ropes[0]), PI * (SWING.rope / 2) ** 2 * (m.swing.hangerZ - m.swing.seatTop));
run.near("the slide drops from the deck to its exit", m.slide.path.h, -(DECK - SLIDE.exit), 0, 1e-9);
run.near("slide exit height", ck.boxOf([cad.part(Q.slideFoot.id)]).max[2], SLIDE.exit, 0, 1e-9);

// Overall box: stringer foot to beam end, slide exit to the back leg, the
// caps on top.
const legHalf = LEG / 2 / cos(atan2(LEG_SPREAD - LEG_TOP, SWING_H));
run.box("overall size", ck.boxOf(parts), {
  min: [F.xFoot - F.halfBand / F.m, m.slide.exit, 0],
  max: [BEAM_END, F.mid + LEG_SPREAD + legHalf, R.roofTop],
}, 1e-6);
run.near("deck height", ck.boxOf(ofMember("deck board")).max[2], DECK);

// ── Rules ────────────────────────────────────────────────────────────
const timing = {};
const time = (name, f) => { const t = Date.now(); const r = f(); timing[name] = Date.now() - t; return r; };
const fail = (what, r) => run.ok(what, r.failures.length === 0, r.failures.join("; "));

// No two parts overlap.
fail("no two parts overlap", time("overlap", () => rules.noOverlap()));

// Every part rests on what carries it: one contact query, a table of who
// carries whom. "ground" is z = 0.
const contacts = time("contacts", () => rules.contacts());
run.ok("every contact was measured", contacts.skipped.length === 0, JSON.stringify(contacts.skipped));
const carried = rules.restsOn({
  c: contacts,
  needs: {
    post: { on: ["ground"] },
    rim: { on: ["post"], min: 2 },
    joist: { on: ["rim"], min: 2 },
    "deck board": { on: ["joist", "rim"], min: 2 },
    "head rail": { on: ["post"], min: 2 },
    "swing beam": { all: ["head rail"], on: ["leg"], min: 2 },
    leg: { all: ["ground", "swing beam"] },
    tie: { on: ["leg"], min: 2 },
    stringer: { all: ["ground", "rim"] },
    tread: { on: ["stringer"], min: 2 },
    riser: { all: ["tread"], on: ["stringer"], min: 2 },
    "stair post": { all: ["ground", "stringer"] },
    handrail: { on: ["stair post"], min: 2 },
    "wall plate": { on: ["post"], min: 2 },
    rafter: { all: ["wall plate", "ridge"] },
    ridge: { on: ["rafter"], min: 2 },
    "roof board": { on: ["rafter"], min: 2 },
    "ridge cap": { on: ["roof board"] },
    "wall board": { on: ["post", "stud", "rim"], min: 2 },
    stud: { all: ["deck board"] },
    "rail post": { on: ["rim"] },
    "top rail": { all: ["post", "rail post"] },
    baluster: { all: ["top rail"], on: ["rim", "post"] },
    "sandbox board": { on: ["ground", "sandbox board", "post"], min: 2 },
    sand: { all: ["ground"] },
    slide: { all: ["deck board", "slide foot"] },
    "slide foot": { all: ["ground"] },
    hanger: { all: ["swing beam"] },
    rope: { all: ["hanger", "seat"] },
    seat: { on: ["rope"], min: 2 },
  },
});
fail("every part rests on what carries it", carried);

// Every post's foot is on the ground.
const standing = parts.filter((p) => ["post", "leg", "stair post"].includes(p.attrs.member)).map((p) => p.id);
fail("every post, leg and stair post stands on the ground", rules.onGround({ ids: standing }));

// The swing beam as the frame's first version checked it: on the head rail
// and both legs, clear of the roof.
const beamOn = contacts.touching.get(Q.beam.id).map((q) => q.attrs.member).filter((k) => k !== "wall board" && k !== "hanger");
run.equal("the swing beam rests on the head rail and both legs", beamOn.sort().join(", "), "head rail, leg, leg");
const roofIds = parts.filter((p) => p.attrs.assembly === "roof").map((p) => p.id);
const headroom = Math.min(...cad.pairsWithin3d(Q.beam, roofIds, 1000).pairs.map((p) => p.distance));
run.near("the swing beam's clearance to the roof", headroom,
  Math.min((R.under(F.mid - BEAM[0] / 2) - (SWING_H + BEAM[1])) * R.cs, R.ridgeTop - R.rgD - (SWING_H + BEAM[1])));

// Free space: a zone round each swing's arc, and one beyond the slide.
// A swing's zone is a ring sector about the rope tops, across the seat's
// width plus FREE.swing.side each side; nothing may be in it but that
// swing's own parts.
const zones = m.swings.map((s) => {
  const [py, pz] = s.pivot;
  const [sw0, sd0, stk] = SWING.seat;
  const rSeat = pz - (SWING.seatZ + stk / 2);
  const a = (sd0 / 2 + FREE.swing.along) / rSeat;            // half the sector's angle
  const rIn = pz - (SWING.seatZ + stk) - FREE.swing.above;
  const rOut = hypot(sd0 / 2, pz - SWING.seatZ) + FREE.swing.below;
  const p = (r, t) => [py + r * sin(t), pz - r * cos(t)];
  const b = tan(a / 2);                                        // bulge of an arc of 2a
  return {
    name: `swing ${s.n}`,
    shape: {
      extrude: [[...p(rOut, -a), b], [...p(rOut, a)], [...p(rIn, a), -b], [...p(rIn, -a)]],
      plane: "yz", at: [s.x[0] - FREE.swing.side, 0, 0], distance: SWING.seat[0] + 2 * FREE.swing.side,
    },
    allow: s.ids,
    angle: Math.round((a * 1800) / PI) / 10,
  };
});
const sl = m.slide;
zones.push({
  name: "the slide's run-out",
  shape: {
    extrude: { min: [sl.x - sl.half - FREE.slide.side, sl.exit - FREE.slide.ahead], max: [sl.x + sl.half + FREE.slide.side, sl.exit] },
    at: [0, 0, 0], distance: FREE.slide.height,
  },
  allow: [Q.slide.id],
});
const free = time("free space", () => rules.freeSpace(zones));
fail("nothing but the swing itself in each swing's free space, nothing in the slide's run-out", free);

// Gaps. Head traps are looked for where a child climbs and leans out: the
// railing and the posts and plate round it, the deck, the stairs with their
// handrails, and the slide's top. The walls are closed boards (their gaps
// are below the finger range) and their window and door openings are wider
// than the head range. Finger traps are looked for where hands go: rails,
// balusters, posts beside them, the slide and the swings, against
// everything in reach (all but the roof, the sand and the swing frame
// overhead).
const ids = (...members) => ofMember(...members).map((p) => p.id);
const climb = ids("baluster", "top rail", "rail post", "post", "wall plate", "deck board", "stringer", "tread", "riser",
  "stair post", "handrail", "slide");
const hands = ids("top rail", "baluster", "rail post", "handrail", "stair post", "slide", "rope", "seat");
const reach = parts.filter((p) => p.attrs.assembly !== "roof" && !["sand", "swing beam", "leg", "tie"].includes(p.attrs.member)
  || p.attrs.member === "wall plate").map((p) => p.id);
const heads = time("head gaps", () => rules.gaps(climb, null, [HEAD]));
fail("no gap that can trap a head", heads);
const fingers = time("finger gaps", () => rules.gaps(hands, reach, [FINGER]));
fail("no gap that can trap a finger where hands go", fingers);

// Guards: the deck's four edges, open only at the stairs and the slide. A
// guard may have gaps narrower than the head range (balusters); the gap
// rule above judges those.
const deckBox = ck.boxOf(ofMember("deck board"));
const [x0, y0] = deckBox.min, [x1, y1] = deckBox.max, z = deckBox.max[2];
run.ok("the platform is high enough to need a guard", z > FALL);
const edges = [
  { name: "front", from: [x0, y0], to: [x1, y0], z },
  { name: "right", from: [x1, y0], to: [x1, y1], z },
  { name: "back", from: [x1, y1], to: [x0, y1], z },
  { name: "left", from: [x0, y1], to: [x0, y0], z },
];
const door = m.cabin.door;
const openings = [
  { edge: "front", from: m.railing.opening[0] - x0, to: m.railing.opening[1] - x0, why: "slide" },
  { edge: "left", from: y1 - door[1], to: y1 - door[0], why: "stairs" },
];
const guard = time("guards", () => rules.guards({ edges, height: GUARD, openings, join: HEAD.min }));
fail(`every platform edge has a guard ${GUARD} high, but at the stairs and the slide`, guard);

run.ok("no temporary part left behind", cad.parts({ has: [rules.PROBE] }).length === 0);
const rulesMs = Date.now() - t0;

// ── Renders ──────────────────────────────────────────────────────────
// Working renders coloured by member; the presentation render on grass in
// daylight, with the model's materials (stained timber, a green roof, a
// yellow plastic slide, rubber seats, rope, galvanised hangers, sand).
const outdoor = {
  materials: {
    "green stained timber": mat.wood("#4d7448", "#33502f", { spacing: 6, strength: 0.45, roughness: 0.7, coat: 0 }),
    "yellow plastic": mat.plastic("#e3a91c", { roughness: 0.3 }),
    "galvanised steel": mat.brushed("#b9bdc1", 0.4, { anisotropy: 0.3 }),
    rope: { color: "#c8b28a", roughness: 0.95 },
    sand: { color: "#dcc89a", roughness: 1 },
  },
  environment: "sky",
  ground: { color: "#7a9a5a" },
};
await run.render([
  { as: "iso", view: "iso-sw", ground: true, colorBy: { attr: "member" }, present: outdoor },
  { as: "exploded", view: "iso-sw", explode: 0.35, colorBy: { attr: "member" } },
  { as: "front", view: "front", colorBy: { attr: "member" } },
  { as: "back", view: "iso-ne", ground: true, colorBy: { attr: "member" } },
  // The picture on the box: from a standing adult's eye height (1.6 m), a
  // few metres off to the south-west, on a lawn.
  eyeView(),
]);

function eyeView() {
  const b = ck.boxOf(cad.parts());
  const c = [(b.min[0] + b.max[0]) / 2, (b.min[1] + b.max[1]) / 2];
  const away = 0.78 * Math.hypot(b.max[0] - b.min[0], b.max[1] - b.min[1]);
  const camera = { eye: [c[0] - 0.62 * away, c[1] - 0.78 * away, 1600], target: [c[0], c[1], 1350], fov: 55 };
  return {
    as: "eye", camera, ground: true, colorBy: { attr: "member" },
    present: { ...outdoor, ground: { material: mat.presets.lawn }, exposure: 0.3 },
  };
}

return run.summary({
  stairs: { rise: F.rise, going: F.going, pitch_deg: Math.round(F.theta * 10) / 10 },
  roof: { pitch_deg: ROOF.pitch, headroom: Math.round(headroom * 10) / 10 },
  slide: { length: Math.round(sl.path.len), run: Math.round(sl.path.s), exit: SLIDE.exit },
  rules: {
    contacts: contacts.pairs.length,
    zones: free.zones.map((zn, i) => ({ name: zn.name, volume_m3: Math.round(zn.volume / 1e6) / 1e3, angle_deg: zones[i].angle })),
    headGapsClosed: heads.closed.length,
    fingerGapsClosed: fingers.closed.length,
    guards: guard.edges.map((e) => ({ edge: e.edge, guarded: e.guarded })),
    ms: { ...timing, all: rulesMs },
  },
});
