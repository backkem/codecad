// Every part call once, on the server's kernel, each on a part with a
// closed-form volume (rung 3D1a). The server test (script.rs) holds the
// closed forms.
const plate = () => cad.extrude({ min: [0, 0], max: [100, 50] }, { distance: 18 });
const vol = (p) => p.volume;
const out = {};

// Create.
out.bulge = vol(cad.extrude([[0, 0], [100, 0], [100, 40, 1], [0, 40]], { distance: 6 }));
out.clockwise = vol(cad.extrude([[0, 0], [0, 40], [100, 40], [100, 0]], { distance: 6 }));
out.tube = vol(cad.revolve({ min: [10, 0], max: [20, 50] }, { plane: "xz", axis: [[0, 0], [0, 1]] }));
out.sweep = vol(cad.sweep(
  { center: [0, 0], radius: 6 },
  [{ to: [0, 0, 100] }, { via: [50, 0, 150], to: [100, 0, 100] }],
));
out.loft = vol(cad.loft(
  [
    { profile: { min: [-20, -20], max: [20, 20] } },
    { profile: { min: [-12, -12], max: [12, 12] }, at: [0, 0, 300] },
  ],
  { ruled: true },
));
// From a 2D entity: a closed polyline with a bulge, passed as the entity.
const outline = cad.addPolyline([[0, 0], [60, 0, 1], [60, 20], [0, 20]], { closed: true });
out.fromEntity = vol(cad.extrude(outline, { distance: 10 }));

// Modify.
const a = plate();
const b = cad.extrude({ min: [50, 25], max: [150, 75] }, { distance: 30 });
out.intersect = vol(cad.intersect(a, { part: b }));
out.union = vol(cad.union(plate(), { extrude: { min: [0, 40], max: [100, 50] }, at: [0, 0, 18], distance: 5 }));
out.cbore = vol(cad.hole(plate(), { at: [60, 10, 18], dia: 5, cbore: [9, 3] }));
out.fillet = vol(cad.fillet3d(plate(), { edges: { cap: "end", segs: [0] }, radius: 3 }));
out.filletNear = vol(cad.fillet3d(plate(), { edges: { near: [[50, 0, 18]] }, radius: 3 }));
out.chamfer = vol(cad.chamfer3d(plate(), { edges: { lateral: [0] }, d: 1 }));

// A local-frame cut on a moved part: the same as a world cut before the move.
const m = plate();
cad.move3d(m, [1000, 0, 0]);
out.localCut = vol(cad.cut(m, { extrude: { min: [0, 0], max: [10, 50] }, distance: 18 }, { frame: "local" }));
out.localCutBox = cad.part(m).bbox;
return out;
