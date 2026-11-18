// Checks, renders and the summary for the 3D examples, loaded with
//
//   const ck = await cad.use("checks");
//   const run = ck.start("03-bookshelf");
//
// Plain script code over the public cad.* calls.
//
// Where the renders go. The isometric working render is the example's
// picture: examples/3d/NN-name.png, relative to the server's working
// directory (run from the repository root). The other views are for
// review and go to target/3d-views/, which git ignores. A caller can send
// every render elsewhere by setting globalThis.EXAMPLE_RENDER_DIR before
// the script runs, or skip renders with globalThis.EXAMPLE_RENDER = false
// (the server test does both).
//
// The isometric view, and any view with a `present` option, is also
// rendered in presentation quality (path traced, with the material presets
// of materials.js looked up by each part's `material` attribute): the
// isometric one next to the picture, examples/3d/NN-name-render.png (the
// site shows it), the others to target/3d-views/NN-name-VIEW-render.png.
// `present` holds options for that render only (more materials, an
// environment, exposure). globalThis.EXAMPLE_PRESENT = false skips these
// renders; globalThis.EXAMPLE_PRESENT_SPP sets their samples.
//
// Files for the site, next to the picture, written with the renders: the
// isometric view as a prebaked scene, examples/3d/NN-name.bake3d
// (cad.bake3d: the parts' meshes, boxes and the render options, with the
// presentation look of that view, so the site's 3D viewer shows both
// render modes without a kernel). A sheet (mode: "2d") is also saved as a
// DWG for the site's 2D viewer, to target/3d-views/NN-name.dwg: the DWG
// writer's bytes differ from run to run, so the committed copy,
// examples/3d/NN-name.dwg, is updated by hand when the sheet changes (copy
// it over), as the 2D examples' DWGs are by their export step. A script
// that renders on its own calls run.bake() and run.drawing() itself.

const sum = (xs) => xs.reduce((a, b) => a + b, 0);

// The world box around a set of parts (cad.parts() objects).
function boxOf(parts) {
  const min = [Infinity, Infinity, Infinity], max = [-Infinity, -Infinity, -Infinity];
  for (const p of parts) {
    for (let i = 0; i < 3; i++) {
      min[i] = Math.min(min[i], p.bbox.min[i]);
      max[i] = Math.max(max[i], p.bbox.max[i]);
    }
  }
  return { min, max };
}

const fmt = (v) => (Array.isArray(v) ? `[${v.map(fmt).join(", ")}]` : Number(v).toPrecision(10));

function start(name) {
  const t0 = Date.now();
  let built = null;
  let renderMs = 0;
  const checks = [];
  const files = [];
  const site = [];

  // A number against the value the script computed: relative tolerance
  // `rel` (and an absolute floor `abs` for values near zero).
  function near(what, got, want, rel = 1e-9, abs = 1e-9) {
    const ok = Math.abs(got - want) <= Math.max(abs, rel * Math.abs(want));
    checks.push(what);
    if (!ok) throw new Error(`check failed: ${what}: got ${fmt(got)}, want ${fmt(want)}`);
  }

  // Two boxes {min, max} equal to `tol` mm.
  function box(what, got, want, tol = 1e-6) {
    for (const k of ["min", "max"]) {
      for (let i = 0; i < 3; i++) {
        if (!(Math.abs(got[k][i] - want[k][i]) <= tol)) {
          throw new Error(
            `check failed: ${what}: box ${k} is ${fmt(got[k])}, want ${fmt(want[k])}`,
          );
        }
      }
    }
    checks.push(what);
  }

  function equal(what, got, want) {
    checks.push(what);
    if (got !== want) throw new Error(`check failed: ${what}: got ${got}, want ${want}`);
  }

  function ok(what, cond, detail = "") {
    checks.push(what);
    if (!cond) throw new Error(`check failed: ${what}${detail ? `: ${detail}` : ""}`);
  }

  // Render each view. `views` is a list of cad.render options plus `as`,
  // the view's file suffix; the one with as: "iso" is the example's
  // picture (a 2D sheet, with mode: "2d", gets no presentation render).
  // Marks the end of the build on first use.
  async function render(views) {
    if (built === null) built = Date.now();
    if (globalThis.EXAMPLE_RENDER === false) return;
    const dir = globalThis.EXAMPLE_RENDER_DIR;
    for (const { as, present, ...opts } of views) {
      const out = as === "iso"
        ? `${dir ?? "examples/3d"}/${name}.png`
        : `${dir ?? "target/3d-views"}/${name}-${as}.png`;
      const r = await cad.render({ size: [960, 640], ...opts, out });
      renderMs += r.ms;
      files.push({ view: as, path: out, ms: r.ms, triangles: r.triangles });
      if ((as === "iso" || present) && opts.mode !== "2d" && globalThis.EXAMPLE_PRESENT !== false) {
        await presentation(as, opts, present ?? {});
      }
      if (as === "iso") await (opts.mode === "2d" ? drawing() : bake(opts, present ?? {}));
    }
  }

  // The prebaked scene for the site: the view's options with the
  // materials and the look of its presentation render.
  async function bake(opts, present = {}) {
    if (globalThis.EXAMPLE_RENDER === false) return;
    const mat = await cad.use("materials");
    // The view's own ground stays: a ground colour or material of the
    // presentation render would cover the working render's background too
    // (the working render draws the ground's colour).
    const { materials: extra, ground: _ground, ...rest } = present;
    const out = `${globalThis.EXAMPLE_RENDER_DIR ?? "examples/3d"}/${name}.bake3d`;
    const r = await cad.bake3d({ ...opts, materials: mat.option(extra ?? {}), ...rest, out });
    site.push({ path: out, bytes: r.bytes, triangles: r.triangles, chord: r.chord });
  }

  // The sheet as a DWG for the site's 2D viewer (see the top).
  async function drawing() {
    if (globalThis.EXAMPLE_RENDER === false) return;
    const out = `${globalThis.EXAMPLE_RENDER_DIR ?? "target/3d-views"}/${name}.dwg`;
    await cad.saveDwg(out);
    site.push({ path: out });
  }

  // The presentation render of one view: the same options, path traced,
  // with the material presets (plus the view's own `present` options).
  async function presentation(as, opts, present) {
    const mat = await cad.use("materials");
    const { materials: extra, ...rest } = present;
    const suffix = as === "iso" ? "render" : `${as}-render`;
    const out = `${globalThis.EXAMPLE_RENDER_DIR ?? (as === "iso" ? "examples/3d" : "target/3d-views")}/${name}-${suffix}.png`;
    const spp = globalThis.EXAMPLE_PRESENT_SPP;
    const r = await cad.render({
      size: [960, 640],
      ...opts,
      quality: "presentation",
      materials: mat.option(extra ?? {}),
      ...(spp ? { spp } : {}),
      ...rest,
      out,
    });
    renderMs += r.ms;
    files.push({ view: "render", path: out, ms: r.ms, samples: r.samples, trace: r.trace });
  }

  // What the script returns: parts, volume, box, checks, files, times.
  function summary(extra = {}) {
    if (built === null) built = Date.now();
    const parts = cad.parts();
    const defs = new Set(parts.map((p) => p.def)).size;
    const b = boxOf(parts);
    const r1 = (v) => Math.round(v * 1000) / 1000;
    return {
      example: name,
      parts: parts.length,
      definitions: defs,
      volume: r1(sum(parts.map((p) => p.volume))),
      bbox: { min: b.min.map(r1), max: b.max.map(r1) },
      checks: checks.length,
      ...extra,
      files: files.map((f) => f.path),
      ...(files.some((f) => f.samples)
        ? { presentation: files.filter((f) => f.samples).map(({ path, ms, samples, trace }) => ({ path, ms, samples, trace })) }
        : {}),
      ...(site.length ? { site } : {}),
      ms: {
        build: built - t0,
        render: Math.round(renderMs * 10) / 10,
        total: Date.now() - t0,
      },
    };
  }

  return { near, box, equal, ok, render, bake, drawing, summary };
}

return { start, boxOf, sum };
