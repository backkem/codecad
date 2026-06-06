// Piston and connecting rod cross-section
// Exercises: addLine, addArc, addCircle, addHatch, addPolyline, layers,
//            mirror symmetry, trim, offset
//
// Half-section view (symmetric about centerline). Shows piston crown,
// ring grooves, skirt, wrist pin bore, and connecting rod.

cad.addLayer("SECTION", { color: [192, 192, 192] });
cad.addLayer("HATCH", { color: [128, 128, 128] });
cad.addLayer("_CL", { color: [128, 128, 128] });
cad.addLayer("DIM", { color: [255, 255, 0] });
cad.addLayer("LABEL", { color: [192, 192, 192] });

// Piston parameters (mm)
const bore = 86;
const R = bore / 2;
const crownH = 8;       // crown thickness
const ringW = 2;        // ring groove width
const ringD = 3;        // ring groove depth
const ringGap = 5;      // gap between grooves
const nRings = 3;
const skirtH = 50;      // skirt length below ring land
const pinD = 22;         // wrist pin bore diameter
const pinY = -40;        // wrist pin center Y (below crown top)
const wallT = 5;         // piston wall thickness

const conrodW = 18;      // connecting rod width
const conrodL = 130;     // connecting rod length (to big end center)
const bigEndD = 50;      // big end bore

// ── Centerline ─────────────────────────────────────────────────────

cad.addLine([0, 30], [0, pinY - conrodL - bigEndD], { layer: "_CL" });

// ── Piston (draw right half, mirror left) ──────────────────────────

function mirrorLines(lines) {
    for (const l of lines) {
        cad.addLine([-l[0][0], l[0][1]], [-l[1][0], l[1][1]], { layer: "SECTION" });
    }
}

// Crown top
cad.addLine([-R, 0], [R, 0], { layer: "SECTION" });

// Crown underside
cad.addLine([-(R - wallT), -crownH], [R - wallT, -crownH], { layer: "SECTION" });

// Right outer wall
const rightLines = [];

// Outer wall from crown down to skirt
let y = 0;
rightLines.push([[R, y], [R, -(crownH + 2)]]);

// Ring grooves
for (let i = 0; i < nRings; i++) {
    const gy = -(crownH + 2 + i * (ringW + ringGap));
    // Land before groove
    rightLines.push([[R, gy], [R, gy - 0.5]]);
    // Groove outer wall
    rightLines.push([[R, gy - 0.5], [R - ringD, gy - 0.5]]);
    rightLines.push([[R - ringD, gy - 0.5], [R - ringD, gy - 0.5 - ringW]]);
    rightLines.push([[R - ringD, gy - 0.5 - ringW], [R, gy - 0.5 - ringW]]);
}

// Wall below rings to skirt bottom
const belowRings = -(crownH + 2 + nRings * (ringW + ringGap));
const skirtBottom = belowRings - skirtH;
rightLines.push([[R, belowRings], [R, skirtBottom]]);

// Skirt bottom (slight chamfer)
rightLines.push([[R, skirtBottom], [R - 2, skirtBottom - 2]]);

// Draw right side
for (const l of rightLines) {
    cad.addLine(l[0], l[1], { layer: "SECTION" });
}
// Mirror left side
mirrorLines(rightLines);

// Inner wall (right side)
const innerLines = [];
innerLines.push([[R - wallT, -crownH], [R - wallT, pinY + pinD / 2 + 5]]);
// Boss around pin bore
innerLines.push([[R - wallT, pinY + pinD / 2 + 5], [pinD / 2 + 8, pinY + pinD / 2 + 5]]);
innerLines.push([[pinD / 2 + 8, pinY - pinD / 2 - 5], [R - wallT, pinY - pinD / 2 - 5]]);
innerLines.push([[R - wallT, pinY - pinD / 2 - 5], [R - wallT, skirtBottom + 5]]);

for (const l of innerLines) {
    cad.addLine(l[0], l[1], { layer: "SECTION" });
}
mirrorLines(innerLines);

// Wrist pin bore
cad.addCircle([0, pinY], pinD / 2, { layer: "SECTION" });

// ── Connecting rod ─────────────────────────────────────────────────

const rodTop = pinY;
const rodBottom = pinY - conrodL;

// Small end (around wrist pin)
cad.addCircle([0, rodTop], pinD / 2 + 4, { layer: "SECTION" });

// Rod shaft
cad.addLine([conrodW / 2, rodTop - pinD / 2 - 4], [conrodW / 2 + 3, rodBottom + bigEndD / 2], { layer: "SECTION" });
cad.addLine([-conrodW / 2, rodTop - pinD / 2 - 4], [-conrodW / 2 - 3, rodBottom + bigEndD / 2], { layer: "SECTION" });

// Big end bore
cad.addCircle([0, rodBottom], bigEndD / 2, { layer: "SECTION" });
cad.addCircle([0, rodBottom], bigEndD / 2 + 8, { layer: "SECTION" });

// ── Hatching (cross-section material) ──────────────────────────────

// Hatch the piston crown (right half only, as convention for half-section)
cad.addHatch(
    [[0, 0], [R, 0], [R, -crownH], [0, -crownH]],
    { angle: 45, spacing: 3, layer: "HATCH" }
);

// Hatch the right wall section
cad.addHatch(
    [[R - wallT, -crownH], [R, -crownH], [R, belowRings], [R - wallT, belowRings]],
    { angle: 45, spacing: 3, layer: "HATCH" }
);

// Hatch connecting rod shaft (simplified rectangular section)
cad.addHatch(
    [[-conrodW / 2, rodTop - pinD], [conrodW / 2, rodTop - pinD],
     [conrodW / 2, rodBottom + bigEndD / 2 + 5], [-conrodW / 2, rodBottom + bigEndD / 2 + 5]],
    { angle: 45, spacing: 2.5, layer: "HATCH" }
);

// ── Dimensions ─────────────────────────────────────────────────────

cad.measure([-R, 0], [R, 0], { offset: 15, layer: "DIM", text_height: 4 });
cad.measure([R + 5, 0], [R + 5, skirtBottom], { offset: 15, layer: "DIM", text_height: 3.5 });

// ── Labels ─────────────────────────────────────────────────────────

cad.addText("PISTON ASSEMBLY", [-R, 25], { height: 5, layer: "LABEL" });
cad.addText("SECTION VIEW", [-R, 18], { height: 3, layer: "LABEL" });

cad.describe();
