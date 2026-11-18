// 09 - Meshing spur gear pair, module 3, 18 / 30 teeth, 20 deg pressure
// angle, at the standard centre distance a = m(z1 + z2) / 2 = 72 mm.
//
// Each gear is built from ONE tooth + ONE root gap: involute flanks (the
// only computed curve, sampled from the involute function and fitted with
// a spline), tip arc, radial flank below the base circle, root arc and
// two true root fillets made with cad.fillet. That master is repeated
// with a polar array. The line of action is the inner common tangent of
// the two base circles (cad.tangentLines) and passes through the pitch
// point. Bores with keyways are cut with trimTo; the wheel gets
// lightening holes by polar array. A gear data table completes the sheet.
//
// Exercises: addSpline (fit), fillet (line-arc), array (polar),
// tangentLines, trimTo, dimDiameter, measure, table.

const draft = await cad.use("drafting");
const { P, dim, text, polar } = draft;
const { ACI } = draft;
draft.setup({
  scale: 1,
  layers: {
    GEAR: { color: ACI.white, lineweight: 0.5 },
    PITCH: { color: ACI.red, linetype: "CL", lineweight: 0.18 },
    BASE: { color: ACI.gray, linetype: "PHAN", lineweight: 0.18 },
    ACTION: { color: ACI.amber, linetype: "PHAN", lineweight: 0.25 },
  },
});

draft.sheet({
  at: [-P(55), -P(105)], w: 297, h: 189,
  title: "SPUR GEAR PAIR m3 18/30",
  subtitle: "INVOLUTE, 20 DEG PRESSURE ANGLE",
  number: "CC-009", scale: "1:1",
});

const m = 3, alpha = (20 * Math.PI) / 180;
const inv = (a) => Math.tan(a) - a;
const deg = (r) => (r * 180) / Math.PI;

// Build one gear: master tooth + gap, then polar array.
function gear(center, z, toothAtDeg) {
  const r = (m * z) / 2, ra = r + m, rf = r - 1.25 * m, rb = r * Math.cos(alpha);
  const halfBase = Math.PI / (2 * z) + inv(alpha);           // half tooth angle at the base circle
  const pc = (toothAtDeg * Math.PI) / 180;
  const flank = (side) => {                                   // side +1 = left (CCW) flank
    const pts = [];
    for (let i = 0; i <= 12; i++) {
      const rho = rb + ((ra - rb) * i) / 12;
      const a = Math.acos(rb / rho);
      pts.push(polar(center, rho, deg(pc + side * (halfBase - inv(a)))));
    }
    return pts;
  };
  const right = flank(-1), left = flank(+1);
  const ids = [];
  ids.push(cad.addSpline(right, { fit: true, layer: "GEAR" }).id);
  ids.push(cad.addSpline(left, { fit: true, layer: "GEAR" }).id);
  ids.push(cad.addArc(center, ra, { p1: right[12], p2: left[12], layer: "GEAR" }).id);
  // Below the base circle the flank continues radially to the root.
  const aL = deg(pc + halfBase), aR2 = deg(pc + (2 * Math.PI) / z - halfBase);
  const radL = cad.addLine(polar(center, rb, aL), polar(center, rf, aL), { layer: "GEAR" });
  const radR = cad.addLine(polar(center, rb, aR2), polar(center, rf, aR2), { layer: "GEAR" });
  const root = cad.addArc(center, rf, { from: aL, to: aR2, layer: "GEAR" });
  const f1 = cad.fillet(radL, root, 0.25 * m);
  const f2 = cad.fillet(radR, root, 0.25 * m);
  ids.push(radL.id, radR.id, root.id, f1.arc.id, f2.arc.id);
  cad.array(ids, { polar: { center, count: z } });

  cad.addCircle(center, r, { layer: "PITCH" });
  cad.addCircle(center, rb, { layer: "BASE" });
  draft.centerMark(center, ra, { ext: 4 });
  return { r, ra, rf, rb };
}

// Bore with a DIN 6885 keyway: the bore circle is trimmed back between
// the keyway sides.
function bore(center, d, b, t2) {
  const R = d / 2;
  const top = center[1] + R + t2;
  const s1 = cad.addLine([center[0] - b / 2, center[1]], [center[0] - b / 2, top], { layer: "GEAR" });
  const s2 = cad.addLine([center[0] + b / 2, center[1]], [center[0] + b / 2, top], { layer: "GEAR" });
  cad.addLine([center[0] - b / 2, top], [center[0] + b / 2, top], { layer: "GEAR" });
  const c = cad.addCircle(center, R, { layer: "GEAR" });
  const arc = cad.trimTo(c, [s1, s2], [center[0], center[1] - R]);
  cad.trimTo(s1, arc, [center[0] - b / 2, top]);
  cad.trimTo(s2, arc, [center[0] + b / 2, top]);
  return c;
}

const z1 = 18, z2 = 30, a = (m * (z1 + z2)) / 2;
const C1 = [0, 0], C2 = [a, 0];
const g1 = gear(C1, z1, 0);                      // pinion: a tooth points at the mesh
const g2 = gear(C2, z2, 180 + 180 / z2);         // wheel: a gap faces the pinion tooth

const b1 = bore(C1, 16, 5, 2.3);
const b2 = bore(C2, 25, 8, 3.3);
// Wheel web: hub + lightening holes (one hole + its centre mark, arrayed)
cad.addCircle(C2, 20, { layer: "THIN" });
cad.addCircle(C2, 36, { layer: "THIN" });
const hc = polar(C2, 28, 30);
const hole = cad.addCircle(hc, 4.5, { layer: "GEAR" });
cad.array([hole], { polar: { center: C2, count: 6 } });
cad.addCircle(C2, 28, { layer: "PITCH" });

// Line of action: inner common tangent of the base circles, through the
// pitch point (left-to-right falling one, the usual drawing).
const tl = cad.tangentLines(C1, g1.rb, C2, g2.rb, "inner");
const [t1, t2] = tl.sort((p, q) => q[0][1] - p[0][1])[0];
const ext = (p, q, k) => [p[0] + (p[0] - q[0]) * k, p[1] + (p[1] - q[1]) * k];
cad.addLine(ext(t1, t2, 0.9), ext(t2, t1, 0.9), { layer: "ACTION" });
cad.addLine([C1[0] - g1.ra - P(4), 0], [C2[0] + g2.ra + P(4), 0], { layer: "CENTER" });
const loa = [ext(t1, t2, 0.9), ext(t2, t1, 0.9)].sort((p, q) => q[1] - p[1])[0];   // upper end
draft.leader(loa, [loa[0] - 8, loa[1] + 10], "LINE OF ACTION 20°", { h: 2.5, arrow: false, layer: "TEXT" });

// ── Dimensions ──────────────────────────────────────────────────────
dim([C1[0], -g2.ra], [C2[0], -g2.ra], -6, { dir: "h", label: "a = <>" });
draft.withOutlines(cad.dimDiameter({ center: C1, radius: g1.ra }, { angle: 150, label: "Ø<>", text_height: 3.5, layer: "DIM" }));
draft.withOutlines(cad.dimDiameter({ center: C2, radius: g2.ra }, { angle: 30, label: "Ø<>", text_height: 3.5, layer: "DIM" }));
draft.leader(polar(C1, 8, 240), [-14, -42], "Ø16 H7\nKEY 5x5", { h: 2.5 });
draft.leader(polar(C2, 12.5, 300), [a + 30, -52], "Ø25 H7\nKEY 8x7", { h: 2.5 });

// ── Gear data ───────────────────────────────────────────────────────
const f1 = (v) => v.toFixed(2).replace(/\.00$/, "");
draft.table([P(200 - 55), P(170 - 105)], [
  ["", "PINION", "WHEEL"],
  ["MODULE m", "3", "3"],
  ["TEETH z", String(z1), String(z2)],
  ["PRESSURE ANGLE", "20°", "20°"],
  ["PITCH DIA d", f1(2 * g1.r), f1(2 * g2.r)],
  ["TIP DIA da", f1(2 * g1.ra), f1(2 * g2.ra)],
  ["ROOT DIA df", f1(2 * g1.rf), f1(2 * g2.rf)],
  ["BASE DIA db", f1(2 * g1.rb), f1(2 * g2.rb)],
  ["FACE WIDTH", "30", "25"],
  ["CENTRE DIST. a", "72", ""],
  ["RATIO z2/z1", (z2 / z1).toFixed(3), ""],
], [32, 22, 22], { title: "GEAR DATA", h: 2.3, rowH: 5.2, align: [null, "right", "right"] });

return { pinion: g1, wheel: g2 };
