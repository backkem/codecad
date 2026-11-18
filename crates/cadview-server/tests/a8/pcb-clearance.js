// Clearance rule, written in script only: no two pieces of copper of
// different nets closer than CLEARANCE, and none touching (a short).
// Uses only generic calls: a selection by layer with block inserts expanded
// into their pads, `pairsWithin` and `overlapping`. What a net is, and how
// a pad gets one, is decided here.


const CLEARANCE = 0.2;
// A track has its own net; a pad takes it from its footprint's insert.
const netOf = (a) => a.net ?? a[`net.${a.pin}`] ?? null;
// A pad is named after its footprint and pin, a track by its own name.
const nameOf = (a) => (a.ref ? `${a.ref}.${a.pin}` : a.name);

const copper = { layer: "CU", expand: true };
const close = cad.pairsWithin(copper, null, CLEARANCE, { attrs: true }).pairs;
const touching = cad.overlapping(copper, null, { attrs: true }).pairs;

const foreign = (p) => netOf(p.aAttrs) !== netOf(p.bAttrs);
const byKey = (x, y) => (x.key < y.key ? -1 : x.key > y.key ? 1 : 0);
const violations = close
  .filter(foreign)
  .map((p) => {
    const between = [nameOf(p.aAttrs), nameOf(p.bAttrs)].sort();
    return { key: between.join("|"), between, gap: Math.round(p.distance * 1000) / 1000 };
  })
  .sort(byKey)
  .map(({ between, gap }) => ({ between, gap }));
const shorts = touching
  .filter(foreign)
  .map((p) => ({ nets: [netOf(p.aAttrs), netOf(p.bAttrs)].sort(), at: p.points }));

return { violations, shorts };
