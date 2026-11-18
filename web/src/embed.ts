// CodeCAD embeddable viewer Web Component.
//
// Usage:
//   <script type="module" src="codecad-viewer.js"></script>
//   <codecad-viewer src="drawing.dwg" style="width:800px;height:400px"></codecad-viewer>
//   <codecad-viewer scene="part.bake3d" quality="working"></codecad-viewer>
//
// `src` shows a DWG in 2D. `scene` shows a prebaked 3D scene (made by
// `cad.bake3d` on a server with a kernel) in 3D, with a switch between the
// working render and the path-traced presentation render. No kernel is
// downloaded: the scene holds the meshes, boxes and render options.
//
// Shadow DOM for full style encapsulation. Canvas element is passed
// directly to start_renderer (no getElementById), so Shadow DOM works.

import {
  cad_call,
  session_create,
  session_destroy,
  session_load_bake,
  session_load_dwg,
  session_use,
  start_renderer,
  start_view3d,
  stop_renderer,
  view3d_command,
  view3d_stats,
} from "cadview-wasm";
import {
  hasWebGpu,
  initCadEmbed,
  NO_WEBGPU_3D,
  setLoadProgressCallback,
} from "./cad";
import { setKernelBase } from "./kernel";

// The kernel module (if a build ships one) sits next to this file. A viewer
// of prebaked scenes never loads it.
setKernelBase(new URL("./kernel/", import.meta.url));

let wasmReady: Promise<void> | null = null;
let viewerCount = 0;

const STYLES = `
:host {
  display: block;
  position: relative;
  overflow: hidden;
  background: #0a0e14;
}
canvas {
  display: block;
  width: 100%;
  height: 100%;
  cursor: crosshair;
}
:host([scene]) canvas { cursor: grab; }
.badge {
  position: absolute;
  bottom: 6px;
  right: 8px;
  font: 10px/1 "JetBrains Mono", monospace;
  color: #6b7280;
  text-decoration: none;
  opacity: 0.6;
  pointer-events: auto;
  user-select: none;
  z-index: 1;
}
.badge:hover { opacity: 1; }
.hint {
  position: absolute;
  bottom: 6px;
  left: 50%;
  transform: translateX(-50%);
  font: 11px/1 "JetBrains Mono", monospace;
  color: #e8e8e8;
  background: rgba(10, 14, 20, 0.85);
  padding: 4px 10px;
  border-radius: 3px;
  opacity: 0;
  pointer-events: none;
  z-index: 1;
  transition: opacity 0.3s;
}
.hint.show {
  animation: hint-fade 2.5s ease-out forwards;
}
@keyframes hint-fade {
  0% { opacity: 0; }
  10% { opacity: 1; }
  70% { opacity: 1; }
  100% { opacity: 0; }
}
.loading {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
}
.loading svg {
  width: 48px;
  height: 48px;
}
.loading line, .loading circle {
  fill: none;
  stroke: #ffb000;
  stroke-width: 1.2;
}
.loading .lv {
  stroke-dasharray: 80;
  stroke-dashoffset: 80;
  animation: ld 1.2s ease-out forwards, lp 2.4s ease-in-out 1.2s infinite;
}
.loading .lh {
  stroke-dasharray: 80;
  stroke-dashoffset: 80;
  animation: ld 1.2s ease-out 0.15s forwards, lp 2.4s ease-in-out 1.35s infinite;
}
.loading .lc {
  stroke-dasharray: 63;
  stroke-dashoffset: 63;
  animation: ld 0.8s ease-out 0.6s forwards, lp 2.4s ease-in-out 1.4s infinite;
}
@keyframes ld { to { stroke-dashoffset: 0; } }
@keyframes lp { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }
.loading span {
  font: 10px/1 "JetBrains Mono", monospace;
  color: #6b7280;
  letter-spacing: 0.1em;
}
.error {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font: 11px/1.4 "JetBrains Mono", monospace;
  color: #6b7280;
  padding: 16px;
  text-align: center;
}
.modes {
  position: absolute;
  top: 8px;
  left: 8px;
  display: none;
  z-index: 1;
  background: rgba(14, 18, 26, 0.92);
  border: 1px solid #6b7280;
  border-radius: 3px;
  overflow: hidden;
}
.modes.show { display: flex; }
.modes button {
  background: none;
  border: none;
  color: #6b7280;
  font: 11px/1.3 "JetBrains Mono", monospace;
  padding: 4px 9px;
  cursor: pointer;
}
.modes button + button { border-left: 1px solid #1e2430; }
.modes button:hover { color: #e8e8e8; }
.modes button[aria-checked="true"] {
  color: #e8e8e8;
  background: rgba(255, 255, 255, 0.06);
  box-shadow: inset 0 -2px 0 #ffb000;
}
.samples {
  position: absolute;
  bottom: 6px;
  left: 8px;
  font: 10px/1 "JetBrains Mono", monospace;
  color: #e8e8e8;
  background: rgba(10, 14, 20, 0.85);
  padding: 3px 6px;
  border-radius: 3px;
  display: none;
  z-index: 1;
  pointer-events: none;
}
.samples.show { display: block; }
`;

class CodecadViewer extends HTMLElement {
  static observedAttributes = ["src", "scene"];

  private shadow: ShadowRoot;
  private canvas: HTMLCanvasElement;
  private hint: HTMLElement;
  private loadingEl: HTMLElement;
  private modes: HTMLElement;
  private samples: HTMLElement;
  private sessionId = "";
  private rendererKey = "";
  private observer: IntersectionObserver | null = null;
  private poll: ReturnType<typeof setInterval> | null = null;
  private initialized = false;
  private hintShown = false;

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "open" });

    const style = document.createElement("style");
    style.textContent = STYLES;

    this.canvas = document.createElement("canvas");

    const badge = document.createElement("a");
    badge.className = "badge";
    badge.textContent = "CodeCAD";
    badge.href = "https://github.com/backkem/codecad";
    badge.target = "_blank";
    badge.rel = "noopener";

    this.hint = document.createElement("div");
    this.hint.className = "hint";
    this.hint.textContent = "drag to pan · scroll to zoom";

    this.loadingEl = document.createElement("div");
    this.loadingEl.className = "loading";
    this.loadingEl.innerHTML =
      '<svg viewBox="0 0 80 80">' +
      '<line x1="40" y1="0" x2="40" y2="80" class="lv"/>' +
      '<line x1="0" y1="40" x2="80" y2="40" class="lh"/>' +
      '<circle cx="40" cy="40" r="10" class="lc"/>' +
      "</svg><span>loading</span>";

    this.modes = document.createElement("div");
    this.modes.className = "modes";
    this.modes.setAttribute("role", "radiogroup");
    for (const [q, label] of [
      ["working", "Working"],
      ["presentation", "Presentation"],
    ]) {
      const b = document.createElement("button");
      b.type = "button";
      b.setAttribute("role", "radio");
      b.dataset.quality = q;
      b.textContent = label;
      b.addEventListener("click", () => this.setQuality(q));
      this.modes.append(b);
    }

    this.samples = document.createElement("div");
    this.samples.className = "samples";

    this.shadow.append(
      style,
      this.canvas,
      badge,
      this.hint,
      this.loadingEl,
      this.modes,
      this.samples,
    );
  }

  connectedCallback() {
    this.sessionId = `embed_${viewerCount++}`;

    // Show hint on first mouse enter
    this.addEventListener("mouseenter", this.onFirstHover, { once: true });

    // Lazy-init: only load WASM + the file when the viewer is near the viewport
    this.observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) {
          this.observer?.disconnect();
          this.observer = null;
          this.initViewer();
        }
      },
      { rootMargin: "200px" },
    );
    this.observer.observe(this);
  }

  disconnectedCallback() {
    this.observer?.disconnect();
    this.removeEventListener("mouseenter", this.onFirstHover);
    this.cleanup();
  }

  attributeChangedCallback(
    name: string,
    oldVal: string | null,
    newVal: string | null,
  ) {
    if (oldVal === newVal || !this.initialized) return;
    if (name === "src" && !this.hasAttribute("scene")) this.loadSrc(newVal);
    if (name === "scene") this.loadScene(newVal);
  }

  private onFirstHover = () => {
    if (!this.hintShown) {
      this.hintShown = true;
      this.hint.classList.add("show");
    }
  };

  private async initViewer() {
    // Singleton WASM init (shared across all <codecad-viewer> on the page)
    if (!wasmReady) {
      const pctEl = this.loadingEl.querySelector("span");
      setLoadProgressCallback((pct) => {
        if (pctEl) pctEl.textContent = `loading ${pct}%`;
      });
      wasmReady = initCadEmbed();
    }
    await wasmReady;

    session_create(this.sessionId);
    session_use(this.sessionId);
    this.initialized = true;

    const scene = this.getAttribute("scene");
    if (scene) {
      await this.loadScene(scene);
      return;
    }

    const result = JSON.parse(
      start_renderer(this.canvas, this.sessionId, "vello"),
    );
    if (result.error) {
      // Vello failed (no WebGPU), try egui
      const fallback = JSON.parse(
        start_renderer(this.canvas, this.sessionId, "egui"),
      );
      this.rendererKey = fallback.key ?? "";
    } else {
      this.rendererKey = result.key;
    }

    await this.loadSrc(this.getAttribute("src"));
  }

  private async loadSrc(src: string | null) {
    if (!src) {
      this.loadingEl.style.display = "none";
      return;
    }
    this.loadingEl.style.display = "flex";
    try {
      const resp = await fetch(src);
      if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
      const buf = await resp.arrayBuffer();
      session_use(this.sessionId);
      session_load_dwg(this.sessionId, new Uint8Array(buf));
      cad_call("fitView", "{}");
      this.loadingEl.style.display = "none";
    } catch (e) {
      this.showError(`Failed to load: ${e}`);
    }
  }

  /** A prebaked 3D scene: no kernel, WebGPU only. */
  private async loadScene(url: string | null) {
    if (!url) return;
    this.hint.textContent =
      "drag to orbit · right-drag to pan · scroll to zoom";
    this.loadingEl.style.display = "flex";
    if (!(await hasWebGpu())) {
      this.showError(NO_WEBGPU_3D);
      return;
    }
    try {
      const resp = await fetch(url);
      if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
      const bytes = new Uint8Array(await resp.arrayBuffer());
      const r = JSON.parse(session_load_bake(this.sessionId, bytes, true));
      if (r.error) throw new Error(r.error);
      const quality = this.getAttribute("quality") ?? "working";
      if (!this.rendererKey) {
        const s = JSON.parse(
          start_view3d(
            this.canvas,
            this.sessionId,
            JSON.stringify({ quality }),
          ),
        );
        if (s.error) throw new Error(s.error);
        this.rendererKey = s.key;
      }
      this.markQuality(quality);
      this.modes.classList.add("show");
      this.startPoll();
      this.loadingEl.style.display = "none";
    } catch (e) {
      this.showError(`Failed to load: ${e}`);
    }
  }

  private setQuality(q: string) {
    if (!this.rendererKey) return;
    view3d_command(this.rendererKey, JSON.stringify({ quality: q }));
    this.markQuality(q);
  }

  private markQuality(q: string) {
    for (const b of this.modes.querySelectorAll("button")) {
      b.setAttribute("aria-checked", String(b.dataset.quality === q));
    }
    this.samples.classList.toggle("show", q === "presentation");
  }

  private startPoll() {
    if (this.poll) return;
    this.poll = setInterval(() => {
      if (!this.rendererKey) return;
      const s = JSON.parse(view3d_stats(this.rendererKey));
      if (s.error && !s.frames) {
        this.showError(s.error);
        return;
      }
      this.samples.textContent = `${s.samples} / ${s.target} spp${s.traceScale > 1 ? " · half res" : ""}`;
    }, 250);
  }

  private showError(msg: string) {
    this.loadingEl.style.display = "flex";
    this.loadingEl.className = "error";
    this.loadingEl.textContent = msg;
  }

  private cleanup() {
    if (this.poll) {
      clearInterval(this.poll);
      this.poll = null;
    }
    if (this.rendererKey) {
      try {
        stop_renderer(this.rendererKey);
      } catch {
        /* ok */
      }
      this.rendererKey = "";
    }
    if (this.sessionId) {
      try {
        session_destroy(this.sessionId);
      } catch {
        /* ok */
      }
      this.sessionId = "";
    }
    this.initialized = false;
  }
}

customElements.define("codecad-viewer", CodecadViewer);
