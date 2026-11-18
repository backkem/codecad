// A weld-neck flange as a part, shared by 01-pipe-flange and 07-pipe-spool:
//
//   const fl = await cad.use("flange");
//   const f = fl.weldNeck();          // DN50 by default; returns {part, dims, volume}
//
// The flange is built standing on its raised face at the origin, axis +z,
// the weld end of the hub at z = height. Plain script code over cad.*.

const sh = await cad.use("shapes");
const { PI } = Math;

// DN50 (pipe Ø60.3 x 3.9), the dimensions of example 01.
const DN50 = {
  D: 165, C: 18,               // disc diameter and thickness
  dRF: 102, fRF: 2,            // raised face diameter and height
  bore: 52.5,                  // bore (pipe inside diameter)
  hubBase: 74, hubEnd: 60.3,   // hub diameter at the disc and at the weld end
  hubH: 45,                    // hub height above the disc
  pcd: 125, bolts: 4, hole: 18,
  ch: 1,                       // chamfer on the disc's outside edges
};

function weldNeck(dims = {}, opts = {}) {
  const d = { ...DN50, ...dims };
  const z1 = d.fRF, z2 = d.fRF + d.C, z3 = z2 + d.hubH;   // levels of the profile
  // The half-section, revolved about the z axis (u = radius, v = height).
  const profile = [
    [d.bore / 2, 0], [d.dRF / 2, 0], [d.dRF / 2, z1], [d.D / 2, z1],
    [d.D / 2, z2], [d.hubBase / 2, z2], [d.hubEnd / 2, z3], [d.bore / 2, z3],
  ];
  const part = cad.revolve(profile, {
    plane: "xz", axis: [[0, 0], [0, 1]], name: "weld-neck flange",
    attrs: { part: "flange", material: "turned steel" }, ...opts,
  });
  // Chamfer the disc's outside edges, picked at +y: a point at +x would
  // be as near to the revolve's seam, which lies in the sketch plane.
  cad.chamfer3d(part, { edges: { near: [[0, d.D / 2, z2], [0, d.D / 2, z1]] }, d: d.ch });
  // Bolt holes straddling the centre lines.
  for (const [x, y] of sh.boltCircle([0, 0], d.pcd, d.bolts, 180 / d.bolts)) {
    cad.hole(part, { at: [x, y, z2], dia: d.hole });
  }
  // Closed form: Pappus for the body, a ring of a small triangle per
  // chamfer, and the holes through the disc.
  const volume = sh.revolvedVolume(profile)
    - 2 * (2 * PI * (d.D / 2 - d.ch / 3) * (d.ch * d.ch / 2))
    - d.bolts * (PI * d.hole * d.hole / 4) * d.C;
  return { part, dims: { ...d, height: z3 }, volume, profile };
}

return { weldNeck, DN50 };
