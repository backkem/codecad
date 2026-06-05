import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { initCad, setLoadProgressCallback } from "./cad";
import "./index.css";

// Wire loader progress to the HTML loader element
const loaderText = document.querySelector(".cad-loader-text");
if (loaderText) {
  setLoadProgressCallback((pct) => {
    loaderText.textContent = `loading ${pct}%`;
  });
}

// WASM must be fully initialized before React renders (ViewportContainer
// calls start_renderer which needs the WASM module ready).
initCad().then(() => {
  console.log(
    "cadview ready. Try: cad.useSession('default'); cad.addLine([0,0],[100,0])",
  );
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
});
