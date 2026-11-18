// A part list from a model's parts and attributes, for any model, loaded
// with
//
//   const pl = await cad.use("partlist");
//   const list = pl.list({ stock, fixings, steps });
//
// Plain script code over cad.parts() and one pairsWithin3d. It reads these
// attributes and nothing else (the model decides what they hold):
//
//   member    what a part is ("post", "rafter"); the key can be changed
//   material  what it is made of
//   section   for sawn stock, "<thickness>x<width>" in mm ("45x145")
//   cut       a note on its end cuts (angles), copied into the list
//   bought    true for a bought item; `item` says what to ask for, and
//             `unit` "m" or "m3" when it is sold by length or volume
//
// Cut length. A sawn piece's length is its own box (the part's local box,
// in the frame it was built in) along its longest side, rounded up to the
// millimetre. For a member cut at an angle that is the length from long
// point to long point: what you mark on the stock before making the angled
// cuts, whose angles the `cut` note gives. The two shorter sides are
// checked against `section` (a part that does not fit its stated section
// is listed in `misfits`).
//
// Cutting plan, per section: first-fit decreasing into the longest stock
// length, with a saw kerf between pieces; then each length of stock is cut
// down to the shortest stock length that still holds its pieces.
//
// Fixings come from rules of thumb the model states: per joint (two parts
// that touch, found with one pairsWithin3d) or per part. A joint no rule
// names is listed in `unfixed`, so a rule can require that every joint has
// its fixings.

const round = (v, d = 0) => Math.round(v * 10 ** d) / 10 ** d;
const asList = (v) => (Array.isArray(v) ? v : [v]);

// The pieces of sawn stock: one per part.
function pieces(parts, key) {
  return parts.filter((p) => p.attrs.bought !== true && p.attrs.section).map((p) => {
    const ext = [0, 1, 2].map((i) => p.localBbox.max[i] - p.localBbox.min[i]).sort((a, b) => b - a);
    const [t, w] = p.attrs.section.split("x").map(Number);
    return {
      id: p.id, member: p.attrs[key], material: p.attrs.material, section: p.attrs.section, cut: p.attrs.cut ?? "",
      length: Math.ceil(ext[0] - 1e-6), fits: ext[1] <= w + 0.5 && ext[2] <= t + 0.5, size: ext.map((v) => round(v, 1)),
    };
  });
}

// First-fit decreasing into `lengths` (mm), kerf between pieces; then each
// bar cut down to the shortest stock that holds it.
function cuttingPlan(list, lengths, kerf) {
  const longest = Math.max(...lengths);
  const sorted = [...list].sort((a, b) => b.length - a.length);
  const bars = [];
  const tooLong = [];
  for (const p of sorted) {
    if (p.length > longest) { tooLong.push(p); continue; }
    let bar = bars.find((b) => b.used + kerf + p.length <= longest);
    if (!bar) { bar = { pieces: [], used: -kerf }; bars.push(bar); }
    bar.pieces.push(p);
    bar.used += kerf + p.length;
  }
  for (const b of bars) {
    b.stock = Math.min(...lengths.filter((L) => L >= b.used));
    b.offcut = b.stock - b.used;
  }
  return { bars, tooLong };
}

function list({ select = {}, key = "member", stock, fixings = [], steps = [], contacts = null }) {
  const parts = cad.parts(select);
  const byId = new Map(parts.map((p) => [p.id, p]));
  const kerf = stock.kerf ?? 3;

  // ── Timber: grouped by material, section, length and end cuts ──────
  const ps = pieces(parts, key);
  const groups = new Map();
  for (const p of ps) {
    const k = [p.material, p.section, p.length, p.cut, p.member].join("|");
    if (!groups.has(k)) groups.set(k, { material: p.material, section: p.section, length: p.length, cut: p.cut, count: 0, members: [], ids: [] });
    const g = groups.get(k);
    g.count++;
    g.ids.push(p.id);
    if (!g.members.includes(p.member)) g.members.push(p.member);
  }
  const area = (s) => s.split("x").map(Number).reduce((a, b) => a * b, 1);
  const timber = [...groups.values()]
    .sort((a, b) => area(b.section) - area(a.section) || a.section.localeCompare(b.section) || b.length - a.length)
    .map((g) => ({ ...g, metres: round((g.count * g.length) / 1000, 2) }));

  // ── Cutting plan per section ───────────────────────────────────────
  const sections = [...new Set(timber.map((g) => g.section))];
  const plans = sections.map((section) => {
    const lengths = (stock.bySection && stock.bySection[section]) || stock.lengths;
    const mine = ps.filter((p) => p.section === section);
    const { bars, tooLong } = cuttingPlan(mine, lengths, kerf);
    const bought = bars.reduce((s, b) => s + b.stock, 0);
    const used = mine.reduce((s, p) => s + p.length, 0);
    const buy = lengths.map((L) => ({ length: L, count: bars.filter((b) => b.stock === L).length })).filter((x) => x.count);
    // Bars with the same stock and the same pieces, drawn once.
    const patterns = new Map();
    for (const b of bars) {
      const k = `${b.stock}:${b.pieces.map((p) => p.length).join(",")}`;
      if (!patterns.has(k)) patterns.set(k, { stock: b.stock, pieces: b.pieces.map((p) => ({ length: p.length, member: p.member })), used: b.used, offcut: b.offcut, count: 0 });
      patterns.get(k).count++;
    }
    return {
      section, lengths, kerf, buy, bars: [...patterns.values()],
      pieces: mine.length, metres: round(used / 1000, 2), boughtMetres: round(bought / 1000, 2),
      offcut: round(bought - used - kerf * bars.reduce((s, b) => s + b.pieces.length - 1, 0)),
      waste: bought ? round((100 * (bought - used)) / bought, 1) : 0,
      tooLong: tooLong.map((p) => `${p.member} ${p.id} (${p.length})`),
    };
  });

  // ── Bought items ───────────────────────────────────────────────────
  const items = new Map();
  for (const p of parts.filter((q) => q.attrs.bought === true)) {
    const k = p.attrs.item ?? p.attrs[key];
    if (!items.has(k)) items.set(k, { item: k, count: 0, unit: p.attrs.unit ?? "pcs", qty: 0, members: [] });
    const it = items.get(k);
    it.count++;
    if (!it.members.includes(p.attrs[key])) it.members.push(p.attrs[key]);
    const ext = [0, 1, 2].map((i) => p.localBbox.max[i] - p.localBbox.min[i]);
    it.qty += it.unit === "m" ? Math.max(...ext) / 1000 : it.unit === "m3" ? p.volume / 1e9 : 1;
  }
  const bought = [...items.values()].map((it) => ({ ...it, qty: round(it.qty, it.unit === "pcs" ? 0 : 2) }));

  // ── Fixings, from the joints ───────────────────────────────────────
  const touching = contacts ?? cad.pairsWithin3d(parts.map((p) => p.id), null, 1e-6, { attrs: true });
  const match = (r, ka, kb) => r.a !== undefined &&
    ((ka === r.a && asList(r.b).includes(kb)) || (kb === r.a && asList(r.b).includes(ka)));
  const joints = [];
  const unfixed = new Map();
  for (const q of touching.pairs) {
    const ka = q.aAttrs[key], kb = q.bAttrs[key];
    const rule = fixings.find((r) => match(r, ka, kb));
    if (!rule) {
      const k = [ka, kb].sort().join(" + ");
      unfixed.set(k, (unfixed.get(k) || 0) + 1);
      continue;
    }
    joints.push({ a: q.a, b: q.b, members: [ka, kb], rule });
  }
  const fixingCount = new Map();
  const addItems = (items, n, where) => {
    for (const [item, per] of Object.entries(items || {})) {
      if (!fixingCount.has(item)) fixingCount.set(item, { item, count: 0, where: [] });
      const f = fixingCount.get(item);
      f.count += per * n;
      if (!f.where.includes(where)) f.where.push(where);
    }
  };
  for (const j of joints) addItems(j.rule.items, 1, `${j.rule.a} to ${asList(j.rule.b).join("/")}`);
  for (const r of fixings.filter((r) => r.each)) {
    const n = parts.filter((p) => asList(r.each).includes(p.attrs[key])).length;
    addItems(r.items, n, `each ${asList(r.each).join("/")}`);
  }
  const fixingList = [...fixingCount.values()].sort((a, b) => a.item.localeCompare(b.item));

  // ── Assembly order ─────────────────────────────────────────────────
  const stepOf = new Map();
  steps.forEach((s, i) => s.members.forEach((mb) => stepOf.set(mb, i)));
  const order = steps.map((s, i) => {
    const mine = parts.filter((p) => stepOf.get(p.attrs[key]) === i);
    const counts = s.members.map((mb) => ({ member: mb, count: mine.filter((p) => p.attrs[key] === mb).length }));
    const used = new Map();
    for (const j of joints) {
      const sa = stepOf.get(j.members[0]), sb = stepOf.get(j.members[1]);
      if (Math.max(sa, sb) !== i) continue;
      for (const [item, per] of Object.entries(j.rule.items || {})) used.set(item, (used.get(item) || 0) + per);
    }
    for (const r of fixings.filter((r) => r.each)) {
      const n = mine.filter((p) => asList(r.each).includes(p.attrs[key])).length;
      for (const [item, per] of Object.entries(r.items || {})) if (n) used.set(item, (used.get(item) || 0) + per * n);
    }
    return { step: i + 1, title: s.title, parts: counts, fixings: [...used].map(([item, count]) => ({ item, count })) };
  });
  const unplaced = [...new Set(parts.map((p) => p.attrs[key]).filter((k) => !stepOf.has(k)))];

  // ── Totals ─────────────────────────────────────────────────────────
  const boughtM = plans.reduce((s, p) => s + p.boughtMetres, 0), usedM = plans.reduce((s, p) => s + p.metres, 0);
  return {
    timber, plans, bought, fixings: fixingList, order,
    totals: {
      pieces: ps.length, metres: round(usedM, 2), stockMetres: round(boughtM, 2),
      stockLengths: plans.reduce((s, p) => s + p.buy.reduce((t, b) => t + b.count, 0), 0),
      waste: round((100 * (boughtM - usedM)) / boughtM, 1),
      boughtItems: bought.reduce((s, b) => s + b.count, 0),
      fixings: fixingList.reduce((s, f) => s + f.count, 0),
      joints: joints.length,
    },
    misfits: ps.filter((p) => !p.fits).map((p) => `${p.member} ${p.id}: ${p.size.join(" x ")} in ${p.section}`),
    unfixed: [...unfixed].map(([k, n]) => `${k} (${n})`),
    unplaced,
    tooLong: plans.flatMap((p) => p.tooLong),
    skipped: touching.skipped,
  };
}

return { list, cuttingPlan, pieces };
