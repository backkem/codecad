// A small board for the clearance rule: tracks carry a `net` attribute,
// and a two-pad footprint gets its pad nets from attributes on the insert
// (`net.1`, `net.2`). The core stores these attributes; only the rule script
// knows what a net is.

cad.addLayer("CU", { color: 1 });
cad.addLayer("SILK", { color: 7 });

// The footprint: pads carry their pin number, the definition its name.
cad.defineBlock(
  "R0603",
  [
    { type: "circle", center: [-0.8, 0], radius: 0.35, layer: "CU", attrs: { pin: "1" } },
    { type: "circle", center: [0.8, 0], radius: 0.35, layer: "CU", attrs: { pin: "2" } },
    { type: "line", start: [-1.2, 0.6], end: [1.2, 0.6], layer: "SILK" },
  ],
  { attrs: { footprint: "R0603" } },
);
cad.place("R0603", { at: [20, 0], layer: "CU", attrs: { ref: "R1", "net.1": "GND", "net.2": "VCC" } });

const track = (net, name, points) =>
  cad.addPolyline(points, { layer: "CU", attrs: { net, name } });

// GND runs to pad 1 and ends on its edge.
track("GND", "g1", [[0, 0], [10, 0], [18.85, 0]]);
track("GND", "g2", [[10, 0], [10, 5]]);
// VCC runs 0.15 above GND for a while: too close.
track("VCC", "v1", [[5, 0.15], [9, 0.15]]);
// VCC passes 0.15 above pad 1 (pad top at y = 0.35): too close.
track("VCC", "v2", [[18, 0.5], [21.6, 0.5]]);
// SIG crosses the GND branch: a short.
track("SIG", "s1", [[8, 3], [12, 3]]);
// Board outline, no net: not copper, so the rule ignores it.
cad.addPolyline([[-2, -2], [25, -2], [25, 8], [-2, 8]], { closed: true, layer: "OUTLINE" });

return cad.describe().entities;
