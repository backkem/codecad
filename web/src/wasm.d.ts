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
    canvas_id: string,
    session_id: string,
    renderer_type: string,
  ): string;
  export function stop_renderer(canvas_id: string): string;

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
