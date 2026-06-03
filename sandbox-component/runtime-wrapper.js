// Runtime wrapper for cadview WASM sandbox component.
//
// Compiled by jco componentize into a WASM component.
// WIT imports (cad-call, rpc-call, read-file) become host functions.
//
// The Rust host prepends a cad API setup snippet to the user's program
// before calling run(). That snippet sets up globalThis.cad using the
// __cadCall / __rpcCall / __readFile bridge functions defined here.

import { cadCall, rpcCall, readFile } from "cadview:sandbox/cad@0.1.0";

// Expose WIT imports as bridge functions on globalThis.
// The host-prepended cad-api setup code reads from these.
globalThis.__cadCall = (method, argsJson) => cadCall(method, argsJson);
globalThis.__rpcCall = (method, argsJson) => rpcCall(method, argsJson);
globalThis.__readFile = (path) => readFile(path);

// AsyncFunction constructor for top-level await support.
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

// WIT export: run(program: string) -> result<string, string>
// componentize-js maps: return value -> Ok, thrown exception -> Err.
export async function run(program) {
  try {
    // The host prepends cad-api-setup.js which sets globalThis.cad.
    // User script accesses `cad` as a global.
    const fn_ = new AsyncFunction(program);
    const result = await fn_.call(globalThis);

    if (result === undefined || result === null) return "";
    if (typeof result === "string") return result;
    return JSON.stringify(result);
  } catch (error) {
    throw error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  }
}
