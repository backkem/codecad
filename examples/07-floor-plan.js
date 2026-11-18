// 07 - Apartment floor plan, 1 bedroom + study, scale 1:50. Every room
// opens off the entrance hall.
//
// Walls are modelled as solids: the 300 mm external walls and 100 mm
// partitions are rectangles fused with a region union, and the door and
// window openings are cut out with a region subtract, so every T-junction
// and jamb comes out clean and the cut walls can be hatched. The rooms
// are the holes of the fused wall region, which also gives their
// areas for the labels and the schedule. Doors are one block, placed
// with rotation and negative scale for handing. Furniture, chained
// dimensions, a north arrow and a room schedule finish the sheet.
//
// Exercises: regionClip (union, subtract) with areas, addHatch with
// holes, defineBlock + place (rotation, mirrored scale), measure chains,
// addFill, table, title block.

const draft = await cad.use("drafting");
const { P, dim, text } = draft;
const { ACI } = draft;
draft.setup({
  scale: 50,
  layers: {
    WALL: { color: ACI.white, lineweight: 0.5 },
    DOOR: { color: ACI.yellow, lineweight: 0.25 },
    WINDOW: { color: ACI.cyan, lineweight: 0.25 },
    FURN: { color: ACI.gray, lineweight: 0.18 },
    ROOM: { color: ACI.white, lineweight: 0.25 },
    DIM: { color: ACI.green, lineweight: 0.18 },
  },
});

draft.sheet({
  at: [-P(32), -P(30)], w: 297, h: 189,
  title: "APARTMENT A1 - PLAN",
  subtitle: "1 BEDROOM + STUDY, GROUND FLOOR",
  number: "CC-007", scale: "1:50",
});

// ── Wall solids (mm) ────────────────────────────────────────────────
const W = 9000, H = 6000, te = 300, ti = 100;
const rect = (x0, y0, x1, y1) => [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
const walls = [
  rect(0, 0, W, te), rect(0, H - te, W, H), rect(0, 0, te, H), rect(W - te, 0, W, H), // external
  rect(5300, 2400, 5300 + ti, H - te),     // V1: living | bedroom
  rect(te, 2400, 5300, 2400 + ti),          // H1: living | bath/hall
  rect(2700, te, 2700 + ti, 2400),          // V2: bath | hall
  rect(5300, 2400, W - te, 2400 + ti),      // H2: bedroom | hall/study
  rect(6400, te, 6400 + ti, 2400),          // V3: hall | study
];

// Openings: [kind, wall-face rectangle]
const openings = {
  // windows
  W1: ["win", rect(1200, H - te, 3200, H), "h"],
  W2: ["win", rect(0, 3300, te, 4900), "v"],
  W3: ["win", rect(6200, H - te, 7900, H), "h"],
  W4: ["win", rect(7000, 0, 8200, te), "h"],
  W5: ["win", rect(1000, 0, 1700, te), "h"],
  // doors (clear openings)
  D1: ["door", rect(3700, 0, 4600, te)],
  D2: ["door", rect(3200, 2400, 4000, 2400 + ti)],
  D3: ["door", rect(2700, 1100, 2700 + ti, 1800)],
  D4: ["door", rect(6400, 600, 6400 + ti, 1400)],
  D5: ["door", rect(5500, 2400, 6300, 2400 + ti)],
};
const grow = (r, d) => { const [a, , c] = [r[0], r[1], r[2]]; return rect(a[0] - d[0], a[1] - d[1], c[0] + d[0], c[1] + d[1]); };
// Cut slightly beyond the wall faces so the subtraction is clean.
const cuts = Object.values(openings).map(([, r, o]) => {
  const w = r[2][0] - r[0][0], h = r[2][1] - r[0][1];
  return grow(r, w < h ? [10, 0] : [0, 10]);
});

const solid = cad.regionClip(walls, [], "union");
const cutWalls = cad.regionClip(walls, cuts, "subtract");   // overlapping rects merge (NonZero)
for (const r of cutWalls) {
  cad.addPolyline(r.outer, { closed: true, layer: "WALL" });
  r.holes.forEach((h) => cad.addPolyline(h, { closed: true, layer: "WALL" }));
  draft.hatch(r.outer, { holes: r.holes, angle: 45, spacing: 1.2, layer: "HATCH" });
}

// ── Windows: reveal lines at the wall faces + double glazing line ───
for (const [kind, r, o] of Object.values(openings)) {
  if (kind !== "win") continue;
  const [x0, y0] = r[0], [x1, y1] = r[2];
  if (o === "h") {
    const ym = (y0 + y1) / 2;
    for (const y of [y0, y1]) cad.addLine([x0, y], [x1, y], { layer: "WINDOW" });
    for (const y of [ym - 30, ym + 30]) cad.addLine([x0, y], [x1, y], { layer: "WINDOW" });
  } else {
    const xm = (x0 + x1) / 2;
    for (const x of [x0, x1]) cad.addLine([x, y0], [x, y1], { layer: "WINDOW" });
    for (const x of [xm - 30, xm + 30]) cad.addLine([x, y0], [x, y1], { layer: "WINDOW" });
  }
}

// ── Doors: one block, hinge at the origin, opening along +x, leaf
// opened 90 deg to +y. Rotation + negative scale give the handing.
cad.defineBlock("DOOR", [
  { type: "polyline", points: [[0, 0], [40, 0], [40, 1000], [0, 1000]], closed: true },
  { type: "arc", center: [0, 0], radius: 1000, from: 0, to: 90 },
], { default_layer: "DOOR" });
// (cad.mirror on a block insert only moves its insertion point, so the
// handing is done with negative scale instead.)
const door = (at, width, rotation, fy = 1, fx = 1) =>
  cad.place("DOOR", { at, rotation, scale: [(fx * width) / 1000, (fy * width) / 1000] });
door([3700, te], 900, 0);            // D1 entry, into the hall
door([3200, 2400 + ti], 800, 0);     // D2 hall -> living
door([2700, 1800], 700, -90, -1);    // D3 hall -> bath
door([6400 + ti, 600], 800, 90, -1); // D4 hall -> study
door([5500, 2400 + ti], 800, 0);     // D5 hall -> bedroom

// ── Furniture & fittings ────────────────────────────────────────────
cad.defineBlock("BED_DOUBLE", [
  { type: "polyline", points: rect(0, 0, 2000, 1600), closed: true },
  { type: "polyline", points: rect(60, 120, 480, 760), closed: true },
  { type: "polyline", points: rect(60, 840, 480, 1480), closed: true },
  { type: "line", start: [700, 0], end: [700, 1600] },
], { default_layer: "FURN" });
cad.defineBlock("WC", [
  { type: "polyline", points: rect(0, -250, 200, 250), closed: true },
  { type: "line", start: [200, -180], end: [430, -180] },
  { type: "line", start: [200, 180], end: [430, 180] },
  { type: "arc", center: [430, 0], radius: 180, from: -90, to: 90 },
], { default_layer: "FURN" });
cad.defineBlock("CHAIR", [{ type: "polyline", points: rect(-220, -220, 220, 220), closed: true }, { type: "line", start: [-220, 160], end: [220, 160] }], { default_layer: "FURN" });

const f = (pts) => cad.addPolyline(pts, { closed: true, layer: "FURN" });
// Living / kitchen
f(rect(3300, 5100, 5300, H - te));                          // worktop along north wall
f(rect(4700, 3600, 5300, 5100));                            // worktop return along V1
f(rect(3500, 5180, 3900, 5620)); f(rect(3950, 5180, 4350, 5620));   // double sink
[[4980, 4500], [4980, 4800], [4980, 3900], [4980, 4200]].forEach((c, i) => cad.addCircle(c, i % 2 ? 110 : 90, { layer: "FURN" })); // hob
f(rect(te + 50, 3200, te + 950, 5000)); f(rect(te + 50, 3200, te + 250, 5000)); // sofa + back
f(rect(1600, 3700, 2200, 4500));                            // coffee table
f(rect(3100, 3800, 4300, 4500));                            // dining table
[[3400, 3580], [4000, 3580]].forEach((c) => cad.place("CHAIR", { at: c, rotation: 180 }));
[[3400, 4720], [4000, 4720]].forEach((c) => cad.place("CHAIR", { at: c }));
// Bedroom
cad.place("BED_DOUBLE", { at: [W - te, 4900], rotation: 180 });
f(rect(6500, 2500, 8650, 3100)); cad.addLine([6500, 2800], [8650, 2800], { layer: "FURN", linetype: "HID" }); // wardrobe
// Study
f(rect(8100, 600, W - te, 2000)); cad.place("CHAIR", { at: [7750, 1300], rotation: 90 });
// Bath
f(rect(te, te, 2000, 1000)); f(rect(te + 80, te + 80, 1920, 920)); cad.addCircle([1780, 650], 40, { layer: "FURN" });
cad.place("WC", { at: [te, 1650] });
f(rect(1300, 1950, 1900, 2400)); cad.addCircle([1600, 2160], 170, { layer: "FURN" });

// ── Rooms: the holes of the fused (uncut) wall region -> areas ──────
const rooms = solid.flatMap((r) => r.holes).map((h) => ({ outer: h, area: cad.regionClip(h, h, "intersect")[0].area }));
const names = [
  ["LIVING / KITCHEN", [1500, 4300 + 900]],
  ["BEDROOM", [6900, 4400 + 600]],
  ["STUDY", [7300, 1900]],
  ["BATH", [1400, 1600]],
  ["HALL", [4900, 1500]],
];
const schedule = [];
for (const [name, at] of names) {
  const room = rooms.find((r) => {
    const xs = r.outer.map((p) => p[0]), ys = r.outer.map((p) => p[1]);
    return at[0] > Math.min(...xs) && at[0] < Math.max(...xs) && at[1] > Math.min(...ys) && at[1] < Math.max(...ys);
  });
  const area = room.area / 1e6;
  schedule.push([name, area.toFixed(1)]);
  text(name, at, { h: 3, align: "center", layer: "ROOM" });
  text(`${area.toFixed(1)} m²`, [at[0], at[1] - P(5)], { h: 2.5, align: "center", layer: "ROOM" });
}

// ── Dimensions: chained internal layout + overall ───────────────────
const chainN = [0, te, 5300, 5300 + ti, W - te, W];
// Narrow 100 mm partition dims: text shifted clear of the extension lines.
const narrow = (a, b) => (b - a <= 100 ? { shift: 8 } : {});
chainN.slice(0, -1).forEach((x, i) => dim([x, H], [chainN[i + 1], H], 8, { dir: "h", h: 2.5, ...narrow(x, chainN[i + 1]) }));
dim([0, H], [W, H], 16, { dir: "h", h: 2.5 });
const chainW = [0, te, 2400, 2400 + ti, H - te, H];
chainW.slice(0, -1).forEach((y, i) => dim([0, y], [0, chainW[i + 1]], 8, { dir: "v", h: 2.5, ...narrow(y, chainW[i + 1]) }));
dim([0, 0], [0, H], 16, { dir: "v", h: 2.5 });

// ── North arrow ─────────────────────────────────────────────────────
const na = [P(222), P(135)], nr = P(8);
cad.addCircle(na, nr, { layer: "THIN" });
const tri = [[na[0], na[1] + nr], [na[0] - nr * 0.35, na[1] - nr * 0.6], [na[0], na[1] - nr * 0.3]];
cad.addFill(tri, { layer: "TEXT" }); cad.addPolyline(tri, { closed: true, layer: "TEXT" });
cad.addPolyline([[na[0], na[1] + nr], [na[0] + nr * 0.35, na[1] - nr * 0.6], [na[0], na[1] - nr * 0.3]], { layer: "TEXT" });
text("N", [na[0], na[1] + nr + P(2)], { h: 4, align: "center" });

// ── Room schedule ───────────────────────────────────────────────────
const total = schedule.reduce((s, [, a]) => s + Number(a), 0);
draft.table([P(205), P(110)], [["ROOM", "AREA m²"], ...schedule, ["TOTAL (NET)", total.toFixed(1)]], [30, 18],
  { title: "ROOM SCHEDULE", h: 2.3, rowH: 5.5, align: [null, "right"] });
text("WALLS: EXT 300, INT 100", [P(205), P(56)], { h: 2.2 });
text("DOOR WIDTHS = CLEAR OPENING", [P(205), P(51)], { h: 2.2 });

return { rooms: schedule, total };
