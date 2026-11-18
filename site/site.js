// The gallery (gallery.html) and the example page (example.html?id=...).
// Plain modules over the data in examples.js: no framework, no build step.

import { EXAMPLES, exampleById } from "./examples.js";

const esc = (s) =>
  String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const LABEL = { model: "Model", drawing: "Drawing" };
const kindsOf = (e) => ["model", "drawing"].filter((k) => e[k]);
const badges = (e) => kindsOf(e).map((k) => `<span class="badge badge-${k}">${LABEL[k]}</span>`).join("");

// The card picture: the model's presentation render (else its working
// render) on the Models shelf and for All; the sheet on the Drawings shelf.
function cardImage(e, shelf) {
  if (e.drawing && (shelf === "drawings" || !e.model || !(e.model.render || e.model.working))) return e.drawing.png;
  return e.model.render ?? e.model.working;
}

function pageUrl(e, shelf) {
  const tab = shelf === "drawings" && e.drawing ? "#drawing" : "";
  return `example.html?id=${encodeURIComponent(e.id)}${tab}`;
}

// ── Gallery ──────────────────────────────────────────────────────────

const SHELVES = [
  { id: "all", label: "All", has: () => true },
  { id: "models", label: "Models", has: (e) => !!e.model },
  { id: "drawings", label: "Drawings", has: (e) => !!e.drawing },
];

function gallery(root) {
  const filters = root.querySelector(".filters");
  const grid = root.querySelector(".examples-gallery");
  filters.innerHTML = SHELVES.map(
    (s) =>
      `<button type="button" role="tab" data-shelf="${s.id}" class="filter filter-${s.id}">${s.label}` +
      ` <span class="count">${EXAMPLES.filter(s.has).length}</span></button>`,
  ).join("");
  const show = (id) => {
    const shelf = SHELVES.find((s) => s.id === id) ?? SHELVES[0];
    for (const b of filters.querySelectorAll("button")) b.setAttribute("aria-selected", String(b.dataset.shelf === shelf.id));
    grid.innerHTML = EXAMPLES.filter(shelf.has)
      .map(
        (e) => `<a href="${pageUrl(e, shelf.id)}" class="example-link">
  <img src="${esc(cardImage(e, shelf.id))}" alt="${esc(e.title)}" loading="lazy" width="960" height="640">
  <div class="caption">
    <div class="badges">${badges(e)}</div>
    <div class="title">${esc(e.title)}</div>
    <div class="desc">${esc(e.desc)}</div>
  </div>
</a>`,
      )
      .join("");
  };
  filters.addEventListener("click", (ev) => {
    const b = ev.target.closest("button[data-shelf]");
    if (!b) return;
    history.replaceState(null, "", b.dataset.shelf === "all" ? location.pathname : `#${b.dataset.shelf}`);
    show(b.dataset.shelf);
  });
  window.addEventListener("hashchange", () => show(location.hash.slice(1)));
  show(location.hash.slice(1));
}

// ── Script source, lightly highlighted ───────────────────────────────

const TOKEN =
  /(\/\/[^\n]*|\/\*[\s\S]*?\*\/)|("(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`)|\b(\d+(?:\.\d+)?(?:e[+-]?\d+)?)\b|\b(const|let|var|function|return|if|else|for|of|in|while|await|async|new|throw|typeof|true|false|null|undefined|break|continue)\b|\b(cad\.[A-Za-z_$][\w$.]*)/g;

function highlight(src) {
  let out = "";
  let at = 0;
  for (const m of src.matchAll(TOKEN)) {
    out += esc(src.slice(at, m.index));
    const cls = m[1] ? "cmt" : m[2] ? "str" : m[3] ? "num" : m[4] ? "kw" : "fn";
    out += `<span class="${cls}">${esc(m[0])}</span>`;
    at = m.index + m[0].length;
  }
  return out + esc(src.slice(at));
}

// The libraries a script loads (`cad.use("name")`), next to it in lib/.
const libPath = (script, name) => `${script.slice(0, script.lastIndexOf("/"))}${script.includes("/lib/") ? "" : "/lib"}/${name}.js`;

async function sources(scripts) {
  const files = [];
  const seen = new Set();
  const queue = [...scripts];
  while (queue.length) {
    const path = queue.shift();
    if (seen.has(path)) continue;
    seen.add(path);
    const resp = await fetch(path);
    if (!resp.ok) {
      files.push({ path, text: `// ${path}: ${resp.status} ${resp.statusText}` });
      continue;
    }
    const text = await resp.text();
    files.push({ path, text });
    for (const m of text.matchAll(/cad\.use\(\s*["']([\w-]+)["']\s*\)/g)) queue.push(libPath(path, m[1]));
  }
  return files;
}

// ── Example page ─────────────────────────────────────────────────────

const TABS = [
  { id: "model", label: "Model" },
  { id: "drawing", label: "Drawing" },
  { id: "script", label: "Script" },
];

function figure(src, caption) {
  return src
    ? `<figure><a href="${esc(src)}"><img src="${esc(src)}" alt="${esc(caption)}" loading="lazy" width="960" height="640"></a><figcaption>${esc(caption)}</figcaption></figure>`
    : "";
}

function modelPanel(e) {
  const m = e.model;
  const script = e.scripts[0];
  return `<div class="viewer-slot" data-kind="model"></div>
<p class="viewer-note">Drag to orbit, right-drag to pan, scroll to zoom. Switch between the working and the presentation render at the top left. The presentation render is path traced in the page and takes a while on a slow GPU. Needs WebGPU.</p>
<div class="figures">${figure(m.working, "Working render, written by the script on the server")}${figure(m.render, "Presentation render, path traced by the script on the server")}</div>
<div class="notes">
  <p>The page shows a prebaked scene: meshes, boxes and render options that the server wrote with <code>cad.bake3d</code> after its solid kernel built the parts. Orbiting, zooming and both render modes run here; nothing is modelled in the page.</p>
  <p class="links"><a href="${esc(m.scene)}" download>Scene (.bake3d)</a></p>
  <pre class="cmd">cargo run -p cadview-server --release -- --exec ${esc(script)}</pre>
</div>`;
}

function drawingPanel(e) {
  const d = e.drawing;
  return `<div class="viewer-slot" data-kind="drawing"></div>
<p class="viewer-note">Drag to pan, scroll to zoom. The DWG is read in the page.</p>
<div class="figures one">${figure(d.png, "The sheet as the script rendered it")}</div>
<p class="links"><a href="app/?file=../${esc(d.dwg)}">Open in the app</a> <a href="${esc(d.dwg)}" download>DWG</a> <a href="${esc(d.png)}">PNG</a></p>`;
}

function examplePage(root) {
  const id = new URLSearchParams(location.search).get("id");
  const e = exampleById(id);
  if (!e) {
    root.innerHTML = `<h1>Example not found</h1><p class="lede">No example named "${esc(id ?? "")}". See <a href="gallery.html">all examples</a>.</p>`;
    return;
  }
  document.title = `${e.title} | CodeCAD`;
  const tabs = TABS.filter((t) => t.id === "script" || e[t.id]);
  const i = EXAMPLES.indexOf(e);
  const prev = EXAMPLES[(i + EXAMPLES.length - 1) % EXAMPLES.length];
  const next = EXAMPLES[(i + 1) % EXAMPLES.length];
  root.innerHTML = `<nav class="crumbs"><a href="gallery.html">Examples</a> <span>/</span> ${esc(e.title)}</nav>
<header class="example-head">
  <div class="badges">${badges(e)}</div>
  <h1>${esc(e.title)}</h1>
  <p class="lede">${esc(e.text)}</p>
</header>
<div class="tabs" role="tablist">${tabs
    .map(
      (t) =>
        `<a role="tab" href="#${t.id}" id="tab-${t.id}" class="tab tab-${t.id}" aria-controls="panel-${t.id}">${t.label}${t.id === "drawing" && e.drawing.title ? `<span class="sub">${esc(e.drawing.title)}</span>` : ""}</a>`,
    )
    .join("")}</div>
${tabs
  .map(
    (t) =>
      `<section role="tabpanel" id="panel-${t.id}" aria-labelledby="tab-${t.id}" class="panel" hidden>${
        t.id === "model" ? modelPanel(e) : t.id === "drawing" ? drawingPanel(e) : '<div class="files" role="tablist"></div><pre class="source">loading</pre>'
      }</section>`,
  )
  .join("\n")}
<nav class="pager"><a href="${pageUrl(prev)}">&larr; ${esc(prev.title)}</a><a href="${pageUrl(next)}">${esc(next.title)} &rarr;</a></nav>`;

  let scriptsLoaded = false;
  const show = (want) => {
    const tab = tabs.find((t) => t.id === want) ?? tabs[0];
    for (const t of tabs) {
      const on = t.id === tab.id;
      root.querySelector(`#tab-${t.id}`).setAttribute("aria-selected", String(on));
      root.querySelector(`#panel-${t.id}`).hidden = !on;
    }
    const panel = root.querySelector(`#panel-${tab.id}`);
    const slot = panel.querySelector(".viewer-slot");
    // A viewer is made the first time its tab shows, so a page loads only
    // what the open tab needs (the scene or the DWG, never a kernel).
    if (slot && !slot.firstChild) {
      const v = document.createElement("codecad-viewer");
      v.className = "viewer";
      if (slot.dataset.kind === "model") {
        v.setAttribute("scene", e.model.scene);
        v.setAttribute("background", "viewer");
      } else {
        v.setAttribute("src", e.drawing.dwg);
      }
      slot.append(v);
    }
    if (tab.id === "script" && !scriptsLoaded) {
      scriptsLoaded = true;
      loadScripts(panel, e);
    }
  };
  root.querySelector(".tabs").addEventListener("click", (ev) => {
    const a = ev.target.closest("a[role=tab]");
    if (!a) return;
    ev.preventDefault();
    history.replaceState(null, "", a.getAttribute("href"));
    show(a.getAttribute("href").slice(1));
  });
  window.addEventListener("hashchange", () => show(location.hash.slice(1)));
  show(location.hash.slice(1));
}

async function loadScripts(panel, e) {
  const list = panel.querySelector(".files");
  const pre = panel.querySelector(".source");
  const files = await sources(e.scripts);
  const name = (p) => (p.includes("/lib/") ? `lib/${p.split("/").pop()}` : p.split("/").pop());
  list.innerHTML = files
    .map((f, i) => `<button type="button" role="tab" data-i="${i}" class="file${f.path.includes("/lib/") ? " lib" : ""}">${esc(name(f.path))}</button>`)
    .join("");
  const pick = (i) => {
    for (const b of list.querySelectorAll("button")) b.setAttribute("aria-selected", String(Number(b.dataset.i) === i));
    pre.innerHTML = highlight(files[i].text);
    pre.scrollTop = 0;
  };
  list.addEventListener("click", (ev) => {
    const b = ev.target.closest("button[data-i]");
    if (b) pick(Number(b.dataset.i));
  });
  pick(0);
}

const g = document.querySelector("[data-gallery]");
if (g) gallery(g);
const ex = document.querySelector("[data-example]");
if (ex) examplePage(ex);
