// 06 - Spur gear pair, module 2, 20 and 40 teeth, the 3D twin of
// 09-spur-gears.
//
// 20° pressure angle, 15 wide, meshed at the standard centre distance
// a = m (z1 + z2) / 2 = 60. Each gear is one extrude of its whole outline:
// per tooth a radial line from the root to the base circle, the involute
// flank as one circular arc through the involute at the base circle, half
// way up and at the tip, the tip arc, the other flank, and the root arc
// to the next tooth (all exact lines and arcs, 120 and 240 segments). A
// hub on the back of each gear, a bore with a keyway through both, and a
// 1 mm chamfer on the end of each hub.
//
// Simplified: one arc per flank (the involute itself would need spline
// profiles, which the reference semantics does not have yet) and no root
// fillets. Not yet: a check that the teeth do not overlap at the mesh
// (interference between parts is a later rung).
//
// The gear axes run along y; the gears lie in the xz plane, faces at
// y = 0 toward the front.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("06-spur-gears");
const { PI, cos, sin, tan, acos } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const M = 2, ALPHA = (20 * PI) / 180, WIDTH = 15;
const PINION = { z: 20, hub: [28, 10], bore: 12, key: [4, 1.8] };   // hub [dia, length], key [width, depth in hub]
const WHEEL = { z: 40, hub: [40, 12], bore: 20, key: [6, 2.8] };
const CH = 1;                                                     // hub end chamfer
const A = (M * (PINION.z + WHEEL.z)) / 2;                         // centre distance

const inv = (a) => tan(a) - a;
const P = ([cx, cy], r, t) => [cx + r * cos(t), cy + r * sin(t)];

// The outline of a gear with z teeth about c, tooth 0 at angle t0
// (radians, in the sketch plane): [u, v, bulge] vertices, counter-clockwise.
function gearLoop(c, z, t0) {
  const r = (M * z) / 2, ra = r + M, rf = r - 1.25 * M, rb = r * cos(ALPHA);
  const hb = PI / (2 * z) + inv(ALPHA);       // half tooth angle at the base circle
  const at = acos(rb / ra);                    // pressure angle at the tip
  const rm = (rb + ra) / 2, am = acos(rb / rm);
  const tip = 2 * (hb - inv(at)), root = (2 * PI) / z - 2 * hb;
  const v = [];
  for (let i = 0; i < z; i++) {
    const t = t0 + (2 * PI * i) / z;
    const Rb = P(c, rb, t - hb), Rm = P(c, rm, t - hb + inv(am)), Rt = P(c, ra, t - hb + inv(at));
    const Lt = P(c, ra, t + hb - inv(at)), Lm = P(c, rm, t + hb - inv(am)), Lb = P(c, rb, t + hb);
    v.push(
      [...P(c, rf, t - hb)],                     // root of the right flank, radial line up
      [...Rb, sh.arc3(Rb, Rm, Rt)],              // right flank
      [...Rt, tan(tip / 4)],                     // tip arc
      [...Lt, sh.arc3(Lt, Lm, Lb)],              // left flank
      [...Lb],                                   // radial line down
      [...P(c, rf, t + hb), tan(root / 4)],      // root arc to the next tooth
    );
  }
  return { loop: v, r, ra, rf, rb };
}

// A bore of diameter d with a keyway b wide reaching t beyond it (+v).
function keyedBore([cx, cy], d, [b, t]) {
  const R = d / 2, yk = Math.sqrt(R * R - (b / 2) ** 2), dl = Math.asin(b / (2 * R));
  return [
    [cx + b / 2, cy + yk], [cx + b / 2, cy + R + t], [cx - b / 2, cy + R + t],
    [cx - b / 2, cy + yk, tan((2 * PI - 2 * dl) / 4)],
  ];
}

function gear(name, g, center, t0) {
  const prof = gearLoop(center, g.z, t0);
  const part = cad.extrude(prof.loop, {
    plane: "xz", at: [0, WIDTH, 0], distance: WIDTH, name, attrs: { part: name, teeth: g.z, module: M },
  });
  const [hd, hl] = g.hub;
  // Hub on the back, sunk 1 mm into the gear body.
  cad.union(part, { extrude: { center, radius: hd / 2 }, plane: "xz", at: [0, WIDTH + hl, 0], distance: hl + 1 });
  cad.chamfer3d(part, { edges: { near: [[center[0], WIDTH + hl, center[1] + hd / 2]] }, d: CH });
  const bore = keyedBore(center, g.bore, g.key);
  cad.cut(part, { extrude: bore, plane: "xz", at: [0, WIDTH + hl + 1, 0], distance: WIDTH + hl + 2 });
  return { part, prof, bore };
}

// Pinion with a tooth toward the wheel; wheel with a gap toward it.
const pinion = gear("pinion", PINION, [0, 0], 0);
const wheel = gear("wheel", WHEEL, [A, 0], PI + PI / WHEEL.z);

// ── Checks ───────────────────────────────────────────────────────────
run.equal("two parts", cad.parts().length, 2);
run.near("centre distance", A, 60, 0);
for (const [g, spec] of [[pinion, PINION], [wheel, WHEEL]]) {
  const [hd, hl] = spec.hub;
  const vol = sh.loopArea(g.prof.loop) * WIDTH
    + (PI * hd * hd / 4) * hl
    - 2 * PI * (hd / 2 - CH / 3) * (CH * CH / 2)
    - sh.loopArea(g.bore) * (WIDTH + hl);
  const p = cad.part(g.part.id);
  run.near(`${spec.z}-tooth gear volume`, p.volume, vol);
  // Its outline reaches the tip circle at the teeth and stays inside it.
  run.ok(`${spec.z}-tooth gear within its tip circle`,
    p.bbox.max[2] <= g.prof.ra + 1e-9 && p.bbox.max[2] > g.prof.ra * cos(PI / spec.z));
  run.ok(`${spec.z}-tooth hub inside the root circle`, hd / 2 < g.prof.rf);
  run.ok(`${spec.z}-tooth keyway inside the hub`, spec.bore / 2 + spec.key[1] < hd / 2 - CH);
}
// Tip of each gear clears the root of the other (the standard clearance 0.25 m).
run.near("tip to root clearance", A - pinion.prof.ra - wheel.prof.rf, 0.25 * M, 1e-12);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  { as: "front", view: "front" },
]);

return run.summary({
  pinion: { tip: 2 * pinion.prof.ra, root: 2 * pinion.prof.rf },
  wheel: { tip: 2 * wheel.prof.ra, root: 2 * wheel.prof.rf },
});
