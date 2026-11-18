// 11 - Play tower with slide and swings: the part list.
//
// Builds the model of 11-play-tower.js (lib/playtower.js, written once),
// makes its part list (lib/partlist.js, generic over attributes) and draws
// it on one sheet with the 2D API: the sawn timber grouped by section and
// cut length, a cutting plan per section drawn to scale (each length of
// stock as a bar, its pieces labelled with their lengths, the offcut
// hatched), the bought items and the fixings, counted from the joints by
// the model's rules of thumb. The sheet goes to a PDF and a 2D render.
//
// Checks: every piece fits its stated section, no piece is longer than the
// longest stock, every joint has a fixing rule (or a rule that it needs
// none), every member has an assembly step, and the cutting plan holds
// every piece once.

const tower = await cad.use("playtower");
const pl = await cad.use("partlist");
const sheet = await cad.use("sheet");
const ck = await cad.use("checks");
const run = ck.start("11-play-tower-partlist");
const { text, rect, table, ACI } = sheet;

// ── The model and its list ───────────────────────────────────────────
const m = tower.build();
const list = pl.list({ stock: m.stock, fixings: m.fixings, steps: m.steps });
const parts = cad.parts();
const sawn = parts.filter((p) => p.attrs.bought !== true);
const T = list.totals;

run.ok("every piece fits its stated section", list.misfits.length === 0, list.misfits.join("; "));
run.ok("no piece is longer than the longest stock", list.tooLong.length === 0, list.tooLong.join("; "));
run.ok("every joint has its fixings, or a rule that it needs none", list.unfixed.length === 0, list.unfixed.join("; "));
run.ok("every contact was measured", list.skipped.length === 0, JSON.stringify(list.skipped));
run.ok("every member has an assembly step", list.unplaced.length === 0, list.unplaced.join(", "));
run.equal("every sawn part is listed once", list.timber.reduce((s, g) => s + g.count, 0), sawn.length);
run.equal("every bought part is listed once", list.bought.reduce((s, b) => s + b.count, 0), parts.length - sawn.length);
for (const p of list.plans) {
  const placed = p.bars.reduce((s, b) => s + b.count * b.pieces.length, 0);
  run.equal(`${p.section}: the plan cuts every piece once`, placed, p.pieces);
  run.ok(`${p.section}: every bar holds its pieces`, p.bars.every((b) => b.used <= b.stock && b.offcut >= 0));
}
run.near("the timber adds up (to the 10 mm the totals are rounded to)", list.timber.reduce((s, g) => s + g.count * g.length, 0) / 1000, T.metres, 0, 0.01);
run.equal("every part has its assembly step", list.order.reduce((s, st) => s + st.parts.reduce((t, x) => t + x.count, 0), 0), parts.length);

// ── The sheet (paper mm, 297 x 198: the 3:2 of the examples' PNGs) ───
sheet.setup({
  PIECE: { color: ACI.white, lineweight: 0.25 },
  OFFCUT: { color: ACI.gray, lineweight: 0.18 },
});
const K = 64;                                        // the cutting plan's scale, 1:K
sheet.frame({
  title: "PLAY TOWER - PART LIST",
  subtitle: "SAWN TIMBER, CUTTING PLAN, BOUGHT ITEMS, FIXINGS",
  number: "CC-3D-11", scale: `PLAN 1:${K}`, date: "2026-11-18",
});
const TOP = 172;
text("PLAY TOWER WITH SLIDE AND SWINGS", [10, 185.5], { h: 4.2, layer: "HEAD" });
text(`${T.pieces} pieces of sawn timber, ${T.metres.toFixed(1)} m, cut from ${T.stockLengths} stock lengths ` +
  `(${T.stockMetres.toFixed(1)} m, ${T.waste.toFixed(1)} % offcut). ${T.boughtItems} bought items, ${T.fixings} fixings for ${T.joints} joints.`,
  [10, 180.5], { h: 2, w: 280, layer: "NOTE" });

// Timber, grouped by section, length, end cuts and member.
const ta = table([10, TOP], [
  { title: "SECTION", w: 13 }, { title: "LENGTH", w: 12, align: "right" }, { title: "QTY", w: 7, align: "right" },
  { title: "METRES", w: 12, align: "right" }, { title: "MEMBER", w: 22 }, { title: "END CUTS (FROM SQUARE)", w: 43 },
], list.timber.map((g) => [g.section, g.length, g.count, g.metres.toFixed(2), g.members.join(", "), g.cut]),
{ rowH: 3.9, h: 1.9, title: "SAWN TIMBER" });
text("Length: long point to long point, rounded up to the mm. Sections: thickness x width.", [10, ta.min[1] - 3.5], { h: 1.6, w: 109, layer: "NOTE" });
text("Angled members: mark the length, then cut the angles given.", [10, ta.min[1] - 6.3], { h: 1.6, w: 109, layer: "NOTE" });

// Assembly order, in two columns under the timber table: the steps the
// construction guide will draw, with what each adds.
const plural = (n, w) => (n === 1 ? w : w === "sand" ? w : `${w}s`);
const oy = ta.min[1] - 12;
text("ASSEMBLY ORDER", [10, oy], { h: 3, layer: "HEAD" });
list.order.forEach((s, i) => {
  const col = Math.floor(i / 6), row = i % 6;
  const what = s.parts.map((x) => `${x.count} ${plural(x.count, x.member)}`).join(", ");
  text(`${s.step}. ${s.title}: ${what}`, [10 + col * 81, oy - 4.5 - row * 3.3], { h: 1.6, w: 79 });
});
const orderBox = { min: [10, oy - 4.5 - 5 * 3.3 - 1], max: [170, oy + 3] };

// Cutting plan: one bar per length of stock (identical bars once, with a
// count), pieces to scale with their lengths, offcuts hatched.
const bx = 124, barX = 134, rowH = 3.5, barH = 2.5;
text(`CUTTING PLAN  (1:${K}, KERF ${m.stock.kerf})`, [bx, TOP + 2], { h: 3, layer: "HEAD" });
let y = TOP;
for (const p of list.plans) {
  y -= 3.6;
  const buy = p.buy.map((b) => `${b.count} x ${b.length}`).join(" + ");
  text(`${p.section}`, [bx, y + 0.6], { h: 2, layer: "HEAD" });
  text(`buy ${buy}   offcut ${p.waste.toFixed(1)} %`, [bx + 14, y + 0.6], { h: 1.7, w: 84, layer: "NOTE" });
  for (const b of p.bars) {
    y -= rowH;
    text(b.stock, [bx, y + barH / 2], { h: 1.6, valign: "middle", layer: "TEXT" });
    rect([barX, y], [barX + b.stock / K, y + barH], "FRAME");
    let x = barX;
    for (const pc of b.pieces) {
      const w = pc.length / K;
      rect([x, y], [x + w, y + barH], "PIECE");
      const t = cad.addText(String(pc.length), [x + w / 2, y + barH / 2], { height: 1.4, layer: "TEXT", align: "center", valign: "middle" });
      if (t.width > w - 0.6) cad.remove(t.id);
      x += w + m.stock.kerf / K;
    }
    const left = barX + b.stock / K - x;
    if (left > 0.4) cad.addHatch([[x, y], [x + left, y], [x + left, y + barH], [x, y + barH]], { layer: "OFFCUT", angle: 45, spacing: 0.7 });
    if (b.count > 1) text(`x ${b.count}`, [barX + 4800 / K + 1.5, y + barH / 2], { h: 1.7, valign: "middle", layer: "HEAD" });
  }
  y -= 0.8;
}
const planBottom = y;

// Bought items, then fixings, then the totals.
const cx = 228;
const tb = table([cx, TOP], [{ title: "BOUGHT ITEM", w: 46 }, { title: "QTY", w: 18, align: "right" }],
  list.bought.map((b) => [b.item, b.unit === "pcs" ? b.count : `${b.qty.toFixed(2)} ${b.unit === "m3" ? "m³" : b.unit}`]),
  { rowH: 3.9, h: 1.9, title: "BOUGHT ITEMS" });
const tf = table([cx, tb.min[1] - 7], [{ title: "FIXING", w: 46 }, { title: "QTY", w: 18, align: "right" }],
  list.fixings.map((f) => [f.item, f.count]), { rowH: 3.6, h: 1.8, title: "FIXINGS (RULES OF THUMB)" });
text("Counted per joint from the model's rules", [cx, tf.min[1] - 3.2], { h: 1.6, w: 64, layer: "NOTE" });
text("(e.g. 2 coach bolts per post to rim).", [cx, tf.min[1] - 5.6], { h: 1.6, w: 64, layer: "NOTE" });

// Stay on the sheet: everything inside the frame, clear of the title block.
const frameBox = { min: [5, 5], max: [292, 193] }, titleBlock = { min: [172, 5], max: [292, 23] };
const ents = cad.entities();
const inside = (b, r) => b.min[0] >= r.min[0] - 1e-6 && b.min[1] >= r.min[1] - 1e-6 && b.max[0] <= r.max[0] + 1e-6 && b.max[1] <= r.max[1] + 1e-6;
const meets = (b, r) => b.min[0] < r.max[0] && b.max[0] > r.min[0] && b.min[1] < r.max[1] && b.max[1] > r.min[1];
const bb = (e) => ({ min: e.bounds.min ?? e.bounds[0], max: e.bounds.max ?? e.bounds[1] });
run.ok("everything drawn stays inside the frame", ents.filter((e) => e.layer !== "FRAME").every((e) => inside(bb(e), frameBox)));
const blocks = [ta, { min: [bx, planBottom], max: [222, TOP] }, tb, tf, orderBox];
run.ok("the tables and the plan stay clear of the title block", blocks.every((b) => !meets(b, titleBlock)),
  JSON.stringify(blocks.map((b) => [b.min, b.max])));
run.ok("the plan stays clear of the bought items column", ents.filter((e) => e.layer === "PIECE" || e.layer === "OFFCUT").every((e) => bb(e).max[0] < cx));

// ── Output: the 2D render (the example's picture), a PDF ─────────────
await run.render([{ as: "iso", mode: "2d" }]);
const dir = globalThis.EXAMPLE_RENDER_DIR ?? "target/3d-views";
let pdf = null;
if (globalThis.EXAMPLE_RENDER !== false) {
  pdf = (await cad.savePdf(`${dir}/11-play-tower-partlist.pdf`, { colors: "paper", lineweights: true })).path;
  // A sharper copy to read on screen (not one of the example's files).
  await cad.render({ mode: "2d", size: [960, 640], pixelRatio: 2, out: `${dir}/11-play-tower-partlist-2x.png` });
}

return run.summary({
  totals: T,
  sections: list.plans.map((p) => ({ section: p.section, pieces: p.pieces, metres: p.metres, buy: p.buy, stockMetres: p.boughtMetres, waste: p.waste })),
  order: list.order.map((s) => `${s.step}. ${s.title}: ${s.parts.map((x) => `${x.count} ${x.member}`).join(", ")}`),
  pdf,
});
