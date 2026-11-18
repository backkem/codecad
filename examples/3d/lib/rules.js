// Rules over parts and attributes, for any model, loaded with
//
//   const rules = await cad.use("rules");
//
// Plain script code over the public set queries (overlapping3d,
// pairsWithin3d, parts): each rule asks the core whole-set questions in a
// few calls and filters the answers. Nothing here knows what a model is
// for; the script that calls a rule says which parts play which role, and
// what the limits are.
//
// Every rule returns what it found, with `failures`: one line per problem,
// naming the parts as "<label> <id>", the label being the value of the
// attribute `key` (default "member"). The calling script decides what a
// failure means (examples/3d/lib/checks.js: run.ok(what, failures.length
// === 0, failures.join("; "))).
//
// A rule that needs a region of space (a free-space zone, a probe along an
// edge, a rod across a gap) adds it as a temporary part with the attribute
// "rules.probe", asks about it, and removes it again before it returns.

const TOUCH = 1e-6;           // closer than this counts as touching (mm)
const PROBE = "rules.probe";  // the attribute that marks a temporary part

const labelOf = (key) => (attrs, id) => `${(attrs && attrs[key]) ?? "part"} ${id}`;
const round = (v, d = 1) => Math.round(v * 10 ** d) / 10 ** d;
const ids = (parts) => parts.map((p) => (typeof p === "string" ? p : p.id));

// Run `ask` with temporary parts made by `make`, then remove them, also
// when `ask` throws.
function withProbes(make, ask) {
  const made = [];
  try {
    make((p) => { made.push(p); return p; });
    return ask(made);
  } finally {
    if (made.length) cad.remove3d(made.map((p) => p.id));
  }
}

// Everything but the probes, as a selection.
const real = () => ids(cad.parts({ geometry: false }).filter((p) => !(PROBE in (p.attrs || {}))));

// ── Contacts ───────────────────────────────────────────────────────────

// Which parts touch which: one pairsWithin3d over the selection. Returns
// { parts, byId, touching: Map(id -> [{id, attrs}]), pairs, skipped }.
function contacts(sel = null, { tol = TOUCH } = {}) {
  const parts = sel === null ? cad.parts().filter((p) => !(PROBE in (p.attrs || {}))) : cad.parts(sel);
  const r = cad.pairsWithin3d(ids(parts), null, tol, { attrs: true });
  const touching = new Map(parts.map((p) => [p.id, []]));
  for (const q of r.pairs) {
    touching.get(q.a)?.push({ id: q.b, attrs: q.bAttrs });
    touching.get(q.b)?.push({ id: q.a, attrs: q.aAttrs });
  }
  return { parts, byId: new Map(parts.map((p) => [p.id, p])), touching, pairs: r.pairs, skipped: r.skipped };
}

// ── What rests on what ─────────────────────────────────────────────────

// Every part rests on what carries it. `needs` maps a value of `key` to
// {on: [values, or "ground"], min = 1, all = []}: the part must touch at
// least `min` parts (or the ground) whose value is in `on`, and one of each
// value in `all`. A part on the ground has its lowest point at `ground`
// (within tol). A value with no entry in `needs` is a failure too, so every
// kind of part is covered. `c` is a contacts() result (made if left out).
function restsOn({ needs, key = "member", ground = 0, tol = TOUCH, c = contacts(null, { tol }) }) {
  const label = labelOf(key);
  const failures = [];
  const count = {};
  for (const p of c.parts) {
    const kind = p.attrs[key];
    const need = needs[kind];
    if (!need) {
      failures.push(`${label(p.attrs, p.id)} has no rule for what carries it`);
      continue;
    }
    const on = need.on || [], all = need.all || [];
    const found = c.touching.get(p.id).filter((q) => on.includes(q.attrs[key]) || all.includes(q.attrs[key]));
    const kinds = found.map((q) => q.attrs[key]);
    if ((on.includes("ground") || all.includes("ground")) && Math.abs(p.bbox.min[2] - ground) <= tol) kinds.push("ground");
    const missing = all.filter((k) => !kinds.includes(k));
    const n = kinds.filter((k) => on.includes(k)).length;
    count[kind] = (count[kind] || 0) + 1;
    if (n < (need.min ?? (on.length ? 1 : 0)) || missing.length) {
      const has = kinds.length ? kinds.join(", ") : "nothing";
      const want = [on.length ? `${need.min ?? 1} of ${on.join("/")}` : "", ...missing.map((k) => `a ${k}`)].filter(Boolean).join(" and ");
      failures.push(`${label(p.attrs, p.id)} rests on ${has}, needs ${want}`);
    }
  }
  return { failures, checked: c.parts.length, kinds: count, skipped: c.skipped };
}

// Parts whose lowest point is at `ground` (a foot on the ground).
function onGround(sel, { key = "member", ground = 0, tol = TOUCH } = {}) {
  const label = labelOf(key);
  const parts = cad.parts(sel);
  const failures = parts.filter((p) => Math.abs(p.bbox.min[2] - ground) > tol)
    .map((p) => `${label(p.attrs, p.id)} has its foot at ${round(p.bbox.min[2], 3)}, not on the ground at ${ground}`);
  return { failures, checked: parts.length };
}

// ── Overlaps ───────────────────────────────────────────────────────────

// No two parts share volume (touching is fine).
function noOverlap(sel = null, { key = "member" } = {}) {
  const label = labelOf(key);
  const r = cad.overlapping3d(sel === null ? real() : sel, undefined, { attrs: true });
  const failures = r.pairs.map((p) => `${label(p.aAttrs, p.a)} and ${label(p.bAttrs, p.b)} overlap by ${round(p.volume, 3)} mm³ (${round(p.depth, 3)} deep)`)
    .concat(r.skipped.map((s) => `${s.a} and ${s.b} not checked: ${s.reason}`));
  return { failures, pairs: r.pairs };
}

// ── Free space ─────────────────────────────────────────────────────────

// Nothing may enter a zone but the parts it allows. `zones`: [{name, shape,
// allow}], shape a creating call's arguments, {extrude: profile, ...opts}
// (or sweep, revolve, loft), and allow a list of part ids (or a function of
// a part's attributes). Each zone becomes a temporary part; one
// overlapping3d finds every part inside one. Touching a zone's boundary is
// not entering it.
function freeSpace(zones, { key = "member" } = {}) {
  const label = labelOf(key);
  const others = real();
  return withProbes((add) => {
    for (const z of zones) {
      const { name } = z;
      const [kind] = ["extrude", "sweep", "revolve", "loft"].filter((k) => k in z.shape);
      const { [kind]: main, ...opts } = z.shape;
      add(cad[kind](main, { ...opts, name: `zone ${name}`, attrs: { [PROBE]: `zone: ${name}` } }));
    }
  }, (made) => {
    const r = cad.overlapping3d(ids(made), others, { attrs: true });
    const failures = [];
    const found = zones.map((z) => ({ name: z.name, volume: cad.part(made[zones.indexOf(z)].id).volume, inside: [] }));
    for (const p of r.pairs) {
      const i = made.findIndex((m) => m.id === p.a);
      const z = zones[i];
      const allowed = typeof z.allow === "function" ? z.allow(p.bAttrs, p.b) : (z.allow || []).includes(p.b);
      if (allowed) continue;
      found[i].inside.push(p.b);
      failures.push(`${label(p.bAttrs, p.b)} is inside the free space of ${z.name} (${round(p.volume, 1)} mm³)`);
    }
    for (const s of r.skipped) failures.push(`${s.a} and ${s.b} not checked: ${s.reason}`);
    return { failures, zones: found };
  });
}

// ── Gaps that trap ─────────────────────────────────────────────────────

// Gaps between parts in a range that can trap (a head, a finger). `a` and
// `b` are selections (b null: pairs within a); `ranges` [{name, min, max}]
// with min <= gap < max. A gap is the distance between two parts, which is
// the width of the opening between them where they face each other. A gap
// counts only where the two parts face each other (across the gap's main
// direction their boxes overlap in both other directions: a post and a
// stringer that only come near at a corner bound no opening), and only if
// the straight line between the two closest points is clear: a rod of
// radius `rod` along it may enter no other part (a riser board between two
// treads closes the gap between them). Not judged: how
// deep an opening is (a niche backed by a wall counts like a hole through
// it), and openings bounded by more than two parts. Returns failures and
// every gap measured in a range.
function gaps(a, b, ranges, { key = "member", rod = 3 } = {}) {
  const label = labelOf(key);
  const most = Math.max(...ranges.map((r) => r.max));
  const r = cad.pairsWithin3d(a, b, most, { attrs: true });
  const hits = [];
  for (const p of r.pairs) {
    for (const range of ranges) {
      if (p.distance >= range.min && p.distance < range.max) hits.push({ ...p, range });
    }
  }
  const blocked = new Set();
  // First the boxes: a segment that passes through a part which fills its
  // own box (an axis-aligned board) is closed by it, exactly, with no probe.
  // A segment that meets only the boxes of other shapes gets a rod probe; a
  // segment that meets no box at all is open.
  const all = cad.parts().filter((p) => !(PROBE in (p.attrs || {})));
  const isBox = (p) => {
    const v = [0, 1, 2].reduce((acc, i) => acc * (p.bbox.max[i] - p.bbox.min[i]), 1);
    return Math.abs(v - p.volume) <= 1e-9 * v;
  };
  const crosses = (p0, p1, b, pad) => {
    let lo = 0, hi = 1;
    for (let i = 0; i < 3; i++) {
      const d = p1[i] - p0[i], min = b.min[i] - pad, max = b.max[i] + pad;
      if (Math.abs(d) < 1e-12) { if (p0[i] <= min || p0[i] >= max) return false; continue; }
      let t0 = (min - p0[i]) / d, t1 = (max - p0[i]) / d;
      if (t0 > t1) [t0, t1] = [t1, t0];
      lo = Math.max(lo, t0); hi = Math.min(hi, t1);
      if (lo >= hi) return false;
    }
    return true;
  };
  // Two parts bound an opening only where they face each other: across the
  // gap's main direction k their boxes overlap in both other directions.
  // The line that tests the opening runs along k through the middle of that
  // overlap, from one box to the other (not between the closest points,
  // which often lie at the opening's side, against the part that bounds
  // it there). Where the boxes overlap along k too (parts at an angle), it
  // runs between the closest points.
  const box = new Map(all.map((p) => [p.id, p.bbox]));
  const across = (h) => {
    const d = h.pb.map((v, i) => Math.abs(v - h.pa[i]));
    const k = d.indexOf(Math.max(...d));
    const A = box.get(h.a), B = box.get(h.b);
    const others = [0, 1, 2].filter((i) => i !== k);
    if (!others.every((i) => Math.min(A.max[i], B.max[i]) - Math.max(A.min[i], B.min[i]) > TOUCH)) return null;
    const [lo, hi] = A.max[k] <= B.min[k] ? [A.max[k], B.min[k]] : B.max[k] <= A.min[k] ? [B.max[k], A.min[k]] : [null, null];
    if (lo === null) return [h.pa, h.pb];
    const p = [0, 0, 0];
    for (const i of others) p[i] = (Math.max(A.min[i], B.min[i]) + Math.min(A.max[i], B.max[i])) / 2;
    return [p.map((v, i) => (i === k ? lo : v)), p.map((v, i) => (i === k ? hi : v))];
  };
  const apart = [];
  const probe = [];
  for (const h of hits) {
    const line = across(h);
    if (!line) { apart.push(h); continue; }
    const d = line[1].map((v, i) => v - line[0][i]);
    const len = Math.hypot(...d);
    const u = d.map((v) => v / len);
    const end = 0.5;   // stop short of both parts
    h.line = { from: line[0].map((v, i) => v + end * u[i]), u, len: len - 2 * end };
    const p1 = line[1].map((v, i) => v - end * u[i]);
    const between = all.filter((p) => p.id !== h.a && p.id !== h.b && crosses(h.line.from, p1, p.bbox, rod));
    if (between.some(isBox)) blocked.add(h);
    else if (between.length) probe.push(h);
  }
  if (probe.length) {
    const others = real();
    withProbes((add) => {
      for (const h of probe) {
        const { from, u, len } = h.line;
        // Any direction square to u for the plane's x.
        const t = Math.abs(u[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0];
        const x = [u[1] * t[2] - u[2] * t[1], u[2] * t[0] - u[0] * t[2], u[0] * t[1] - u[1] * t[0]];
        h.rod = add(cad.extrude({ center: [0, 0], radius: rod }, {
          plane: { origin: from, x, normal: u }, distance: len, name: "gap rod", attrs: { [PROBE]: "gap rod" },
        }));
      }
    }, (made) => {
      const q = cad.overlapping3d(ids(made), others, { attrs: true });
      for (const p of q.pairs) {
        const h = probe.find((x) => x.rod.id === p.a);
        if (p.b !== h.a && p.b !== h.b) blocked.add(h);
      }
    });
  }
  const open = hits.filter((h) => !blocked.has(h) && !apart.includes(h));
  const failures = open.map((h) => `${h.range.name} gap ${round(h.distance)} between ${label(h.aAttrs, h.a)} and ${label(h.bAttrs, h.b)}`)
    .concat(r.skipped.map((s) => `${s.a} and ${s.b} not checked: ${s.reason}`));
  return {
    failures,
    measured: r.pairs.length,
    probed: probe.length,
    apart: apart.length,
    closed: [...blocked].map((h) => ({ a: h.a, b: h.b, gap: round(h.distance) })),
    open: open.map((h) => ({ range: h.range.name, a: h.a, b: h.b, gap: round(h.distance) })),
  };
}

// ── Guards along edges ─────────────────────────────────────────────────

// Every edge of a raised floor has a guard at least `height` above it,
// except at its openings. `edges`: [{name, from: [x, y], to: [x, y], z}]
// (z the floor's top); `openings`: [{edge, from, to}] in mm along the edge
// from its `from` end. A part guards a stretch of an edge if it meets a
// probe along the edge (from z up to z + height, `band` mm to either side)
// and its top reaches z + height; the stretch is its box along the edge.
// Stretches closer than `join` count as one. Returns failures and, per
// edge, the guarded stretches and the parts that guard it.
function guards({ edges, height, openings = [], band = 100, join = 0, key = "member", among = null }) {
  const label = labelOf(key);
  const pool = among ?? real();
  return withProbes((add) => {
    for (const e of edges) {
      const d = [e.to[0] - e.from[0], e.to[1] - e.from[1]];
      const len = Math.hypot(...d);
      const u = [d[0] / len, d[1] / len], n = [-u[1], u[0]];
      const at = (s, t) => [e.from[0] + s * u[0] + t * n[0], e.from[1] + s * u[1] + t * n[1]];
      const ring = [at(0, -band), at(len, -band), at(len, band), at(0, band)];
      add(cad.extrude(ring, { at: [0, 0, e.z], distance: height, name: `guard probe ${e.name}`, attrs: { [PROBE]: `guard: ${e.name}` } }));
    }
  }, (made) => {
    const r = cad.overlapping3d(ids(made), pool, { attrs: true });
    const failures = [];
    const report = edges.map((e, i) => {
      const d = [e.to[0] - e.from[0], e.to[1] - e.from[1]];
      const len = Math.hypot(...d);
      const u = [d[0] / len, d[1] / len];
      const along = ([x, y]) => (x - e.from[0]) * u[0] + (y - e.from[1]) * u[1];
      const spans = [], by = [], low = [];
      for (const p of r.pairs.filter((q) => q.a === made[i].id)) {
        const part = cad.part(p.b);
        const { min, max } = part.bbox;
        const s = [[min[0], min[1]], [max[0], min[1]], [min[0], max[1]], [max[0], max[1]]].map(along);
        const span = [Math.max(0, Math.min(...s)), Math.min(len, Math.max(...s))];
        if (part.bbox.max[2] < e.z + height - TOUCH) { low.push({ name: label(p.bAttrs, p.b), span }); continue; }
        spans.push(span);
        by.push(label(p.bAttrs, p.b));
      }
      for (const o of openings.filter((o) => o.edge === e.name)) spans.push([o.from, o.to]);
      spans.sort((p, q) => p[0] - q[0]);
      const merged = [];
      for (const s of spans.filter(([p, q]) => q > p)) {
        const last = merged[merged.length - 1];
        if (last && s[0] <= last[1] + join + TOUCH) last[1] = Math.max(last[1], s[1]);
        else merged.push([...s]);
      }
      const open = [];
      let at = 0;
      for (const [p, q] of merged) { if (p > at + TOUCH) open.push([at, p]); at = Math.max(at, q); }
      if (at < len - TOUCH) open.push([at, len]);
      for (const [p, q] of open) {
        const there = [...new Set(low.filter((l) => l.span[0] < q && l.span[1] > p).map((l) => l.name))];
        failures.push(`edge ${e.name} has no guard ${height} high from ${round(p)} to ${round(q)}` +
          (there.length ? ` (lower parts there: ${there.join(", ")})` : ""));
      }
      return { edge: e.name, length: len, guarded: merged.map(([p, q]) => [round(p), round(q)]), by: [...new Set(by)] };
    });
    for (const s of r.skipped) failures.push(`${s.a} and ${s.b} not checked: ${s.reason}`);
    return { failures, edges: report };
  });
}

return { contacts, restsOn, onGround, noOverlap, freeSpace, gaps, guards, PROBE, TOUCH };
