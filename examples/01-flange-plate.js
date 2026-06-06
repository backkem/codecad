// Parametric flange plate with bolt circle
// Exercises: addCircle, addLine, measure, addText, layers, linetypes, polar pattern
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

// Layers (ACI-safe colors for lossless DWG roundtrip)
cad.addLayer("FLANGE", { color: [192, 192, 192] });
cad.addLayer("HOLES", { color: [0, 0, 255] });
cad.addLayer("DIM", { color: [255, 255, 0] });
cad.addLayer("_CL", { color: [128, 128, 128], linetype: "Center" });

// Centerlines (inherit Center linetype from layer)
cad.addLine([-R - 20, 0], [R + 20, 0], { layer: "_CL" });
cad.addLine([0, -R - 20], [0, R + 20], { layer: "_CL" });

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
cad.addCircle([0, 0], boltR, { layer: "_CL", linetype: "Dashed" });

// Dimensions
cad.measure([0, 0], [R, 0], { offset: -30, layer: "DIM", text_height: 6 });
cad.measure([0, 0], [boreR, 0], { offset: 20, layer: "DIM", text_height: 6 });

// Title
cad.addText("FLANGE PLATE", [-40, -R - 35], { height: 8, layer: "DIM" });
cad.addText(`${nBolts}x M${boltHoleD - 1} ON PCD ${boltCircleD}`, [-55, -R - 48], { height: 5, layer: "DIM" });

cad.describe();
