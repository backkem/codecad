// Howe roof truss - structural timber truss
// Exercises: addLine, addText, measure, layers, symmetry (manual mirror)
//
// Symmetric truss: draw left half, mirror for right.
// Span 12m, rise 3m, 6 panels.

const span = 12000;    // mm
const rise = 3000;
const panels = 6;
const panelW = span / panels;
const half = panels / 2;

cad.addLayer("CHORD", { color: [200, 160, 100] });   // top/bottom chords
cad.addLayer("WEB", { color: [140, 140, 140] });      // diagonals + verticals
cad.addLayer("DIM", { color: [180, 180, 0] });
cad.addLayer("LABEL", { color: [150, 150, 150] });
cad.addLayer("SUPPORT", { color: [100, 200, 100] });

// Bottom chord
cad.addLine([0, 0], [span, 0], { layer: "CHORD" });

// Top chord segments (left and right slopes)
for (let i = 0; i < half; i++) {
    const x0 = i * panelW;
    const x1 = (i + 1) * panelW;
    const y0 = (i / half) * rise;
    const y1 = ((i + 1) / half) * rise;
    // Left slope
    cad.addLine([x0, y0], [x1, y1], { layer: "CHORD" });
    // Right slope (mirror)
    cad.addLine([span - x0, y0], [span - x1, y1], { layer: "CHORD" });
}

// Verticals at each panel point
for (let i = 1; i < panels; i++) {
    const x = i * panelW;
    const y = (i <= half ? i : panels - i) / half * rise;
    cad.addLine([x, 0], [x, y], { layer: "WEB" });
}

// Diagonals (Howe pattern: diagonals slope toward center)
for (let i = 0; i < half; i++) {
    const x0 = i * panelW;
    const x1 = (i + 1) * panelW;
    const yTop = ((i + 1) / half) * rise;
    // Left side: diagonal from bottom-left to top-right of panel
    cad.addLine([x0, 0], [x1, yTop], { layer: "WEB" });
    // Right side (mirror)
    cad.addLine([span - x0, 0], [span - x1, yTop], { layer: "WEB" });
}

// Support triangles at bearings
const triH = 400;
const triW = 300;
// Left support
cad.addLine([0, 0], [-triW, -triH], { layer: "SUPPORT" });
cad.addLine([0, 0], [triW, -triH], { layer: "SUPPORT" });
cad.addLine([-triW, -triH], [triW, -triH], { layer: "SUPPORT" });
// Right support
cad.addLine([span, 0], [span - triW, -triH], { layer: "SUPPORT" });
cad.addLine([span, 0], [span + triW, -triH], { layer: "SUPPORT" });
cad.addLine([span - triW, -triH], [span + triW, -triH], { layer: "SUPPORT" });

// Dimensions
cad.measure([0, 0], [span, 0], { offset: -800, layer: "DIM", text_height: 150 });
cad.measure([span + 200, 0], [span + 200, rise], { offset: 600, layer: "DIM", text_height: 150 });

// Panel spacing dimensions
for (let i = 0; i < panels; i++) {
    cad.measure(
        [i * panelW, 0], [(i + 1) * panelW, 0],
        { offset: -400, layer: "DIM", text_height: 100 }
    );
}

// Labels
cad.addText("HOWE ROOF TRUSS", [span / 2 - 2500, -1400], { height: 250, layer: "LABEL" });
cad.addText(`Span: ${span / 1000}m  Rise: ${rise / 1000}m  ${panels} panels`, [span / 2 - 3000, -1800], { height: 150, layer: "LABEL" });

cad.describe();
