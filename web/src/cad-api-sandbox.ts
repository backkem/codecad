// cad-api-sandbox.ts - Sandbox entry point.
//
// esbuild bundles this into a single .js file that the Rust host
// prepends to user scripts. It reads __cadCall / __rpcCall / __readFile
// bridge functions from globalThis (set by runtime-wrapper.js) and
// builds globalThis.cad using the shared buildCadApi factory.

import { buildCadApi } from "./cad-api";

const g = globalThis as Record<string, unknown>;

g.cad = buildCadApi({
  call: g.__cadCall as (method: string, argsJson: string) => string,
  rpc: g.__rpcCall as (method: string, argsJson: string) => string,
  readFile: g.__readFile as ((path: string) => string) | null,
});
