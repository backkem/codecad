import { defineConfig } from "vite";
import react from "@vitejs/plugin-react-swc";
import { resolve } from "path";

const distDir = resolve(__dirname, "../dist");

export default defineConfig({
  plugins: [react()],
  root: ".",
  publicDir: false,
  resolve: {
    alias: {
      // wasm-bindgen outputs cadview-web.js + .wasm into dist/
      "cadview-wasm": resolve(distDir, "cadview-web.js"),
    },
  },
  base: "./", // relative paths so app works from any subdirectory
  build: {
    outDir: distDir,
    emptyOutDir: false, // keep wasm-bindgen output (.js, .wasm)
    target: "esnext",
  },
  server: {
    port: 5173,
    fs: {
      allow: [distDir, "."], // serve wasm files from dist/ during dev
    },
  },
});
