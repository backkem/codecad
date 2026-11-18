// 07 - Pipe spool, the 3D side of 11-pid.
//
// A DN50 line, Ø60.3 x 3.9: a weld-neck flange (the part of example 01,
// from lib/flange.js), 500 along x, a 90° long-radius elbow up (R = 1.5 D
// = 76.2), 300 up, a second elbow turning along y, 400 along y, and a
// second flange. A third flange, bolted face to face to the second with a
// short stub of pipe, starts the next spool. Two saddle supports carry it
// from the ground.
//
// The pipe is one sweep of a ring (the hollow section) along lines and
// arcs; the elbows are exact torus segments. The three flanges are one
// definition placed three times.
//
// Not yet: gaskets, bolts and welds; a check that the supports touch the
// pipe and nothing else (distance between parts is a later rung).
//
// z = up; the ground is 250 below the first run's axis.

const fl = await cad.use("flange");
const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("07-pipe-spool");
const { PI } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const OD = 60.3, WALL = 3.9;
const R = 1.5 * 50.8;              // long-radius elbow: 1.5 x nominal size (2 in)
const RUN = [500, 300, 400];       // straight lengths: along x, up, along y
const STUB = 150;                  // pipe on the next spool's flange
const GROUND = -250;               // ground level (z)
const SUPPORT = { t: 20, w: 120, lip: 10 };   // saddle thickness, width, how far up the pipe it reaches
const S1_X = 300, S2_Y = 250;      // where the supports stand

const ro = OD / 2, ri = ro - WALL;
const ring = (c = [0, 0]) => ({ outer: { center: c, radius: ro }, holes: [{ center: c, radius: ri }] });

// ── Flanges: one definition, three places ────────────────────────────
const f = fl.weldNeck();
const H = f.dims.height;           // raised face to weld end
const x0 = H;                      // the pipe starts at the first flange's weld end

// ── Pipe: a ring swept along lines and two quarter arcs ──────────────
const s45 = Math.SQRT1_2;
const p1 = [x0 + RUN[0], 0, 0];                             // start of elbow 1
const e1 = [p1[0] + R, 0, R];                               // end of elbow 1 (going up)
const p2 = [e1[0], 0, R + RUN[1]];                          // start of elbow 2
const e2 = [p2[0], R, p2[2] + R];                           // end of elbow 2 (going along y)
const end = [e2[0], R + RUN[2], e2[2]];
const pipe = cad.sweep(ring(), [
  { to: p1 },
  { via: [p1[0] + R * s45, 0, R - R * s45], to: e1 },
  { to: p2 },
  { via: [p2[0], R - R * s45, p2[2] + R * s45], to: e2 },
  { to: end },
], { plane: { origin: [x0, 0, 0], x: [0, 1, 0], normal: [1, 0, 0] }, name: "pipe", attrs: { part: "pipe", size: "DN50" } });

// The flanges: the second's hub on the pipe's end, raised face out along
// +y; the third face to face with it; the first turned to face -x.
const face = [end[0], end[1] + H, end[2]];
const f2 = cad.copy3d(f.part, { rotate: { angle: 90, axis: [1, 0, 0] }, move: face });
const f3 = cad.copy3d(f.part, { rotate: { angle: -90, axis: [1, 0, 0] }, move: face });
cad.rotate3d(f.part, { angle: 90, axis: [0, 1, 0] });
const stub = cad.extrude(ring(), {
  plane: { origin: [face[0], face[1] + H, face[2]], x: [1, 0, 0], normal: [0, 1, 0] },
  distance: STUB, name: "stub", attrs: { part: "pipe", size: "DN50" },
});

// ── Supports: a plate from the ground with a saddle cut to the pipe ──
function support(min, max, cradle) {
  const s = sh.box(min, max, { name: "support", attrs: { part: "support" } });
  cad.cut(s, cradle);
  return s;
}
const t = SUPPORT.t, w = SUPPORT.w, lip = SUPPORT.lip;
const s1 = support([S1_X - t / 2, -w / 2, GROUND], [S1_X + t / 2, w / 2, -lip],
  { extrude: { center: [0, 0], radius: ro }, plane: "yz", at: [S1_X - t, 0, 0], distance: 2 * t });
const s2 = support([end[0] - w / 2, S2_Y - t / 2, GROUND], [end[0] + w / 2, S2_Y + t / 2, end[2] - lip],
  { extrude: { center: [0, 0], radius: ro }, plane: { origin: [end[0], S2_Y - t, end[2]], x: [1, 0, 0], normal: [0, 1, 0] }, distance: 2 * t });

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("seven parts", parts.length, 7);
run.equal("three flanges, one definition",
  new Set(parts.filter((p) => p.attrs.part === "flange").map((p) => p.def)).size, 1);
const ringArea = PI * (ro * ro - ri * ri);
const path = RUN[0] + RUN[1] + RUN[2] + 2 * (PI / 2) * R;
run.near("pipe volume (ring times centre line)", cad.part(pipe.id).volume, ringArea * path);
run.near("stub volume", cad.part(stub.id).volume, ringArea * STUB);
for (const p of [f.part, f2, f3]) run.near(`flange ${p.id} volume`, cad.part(p.id).volume, f.volume);
const saddle = sh.segmentArea(ro, lip) * t;
run.near("support 1 volume", cad.part(s1.id).volume, t * w * (-lip - GROUND) - saddle);
run.near("support 2 volume", cad.part(s2.id).volume, t * w * (end[2] - lip - GROUND) - saddle);
// The flanges sit where the pipe ends: bore axis through the pipe's end.
const c2 = cad.part(f2.id).centroid, c3 = cad.part(f3.id).centroid;
run.near("second flange on the pipe's axis", Math.hypot(c2[0] - end[0], c2[2] - end[2]), 0, 0, 1e-9);
run.near("mating flanges coaxial", Math.hypot(c3[0] - c2[0], c3[2] - c2[2]), 0, 0, 1e-9);
const D = f.dims.D;
run.box("overall size", ck.boxOf(parts), {
  min: [0, -D / 2, GROUND],
  max: [end[0] + D / 2, face[1] + H + STUB, end[2] + D / 2],
});

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "top", view: "top" },
]);

return run.summary({ centreLine: Math.round(path * 1000) / 1000 });
