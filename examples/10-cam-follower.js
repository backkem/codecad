// 10 - Disc cam with in-line roller follower, laid out from its motion law.
//
// Motion: rise 20 mm over 120 deg, dwell 60, return over 120, dwell 60,
// both moves cycloidal (zero acceleration at the ends). The cam turns
// clockwise, so the pitch curve is laid out counter-clockwise from the
// follower axis: P(t) at polar angle 90 + t, radius Rb + Rr + s(t).
// The cam profile is the inner envelope of the roller: the pitch curve
// offset inward along its normal by the roller radius. Both curves are
// fitted splines; the displacement diagram beside the cam plots the same
// law. Transition angles are dimensioned with dimAngle.
//
// Exercises: addSpline (closed fit), dimAngle, trimTo (keyway), addHatch
// (follower guide), measure, table.

const draft = await cad.use("drafting");
const { P, dim, text, polar, add, sub, mul, unit, perp } = draft;
const { ACI } = draft;
draft.setup({
  scale: 1,
  layers: {
    CAM: { color: ACI.white, lineweight: 0.5 },
    PITCH: { color: ACI.amber, linetype: "PHAN", lineweight: 0.25 },
    CURVE: { color: ACI.amber, lineweight: 0.5 },
    GRID: { color: ACI.gray, lineweight: 0.13 },
  },
});

draft.sheet({
  at: [-P(70), -P(85)], w: 297, h: 189,
  title: "DISC CAM - ROLLER FOLLOWER",
  subtitle: "CYCLOIDAL RISE / RETURN, 20 mm LIFT",
  number: "CC-010", scale: "1:1",
});

// ── Motion law ──────────────────────────────────────────────────────
const Rb = 30, Rr = 8, h = 20;                 // base circle, roller radius, lift
const segs = [                                 // [start, end, kind]
  [0, 120, "rise"], [120, 180, "dwell-high"], [180, 300, "return"], [300, 360, "dwell-low"],
];
const cyclo = (u) => u - Math.sin(2 * Math.PI * u) / (2 * Math.PI);
function s(t) {
  t = ((t % 360) + 360) % 360;
  for (const [a, b, k] of segs) {
    if (t < a || t > b) continue;
    const u = (t - a) / (b - a);
    if (k === "rise") return h * cyclo(u);
    if (k === "return") return h * (1 - cyclo(u));
    return k === "dwell-high" ? h : 0;
  }
  return 0;
}

// ── Pitch curve and cam profile ─────────────────────────────────────
const C = [0, 0];
const pitch = (t) => polar(C, Rb + Rr + s(t), 90 + t);
const N = 144, pitchPts = [], profPts = [];
let maxPA = 0;
for (let i = 0; i < N; i++) {
  const t = (360 * i) / N;
  const p = pitch(t), q0 = pitch(t - 0.25), q1 = pitch(t + 0.25);
  const tan = unit(sub(q1, q0));
  const nIn = perp(tan);                       // left of the CCW tangent = toward the centre
  pitchPts.push(p);
  profPts.push(add(p, mul(nIn, Rr)));
  // pressure angle between the follower axis (radial) and the normal
  const radial = unit(sub(p, C));
  maxPA = Math.max(maxPA, (Math.acos(Math.min(1, Math.abs(radial[0] * nIn[0] + radial[1] * nIn[1]))) * 180) / Math.PI);
}
cad.addSpline(profPts, { fit: true, closed: true, layer: "CAM" });
cad.addSpline(pitchPts, { fit: true, closed: true, layer: "PITCH" });
cad.addCircle(C, Rb, { layer: "THIN", linetype: "HID" });
draft.centerMark(C, Rb + h + 4, { ext: 2 });

// Bore Ø16 with keyway 5 x 2.3 (bore trimmed between the key sides)
const R0 = 8, kb = 5, kt = 2.3;
const k1 = cad.addLine([-kb / 2, 0], [-kb / 2, R0 + kt], { layer: "CAM" });
const k2 = cad.addLine([kb / 2, 0], [kb / 2, R0 + kt], { layer: "CAM" });
cad.addLine([-kb / 2, R0 + kt], [kb / 2, R0 + kt], { layer: "CAM" });
const boreArc = cad.trimTo(cad.addCircle(C, R0, { layer: "CAM" }), [k1, k2], [0, -R0]);
cad.trimTo(k1, boreArc, [-kb / 2, R0 + kt]);
cad.trimTo(k2, boreArc, [kb / 2, R0 + kt]);
cad.addCircle(C, 16, { layer: "THIN" });      // hub

// Transition rays + angle dimensions (laid out CCW from the follower axis)
for (const t of [0, 120, 180, 300]) cad.addLine(polar(C, R0 + 2, 90 + t), polar(C, 61, 90 + t), { layer: "CENTER" });
// Ray points on the dimension radius itself, so no extension lines cross the cam.
const angDim = (a, b, label, r) => draft.withOutlines(cad.dimAngle(C, polar(C, r, 90 + a), polar(C, r, 90 + b), { radius: r, text_height: 3, layer: "DIM", label }));
angDim(0, 120, "RISE 120°", 58);
angDim(120, 180, "60°", 58);
angDim(180, 300, "RETURN 120°", 58);
angDim(300, 360, "60°", 58);

// Rotation arrow (clockwise)
const ra = 22;
cad.addArc(C, ra, { from: 200, to: 250, layer: "DIM" });
const tA = (200 * Math.PI) / 180;            // clockwise tangent at the arrow end
draft.arrow(polar(C, ra, 200), [Math.sin(tA), -Math.cos(tA)], { layer: "DIM", size: 3 });
text("CW", polar(C, ra - 7, 228), { h: 2.5, align: "center", layer: "DIM" });

// ── Follower at t = 0 (on the base circle) + ghost at full lift ─────
const yR = Rb + Rr;                            // roller centre at t = 0
cad.addCircle([0, yR], Rr, { layer: "CAM" });
cad.addCircle([0, yR], 2.5, { layer: "CAM" });
cad.addCircle([0, yR + h], Rr, { layer: "PHANTOM" });
const sw = 5;                                  // stem half-width
cad.addLine([-sw, yR + Rr - 0.5], [-sw, yR + 50], { layer: "CAM" });
cad.addLine([sw, yR + Rr - 0.5], [sw, yR + 50], { layer: "CAM" });
cad.addLine([-sw, yR + 50], [sw, yR + 50], { layer: "CAM" });
cad.addLine([0, yR - Rr - 4], [0, yR + 54], { layer: "CENTER" });
// Guide bushing (section), both sides of the stem
for (const sgn of [-1, 1]) {
  const g = [[sgn * sw, yR + 30], [sgn * (sw + 8), yR + 30], [sgn * (sw + 8), yR + 44], [sgn * sw, yR + 44]];
  cad.addPolyline(g, { closed: true, layer: "CAM" });
  draft.hatch(g, { angle: 45, spacing: 1.5 });
}
draft.leader([sw + 8, yR + 40], [sw + 22, yR + 48], "GUIDE", { h: 2.5, layer: "TEXT" });
dim([0, yR], [0, yR + h], -16, { dir: "v", label: "LIFT <>", h: 2.5 });

// ── Displacement diagram ────────────────────────────────────────────
const X0 = P(145 - 70), Y0 = P(98 - 85), kx = 0.38, ky = 2;   // mm per deg, mm per mm
const gx = (t) => X0 + t * kx, gy = (v) => Y0 + v * ky;
for (let t = 0; t <= 360; t += 30) cad.addLine([gx(t), gy(0)], [gx(t), gy(h)], { layer: "GRID" });
for (let v = 0; v <= h; v += 5) cad.addLine([gx(0), gy(v)], [gx(360), gy(v)], { layer: "GRID" });
cad.addLine([gx(0), gy(0)], [gx(372), gy(0)], { layer: "THIN" });
cad.addLine([gx(0), gy(0)], [gx(0), gy(h) + 10], { layer: "THIN" });
draft.arrow([gx(372), gy(0)], [1, 0], { layer: "THIN", size: 2.5 });
draft.arrow([gx(0), gy(h) + 10], [0, 1], { layer: "THIN", size: 2.5 });
for (let t = 0; t <= 360; t += 90) text(String(t), [gx(t), gy(0) - 5], { h: 2.5, align: "center" });
text("CAM ANGLE (deg)", [gx(360), gy(0) - 10], { h: 2.5, align: "right" });
text("s (mm)", [gx(0) - 2, gy(h) + 12], { h: 2.5, align: "right" });
for (const v of [0, 10, 20]) text(String(v), [gx(0) - 2, gy(v) - 1.2], { h: 2.5, align: "right" });
const curve = [];
for (let t = 0; t <= 360; t += 2.5) curve.push([gx(t), gy(s(t))]);
cad.addPolyline(curve, { layer: "CURVE" });
segs.forEach(([a, b, k]) => text(k.startsWith("dwell") ? "DWELL" : k.toUpperCase(), [gx((a + b) / 2), gy(h) + 3], { h: 2.3, align: "center" }));
text("DISPLACEMENT DIAGRAM", [gx(0), gy(h) + 22], { h: 3.5 });

// ── Data ────────────────────────────────────────────────────────────
draft.table([X0, P(72 - 85)], [
  ["SEGMENT", "ANGLE", "LAW"],
  ["RISE 20", "0-120", "CYCLOIDAL"],
  ["DWELL", "120-180", "-"],
  ["RETURN 20", "180-300", "CYCLOIDAL"],
  ["DWELL", "300-360", "-"],
], [28, 22, 26], { title: "MOTION", h: 2.3, rowH: 5 });
text(`BASE CIRCLE R${Rb}  ROLLER Ø${2 * Rr}  MAX PRESSURE ANGLE ${maxPA.toFixed(1)}°`, [X0, P(36 - 85)], { h: 2.3 });

return { maxPressureAngle: maxPA };
