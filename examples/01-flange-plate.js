// Parametric flange plate with bolt circle
// Exercises: addCircle, addLine, measure, addText, layers, polar pattern
//
// A simple pipe flange: center bore, bolt circle with N holes,
// outer diameter, and dimensioned radii.

const OD = 200;        // outer diameter (mm)
const boreD = 80;      // center bore diameter
const boltCircleD = 150; // bolt circle diameter
const boltHoleD = 14;  // bolt hole diameter
const nBolts = 8;      // number of bolt holes

const R = OD / 2;
const boreR = boreD / 2;
const boltR = boltCircleD / 2;
const holeR = boltHoleD / 2;

// Layers
cad.addLayer("FLANGE", { color: [200, 200, 200] });
cad.addLayer("HOLES", { color: [100, 180, 255] });
cad.addLayer("DIM", { color: [180, 180, 0] });
cad.addLayer("_CL", { color: [80, 80, 80] });

// Centerlines (long-dash-short-dash center line pattern)
cad.addLine([-R - 20, 0], [R + 20, 0], { layer: "_CL", dash: [12, 3, 3, 3] });
cad.addLine([0, -R - 20], [0, R + 20], { layer: "_CL", dash: [12, 3, 3, 3] });

// Main geometry
cad.addCircle([0, 0], R, { layer: "FLANGE" });
cad.addCircle([0, 0], boreR, { layer: "FLANGE" });

// Bolt holes - polar pattern
for (let i = 0; i < nBolts; i++) {
    const angle = (i * 360 / nBolts) * Math.PI / 180;
    const cx = boltR * Math.cos(angle);
    const cy = boltR * Math.sin(angle);
    cad.addCircle([cx, cy], holeR, { layer: "HOLES" });
}

// Bolt circle (dashed reference)
cad.addCircle([0, 0], boltR, { layer: "_CL", dash: [8, 4] });

// Dimensions
cad.measure([0, 0], [R, 0], { offset: -30, layer: "DIM", text_height: 6 });
cad.measure([0, 0], [boreR, 0], { offset: 20, layer: "DIM", text_height: 6 });

// Title
cad.addText("FLANGE PLATE", [-40, -R - 35], { height: 8, layer: "DIM" });
cad.addText(`${nBolts}x M${boltHoleD - 1} ON PCD ${boltCircleD}`, [-55, -R - 48], { height: 5, layer: "DIM" });

cad.describe();
