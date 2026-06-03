import { defineConfig } from "vite";
import { resolve } from "path";

// Vite library build: emits cad-api-setup.js for the server-side sandbox.
// The Rust host include_str!s this and prepends it to user scripts.
export default defineConfig({
  build: {
    lib: {
      entry: resolve(__dirname, "src/cad-api-sandbox.ts"),
      formats: ["iife"],
      name: "cadApiSetup",
      fileName: () => "cad-api-setup.js",
    },
    outDir: resolve(__dirname, "../cad-client"),
    emptyOutDir: false,
  },
});
