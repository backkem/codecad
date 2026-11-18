// Type declarations for the cadview-web wasm-bindgen module.
// wasm-bindgen generates cadview-web.js + cadview-web.d.ts into dist/,
// but we import from a path alias resolved by Vite at build time.

declare module "cadview-wasm" {
  // cad_call targets the js_target session (set by session_use)
  export function cad_call(method: string, args_json: string): string;

  // Session lifecycle
  export function session_create(session_id: string): string;
  export function session_destroy(session_id: string): string;
  export function session_use(session_id: string): string;
  export function session_current(): string;
  export function session_list(): string;
  export function session_load_dwg(
    session_id: string,
    data: Uint8Array,
  ): string;

  // Renderer lifecycle (per canvas, renderer_type = "vello" | "egui")
  export function start_renderer(
    canvas: HTMLCanvasElement,
    session_id: string,
    renderer_type: string,
  ): string;
  export function stop_renderer(renderer_key: string): string;

  // 3D view (WebGPU only); stopped with stop_renderer
  export function start_view3d(
    canvas: HTMLCanvasElement,
    session_id: string,
    options_json: string,
  ): string;
  export function view3d_command(key: string, command_json: string): string;
  export function view3d_stats(key: string): string;
  /** A prebaked 3D scene; adopt: the session takes its parts. */
  export function session_load_bake(
    session_id: string,
    data: Uint8Array,
    adopt: boolean,
  ): string;
  export function gpu_devices_created(): number;

  // The solid kernel module (loaded on demand by kernel.ts)
  export function kernel_wire_version(): number;
  export function kernel_expect(hint: string): void;
  export function kernel_connect(): string;
  export function kernel_status(): string;

  // Binary export (returns bytes for browser download)
  export function export_dwg_bytes(session_id: string): Uint8Array;
  /** opts_json: JSON of SavePdfOptions; throws on an unknown `colors`. */
  export function export_pdf_bytes(
    session_id: string,
    opts_json: string,
  ): Uint8Array;

  // Yrs sync (per-session, takes explicit session_id)
  export function yrs_state_vector(session_id: string): Uint8Array;
  export function yrs_apply_update(
    session_id: string,
    update: Uint8Array,
  ): string;
  export function yrs_encode_update(
    session_id: string,
    remote_sv: Uint8Array,
  ): Uint8Array;
  export function yrs_pending_update(session_id: string): Uint8Array;

  export default function init(): Promise<void>;
}
