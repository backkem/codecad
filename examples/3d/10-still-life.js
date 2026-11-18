// 10 - Still life.
//
// A wine glass, a glazed bowl, a steel flask with its cap and a wooden
// pepper mill with a brass finial, on an oak table. Every object is one
// revolve of exact lines and arcs about its own axis, placed on the
// table top; the table is a slab with a rounded front edge on four legs.
//
// The kernel refuses an arc whose circle crosses the revolve axis (it
// would make a spindle torus), even where the arc itself stays clear of
// it. That shaped every profile here:
//
// - The glass bowl has a flat floor: its rounded bottom is a quarter of a
//   circle centred 22 mm off the axis (radius 20), not one large arc down
//   to the stem, which would cross the axis. Above it the wall is a
//   straight 6 degree taper to the rim, tangent to the arc.
// - The bowl's wall is one arc of radius 38 centred 40 off the axis; its
//   inside floor is flat.
// - The flask's shoulder is a quarter circle (radius 18, centred 20 off the
//   axis); a gentle dome would cross the axis.
// - The mill has no convex swell: a long, gentle convex curve would need
//   a circle much larger than the mill's radius. Its waist is a long cove
//   (concave, so its circle lies outside) between small half-round beads.
//
// The script checks that no arc of any profile crosses the axis, so a
// change that breaks the rule fails here with the profile's name, not in
// the kernel.
//
// z = up, millimetres. The table top is at z = 750.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("10-still-life");
const { PI, sin, cos, tan, atan, atan2, hypot } = Math;
const rad = (d) => (d * PI) / 180;

// ── Parameters (mm) ──────────────────────────────────────────────────
const TABLE = { w: 900, d: 560, t: 28, h: 750, leg: 45, inset: 40, r: 6 };
const GLASS = {
  foot: 36, footT: 2, footTop: 6,      // foot radius, edge thickness, rise to the stem
  stem: 3.5, fillet: 6.5,              // stem radius, fillets into the foot and the bowl
  bowlZ: 100,                          // underside of the bowl
  flat: 22, round: 20,                 // flat floor radius, radius of the rounded bottom
  taper: 6, depth: 85,                 // wall taper (degrees), bowl underside to rim
  t: 1.4,                              // wall thickness
};
const BOWL = {
  foot: 37, footIn: 30, recess: 3,     // foot ring outside and inside radius, recess height
  centre: [40, 42], r: 38,             // the wall's circle
  rim: 12,                             // where the wall ends, degrees above the circle's middle
  t: 5,                                // wall thickness
};
const FLASK = { r: 38, base: 6, shoulderZ: 200, shoulder: 18, neck: 14, neckZ: 232 };
const CAP = { r: 19, bottom: 222, top: 262, bore: 14.2, round: 4 };
const MILL = { base: 32, waist: 21, top: 202, finial: 6, finialH: 8 };

// Where each object stands on the table top (x, y of its axis).
// The glass stands in front of the flask, so the steel is seen through it.
const AT = {
  glass: [95, -130],
  bowl: [-80, -20],
  flask: [190, 60],
  mill: [-240, 90],
};
const Z0 = TABLE.h;

// ── Profile helpers ──────────────────────────────────────────────────

// The bulge (tan of a quarter of the sweep) of the arc from p0 to p1
// about centre c, counter-clockwise or not.
function about(p0, p1, c, ccw) {
  let sweep = atan2(p1[1] - c[1], p1[0] - c[0]) - atan2(p0[1] - c[1], p0[0] - c[0]);
  if (ccw) while (sweep <= 0) sweep += 2 * PI;
  else while (sweep >= 0) sweep -= 2 * PI;
  return tan(sweep / 4);
}

// Every arc of a profile ([u, v, bulge?] vertices, u = radius) must keep
// its whole circle off the axis: the kernel refuses a spindle torus.
function arcsClearAxis(name, profile) {
  for (let i = 0; i < profile.length; i++) {
    const [u0, v0, b = 0] = profile[i];
    if (b === 0) continue;
    const [u1, v1] = profile[(i + 1) % profile.length];
    const th = 4 * atan(Math.abs(b));
    const c = hypot(u1 - u0, v1 - v0);
    const r = c / (2 * sin(th / 2));
    const h = (c / 2) / tan(th / 2);              // centre to chord, signed by the sweep
    const s = Math.sign(b);                       // left of the chord for counter-clockwise
    const cu = (u0 + u1) / 2 - s * ((v1 - v0) / c) * h;
    run.ok(`${name}: arc ${i} keeps its circle off the axis`, cu - r >= -1e-9,
      `centre ${cu.toFixed(3)}, radius ${r.toFixed(3)}`);
  }
}

// One revolve about the z axis, stood at (x, y) on the table top.
function turned(name, profile, [x, y], material, extra = {}) {
  arcsClearAxis(name, profile);
  const p = cad.revolve(profile, {
    plane: "xz", axis: [[0, 0], [0, 1]], name, attrs: { part: name, material, ...extra },
  });
  cad.move3d(p, [x, y, Z0]);
  return { part: p, volume: sh.revolvedVolume(profile), profile };
}

// ── Table: a slab with a rounded front edge on four legs ─────────────
const top = sh.box([-TABLE.w / 2, -TABLE.d / 2, Z0 - TABLE.t], [TABLE.w / 2, TABLE.d / 2, Z0], {
  name: "table top", attrs: { part: "table top", material: "oak" },
});
cad.fillet3d(top, { edges: { near: [[0, -TABLE.d / 2, Z0]] }, radius: TABLE.r });
const legs = [];
for (const sx of [-1, 1]) {
  for (const sy of [-1, 1]) {
    const x = sx * (TABLE.w / 2 - TABLE.inset), y = sy * (TABLE.d / 2 - TABLE.inset);
    legs.push(sh.box([x - TABLE.leg / 2, y - TABLE.leg / 2, 0], [x + TABLE.leg / 2, y + TABLE.leg / 2, Z0 - TABLE.t], {
      name: "table leg", attrs: { part: "table leg", material: "oak" },
    }));
  }
}

// ── Wine glass ───────────────────────────────────────────────────────
const G = GLASS;
const ga = rad(G.taper);
const gc = [G.flat, G.bowlZ + G.round];                        // centre of the rounded bottom
const gOut = [gc[0] + G.round * cos(ga), gc[1] + G.round * sin(ga)];
const rimZ = G.bowlZ + G.depth;
const rimR = gOut[0] - (rimZ - gOut[1]) * tan(ga);             // the taper, tangent to the arc
const gi = G.round - G.t;
const gIn = [gc[0] + gi * cos(ga), gc[1] + gi * sin(ga)];
const rimIn = [rimR - G.t * cos(ga), rimZ - G.t * sin(ga)];
const sf = G.stem + G.fillet;
const glassProfile = [
  [0, 0], [G.foot, 0], [G.foot, G.footT],
  [sf, G.footTop, about([sf, G.footTop], [G.stem, G.footTop + G.fillet], [sf, G.footTop + G.fillet], false)],
  [G.stem, G.footTop + G.fillet],
  [G.stem, G.bowlZ - G.fillet, about([G.stem, G.bowlZ - G.fillet], [sf, G.bowlZ], [sf, G.bowlZ - G.fillet], false)],
  [sf, G.bowlZ],
  [gc[0], G.bowlZ, about([gc[0], G.bowlZ], gOut, gc, true)],
  gOut,
  [rimR, rimZ, 1],                                             // a half-round rim
  rimIn,
  [gIn[0], gIn[1], about(gIn, [gc[0], gc[1] - gi], gc, false)],
  [gc[0], gc[1] - gi],
  [0, gc[1] - gi],
];
const glass = turned("wine glass", glassProfile, AT.glass, "glass");

// ── Bowl ─────────────────────────────────────────────────────────────
const B = BOWL;
const ba = rad(B.rim);
const bOut = [B.centre[0] + B.r * cos(ba), B.centre[1] + B.r * sin(ba)];
const bi = B.r - B.t;
const bIn = [B.centre[0] + bi * cos(ba), B.centre[1] + bi * sin(ba)];
const bBottom = [B.centre[0], B.centre[1] - B.r];
const bowlProfile = [
  [0, B.recess], [B.footIn, B.recess], [B.footIn + 1, 0], [B.foot, 0],
  [bBottom[0], bBottom[1], about(bBottom, bOut, B.centre, true)],
  bOut.concat([1]),                                            // a half-round rim
  [bIn[0], bIn[1], about(bIn, [B.centre[0], B.centre[1] - bi], B.centre, false)],
  [B.centre[0], B.centre[1] - bi],
  [0, B.centre[1] - bi],
];
const bowl = turned("bowl", bowlProfile, AT.bowl, "blue glaze");

// ── Flask and cap ────────────────────────────────────────────────────
const F = FLASK;
const fs = [F.r - F.shoulder, F.shoulderZ];                   // centre of the shoulder
const flaskProfile = [
  [0, 0], [F.r - F.base, 0, about([F.r - F.base, 0], [F.r, F.base], [F.r - F.base, F.base], true)],
  [F.r, F.base],
  [F.r, F.shoulderZ, about([F.r, F.shoulderZ], [fs[0], fs[1] + F.shoulder], fs, true)],
  [fs[0], fs[1] + F.shoulder],
  [F.neck, fs[1] + F.shoulder], [F.neck, F.neckZ], [0, F.neckZ],
];
const flask = turned("flask", flaskProfile, AT.flask, "stainless");
const C = CAP;
const capProfile = [
  [C.bore, C.bottom], [C.r, C.bottom],
  [C.r, C.top - C.round, about([C.r, C.top - C.round], [C.r - C.round, C.top], [C.r - C.round, C.top - C.round], true)],
  [C.r - C.round, C.top], [0, C.top], [0, F.neckZ], [C.bore, F.neckZ],
];
const cap = turned("flask cap", capProfile, AT.flask, "black plastic");

// ── Pepper mill and finial ───────────────────────────────────────────
const M = MILL;
const millProfile = [
  [0, 0], [M.base - 3, 0, about([M.base - 3, 0], [M.base, 3], [M.base - 3, 3], true)],
  [M.base, 3], [M.base, 14, 1],                                // a bead
  [M.base, 20], [28, 25, sh.arc3([28, 25], [M.waist, 82.5], [28, 140])],   // the long cove
  [28, 140, 1],                                                // a bead
  [28, 148], [31, 152], [31, 162, 1],                          // a bead
  [31, 168], [27, 172],
  [27, 196, about([27, 196], [21, M.top], [21, 196], true)],
  [21, M.top], [0, M.top],
];
const mill = turned("pepper mill", millProfile, AT.mill, "walnut");
const fr = 2.5;
const finialProfile = [
  [0, M.top], [M.finial, M.top],
  [M.finial, M.top + M.finialH - fr, about([M.finial, M.top + M.finialH - fr], [M.finial - fr, M.top + M.finialH], [M.finial - fr, M.top + M.finialH - fr], true)],
  [M.finial - fr, M.top + M.finialH], [0, M.top + M.finialH],
];
const finial = turned("finial", finialProfile, AT.mill, "brass");

// ── Checks ───────────────────────────────────────────────────────────
const parts = cad.parts();
run.equal("eleven parts", parts.length, 11);
for (const o of [glass, bowl, flask, cap, mill, finial]) {
  run.near(`${o.part.name} volume (Pappus)`, cad.part(o.part.id).volume, o.volume);
}
const round = (1 - PI / 4) * TABLE.r * TABLE.r;
run.near("table top volume", cad.part(top.id).volume, TABLE.w * TABLE.d * TABLE.t - round * TABLE.w);
for (const l of legs) run.near("leg volume", cad.part(l.id).volume, TABLE.leg ** 2 * (Z0 - TABLE.t));
// Everything on the table stands on its top.
for (const o of [glass, bowl, flask, mill]) {
  run.near(`${o.part.name} stands on the table`, cad.part(o.part.id).bbox.min[2], Z0, 0, 1e-9);
}
// The rim is a half circle on the wall's end; its highest point is half
// the wall above the middle of the wall's end.
const rimTop = (rimZ + rimIn[1]) / 2 + G.t / 2;
run.near("glass height to the rim", cad.part(glass.part.id).bbox.max[2], Z0 + rimTop, 0, 1e-6);
run.ok("the glass wall is as thick at the rim as below", Math.abs(hypot(rimIn[0] - rimR, rimIn[1] - rimZ) - G.t) < 1e-12);
// Nothing overlaps; the objects touch the table, the cap sits on the
// neck, the finial on the mill.
const clash = cad.overlapping3d(null);
run.ok("no two parts overlap", clash.pairs.length === 0 && clash.skipped.length === 0, JSON.stringify(clash));
const others = parts.filter((p) => p.id !== top.id).map((p) => p.id);
const onTop = cad.pairsWithin3d(top, others, 1e-6);
run.equal("four objects and four legs touch the table top", onTop.pairs.length, 8);

// ── Renders ──────────────────────────────────────────────────────────
// The camera stands in front of the table, a little above it. The
// presentation render dims the studio: a darker dome and floor, so the
// glass's edges and the steel's reflections stand out, with the soft key
// light from the left.
const CAMERA = { eye: [10, -980, 1100], target: [0, 20, 835], fov: 24 };
const STUDIO = {
  environment: { preset: "studio", zenith: [176, 178, 182], horizon: [92, 92, 96], ground: [40, 40, 42] },
  ground: { color: [72, 70, 68] },
  exposure: 0.3,
};
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "camera", camera: CAMERA, ground: true, present: STUDIO },
]);

return run.summary({ rim: Math.round(rimR * 2 * 100) / 100 });
