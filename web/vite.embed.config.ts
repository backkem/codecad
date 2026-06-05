import { defineConfig, type Plugin } from "vite";
import { resolve } from "path";
import { copyFileSync } from "fs";

const distDir = resolve(__dirname, "../dist");
const outDir = resolve(__dirname, "../dist-embed");

/** Copy the wasm-bindgen JS glue and WASM binary to the output directory. */
function copyWasmFiles(): Plugin {
  return {
    name: "copy-wasm-files",
    writeBundle() {
      copyFileSync(
        resolve(distDir, "cadview-web.js"),
        resolve(outDir, "cadview-web.js"),
      );
      copyFileSync(
        resolve(distDir, "cadview-web_bg.wasm"),
        resolve(outDir, "cadview-web_bg.wasm"),
      );
    },
  };
}

export default defineConfig({
  plugins: [copyWasmFiles()],
  root: ".",
  publicDir: false,
  resolve: {
    alias: {
      // Resolve the module path so rollup's external pattern can match it
      "cadview-wasm": resolve(distDir, "cadview-web.js"),
    },
  },
  build: {
    outDir,
    emptyOutDir: true,
    target: "esnext",
    lib: {
      entry: resolve(__dirname, "src/embed.ts"),
      formats: ["es"],
      fileName: "codecad-viewer",
    },
    rollupOptions: {
      // Keep wasm-bindgen module external so Vite doesn't inline the WASM.
      // The cadview-web.js + .wasm files are copied alongside the output.
      external: [/cadview-web\.js$/],
      output: {
        paths(id) {
          if (id.endsWith("cadview-web.js")) return "./cadview-web.js";
          return id;
        },
      },
    },
  },
});
