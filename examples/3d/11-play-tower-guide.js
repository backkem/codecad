// 11 - Play tower with slide and swings: the construction guide.
//
// Builds the model of 11-play-tower.js (lib/playtower.js), makes its part
// list (lib/partlist.js) and draws one step of the assembly order per panel
// (lib/guide.js, generic over parts and attributes): an isometric view from
// cad.project of everything built so far, the parts this step adds drawn
// strong and labelled with their count, section and length from the part
// list, markers where this step's fixings go, a table of those fixings and
// one line of instruction written from the step's data. Three steps get
// larger detail views: the stringer's foot and head, the rafter seat on
// the wall plate, the swing hangers. Two steps per sheet, six sheets, each
// saved as a PDF page and a 2D PNG; the first sheet is the example's
// picture (the others and the PDFs go to target/3d-views/).
//
// What this script decides, and the guide library does not: the words (the
// instructions, the label lines), which steps get a detail and what it
// shows, and where on each part a fixing marker goes.
//
// Checks: every part appears in exactly one step (and in its step's view),
// every label's count equals the part list's count for that step, the
// fixings over all steps add up to the part list's total, every sheet's
// content stays inside its frame and clear of the title block, no two
// labels overlap and no label covers the drawing.

const tower = await cad.use("playtower");
const pl = await cad.use("partlist");
const sheet = await cad.use("sheet");
const guide = await cad.use("guide");
const ck = await cad.use("checks");
const run = ck.start("11-play-tower-guide");
const T0 = Date.now();

// ── The model, its contacts and its list ─────────────────────────────
const m = tower.build();
const parts = cad.parts();
const byId = new Map(parts.map((p) => [p.id, p]));
const tContacts = Date.now();
const contacts = cad.pairsWithin3d(parts.map((p) => p.id), null, 1e-6, { attrs: true });
const contactMs = Date.now() - tContacts;
const list = pl.list({ stock: m.stock, fixings: m.fixings, steps: m.steps, contacts });
run.ok("every contact was measured", contacts.skipped.length === 0, JSON.stringify(contacts.skipped));
run.ok("every member has an assembly step", list.unplaced.length === 0, list.unplaced.join(", "));
run.ok("every joint has its fixings, or a rule that it needs none", list.unfixed.length === 0, list.unfixed.join("; "));

const member = (p) => p.attrs.member;
const asList = (v) => (Array.isArray(v) ? v : [v]);
const stepOf = new Map();
m.steps.forEach((s, i) => s.members.forEach((mb) => stepOf.set(mb, i)));

// ── Joints and their fixings, per step ───────────────────────────────
// The same rules and the same step for each joint as the part list (the
// later of its two parts' steps), plus where it is: the middle of the
// overlap of the two parts' boxes, which for two touching boxes is the
// middle of the face they share.
const matches = (r, ka, kb) => r.a !== undefined &&
  ((ka === r.a && asList(r.b).includes(kb)) || (kb === r.a && asList(r.b).includes(ka)));
const ruleOf = (ka, kb) => m.fixings.find((r) => matches(r, ka, kb));
const joints = [];
for (const q of contacts.pairs) {
  const rule = ruleOf(q.aAttrs.member, q.bAttrs.member);
  if (!rule || !rule.items) continue;
  const A = byId.get(q.a).bbox, B = byId.get(q.b).bbox;
  const lo = [0, 1, 2].map((i) => Math.max(A.min[i], B.min[i])), hi = [0, 1, 2].map((i) => Math.min(A.max[i], B.max[i]));
  joints.push({
    a: q.a, b: q.b, rule, step: Math.max(stepOf.get(q.aAttrs.member), stepOf.get(q.bAttrs.member)),
    point: lo.map((v, i) => (v + hi[i]) / 2),
  });
}
// Per-part rules (a ground anchor under each post): at the part's foot.
for (const r of m.fixings.filter((r) => r.each)) {
  for (const p of parts.filter((p) => asList(r.each).includes(member(p)))) {
    joints.push({ a: p.id, b: null, rule: r, step: stepOf.get(member(p)), point: guide.footOf(p) });
  }
}

// ── Words ────────────────────────────────────────────────────────────
const plural = (n, w) => guide.plural(n, w, { same: ["sand"] });
const r0 = (v) => Math.round(v);
const ruleName = (r) => (r.each ? `each ${asList(r.each).join("/")}` : `${r.a} to ${asList(r.b).join("/")}`);

// Label text for a member: count and name; section and cut length (from the
// part list), or what to buy.
function labelLines(mb, ids) {
  const n = ids.length;
  const groups = list.timber.filter((g) => g.ids.some((id) => ids.includes(id)));
  const first = { text: `${n} ${plural(n, mb).toUpperCase()}`, h: 2.1, layer: "TEXT" };
  if (groups.length) {
    const lens = [...new Set(groups.map((g) => g.length))].sort((a, b) => b - a);
    const L = lens.length <= 3 ? `${lens.join(" / ")} long` : `${lens.length} lengths, ${lens[lens.length - 1]} to ${lens[0]}`;
    return [first, { text: `${groups[0].section}, ${L}`, h: 1.6, layer: "G_NOTE" }];
  }
  const b = list.bought.find((x) => x.members.includes(mb));
  const what = b.unit === "pcs" ? `${b.item}, bought` : `${b.qty.toFixed(2)} ${b.unit === "m3" ? "m³" : b.unit} ${b.item}`;
  return [first, { text: what, h: 1.6, layer: "G_NOTE" }];
}

// One line of instruction per step. The words are the script's; every
// count and fixing comes from the model's data: n(member) is "4 posts" (or
// "the tie" for one), fix(a, b) the items one joint of a and b takes.
function sayer(fresh) {
  const count = (mb) => fresh.filter((p) => member(p) === mb).length;
  const n = (mb) => (count(mb) === 1 ? `the ${mb}` : `${count(mb)} ${plural(count(mb), mb)}`);
  const fix = (a, b) => Object.entries(ruleOf(a, b).items).map(([it, k]) => `${k} ${plural(k, it)}`).join(" and ");
  return { n, fix };
}
const F = m.frame;
const SAY = [
  ({ n, fix }) => `Stand ${n("post")} in ground anchors. Bolt ${n("rim")} between them (${fix("rim", "post")} at each post) and hang ${n("joist")} in joist hangers.`,
  ({ n, fix }) => `Lay ${n("deck board")} across the joists, ${r0(F.gap)} mm apart, notched round the posts. Fix ${fix("deck board", "joist")} at every crossing.`,
  ({ n, fix }) => `Fix ${n("head rail")} between the right-hand posts, rest ${n("swing beam")} on it (${fix("swing beam", "head rail")}), stand ${n("leg")} under its end (${fix("swing beam", "leg")} each) and bolt on ${n("tie")}.`,
  ({ n, fix }) => `Bolt ${n("stringer")} to the left rim (${fix("stringer", "rim")} each), fix ${n("tread")} on stair angles (${fix("tread", "stringer")} per tread) and ${n("riser")} under the nosings.`,
  ({ n, fix }) => `Screw ${n("wall plate")} on the post tops (${fix("wall plate", "post")} per post). Set ${n("rafter")} on the plates, birdsmouth over the plate, heads against ${n("ridge")} (${fix("rafter", "wall plate")} per end).`,
  ({ n, fix }) => `Screw ${n("roof board")} across the rafters, eave to ridge, ${m.params.ROOF.board.gap} mm apart (${fix("roof board", "rafter")} per rafter). Screw ${n("ridge cap")} along the top.`,
  ({ n, fix }) => `Fix ${n("stud")} beside the openings (${fix("stud", "deck board")} at the foot), then ${n("wall board")} outside the posts, ${m.params.CLAD.gap} mm apart (${fix("wall board", "post")} per post or stud).`,
  ({ n, fix }) => `Bolt ${n("rail post")} to the front rim (${fix("rail post", "rim")}), fix ${n("top rail")} from it to the corner post and ${n("baluster")} on the outside (${fix("baluster", "rim")} at each rail).`,
  ({ n, fix }) => `Bolt ${n("stair post")} to the stringers (${fix("stair post", "stringer")} each), then ${n("handrail")} along them (${fix("handrail", "stair post")} per post, a bracket at the wall).`,
  ({ n, fix }) => `Screw ${n("sandbox board")} in two rows inside the posts (${fix("sandbox board", "post")} per joint) and fill with ${list.bought.find((b) => b.members.includes("sand")).qty.toFixed(2)} m³ of sand.`,
  ({ n, fix }) => `Set ${n("slide foot")} under the exit and screw ${n("slide")} to the deck (${fix("slide", "deck board")} per board it rests on).`,
  ({ n, fix }) => `Bolt ${n("hanger")} under the swing beam (${fix("hanger", "swing beam")} each), hang ${n("rope")} from them and tie on ${n("seat")} ${m.params.SWING.seatZ} mm above the ground.`,
];

// ── Details: a few parts, larger, cut to a window ────────────────────
const P = m.params, R = m.roof;
const find = (mb, test) => parts.filter((p) => member(p) === mb && test(p));
const DETAILS = {
  // The near stringer at its foot (cut level on the ground) and at its head
  // (cut plumb against the left rim and level with the deck), from the front.
  4: () => {
    const s = byId.get(m.parts.stringer.id);
    const rim = find("rim", (p) => p.bbox.max[0] <= P.JOIST[0] + 1e-6)[0];
    const top = find("tread", (p) => p.bbox.max[2] > P.DECK - 200)[0];
    const toe = s.bbox.min[0];
    const cuts = s.attrs.cut.split(", ");
    const th = (F.theta * Math.PI) / 180;
    return [{
      title: "Stringer foot", view: "front", parts: [s], size: [44, 40],
      window: { min: [toe - 60, -1e4, -40], max: [toe + 380, 1e4, 300] },
      note: `Cut ${cuts[0]} from square, flat on the ground.`,
      annotate: (dv) => {
        cad.addLine(dv.toSheet([toe - 60, 0, 0]), dv.toSheet([toe + 380, 0, 0]), { layer: "G_NOTE" });
        const a = dv.toSheet([toe, 0, 0]);
        cad.dimAngle(a, [a[0] + 10, a[1]], [a[0] + 10 * Math.cos(th), a[1] + 10 * Math.sin(th)], { radius: 12, text_height: 1.6, decimals: 1, layer: "G_FIX" });
      },
    }, {
      title: "Stringer head", view: "front", parts: [s, rim, top], size: [44, 40],
      window: { min: [-330, -1e4, P.DECK - 360], max: [rim.bbox.max[0] + 40, 1e4, P.DECK + 30] },
      note: `Cut ${cuts[1]} against the rim, ${cuts[2]} with the deck.`,
    }];
  },
  // The first rafter's seat on the front wall plate, seen along the ridge.
  5: () => {
    const plate = find("wall plate", (p) => p.bbox.min[1] < P.SIZE / 2)[0];
    const raft = find("rafter", (p) => p.bbox.min[1] < 0).sort((a, b) => a.bbox.min[0] - b.bbox.min[0])[0];
    const post = find("post", (p) => p.bbox.min[0] === 0 && p.bbox.min[1] === 0)[0];
    const y0 = R.plateY - 110, y1 = R.yIn + 110;
    return [{
      title: "Rafter seat", view: "right", parts: [plate, raft, post], size: [50, 56],
      window: { min: [-1e4, y0, P.POST_H - 60], max: [1e4, y1, R.top(y1) + 20] },
      note: `Cut ${raft.attrs.cut}.`,
      annotate: (dv) => {
        // The pitch, at the plate's inner top edge where the seat ends.
        const a = dv.toSheet([0, R.yIn, R.zp]), b = dv.toSheet([0, R.yIn + 100, R.zp]), c = dv.toSheet([0, R.yIn + 100, R.under(R.yIn + 100)]);
        cad.dimAngle(a, b, c, { radius: 9, text_height: 1.6, decimals: 0, layer: "G_FIX" });
      },
    }];
  },
  // One swing's hangers under the beam, from the front.
  12: () => {
    const sw = m.swings[0];
    const beam = byId.get(m.parts.beam.id);
    const hs = sw.hangers.map((h) => byId.get(h.id)), rs = sw.ropes.map((r) => byId.get(r.id));
    const x0 = sw.x[0] - 60, x1 = sw.x[1] + 60;
    const per = Object.entries(ruleOf("hanger", "swing beam").items).map(([it, k]) => `${k} ${plural(k, it)}`).join(" and ");
    return [{
      title: "Swing hangers", view: "front", parts: [beam, ...hs, ...rs], size: [60, 40],
      window: { min: [x0, -1e4, m.swing.hangerZ - 110], max: [x1, 1e4, P.SWING_H + 70] },
      note: `${per[0].toUpperCase()}${per.slice(1)} per hanger, through the beam.`,
      annotate: (dv) => {
        const xs = rs.map((r) => (r.bbox.min[0] + r.bbox.max[0]) / 2);
        const z = m.swing.hangerZ - 75;
        cad.measure(dv.toSheet([xs[0], 0, z]), dv.toSheet([xs[1], 0, z]), { offset: -2.5, text_height: 1.6, label: `${r0(xs[1] - xs[0])}`, layer: "G_FIX" });
      },
    }];
  },
};

// ── The steps ────────────────────────────────────────────────────────
guide.setup();
const steps = [];
const builtSoFar = [];
let prevView = null;
for (const [i, s] of m.steps.entries()) {
  const fresh = parts.filter((p) => stepOf.get(member(p)) === i);
  builtSoFar.push(...fresh);
  const built = [...builtSoFar];
  const tView = Date.now();
  const { view, scores } = guide.chooseView(fresh, built, { prev: prevView });
  const viewMs = Date.now() - tView;
  prevView = view;
  const order = list.order[i];
  const labels = s.members.map((mb) => {
    const ids = fresh.filter((p) => member(p) === mb).map((p) => p.id);
    return { member: mb, count: ids.length, parts: ids, lines: labelLines(mb, ids) };
  });
  // Fixings: one marker shape per rule, rows per item.
  const mine = joints.filter((j) => j.step === i);
  const rules = [...new Set(mine.map((j) => j.rule))];
  const kindOf = new Map(rules.map((r, k) => [r, guide.MARKERS[k % guide.MARKERS.length]]));
  const rows = rules.flatMap((r) => {
    const nj = mine.filter((j) => j.rule === r).length;
    return Object.entries(r.items).map(([item, per], k) => ({
      kind: k === 0 ? kindOf.get(r) : null, qty: per * nj, raw: item, item: plural(per * nj, item),
      where: `${per} per ${r.each ? asList(r.each).join("/") : ruleName(r)}${k === 0 ? ` (${nj})` : ""}`, rule: r, per, joints: nj,
    }));
  });
  steps.push({
    number: i + 1, title: s.title, fresh, built, view, scores, viewMs, labels, rows,
    joints: mine.map((j) => ({ point: j.point, kind: kindOf.get(j.rule), parts: j.b ? [j.a, j.b] : [j.a] })),
    instruction: SAY[i](sayer(fresh)),
    details: DETAILS[i + 1] ? DETAILS[i + 1]() : [],
    order,
  });
}

// ── Checks on the steps ──────────────────────────────────────────────
const seen = new Map();
for (const st of steps) for (const p of st.fresh) seen.set(p.id, (seen.get(p.id) || 0) + 1);
run.ok("every part appears in exactly one step", parts.every((p) => seen.get(p.id) === 1) && seen.size === parts.length,
  parts.filter((p) => seen.get(p.id) !== 1).map((p) => p.id).join(", "));
for (const st of steps) {
  const want = Object.fromEntries(st.order.parts.map((x) => [x.member, x.count]));
  run.ok(`step ${st.number}: every label's count is the part list's`, st.labels.every((l) => want[l.member] === l.count) && st.labels.length === st.order.parts.length,
    JSON.stringify({ labels: st.labels.map((l) => [l.member, l.count]), want }));
  const fromRows = {};
  for (const r of st.rows) fromRows[r.raw] = (fromRows[r.raw] || 0) + r.qty;
  const fromList = Object.fromEntries(st.order.fixings.map((f) => [f.item, f.count]));
  run.ok(`step ${st.number}: its fixings are the part list's`, JSON.stringify(Object.entries(fromRows).sort()) === JSON.stringify(Object.entries(fromList).sort()),
    JSON.stringify({ fromRows, fromList }));
}
const fixTotal = steps.reduce((s, st) => s + st.rows.reduce((t, r) => t + r.qty, 0), 0);
run.equal("the fixings over all steps add up to the part list's total", fixTotal, list.totals.fixings);

// ── Sheets: two steps each ───────────────────────────────────────────
const PER = 2;
const nSheets = Math.ceil(steps.length / PER);
const frameBox = { min: [5, 5], max: [292, 193] };
const panels = [{ min: [8, 26], max: [146, 190] }, { min: [151, 26], max: [289, 190] }];
const dir = globalThis.EXAMPLE_RENDER_DIR ?? "target/3d-views";
const draw = globalThis.EXAMPLE_RENDER !== false;
const sheets = [];
for (let k = 0; k < nSheets; k++) {
  const t0 = Date.now();
  { const all = cad.entities().map((e) => e.id); if (all.length) cad.remove(all); }
  const mine = steps.slice(k * PER, k * PER + PER);
  const fr = sheet.frame({
    title: "PLAY TOWER - CONSTRUCTION GUIDE",
    subtitle: `SHEET ${k + 1} OF ${nSheets}: STEPS ${mine[0].number} TO ${mine[mine.length - 1].number} OF ${steps.length}`,
    number: `CC-3D-11-G${k + 1}`, scale: "AS SHOWN", date: "2026-11-18",
  });
  cad.addLine([148.5, 26], [148.5, 190], { layer: "GRID" });
  // The legend, under the left panel.
  const lg = 8;
  cad.addLine([lg, 20], [lg + 6, 20], { layer: "G_NEW" });
  sheet.text("added in this step", [lg + 7.5, 19.4], { h: 1.6, layer: "G_NOTE" });
  cad.addLine([lg + 36, 20], [lg + 42, 20], { layer: "G_OLD" });
  sheet.text("built before", [lg + 43.5, 19.4], { h: 1.6, layer: "G_NOTE" });
  cad.addLine([lg, 15.5], [lg + 6, 15.5], { layer: "G_NEW_HIDDEN" });
  sheet.text("hidden edges, where the new part is mostly hidden", [lg + 7.5, 14.9], { h: 1.6, layer: "G_NOTE" });
  guide.marker("ring", [lg + 3, 11], 0.9);
  sheet.text("fixing markers: one shape per kind of joint, see each step's table", [lg + 7.5, 10.4], { h: 1.6, layer: "G_NOTE" });
  sheet.text("Lengths are cut lengths from the part list (long point to long point).", [lg, 6.6], { h: 1.4, w: 150, layer: "G_NOTE" });
  const drawn = mine.map((st, j) => {
    const corner = { iso: "front right", "iso-sw": "front left", "iso-ne": "back right", "iso-nw": "back left" }[st.view] ?? st.view;
    const tS = Date.now();
    const out = guide.drawStep({ ...st, note: `${st.fresh.length} parts added; ${st.built.length} built; seen from the ${corner}` }, panels[j]);
    out.total = Date.now() - tS;
    return { st, out };
  });
  // Checks on the sheet.
  const ents = cad.entities({ expand: true });
  const bb = (e) => e.bounds;
  const inside = (b, r) => b.min[0] >= r.min[0] - 1e-6 && b.min[1] >= r.min[1] - 1e-6 && b.max[0] <= r.max[0] + 1e-6 && b.max[1] <= r.max[1] + 1e-6;
  const outside = ents.filter((e) => e.layer !== "FRAME" && !inside(bb(e), frameBox));
  run.ok(`sheet ${k + 1}: everything stays inside the frame`, outside.length === 0, JSON.stringify(outside.slice(0, 3).map((e) => [e.id, e.type, e.layer, e.bounds])));
  const tb = fr.titleBlock;
  const onTb = drawn.flatMap(({ out }) => out.entities).map((id) => cad.entity(id)).filter((e) => guide.boxesMeet(e.bounds, tb, -1e-6));
  run.ok(`sheet ${k + 1}: the steps stay clear of the title block`, onTb.length === 0, JSON.stringify(onTb.slice(0, 3).map((e) => [e.id, e.type, e.bounds])));
  const labs = drawn.flatMap(({ out }) => out.labels);
  const clash = [];
  for (let a = 0; a < labs.length; a++) for (let b = a + 1; b < labs.length; b++) if (guide.boxesMeet(labs[a].box, labs[b].box)) clash.push([labs[a].member, labs[b].member]);
  run.ok(`sheet ${k + 1}: no two labels overlap`, clash.length === 0, JSON.stringify(clash));
  run.ok(`sheet ${k + 1}: no label covers the drawing`, labs.every((l) => l.under === 0), JSON.stringify(labs.filter((l) => l.under > 0).map((l) => [l.member, l.under])));
  for (const { st, out } of drawn) {
    const ids = new Set(out.main.kids.map((e) => e.attrs.partId));
    run.ok(`step ${st.number}: every part it adds is in its view`, st.fresh.every((p) => ids.has(p.id)), st.fresh.filter((p) => !ids.has(p.id)).map((p) => p.id).join(", "));
  }
  // Output.
  const tR = Date.now();
  let pdf = null;
  if (draw) {
    const bounds = { min: [-1.5, -1], max: [298.5, 199] };
    await run.render([{ as: k === 0 ? "iso" : `sheet-${k + 1}`, mode: "2d", bounds }]);
    pdf = `${dir}/11-play-tower-guide-${k + 1}.pdf`;
    await cad.savePdf(pdf, { colors: "paper", lineweights: true });
    if (!globalThis.EXAMPLE_RENDER_DIR) {
      await cad.render({ mode: "2d", size: [960, 640], pixelRatio: 2, bounds, out: `${dir}/11-play-tower-guide-${k + 1}-2x.png` });
    }
  }
  sheets.push({
    sheet: k + 1, pdf,
    steps: drawn.map(({ st, out }) => ({
      step: st.number, view: st.view, scale: `1:${out.main.n}`, projectMs: out.main.ms, detailMs: out.details.reduce((s, d) => s + d.v.ms, 0), ms: out.total, phases: out.ms, viewMs: st.viewMs,
      coverage: Object.fromEntries(Object.entries(st.scores).map(([v, c]) => [v, Math.round(c * 100) / 100])),
      labels: out.labels.map((l) => ({ member: l.member, count: l.count, cost: Math.round(l.cost), under: l.under })),
    })),
    ms: { draw: tR - t0, output: Date.now() - tR },
  });
}

return run.summary({
  steps: steps.length,
  sheets,
  contactsMs: contactMs,
  totalMs: Date.now() - T0,
  fixings: list.totals.fixings,
});
