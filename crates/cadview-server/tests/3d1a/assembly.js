// A small assembly of twelve parts (rung 3D1a): a base plate with holes and
// a slot, two uprights with a groove, a shelf resting in the grooves, four
// pins and four feet. Every part has a closed-form volume and box; the
// server test (script.rs) compares them with what cad.parts() reports.

const T = 12; // base thickness

// Base plate 400 x 300 x 12 with four corner holes and a slot.
const base = cad.extrude({ min: [0, 0], max: [400, 300] }, {
  distance: T, name: "base", attrs: { group: "base" },
});
for (const [x, y] of [[30, 30], [370, 30], [30, 270], [370, 270]]) {
  cad.hole(base, { at: [x, y, T], dir: [0, 0, -1], dia: 10, depth: "through" });
}
// A slot: 100 between centres, 20 wide, semicircular ends (exact arcs).
cad.cut(base, {
  extrude: [[150, 140], [250, 140, 1], [250, 160], [150, 160, 1]],
  distance: T,
});

// Left upright: 300 x 200 x 10 on the yz plane, standing on the base, with
// a groove 6 deep and 10 high on its inner face, and a blind hole 8 x 5
// from its outer face.
const left = cad.extrude({ min: [0, 0], max: [300, 200] }, {
  plane: "yz", at: [20, 0, T], distance: 10, name: "upright", attrs: { group: "upright" },
});
cad.cut(left, { extrude: { min: [0, 100], max: [300, 110] }, plane: "yz", at: [24, 0, 0], distance: 6 });
cad.hole(left, { at: [20, 150, 180], dir: [1, 0, 0], dia: 8, depth: 5 });
// Right upright: the same definition, turned half a turn about the
// vertical axis through the middle of the base.
const right = cad.copy3d(left, {
  rotate: { angle: 180, axis: [0, 0, 1], about: [200, 150, 0] },
});

// Shelf in the grooves, with two through holes and a lip along the back.
const shelf = cad.extrude({ min: [24, 0], max: [376, 300] }, {
  at: [0, 0, 100], distance: 10, name: "shelf", attrs: { group: "shelf" },
});
cad.hole(shelf, { at: [100, 150, 110], dia: 6 });
cad.hole(shelf, { at: [300, 150, 110], dia: 6 });
cad.union(shelf, { extrude: { min: [24, 290], max: [376, 300] }, at: [0, 0, 110], distance: 5 });

// Four pins through the corner holes, sharing one definition.
const pin = cad.extrude({ center: [30, 30], radius: 5 }, {
  at: [0, 0, -4], distance: 20, name: "pin", attrs: { group: "pin" },
});
cad.copy3d(pin, { move: [340, 0, 0] });
cad.copy3d({ attrs: { group: "pin" } }, { move: [0, 240, 0] });

// Four feet under the base.
const foot = cad.extrude({ min: [50, 50], max: [90, 90] }, {
  at: [0, 0, -8], distance: 8, name: "foot", attrs: { group: "foot" },
});
cad.copy3d(foot, { move: [260, 0, 0] });
cad.copy3d({ attrs: { group: "foot" } }, { move: [0, 160, 0] });

const parts = cad.parts();
return {
  ids: { base: base.id, left: left.id, right: right.id, shelf: shelf.id, pin: pin.id, foot: foot.id },
  parts: parts.map((p) => ({
    id: p.id, def: p.def, group: p.attrs.group, instances: p.instances,
    volume: p.volume, bbox: p.bbox, centroid: p.centroid,
  })),
  describe: cad.describe(),
};
