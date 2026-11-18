// 05 - Piston and connecting rod, the 3D twin of 06-piston-section.
//
// A Ø80 piston, 65 high, with three ring grooves (1.2, 1.5 and 3 wide,
// 3.6 deep) and Ø22 pin bores in two bosses inside the skirt; a hollow
// pin Ø22 / Ø13 x 60; and a connecting rod 140 between centres, 24 thick,
// with a Ø22 small end and a Ø52 big end, its outline two circles joined
// by tangent lines (exact arcs).
//
// The piston is one revolve of lines (grooves included), then booleans:
// two blocks for the bosses, a blind hole in each for the pin. The crown
// edge is rounded R1.
//
// Simplified, and why: the bosses are square blocks and the pin bores
// stop short of the skirt, so the skirt has no pin holes. A round boss or
// a bore through the skirt makes two cylinders meet; the kernel adapter
// builds that solid but cannot give its box ("a tight bounding box of a
// NURBS edge is not supported by this kernel yet"), so the part could be
// neither queried nor rendered.
//
// The pin axis is x, the piston axis z; the rod swings in the yz plane.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const run = ck.start("05-piston-and-rod");
const { PI, asin, cos, sin, tan } = Math;

// ── Parameters (mm) ──────────────────────────────────────────────────
const D = 80, HEIGHT = 65;           // piston diameter and height
const CROWN = 8;                     // crown thickness
const SKIRT_ID = 70;                 // skirt inside diameter
const BELT_ID = 60;                  // inside diameter behind the ring belt
const BELT_FROM = 44;                // where the ring belt support starts (z)
const GROOVES = [1.2, 1.5, 3.0];     // ring groove widths, from the top
const LAND = [6, 4, 4];              // land above each groove
const GROOVE_DEPTH = 3.6;
const PIN_Z = 26, PIN_D = 22, PIN_ID = 13, PIN_L = 60;
const BOSS_D = 34, GAP = 26;         // pin bosses, and the gap between them
const ROD = { ctc: 140, t: 24, small: 34, big: 70, bigBore: 52 };
const R_CROWN = 1;

// ── Piston body: one revolve (u = radius, v = height) ────────────────
const ro = D / 2, rg = ro - GROOVE_DEPTH;
const outer = [];
let z = HEIGHT;
GROOVES.forEach((w, i) => {
  z -= LAND[i];
  outer.push([ro, z], [rg, z], [rg, z - w], [ro, z - w]);
  z -= w;
});
outer.reverse();                     // bottom to top
const profile = [
  [SKIRT_ID / 2, 0], [ro, 0], ...outer, [ro, HEIGHT], [0, HEIGHT],
  [0, HEIGHT - CROWN], [BELT_ID / 2, HEIGHT - CROWN], [BELT_ID / 2, BELT_FROM], [SKIRT_ID / 2, BELT_FROM],
];
const piston = cad.revolve(profile, {
  plane: "xz", axis: [[0, 0], [0, 1]], name: "piston", attrs: { part: "piston", material: "aluminium" },
});
// Crown edge rounded (picked at +y, away from the revolve's seam at +x).
cad.fillet3d(piston, { edges: { near: [[0, ro, HEIGHT]] }, radius: R_CROWN });

// Pin bosses: a block on each side of the gap the rod swings in, from the
// gap into the skirt wall, each with a blind bore for the pin.
const rb = BOSS_D / 2, xw = SKIRT_ID / 2 + 1;   // half width; outer end in the wall
for (const s of [1, -1]) {
  const [x0, x1] = s > 0 ? [GAP / 2, xw] : [-xw, -GAP / 2];
  cad.union(piston, sh.boxTool([x0, -rb, PIN_Z - rb], [x1, rb, PIN_Z + rb]));
  cad.hole(piston, { at: [s * GAP / 2, 0, PIN_Z], dir: [s, 0, 0], dia: PIN_D, depth: PIN_L / 2 - GAP / 2 });
}

// ── Pin ──────────────────────────────────────────────────────────────
const pin = cad.extrude(
  { outer: { center: [0, PIN_Z], radius: PIN_D / 2 }, holes: [{ center: [0, PIN_Z], radius: PIN_ID / 2 }] },
  { plane: "yz", at: [-PIN_L / 2, 0, 0], distance: PIN_L, name: "pin", attrs: { part: "pin", material: "steel" } },
);

// ── Connecting rod: two circles and their outer tangents ─────────────
const r1 = ROD.small / 2, r2 = ROD.big / 2, zs = PIN_Z, zb = PIN_Z - ROD.ctc;
const a = asin((r2 - r1) / ROD.ctc);          // tilt of the tangent lines
const pt = (zc, r, s) => [s * r * cos(a), zc - r * sin(a)];
const rodLoop = [
  [...pt(zb, r2, 1)], [...pt(zs, r1, 1), tan((PI + 2 * a) / 4)],   // up the right side, over the small end
  [...pt(zs, r1, -1)], [...pt(zb, r2, -1), tan((PI - 2 * a) / 4)], // down the left, round the big end
];
const rod = cad.extrude(
  { outer: rodLoop, holes: [{ center: [0, zs], radius: PIN_D / 2 }, { center: [0, zb], radius: ROD.bigBore / 2 }] },
  { plane: "yz", at: [-ROD.t / 2, 0, 0], distance: ROD.t, name: "rod", attrs: { part: "rod", material: "steel" } },
);

// ── Checks ───────────────────────────────────────────────────────────
run.equal("three parts", cad.parts().length, 3);
const disc = (d) => PI * d * d / 4;
// Piston: Pappus, the round on the crown, the bosses inside the skirt
// less the gap, and the pin bore through bosses and skirt.
const body = sh.revolvedVolume(profile) - sh.roundRing(ro, R_CROWN);
// A boss inside the skirt: for each y across it, from the gap to the
// skirt's inside wall (radius Rs), times its height.
const Rs = SKIRT_ID / 2;
const chordInt = rb * Math.sqrt(Rs * Rs - rb * rb) + Rs * Rs * asin(rb / Rs);  // integral of sqrt(Rs^2 - y^2), |y| < rb
const bosses = 2 * (chordInt - GAP / 2 * 2 * rb) * BOSS_D;
const bore = 2 * disc(PIN_D) * (PIN_L / 2 - GAP / 2);
run.ok("the bosses stay below the ring belt support", PIN_Z + rb < BELT_FROM);
run.ok("the boss ends lie inside the skirt wall", Math.hypot(xw, rb) < ro);
run.ok("the pin bores stop inside the bosses",
  PIN_L / 2 < Math.sqrt(Rs * Rs - (PIN_D / 2) ** 2));
run.near("piston volume", cad.part(piston.id).volume, body + bosses - bore, 1e-8);
run.near("pin volume", cad.part(pin.id).volume, (disc(PIN_D) - disc(PIN_ID)) * PIN_L);
const rodArea = sh.loopArea(rodLoop) - disc(PIN_D) - disc(ROD.bigBore);
run.near("rod volume", cad.part(rod.id).volume, rodArea * ROD.t);
run.box("rod box", cad.part(rod.id).bbox,
  // Widest across the lower tangent points, which lie above the big end's
  // centre line.
  { min: [-ROD.t / 2, -r2 * cos(a), zb - r2], max: [ROD.t / 2, r2 * cos(a), zs + r1] });
run.ok("rod clears the bosses", ROD.t < GAP);

// ── Renders ──────────────────────────────────────────────────────────
await run.render([
  { as: "iso", view: "iso", ground: true },
  // Quarter cutaway: the quarter toward the viewer (x > 0 and y < 0) taken
  // out, showing the grooves, crown, a boss, the hollow pin and both rod
  // eyes in section.
  {
    as: "cutaway", view: "iso", ground: true, clipMode: "all",
    clip: [{ point: [0, 0, 0], normal: [1, 0, 0] }, { point: [0, 0, 0], normal: [0, -1, 0] }],
  },
]);

return run.summary();
