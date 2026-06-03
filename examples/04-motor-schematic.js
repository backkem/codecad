// 3-phase motor starter schematic (single-line)
// Exercises: defineBlock, place, addLine, addText, addCircle, layers, blocks
//
// Simple DOL (Direct On Line) starter: main breaker -> contactor ->
// overload relay -> motor. With control circuit.

const G = 5; // grid spacing in mm

function gpt(gx, gy) { return [gx * G, gy * G]; }

// Layers
cad.addLayer("SCH_COMP", { color: [0, 0, 0] });
cad.addLayer("SCH_WIRE", { color: [0, 80, 200] });
cad.addLayer("SCH_LABEL", { color: [120, 120, 120] });
cad.addLayer("SCH_CTRL", { color: [200, 0, 0] });
cad.addLayer("SCH_FRAME", { color: [60, 60, 60] });

// ── Block definitions ──────────────────────────────────────────────

// Circuit breaker (3-pole): rectangle with CB symbol
cad.defineBlock("CB_3P", [
    { type: "polyline", points: [gpt(0,0), gpt(6,0), gpt(6,8), gpt(0,8)], closed: true },
    { type: "line", start: gpt(1, 2), end: gpt(5, 6) },
    { type: "line", start: gpt(5, 6), end: gpt(4, 5) },
    { type: "line", start: gpt(5, 6), end: gpt(5, 5) },
    // Connection dots
    { type: "circle", center: gpt(3, 0), radius: 1.2 },
    { type: "circle", center: gpt(3, 8), radius: 1.2 },
], { insert_point: gpt(0, 0), default_layer: "SCH_COMP" });

// Contactor: rectangle with contact symbol
cad.defineBlock("CONTACTOR", [
    { type: "polyline", points: [gpt(0,0), gpt(6,0), gpt(6,8), gpt(0,8)], closed: true },
    { type: "line", start: gpt(1, 3), end: gpt(5, 5) },
    { type: "line", start: gpt(1, 3), end: gpt(1, 2) },
    { type: "line", start: gpt(5, 5), end: gpt(5, 6) },
    // Connection dots
    { type: "circle", center: gpt(3, 0), radius: 1.2 },
    { type: "circle", center: gpt(3, 8), radius: 1.2 },
], { insert_point: gpt(0, 0), default_layer: "SCH_COMP" });

// Overload relay: rectangle with OL symbol (wavy line)
cad.defineBlock("OVERLOAD", [
    { type: "polyline", points: [gpt(0,0), gpt(6,0), gpt(6,8), gpt(0,8)], closed: true },
    { type: "arc", center: gpt(2, 4), radius: G * 1.5, from: 270, to: 90 },
    { type: "arc", center: gpt(4, 4), radius: G * 1.5, from: 90, to: 270 },
    { type: "circle", center: gpt(3, 0), radius: 1.2 },
    { type: "circle", center: gpt(3, 8), radius: 1.2 },
], { insert_point: gpt(0, 0), default_layer: "SCH_COMP" });

// Motor symbol: circle with M
cad.defineBlock("MOTOR", [
    { type: "circle", center: gpt(3, 4), radius: G * 3 },
    // M is hard without text in blocks, use cross pattern
    { type: "line", start: gpt(1, 2), end: gpt(3, 6) },
    { type: "line", start: gpt(3, 6), end: gpt(5, 2) },
    { type: "circle", center: gpt(3, 0), radius: 1.2 },
], { insert_point: gpt(0, 0), default_layer: "SCH_COMP" });

// Push button (NO): circle with line through it
cad.defineBlock("PB_NO", [
    { type: "circle", center: gpt(2, 2), radius: G * 1.5 },
    { type: "line", start: gpt(0, 2), end: gpt(4, 2) },
    { type: "circle", center: gpt(2, 0), radius: 1 },
    { type: "circle", center: gpt(2, 4), radius: 1 },
], { insert_point: gpt(0, 0), default_layer: "SCH_CTRL" });

// ── Place power circuit (vertical, top to bottom) ──────────────────

const powerX = 20;
let y = 120;

// Supply label
cad.addText("L1 L2 L3", gpt(powerX / G - 2, y / G + 2), { height: 4, layer: "SCH_LABEL" });

// Main CB
const cb = cad.place("CB_3P", { at: [powerX, y - 40] });
cad.addText("Q1", [powerX + 35, y - 20], { height: 4, layer: "SCH_LABEL" });

// Wire CB -> Contactor
cad.addLine([powerX + 15, y - 40], [powerX + 15, y - 55], { layer: "SCH_WIRE" });

// Contactor
const km = cad.place("CONTACTOR", { at: [powerX, y - 95] });
cad.addText("KM1", [powerX + 35, y - 75], { height: 4, layer: "SCH_LABEL" });

// Wire Contactor -> Overload
cad.addLine([powerX + 15, y - 95], [powerX + 15, y - 110], { layer: "SCH_WIRE" });

// Overload relay
const ol = cad.place("OVERLOAD", { at: [powerX, y - 150] });
cad.addText("F1", [powerX + 35, y - 130], { height: 4, layer: "SCH_LABEL" });

// Wire Overload -> Motor
cad.addLine([powerX + 15, y - 150], [powerX + 15, y - 170], { layer: "SCH_WIRE" });

// Motor
const mot = cad.place("MOTOR", { at: [powerX, y - 210] });
cad.addText("M1", [powerX + 35, y - 195], { height: 4, layer: "SCH_LABEL" });
cad.addText("3~ Motor", [powerX + 35, y - 205], { height: 3, layer: "SCH_LABEL" });

// ── Control circuit (right side) ───────────────────────────────────

const ctrlX = 90;

// Stop button (NC - draw as line with gap)
cad.addLine([ctrlX, y], [ctrlX, y - 20], { layer: "SCH_CTRL" });
cad.addText("STOP", [ctrlX + 8, y - 10], { height: 3, layer: "SCH_LABEL" });

// Start button
const startBtn = cad.place("PB_NO", { at: [ctrlX - 5, y - 40] });
cad.addText("START", [ctrlX + 8, y - 35], { height: 3, layer: "SCH_LABEL" });

// Wire to coil
cad.addLine([ctrlX, y - 60], [ctrlX, y - 80], { layer: "SCH_CTRL" });

// Contactor coil (circle)
cad.addCircle([ctrlX, y - 90], 8, { layer: "SCH_CTRL" });
cad.addText("KM1", [ctrlX + 12, y - 92], { height: 3, layer: "SCH_LABEL" });

// Neutral wire
cad.addLine([ctrlX, y - 98], [ctrlX, y - 110], { layer: "SCH_CTRL" });
cad.addText("N", [ctrlX + 5, y - 108], { height: 3, layer: "SCH_LABEL" });

// ── Drawing frame ──────────────────────────────────────────────────

cad.addPolyline([[-10, 130], [140, 130], [140, -100], [-10, -100]], { closed: true, layer: "SCH_FRAME" });
cad.addText("DOL MOTOR STARTER", [10, -92], { height: 6, layer: "SCH_FRAME" });

cad.describe();
