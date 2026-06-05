// CodeCAD embeddable viewer Web Component.
//
// Usage:
//   <script type="module" src="codecad-viewer.js"></script>
//   <codecad-viewer src="drawing.dwg" style="width:800px;height:400px"></codecad-viewer>
//
// Shadow DOM for full style encapsulation. Canvas element is passed
// directly to start_renderer (no getElementById), so Shadow DOM works.

import {
  session_create,
  session_destroy,
  session_load_dwg,
  session_use,
  cad_call,
  start_renderer,
  stop_renderer,
} from "cadview-wasm";
import { initCadEmbed } from "./cad";

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
  align-items: center;
  justify-content: center;
  font: 12px/1 "JetBrains Mono", monospace;
  color: #6b7280;
}
`;

class CodecadViewer extends HTMLElement {
  static observedAttributes = ["src"];

  private shadow: ShadowRoot;
  private canvas: HTMLCanvasElement;
  private hint: HTMLElement;
  private loadingEl: HTMLElement;
  private sessionId = "";
  private rendererKey = "";
  private observer: IntersectionObserver | null = null;
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
    badge.href = "https://github.com/nicholasgasior/codecad";
    badge.target = "_blank";
    badge.rel = "noopener";

    this.hint = document.createElement("div");
    this.hint.className = "hint";
    this.hint.textContent = "drag to pan \u00b7 scroll to zoom";

    this.loadingEl = document.createElement("div");
    this.loadingEl.className = "loading";
    this.loadingEl.textContent = "Loading\u2026";

    this.shadow.append(style, this.canvas, badge, this.hint, this.loadingEl);
  }

  connectedCallback() {
    this.sessionId = `embed_${viewerCount++}`;

    // Show hint on first mouse enter
    this.addEventListener("mouseenter", this.onFirstHover, { once: true });

    // Lazy-init: only load WASM + DWG when viewer is near the viewport
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

  attributeChangedCallback(name: string, oldVal: string | null, newVal: string | null) {
    if (name === "src" && oldVal !== newVal && this.initialized) {
      this.loadSrc(newVal);
    }
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
      wasmReady = initCadEmbed();
    }
    await wasmReady;

    session_create(this.sessionId);
    session_use(this.sessionId);

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

    this.initialized = true;
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
      this.loadingEl.textContent = `Failed to load: ${e}`;
    }
  }

  private cleanup() {
    if (this.rendererKey) {
      try { stop_renderer(this.rendererKey); } catch { /* ok */ }
      this.rendererKey = "";
    }
    if (this.sessionId) {
      try { session_destroy(this.sessionId); } catch { /* ok */ }
      this.sessionId = "";
    }
    this.initialized = false;
  }
}

customElements.define("codecad-viewer", CodecadViewer);
