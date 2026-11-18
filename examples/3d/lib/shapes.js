// Small solid helpers for the 3D examples, loaded with
//
//   const sh = await cad.use("shapes");
//
// Plain script code over the public cad.* calls: nothing here is known to
// the core. Millimetres, degrees, Z up.

const { PI, sqrt, acos, atan2, cos, sin, hypot } = Math;
const deg = (r) => (r * 180) / PI;
const rad = (d) => (d * PI) / 180;

// ── Parts ──────────────────────────────────────────────────────────

// An axis-aligned box from corner `min` to corner `max` ([x, y, z]).
function box(min, max, opts = {}) {
  return cad.extrude(
    { min: [min[0], min[1]], max: [max[0], max[1]] },
    { at: [0, 0, min[2]], distance: max[2] - min[2], ...opts },
  );
}

// A box-shaped tool (for cut, union, intersect) from `min` to `max`.
function boxTool(min, max) {
  return {
    extrude: { min: [min[0], min[1]], max: [max[0], max[1]] },
    at: [0, 0, min[2]],
    distance: max[2] - min[2],
  };
}

// A straight member of rectangular section [a, b] from point `p` to point
// `q`. It is built along +Z and tilted about the horizontal axis
// perpendicular to it, so for a member in a vertical plane side `b` stays
// horizontal (across the plane) and side `a` lies in the plane. Its ends
// are square to its length, `over` beyond p and q (default 0), so that
// `keep` can then cut them level or plumb through p and q.
function member(p, q, [a, b], { over = 0, ...opts } = {}) {
  let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
  const len0 = hypot(...d);
  const u = d.map((c) => c / len0);
  p = p.map((c, i) => c - over * u[i]);
  d = d.map((c, i) => c + 2 * over * u[i]);
  const len = len0 + 2 * over;
  const part = box([-a / 2, -b / 2, 0], [a / 2, b / 2, len], opts);
  const axis = [-d[1], d[0], 0]; // Z x d
  const tilt = deg(acos(d[2] / len));
  if (hypot(axis[0], axis[1]) > 1e-12 && tilt > 1e-12) {
    // Turn the section so side b lies along the tilt axis (across the
    // member's vertical plane), then tilt Z onto the member.
    cad.rotate3d(part, { angle: deg(atan2(d[1], d[0])), axis: [0, 0, 1] });
    cad.rotate3d(part, { angle: tilt, axis });
  }
  cad.move3d(part, p);
  return part;
}

// Keep only the part of `part` inside the box `min`..`max` (end cuts,
// level and plumb). Coordinates left out are open.
function keep(part, min, max) {
  const big = 1e6;
  const lo = min.map((v) => v ?? -big);
  const hi = max.map((v) => v ?? big);
  return cad.intersect(part, boxTool(lo, hi));
}

// ── Profiles (exact lines and arcs) ────────────────────────────────

// A w x h rectangle with corners rounded to r, lower-left corner at
// [x, y]: vertices with bulges (quarter arcs, counter-clockwise).
function roundedRect(x, y, w, h, r) {
  const q = Math.tan(PI / 8); // bulge of a quarter arc
  return [
    [x + r, y], [x + w - r, y, q],
    [x + w, y + r], [x + w, y + h - r, q],
    [x + w - r, y + h], [x + r, y + h, q],
    [x, y + h - r], [x, y + r, q],
  ];
}

// A slot `len` long overall and `w` wide, centred on [cx, cy], along v
// (vertical in the sketch) or along u.
function slot(cx, cy, len, w, along = "v") {
  const r = w / 2, s = (len - w) / 2;
  if (along === "v") {
    return [[cx + r, cy - s], [cx + r, cy + s, 1], [cx - r, cy + s], [cx - r, cy - s, 1]];
  }
  return [[cx - s, cy - r], [cx + s, cy - r, 1], [cx + s, cy + r], [cx - s, cy + r, 1]];
}

// The bulge of the circular arc from p0 through pm to p1 (for the vertex
// at p0): tan(sweep / 4), positive when the arc turns counter-clockwise.
function arc3(p0, pm, p1) {
  // Centre: where the perpendicular bisectors of p0-pm and pm-p1 meet.
  const [ax, ay] = p0, [bx, by] = pm, [cx, cy] = p1;
  const d = 2 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
  const a2 = ax * ax + ay * ay, b2 = bx * bx + by * by, c2 = cx * cx + cy * cy;
  const ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
  const uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
  const ang = ([x, y]) => atan2(y - uy, x - ux);
  const norm = (t) => ((t % (2 * PI)) + 2 * PI) % (2 * PI);
  // Counter-clockwise sweeps from p0 to pm and to p1.
  const toM = norm(ang(pm) - ang(p0)), toEnd = norm(ang(p1) - ang(p0));
  const sweep = toM < toEnd ? toEnd : toEnd - 2 * PI;   // through pm
  return Math.tan(sweep / 4);
}

// `n` points on a circle of diameter `pcd` about [cx, cy], the first at
// `start` degrees from +X.
function boltCircle([cx, cy], pcd, n, start = 0) {
  return Array.from({ length: n }, (_, i) => {
    const a = rad(start + (360 * i) / n);
    return [cx + (pcd / 2) * cos(a), cy + (pcd / 2) * sin(a)];
  });
}

// ── Closed forms the examples check against ────────────────────────

// Area of a w x h rectangle with corners rounded to r.
const roundedRectArea = (w, h, r) => w * h - (4 - PI) * r * r;

// Area of the part of a circle of radius r beyond a chord at distance d
// from its centre.
const segmentArea = (r, d) => r * r * acos(d / r) - d * sqrt(r * r - d * d);

// Area and first moment about u = 0 of a closed loop of [u, v, bulge?]
// vertices, exact: the polygon through the vertices plus or minus the
// circular segment of each bulged edge (bulge = tan(sweep / 4), positive
// counter-clockwise). Both are positive for a counter-clockwise loop. For
// a profile revolved a full turn about u = 0, volume = 2 pi * moment
// (Pappus).
function loopMoments(verts) {
  let area = 0, moment = 0;
  for (let i = 0; i < verts.length; i++) {
    const [u0, v0, b = 0] = verts[i], [u1, v1] = verts[(i + 1) % verts.length];
    const c = u0 * v1 - u1 * v0;
    area += c / 2;
    moment += ((u0 + u1) * c) / 6;
    if (b !== 0) {
      // The circular segment between the chord and the arc: its area, and
      // its centroid on the bisector, 4 r sin^3(th/2) / (3 (th - sin th))
      // from the centre, on the arc's side of the chord.
      const th = 4 * Math.atan(Math.abs(b));
      const chord = hypot(u1 - u0, v1 - v0);
      const r = chord / (2 * sin(th / 2));
      const seg = (r * r / 2) * (th - sin(th));
      // The arc lies right of the direction of travel for a positive
      // (counter-clockwise) bulge, left for a negative one.
      const s = Math.sign(b);
      const nu = (s * (v1 - v0)) / chord;            // u of that unit normal
      const h = r * cos(th / 2);                      // centre to chord
      const dc = (4 * r * sin(th / 2) ** 3) / (3 * (th - sin(th)));
      area += s * seg;
      moment += s * seg * ((u0 + u1) / 2 + nu * (dc - h));
    }
  }
  return { area, moment };
}

const loopArea = (verts) => loopMoments(verts).area;

// Volume of a profile (either orientation) revolved a full turn about u = 0.
const revolvedVolume = (verts) => 2 * PI * Math.abs(loopMoments(verts).moment);

// The ring a round of radius r takes off a convex circular edge of
// radius R (Pappus: the spandrel's area times its centroid's circle).
const roundRing = (R, r) => 2 * PI * (R - (r * (10 - 3 * PI)) / (12 - 3 * PI)) * (1 - PI / 4) * r * r;

return {
  box, boxTool, member, keep,
  roundedRect, slot, boltCircle, arc3,
  roundedRectArea, segmentArea, loopMoments, loopArea, revolvedVolume, roundRing,
  deg, rad,
};
