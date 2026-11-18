// 12 - Projected views: a shop drawing of a pillow block.
//
// A cast pillow block of our own design: a 140 x 60 x 14 base with R8
// corners and two counterbored Ø11 bolt holes, an upright with a
// semicircular top, a Ø32 bore with 2 mm chamfers, and a Ø6 oil hole from
// the top into the bore. Then the drawing: front, top and left views in
// first-angle order and an isometric view, each from cad.project (exact
// lines, arcs, circles and ellipses; hidden edges dashed on their own
// layer), on an A3 sheet with a title block and a few dimensions placed
// through the view mapping (lib/views.js toSheet). Written as a 2D PNG and
// a PDF.
//
// The oil hole meets the bore in a curve that is not a conic (where two
// cylinders cross); its views are fitted cubic splines.
//
// x = width, y = depth, z = up; the bore runs along y.

const sh = await cad.use("shapes");
const ck = await cad.use("checks");
const vw = await cad.use("views");
const run = ck.start("12-projected-views");

// ── The part (mm) ────────────────────────────────────────────────────
const L = 140, D = 60, T = 14;     // base
const W = 70, B = 40;              // upright width and depth
const AXIS = 50, BORE = 32;        // bore axis height, bore diameter
const R = W / 2;                   // top of the upright: a half circle about the bore axis
const top = AXIS + R;

const block = sh.box([-L / 2, -D / 2, 0], [L / 2, D / 2, T], {
  name: "pillow block", attrs: { part: "pillow block", material: "cast iron" },
});
// Round the four vertical corners of the base.
cad.fillet3d(block, {
  edges: { near: [[-L / 2, -D / 2, T / 2], [L / 2, -D / 2, T / 2], [L / 2, D / 2, T / 2], [-L / 2, D / 2, T / 2]] },
  radius: 8,
});
// The upright: a profile in the xz plane, swept 40 along -y from y = 20.
cad.union(block, {
  extrude: [[-R, T], [R, T], [R, AXIS, 1], [-R, AXIS]],
  plane: "xz", at: [0, B / 2, 0], distance: B,
});
for (const x of [-50, 50]) cad.hole(block, { at: [x, 0, T], dia: 11, cbore: [18, 6] });
cad.hole(block, { at: [0, -B / 2, AXIS], dir: [0, 1, 0], dia: BORE });
// The oil hole, 8 mm off the centre line: it starts above the arch and
// ends inside the bore. (On the centre line, where its axis would meet the
// bore's, arris 0.11.1 refuses the cut.)
const OIL = 8;
cad.hole(block, { at: [OIL, 0, top + 5], dia: 6, depth: R - BORE / 2 + 11 });
cad.chamfer3d(block, {
  edges: { near: [[0, -B / 2, AXIS + BORE / 2], [0, B / 2, AXIS + BORE / 2]] }, d: 2,
});

const part = cad.part(block.id);
run.box("part box", part.bbox, { min: [-L / 2, -D / 2, 0], max: [L / 2, D / 2, top] });

// ── The sheet: A3 landscape, 1:1, first angle ────────────────────────
vw.setup();
vw.sheet({ title: "PILLOW BLOCK", note: "cast iron, all chamfers 2 x 45", scale: "1:1" });
const box = vw.boxOf([part]);
const at = vw.arrange(box, { front: [120, 185], gap: 25 });
const t0 = Date.now();
const views = {};
const ms = {};
for (const name of ["front", "top", "left"]) {
  const t = Date.now();
  views[name] = vw.view({ view: name, at: at[name], name: `view-${name}` });
  ms[name] = Date.now() - t;
}
{
  const t = Date.now();
  views.iso = vw.view({ view: "iso", at: [335, 178], scale: 0.8, name: "view-iso" });
  ms.iso = Date.now() - t;
}
const projectMs = Date.now() - t0;
// Labels go below the dimensions under each view.
vw.label(views.front, "FRONT", { below: 22 });
vw.label(views.top, "TOP", { below: 22 });
vw.label(views.left, "LEFT", { below: 22 });
vw.label(views.iso, "ISOMETRIC (0.8 : 1)");

// ── Dimensions on the views, placed through toSheet ──────────────────
const f = (p) => vw.toSheet(views.front, p);
const dim = { text_height: 3.5, layer: "dims", decimals: 0 };
cad.measure(f([-L / 2, 0, 0]), f([L / 2, 0, 0]), { ...dim, offset: -10 });
cad.measure(f([L / 2, 0, 0]), f([L / 2, 0, top]), { ...dim, dir: "v", offset: 12 });
cad.measure(f([-L / 2, 0, 0]), f([-L / 2, 0, AXIS]), { ...dim, dir: "v", offset: -12 });
cad.dimDiameter({ center: f([0, 0, AXIS]), radius: BORE / 2 }, { ...dim, angle: 45 });
const tp = (p) => vw.toSheet(views.top, p);
cad.measure(tp([-50, 0, T]), tp([50, 0, T]), { ...dim, offset: -40 });
const lf = (p) => vw.toSheet(views.left, p);
cad.measure(lf([0, D / 2, 0]), lf([0, -D / 2, 0]), { ...dim, offset: -10 });

// ── Checks on what the views hold ────────────────────────────────────
const kids = (v) => cad.children(v.insert);
const near = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]) <= 1e-9;
const front = kids(views.front);
const bore = front.filter((e) => e.type === "circle" && Math.abs(e.radius - BORE / 2) <= 1e-9);
run.equal("front view: the bore is one visible circle", bore.length, 1);
run.ok("front view: the bore's circle is where toSheet puts the axis",
  near(bore[0].center, f([0, 0, AXIS])), JSON.stringify(bore[0]));
run.ok("front view: the chamfers show as circles of radius 18",
  front.some((e) => e.type === "circle" && Math.abs(e.radius - (BORE / 2 + 2)) <= 1e-9));
const hiddenTop = kids(views.top).filter((e) => e.attrs?.edge === "hidden");
run.ok("top view: the bore and the oil hole are hidden lines", hiddenTop.length >= 4,
  `${hiddenTop.length} hidden`);
const iso = kids(views.iso);
run.ok("isometric view: holes and bore rims are ellipses",
  iso.filter((e) => e.type === "ellipse").length >= 6);
const types = new Set(Object.values(views).flatMap((v) => kids(v).map((e) => e.type)));
run.ok("views hold only lines, arcs, circles, ellipses and fitted splines",
  [...types].every((t) => ["line", "arc", "circle", "ellipse", "spline"].includes(t)),
  [...types].join(", "));
run.ok("every view entity names its part",
  Object.values(views).every((v) => kids(v).every((e) => e.attrs?.partId === block.id)));
run.ok("nothing open: every curved face has a closed-form silhouette",
  Object.values(views).every((v) => v.open.length === 0),
  JSON.stringify(Object.values(views).map((v) => v.open)));

// ── Output: a 2D PNG and a PDF ───────────────────────────────────────
const files = [];
let pdf = null;
let renderMs = 0;
if (globalThis.EXAMPLE_RENDER !== false) {
  const dir = globalThis.EXAMPLE_RENDER_DIR;
  const out = `${dir ?? "examples/3d"}/12-projected-views.png`;
  const r = await cad.render({
    mode: "2d", size: [960, 640], out, bounds: { min: [0, 0], max: [420, 297] },
  });
  renderMs = r.ms;
  files.push(out);
  pdf = `${dir ?? "target/3d-views"}/12-projected-views.pdf`;
  await cad.savePdf(pdf, { colors: "paper", lineweights: true });
  // For the site: the sheet as a DWG and the part as a prebaked scene.
  await run.drawing();
  await run.bake({ view: "iso", ground: true });
}

const summary = run.summary({
  views: Object.fromEntries(Object.entries(views).map(([k, v]) => [k, { entities: v.entities, counts: v.counts }])),
  project: { ms, total: projectMs },
  pdf,
});
summary.files = files;
summary.ms.render = Math.round(renderMs * 10) / 10;
return summary;
