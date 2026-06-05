// Fibonacci golden spiral - geometric construction
// Exercises: addArc (p1/p2 mode), addPolyline, addLine, addText, layers
//
// Golden rectangle subdivided into squares. Quarter-circle arcs form
// one continuous spiral. Diagonal construction lines and number labels.

const fib = [1, 1, 2, 3, 5, 8];
const scale = 20; // mm per unit

cad.addLayer("SPIRAL", { color: [255, 120, 0] });
cad.addLayer("GRID", { color: [120, 120, 120] });
cad.addLayer("DIAG", { color: [80, 80, 80] });
cad.addLayer("LABEL", { color: [160, 160, 160] });

// Square positions (bottom-left corner, in grid units)
const positions = [
    { x: 0, y: 0, s: 1 },
    { x: 1, y: 0, s: 1 },
    { x: 0, y: -2, s: 2 },
    { x: -3, y: -2, s: 3 },
    { x: -3, y: 1, s: 5 },
    { x: 2, y: -2, s: 8 },
];

// Arc centers (relative to BL of each square, as fraction of side).
// Center is at the INNER corner (closest to spiral origin) so the
// short arc curves OUTWARD along the square edges.
//
// Spiral path: (0,0)->(1,1)->(2,0)->(0,-2)->(-3,1)->(2,6)->(10,-2)
const arcCenters = [
    { cx: 1, cy: 0 },  // BR of sq0 (CW sweep, matches rest of spiral)
    { cx: 0, cy: 0 },  // BL of sq1 (CW sweep)
    { cx: 0, cy: 1 },  // TL of sq2 (= point (0,0))
    { cx: 1, cy: 1 },  // TR of sq3 (= point (0,1))
    { cx: 1, cy: 0 },  // BR of sq4 (= point (2,1))
    { cx: 0, cy: 0 },  // BL of sq5 (= point (2,-2))
];

// Spiral connection points (in grid units)
const spiralPts = [[0,0],[1,1],[2,0],[0,-2],[-3,1],[2,6],[10,-2]];

for (let i = 0; i < positions.length; i++) {
    const { x, y, s } = positions[i];
    const sx = x * scale, sy = y * scale, ss = s * scale;

    // Square outline
    cad.addPolyline(
        [[sx, sy], [sx + ss, sy], [sx + ss, sy + ss], [sx, sy + ss]],
        { closed: true, layer: "GRID" }
    );

    // Diagonal
    cad.addLine([sx, sy], [sx + ss, sy + ss], { layer: "DIAG" });

    // Fibonacci number label
    cad.addText(String(s), [sx + ss * 0.3, sy + ss * 0.3],
        { height: ss * 0.35, layer: "LABEL" });

    // Quarter-circle arc using p1/p2 (always short arc, correct curvature)
    const ac = arcCenters[i];
    const center = [sx + ac.cx * ss, sy + ac.cy * ss];
    const p1 = [spiralPts[i][0] * scale, spiralPts[i][1] * scale];
    const p2 = [spiralPts[i + 1][0] * scale, spiralPts[i + 1][1] * scale];
    cad.addArc(center, ss, { p1, p2, layer: "SPIRAL" });
}

// Outer bounding rectangle
cad.addPolyline(
    [[-60, -40], [200, -40], [200, 120], [-60, 120]],
    { closed: true, layer: "GRID" }
);

cad.describe();
