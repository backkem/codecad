// Free-space rule, written in script only: every item stands inside the
// site, no two items overlap, and each keeps the free space its `free`
// attribute asks for. Uses only generic calls: `inside`, `overlapping`
// with closed areas, and `pairsWithin`.


const items = { layer: "PLAY", has: ["item"] };
const site = { layer: "SITE", attrs: { role: "site" } };
const all = cad.entities(items);
const nameOf = Object.fromEntries(all.map((e) => [e.id, e.attrs.item]));
const problems = [];

const inSite = new Set(cad.inside(items, site).pairs.map((p) => p.a));
for (const e of all) {
  if (!inSite.has(e.id)) problems.push(`${e.attrs.item} is not inside the site`);
}

// Overlapping (or one inside the other) is reported once, as an overlap.
const overlap = new Set();
for (const p of cad.overlapping(items, null, { areas: true }).pairs) {
  overlap.add(`${p.a} ${p.b}`);
  problems.push(`${nameOf[p.a]} and ${nameOf[p.b]} overlap`);
}

const most = Math.max(0, ...all.map((e) => e.attrs.free));
for (const p of cad.pairsWithin(items, null, most, { attrs: true }).pairs) {
  const need = Math.max(p.aAttrs.free, p.bAttrs.free);
  if (!overlap.has(`${p.a} ${p.b}`) && p.distance < need) {
    problems.push(`${nameOf[p.a]} and ${nameOf[p.b]} are ${Math.round(p.distance)} apart, need ${need}`);
  }
}

return problems;
