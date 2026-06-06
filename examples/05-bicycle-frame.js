// Bicycle frame geometry (road bike side profile)
// Exercises: addLine, addArc, offset, trim, layers, measure, addText
//
// Parametric diamond frame with tube center-lines and wall thickness.
// Key geometry: seat tube angle, head tube angle, chainstay length.

const seatAngle = 73;     // degrees from horizontal
const headAngle = 72;
const ttLength = 560;     // top tube (mm)
const stLength = 530;     // seat tube
const csLength = 410;     // chainstay
const bbDrop = 70;        // bottom bracket drop below axle line
const wheelBase = 1000;
const tubeOD = 32;        // tube outer diameter -> wall offset = OD/2
const forkRake = 45;

cad.addLayer("FRAME", { color: [255, 255, 0] });
cad.addLayer("WHEELS", { color: [128, 128, 128] });
cad.addLayer("DIM", { color: [255, 255, 0] });
cad.addLayer("LABEL", { color: [192, 192, 192] });
cad.addLayer("_CL", { color: [128, 128, 128] });

const DEG = Math.PI / 180;

// Key points (all derived from BB as origin)
const bb = [0, 0]; // bottom bracket

// Seat tube: from BB upward at seatAngle
const stTop = [
    -stLength * Math.cos(seatAngle * DEG),
    stLength * Math.sin(seatAngle * DEG)
];

// Rear axle: chainstay goes backward and down
const rearAxle = [-csLength, -bbDrop];

// Front axle
const frontAxle = [wheelBase - csLength, -bbDrop];

// Head tube bottom: derive from top tube + head angle
// Top tube runs horizontal from seat tube top
const ttEnd = [stTop[0] + ttLength, stTop[1]];

// Head tube runs down from ttEnd at headAngle
const htLength = 180;
const htBottom = [
    ttEnd[0] + htLength * Math.cos(headAngle * DEG),
    ttEnd[1] - htLength * Math.sin(headAngle * DEG)
];

// ── Centerlines ────────────────────────────────────────────────────

// Seat tube
cad.addLine(bb, stTop, { layer: "_CL" });

// Top tube
cad.addLine(stTop, ttEnd, { layer: "_CL" });

// Down tube (BB to head tube bottom)
cad.addLine(bb, htBottom, { layer: "_CL" });

// Head tube
cad.addLine(ttEnd, htBottom, { layer: "_CL" });

// Chainstays (BB to rear axle)
cad.addLine(bb, rearAxle, { layer: "_CL" });

// Seatstays (seat tube top to rear axle)
cad.addLine(stTop, rearAxle, { layer: "_CL" });

// Fork (head tube bottom toward front axle, with rake)
const forkEnd = [frontAxle[0], frontAxle[1]];
cad.addLine(htBottom, forkEnd, { layer: "_CL" });

// ── Frame tubes (offset from centerlines) ──────────────────────────

const hw = tubeOD / 2; // half-width

// Draw each tube as two offset lines
function drawTube(p0, p1, width) {
    const dx = p1[0] - p0[0], dy = p1[1] - p0[1];
    const len = Math.sqrt(dx * dx + dy * dy);
    const nx = -dy / len * width, ny = dx / len * width;
    cad.addLine([p0[0] + nx, p0[1] + ny], [p1[0] + nx, p1[1] + ny], { layer: "FRAME" });
    cad.addLine([p0[0] - nx, p0[1] - ny], [p1[0] - nx, p1[1] - ny], { layer: "FRAME" });
}

drawTube(bb, stTop, hw);
drawTube(stTop, ttEnd, hw);
drawTube(bb, htBottom, hw);
drawTube(ttEnd, htBottom, hw * 0.8); // head tube slightly thinner
drawTube(bb, rearAxle, hw * 0.6);    // chainstays thinner
drawTube(stTop, rearAxle, hw * 0.5); // seatstays thinnest
drawTube(htBottom, forkEnd, hw * 0.6);

// ── Wheels ─────────────────────────────────────────────────────────

const wheelR = 340; // 700c
const hubR = 25;

// Rear wheel
cad.addCircle(rearAxle, wheelR, { layer: "WHEELS" });
cad.addCircle(rearAxle, hubR, { layer: "WHEELS" });

// Front wheel
cad.addCircle(frontAxle, wheelR, { layer: "WHEELS" });
cad.addCircle(frontAxle, hubR, { layer: "WHEELS" });

// ── Bottom bracket circle ──────────────────────────────────────────
cad.addCircle(bb, 35, { layer: "FRAME" });

// ── Dimensions ─────────────────────────────────────────────────────

// Wheelbase
cad.measure(rearAxle, frontAxle, { offset: -wheelR - 60, layer: "DIM", text_height: 20 });

// Seat tube
cad.measure(bb, stTop, { offset: -50, layer: "DIM", text_height: 15 });

// Top tube
cad.measure(stTop, ttEnd, { offset: 40, layer: "DIM", text_height: 15 });

// Chainstay
cad.measure(bb, rearAxle, { offset: -40, layer: "DIM", text_height: 12 });

// ── Labels ─────────────────────────────────────────────────────────

cad.addText("ROAD BIKE FRAME", [-csLength, -wheelR - 100], { height: 25, layer: "LABEL" });
cad.addText(`ST ${seatAngle} deg  HT ${headAngle} deg  TT ${ttLength}mm`, [-csLength, -wheelR - 135], { height: 15, layer: "LABEL" });

cad.describe();
