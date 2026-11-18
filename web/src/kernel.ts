// The solid kernel, loaded on demand.
//
// The page's main module has no kernel. `just build-kernel` writes it as a
// second WebAssembly module, `kernel/cadview-kernel-<hash>.wasm`, with
// `kernel/manifest.json` naming that file and its wire version. Nothing is
// fetched until `loadKernel()` runs: a viewer showing a prebaked scene never
// downloads it. The file name carries its content hash, so the HTTP cache
// may keep it forever; the manifest is fetched fresh, and a manifest (or a
// module) whose wire version differs from the main module's is refused, so
// a stale kernel is never used.
//
// The module runs on the page's thread, called synchronously through
// `globalThis.__codecadKernel.call(bytes) -> bytes` by the main module's
// RemoteEvaluator (cadview-core `solid::remote`).

import { kernel_connect, kernel_wire_version } from "cadview-wasm";

/** What `loadKernel()` reports. */
export interface KernelInfo {
  /** Kernel name and version, e.g. "arris 0.11.1". */
  kernel: string;
  file: string;
  /** Size of the module file (uncompressed). */
  bytes: number;
  /** Fetch and compile, and instantiate and connect (ms). */
  compileMs: number;
  connectMs: number;
}

interface KernelManifest {
  file: string;
  wire: number;
  bytes: number;
  sha256: string;
}

interface KernelExports {
  memory: WebAssembly.Memory;
  ck_alloc(len: number): number;
  ck_free(ptr: number, len: number): void;
  ck_call(ptr: number, len: number): number;
  ck_reply(): number;
  ck_wire(): number;
}

declare global {
  var __codecadKernel: { call(request: Uint8Array): Uint8Array } | undefined;
}

let _base: URL | null = null;
let _loading: Promise<KernelInfo> | null = null;

/** Where `manifest.json` and the module are (default: `kernel/` next to the page). */
export function setKernelBase(url: string | URL): void {
  _base = new URL(url);
}

/** The message a call that needs geometry gives before the kernel is loaded. */
export const KERNEL_HINT =
  "the solid kernel is not loaded in this page yet: run `await cad.loadKernel()` once (about 1.5 MB), then call again";

/** Load the kernel module once (later calls share the first load). */
export function loadKernel(): Promise<KernelInfo> {
  if (!_loading) {
    _loading = load().catch((e) => {
      _loading = null;
      throw e;
    });
  }
  return _loading;
}

/** Imports the module's dependency tree declares but never calls. */
function stubImports(module: WebAssembly.Module): WebAssembly.Imports {
  const imports: Record<string, Record<string, unknown>> = {};
  for (const imp of WebAssembly.Module.imports(module)) {
    imports[imp.module] ??= {};
    imports[imp.module][imp.name] = () => {
      throw new Error(`kernel module called ${imp.module}.${imp.name}`);
    };
  }
  return imports as WebAssembly.Imports;
}

async function load(): Promise<KernelInfo> {
  const root = _base ?? new URL("kernel/", document.baseURI);
  const wire = kernel_wire_version();
  const t0 = performance.now();
  const resp = await fetch(new URL("manifest.json", root), {
    cache: "no-cache",
  });
  if (!resp.ok) {
    throw new Error(
      `this build has no solid kernel module (kernel/manifest.json: ${resp.status})`,
    );
  }
  const man = (await resp.json()) as KernelManifest;
  if (man.wire !== wire) {
    throw new Error(
      `the kernel module speaks wire version ${man.wire} and this page ${wire}: a stale build`,
    );
  }
  const url = new URL(man.file, root);
  let module: WebAssembly.Module;
  try {
    module = await WebAssembly.compileStreaming(fetch(url));
  } catch {
    // A server that does not send application/wasm.
    const r = await fetch(url);
    if (!r.ok) throw new Error(`${man.file}: ${r.status}`);
    module = await WebAssembly.compile(await r.arrayBuffer());
  }
  const t1 = performance.now();
  const imports = stubImports(module);
  let instance = await WebAssembly.instantiate(module, imports);
  const ex = () => instance.exports as unknown as KernelExports;
  if (ex().ck_wire() !== wire) {
    throw new Error(
      `the kernel module speaks wire version ${ex().ck_wire()} and this page ${wire}`,
    );
  }
  globalThis.__codecadKernel = {
    call(request: Uint8Array): Uint8Array {
      const e = ex();
      const p = e.ck_alloc(request.length);
      new Uint8Array(e.memory.buffer, p, request.length).set(request);
      let n: number;
      try {
        n = e.ck_call(p, request.length);
      } catch (err) {
        // A trap leaves the module's state unusable: start a fresh one. Its
        // solids are gone, so the page must connect again.
        instance = new WebAssembly.Instance(module, imports);
        _loading = null;
        throw new Error(
          `the solid kernel stopped (${err}); run \`await cad.loadKernel()\` again`,
        );
      }
      const out = new Uint8Array(e.memory.buffer, e.ck_reply(), n).slice();
      e.ck_free(p, request.length);
      return out;
    },
  };
  const r = JSON.parse(kernel_connect());
  if (r.error) throw new Error(r.error);
  const t2 = performance.now();
  console.log(
    `[CodeCAD] solid kernel loaded: ${r.kernel} (${man.file}, ${man.bytes} bytes, ${Math.round(t1 - t0)} ms to fetch and compile)`,
  );
  return {
    kernel: r.kernel,
    file: man.file,
    bytes: man.bytes,
    compileMs: t1 - t0,
    connectMs: t2 - t1,
  };
}
