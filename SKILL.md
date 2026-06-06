---
name: codecad
description: Create and modify 2D CAD drawings via CodeCAD. Use when asked to draw, sketch, or produce technical drawings (floor plans, mechanical sections, schematics, geometric constructions).
---

# CodeCAD drawing skill

## Purpose

Create and modify 2D CAD drawings through JavaScript code executed in
the browser or server sandbox. The `cad` global is your drawing surface.
You write JS, the geometry appears live in the viewport, syncs to the
server via Yrs CRDT, and can be saved to disk as DWG.

Multi-document: multiple drawings can be open in tabs. Use
`cad.useSession(id)` to select which drawing your code targets.

## When to use

Use when asked to draw, sketch, illustrate, or produce 2D technical
drawings: floor plans, mechanical cross-sections, schematics, site plans,
geometric constructions, or any 2D geometry.

## API reference

All via `evaluate_script` in the browser (chrome-devtools MCP), or
directly in the browser console. The `cad` object is global.

### Session management

```js
cad.useSession("floor-plan.dwg")   // target an open document
cad.currentSession()                // verify: "floor-plan.dwg"
cad.sessions.create("my-drawing")   // creates + syncs to server
cad.sessions.destroy("my-drawing")
cad.sessions.list()                 // [{id, entity_count, ...}]
```

### Layers

```js
cad.addLayer("NAME", { color: [r,g,b] })   // create layer with color
```

Entities inherit their layer's color when no explicit color is given.

### Geometry creation

```js
cad.addLine([x1, y1], [x2, y2], { layer, color, dash })
cad.addCircle([cx, cy], radius, { layer, color, dash })
cad.addArc([cx, cy], radius, { from: deg, to: deg, layer, color, dash })
cad.addArc([cx, cy], radius, { p1: [x,y], p2: [x,y] })  // tangent points, always short arc
cad.addArc([cx, cy], radius, { from, to, shortest: true }) // auto-correct to <180 sweep
cad.addPolyline([[x,y], ...], { closed: bool, layer, color, dash })
cad.addText("label", [x, y], { height: 5, layer, color })
cad.addHatch([[x,y], ...], { angle: 45, spacing: 5, layer, color })
```

All return the created entity (with `.id`). Coordinates are in drawing
units (mm for mechanical, arbitrary for schematics). Color is optional:
omit it to inherit from the layer.

### Line dashing

Pass `dash: [on, off, ...]` to any geometry command. Values are
alternating dash/gap lengths in drawing units. The pattern repeats
along the entity's path.

```js
// Standard dashed line
cad.addLine([0,0], [100,0], { dash: [8, 4] })

// Center line pattern (long-short-long-short)
cad.addLine([0,0], [100,0], { dash: [12, 3, 3, 3] })

// Dashed circle (bolt circle, reference arc)
cad.addCircle([0,0], 75, { dash: [8, 4] })

// Dotted polyline (0.5 is rendered as a very short dash)
cad.addPolyline([[0,0],[50,0],[50,50]], { dash: [0.5, 4] })
```

Common patterns:
- **Dashed**: `[8, 4]`
- **Center line**: `[12, 3, 3, 3]`
- **Hidden/phantom**: `[6, 3, 2, 3]`
- **Dotted**: `[0.5, 4]`

DWG files imported with linetypes automatically get their dash patterns
resolved. The entity's `dash` field appears in `entities()` output.

### Blocks (reusable symbols)

```js
cad.defineBlock("NAME", [
  { type: "line", start: [x,y], end: [x,y] },
  { type: "circle", center: [x,y], radius: r },
  { type: "arc", center: [x,y], radius: r, from: deg, to: deg },
  { type: "polyline", points: [...], closed: bool },
], { insert_point: [x,y], default_layer: "LAYER" })

cad.place("NAME", { at: [x,y], rotation: degrees, layer: "OVERRIDE" })
```

`cad.place()` creates a `block_insert` entity plus flattened child
geometry. The block_insert has `block_name`, `center`, `rotation`
(degrees), and `children` (array of child entity IDs).

### Queries

```js
cad.describe()                // summary: entity count, layers, bounds
cad.entities()                // all entities as array (filterable)
cad.entities({expand: true})  // flatten block inserts into world-coord sub-entities
cad.entities({layer: "NAME"}) // filter by layer (works with expand)
cad.entity("e_42")            // single entity by ID
cad.children("e_42")          // expanded sub-entities of a block insert
cad.connectedTo("e_42")      // entities sharing an endpoint (tolerance 0.01)
```

**`expand: true`** is critical when you need to access geometry inside
block inserts (e.g. C_WINDOWS, furniture blocks). Without it, block
inserts appear as single point entities with no child lines. With it,
each block's internal shapes are transformed to world coordinates and
returned as individual entities with their block's layer.

### Introspection

```js
cad.methods()   // list all cad_call methods with args and descriptions
```

Returns an array of `{name, args, desc}` objects. Useful for discovering
available API methods and their parameters at runtime.

### Measurements and geometry helpers

```js
cad.distance([x,y], [x,y])                      // -> 111.8
cad.midpoint([x,y], [x,y])                       // -> [50, 25]
cad.direction([x,y], [x,y])                      // -> 26.5 (degrees, 0=east, 90=north)
cad.angleOf([px,py], [cx,cy])                     // -> -26.5 (degrees)
cad.projectOnto([px,py], [[lx1,ly1],[lx2,ly2]])   // -> [x, y]
cad.projectOntoCircle([px,py], [cx,cy], r)         // -> [x, y]
cad.lineCircleIntersection([[lx1,ly1],[lx2,ly2]], [cx,cy], r)  // -> [[x,y], ...]
cad.circleCircleIntersection([c1x,c1y], r1, [c2x,c2y], r2)    // -> [[x,y], ...]
```

All return bare numbers or arrays, no wrapped objects.

### Mutations

```js
cad.remove("e_42")                           // single ID
cad.remove(["e_1", "e_2"])                   // array of IDs
cad.remove(cad.entities().filter(predicate)) // filtered selection
cad.move(target, dx, dy)                     // relative move
cad.copy(target, dx, dy)                     // duplicate + offset
cad.rotate(target, center, degrees)          // rotate around point
cad.mirror(target, p1, p2)                   // mirror across line
cad.trim(id, cutPoint, keep)                 // shorten line/arc ("start"/"end")
cad.offset(entity, distance)                 // parallel copy (line/circle/arc)
cad.clear()                                  // remove everything
```

### Dimensioning

```js
cad.measure([x1,y1], [x2,y2], { offset, text_height, layer })
```

Draws extension lines, arrows, and measurement text. Returns
`{distance, ids}`.

### Persistence

```js
await cad.save("output.json")         // save as JSON to server disk
await cad.saveDwg("output.dwg")       // save as DWG to server disk
await cad.loadDwg("input.dwg")        // load DWG from server disk
await cad.exec("scripts/foo.js")      // run server-side script
```

`save` and `saveDwg` flush pending Yrs sync before writing. Paths are
relative to the server's working directory.

### Viewport

```js
cad.fitView()                         // zoom to fit all entities
cad.zoomTo({bounds: {min, max}})      // zoom to bounding box
cad.zoomTo({id: "e_42"})             // zoom to entity (with padding)
cad.zoomTo({center: [x,y], zoom: N}) // explicit center + zoom
cad.zoomTo(entity)                    // zoom to an entity object
cad.zoomTo(entityArray)               // zoom to combined selection bounds
cad.getView()                         // returns {center: [x,y], zoom: N}
```

### Entity shapes by type

```js
// line:         { id, type, layer, color, bounds, start, end }
// circle:       { id, type, layer, color, bounds, center, radius }
// arc:          { id, type, layer, color, bounds, center, radius, from, to, points }
// polyline:     { id, type, layer, color, bounds, points, closed }
// block_insert: { id, type, layer, color, bounds, block_name, center, rotation, children }
```

Arc `from`/`to` are in degrees (CCW). `points` contains tessellated
waypoints including start and end of the sweep.

## Core creed

**Compute nothing that the geometry can tell you.**

Every point in a drawing should come from an intersection, projection,
or query on existing geometry. Not from arithmetic. Not from
trigonometry. Not from "I know the radius is 300 so x = sqrt(...)".

If you need a point where a line meets a circle: draw both, intersect.
If you need a point on a circle at a certain angle: raycast from center,
intersect with the circle. If you need a fillet center: offset both
parent surfaces, intersect the offsets.

The geometry engine does exact math. You don't. Every time you compute
a coordinate by hand, you introduce a potential error that compounds
with every subsequent step. The geometry engine doesn't drift.

This applies even inside scripts. You can draw construction geometry,
intersect it, use the results, and delete the construction, all in
one script. Nobody sees the scaffolding.

## Workflow

### Sketch, intersect, draw, round

The universal CAD sequence:

- **Sketch**: draw construction geometry generously (reference circles,
  rays from center, offset lines). Overshoot.
- **Intersect**: use `lineCircleIntersection`, `circleCircleIntersection`,
  `projectOnto`, `projectOntoCircle` to find exact junction points.
- **Draw**: draw the final geometry between the found points. Share
  each junction point between both entities that meet there.
- **Round**: add fillets at corners (offset, intersect, arc, trim).
- **Clean up**: remove construction geometry.

### Build incrementally

Do NOT try to one-shot complex drawings. Build in stages:

- Major outlines first (casing, walls, boundaries)
- Internal structure (chambers, partitions, components)
- Details (fasteners, fillets, hatching, symbols)
- Verify after each stage with `cad.describe()` and screenshots
- Checkpoint before risky operations (fillets, trim). Undo if wrong.

### Find points, don't compute coordinates

Bad (fragile, error-prone):
```js
const x = Math.sqrt(R*R - y*y);  // easy to get wrong
```

Good (robust, self-verifying):
```js
const pts = cad.lineCircleIntersection([[0, y], [500, y]], [0, 0], R);
const junction = pts.sort((a, b) => b[0] - a[0])[0];  // rightmost
```

Good (discover from existing entities):
```js
const flangeLines = cad.entities().filter(e =>
  e.type === "line" && Math.abs(e.start[0] - e.end[0]) < 1 && e.start[0] > 400
);
const flangeX = flangeLines[0].start[0];  // found, not assumed
```

Every coordinate derived from another entity should use a query, not a
magic number. If the casing radius changes from 300 to 350, every
derived position should still be correct.

### Entity discovery

Find things by properties, not by ID:

```js
// Circles at a known center
cad.entities().find(e =>
  e.type === "circle" && Math.abs(e.center[0]) < 1 && e.radius < 40);

// Vertical lines in a region
cad.entities().filter(e =>
  e.type === "line" &&
  Math.abs(e.start[0] - e.end[0]) < 1 &&  // vertical
  e.start[0] > 400);                        // right side

// All entities on a layer
cad.entities().filter(e => e.layer === "S_FASTENER")
```

### Verify at every stage

After placing geometry, check it:

```js
// Numeric: verify distances and positions
const d = cad.distance(tangentPoint, expectedPoint);
if (d > 0.1) console.warn(`tangent off by ${d}`);

// Structural: verify entity counts
const desc = cad.describe();
console.log(`${desc.entities} entities on ${desc.layers.length} layers`);

// Visual: zoom in to the area you just modified, then screenshot
cad.zoomTo({ bounds: { min: [x-20, y-20], max: [x+20, y+20] } });
```

Do not glance at a zoomed-out thumbnail and declare success. Use
`zoomTo` to frame the area you just modified before screenshotting.

### Layers

```js
{ layer: "S_SECTION" }      // section cut geometry
{ layer: "S_FASTENER" }     // bolts, nuts, tapped holes
{ layer: "_CL" }            // centerlines
{ layer: "_CONSTRUCTION" }  // temporary construction geometry
{ layer: "E_POWR" }         // electrical: sockets
{ layer: "E_LITE" }         // electrical: lights
```

### Symmetric drawings

Draw one half, mirror with `cad.copy` + `cad.mirror`:

```js
const topIds = [line1.id, arc1.id, line2.id];
const copies = cad.copy(topIds, 0, 0);      // duplicate in place
cad.mirror(copies.map(e => e.id), [0,0], [1,0]);  // flip across X axis
```

Mirror in place (mutates the originals):
```js
cad.mirror(target, [0,0], [1,0]);  // mirror across X axis
cad.mirror(target, [0,0], [0,1]);  // mirror across Y axis
cad.mirror(target, p1, p2);        // mirror across arbitrary line
```

## Recipes

### Fillets

Fillets require careful reasoning about which side the arc sits on.

**Procedure:**

- Identify the two parent surfaces (line + circle, line + line, etc.)
- Decide which side: interior (machined radius) or exterior (weld bead).
  This determines the offset directions.
- Offset both surfaces away from the fillet arc center:
  - **Exterior fillet**: both offsets go OUTWARD from the corner
  - **Interior fillet**: both offsets go INWARD toward the corner
- Intersect the offset constructions to find the fillet center
- Project from center to each surface to find tangent points
- Draw the arc, trim the parent surfaces

**Offset rules by type:**

| Type | Offsets | Arc midpoint check |
|------|---------|-------------------|
| Internal (rounding inside corner) | Both INWARD | Inside the shape |
| External (bridging line and circle) | Both away from void | Outside parent circle |
| Weld (material at exterior junction) | Both OUTWARD into material | Outside parent circle |

The validation pattern is always the same: compute the arc midpoint,
check which side of the parent surface it's on. If wrong, swap the
arc's from/to angles.

**Complete fillet workflow (trim + arc):**

```js
const fr = 15; // fillet radius
const fc = [cornerX - fr, cornerY + fr]; // for a right-angle corner

const tH = cad.projectOnto(fc, [hLine.start, hLine.end]);
const tV = cad.projectOnto(fc, [vLine.start, vLine.end]);

cad.trim(hLine.id, tH, "start");
cad.trim(vLine.id, tV, "end");

// p1/p2 mode: always gets the short arc, no angle reasoning needed
cad.addArc(fc, fr, { p1: tH, p2: tV, layer: "BRACKET" });
```

Three ways to specify arcs. Prefer `p1/p2` for fillets:

```js
cad.addArc(center, r, { p1: [x1,y1], p2: [x2,y2] })          // BEST for fillets
cad.addArc(center, r, { from: deg, to: deg, shortest: true }) // safe: auto-corrects
cad.addArc(center, r, { from: deg, to: deg })                 // raw CCW sweep
```

**Trim:** `cad.trim(entity, cutPoint, keep)` shortens a line or arc.
`keep` = `"start"`/`"from"` (keep start..cut) or `"end"`/`"to"`
(keep cut..end). Returns the new entity; original is removed.

**Common fillet mistakes:**
- Wrong offset direction: where is material, where is void? Validate
  with arc midpoint distance check.
- Forgetting to trim: parent lines still run through the arc. Trim
  both to their tangent points.
- Wrong arc sweep: `addArc` sweeps CCW. If sweep > 180, swap angles.

### Polar patterns

For geometry repeating around a center (gear teeth, bolt circles,
spoke patterns): design ONE instance, then rotate-copy.

```js
const master = [e1, e2, e3];  // IDs of the master instance
for (let i = 1; i < N; i++) {
  const copies = cad.copy(master, 0, 0);
  cad.rotate(copies.map(e => e.id), center, i * (360/N));
}
```

Do NOT compute each instance independently. Accumulated floating point
errors create visible gaps. One master + rotation is exact.

For bolt circles, use `projectOntoCircle` to place holes:

```js
for (let i = 0; i < N; i++) {
  const rad = i * (2 * Math.PI / N);
  const farPt = [500 * Math.cos(rad), 500 * Math.sin(rad)];
  const pt = cad.projectOntoCircle(farPt, center, boltCircleR);
  cad.addCircle(pt, holeR, { layer: "HOLES" });
}
```

### External tangent lines (center of similitude)

For two circles c1(r1) and c2(r2) where r1 > r2:

```js
const dr = rA - rB;
const S = [(rA*cB[0] - rB*cA[0])/dr, (rA*cB[1] - rB*cA[1])/dr];
const tangLen = Math.sqrt(cad.distance(S, cA)**2 - rA**2);
const tptsA = cad.circleCircleIntersection(cA, rA, S, tangLen);
const tangents = tptsA.map(tA => ({
  tA, tB: cad.projectOnto(cB, [tA, S])
}));
```

For trimmed profiles, replace full circles with arcs between their
tangent points. The "outside" arc (away from the other circle) is
the long sweep in CCW convention.

### Tangent circle construction (Apollonius-style)

When finding a circle tangent to multiple elements, reduce to distance
equations and solve algebraically:

- Tangent to line: `cx = r` (or `cy = r`)
- External tangent to circle (c, R): `dist(center, c) = r + R`
- Internal tangent: `dist(center, c) = |r - R|`

Substitute the line constraint first (reduces unknowns by one). Expand
distance equations, subtract pairwise to eliminate r^2 terms. Solve
the resulting linear + quadratic system.

Always verify solutions numerically: check all tangency distances.
Algebraic errors are common; verification is cheap.

### Involute gear tooth

The involute is the one curve that must be computed parametrically.
Everything else comes from intersections.

```js
function invPt(t, baseAngle, dir) {
  const r = baseR / Math.cos(t);
  const ang = baseAngle + dir * (Math.tan(t) - t);
  return [r * Math.cos(ang), r * Math.sin(ang)];
}
```

**Direction sign** for external spur gear:
- Right flank (CW side): `dir = -1` (angle decreases outward)
- Left flank (CCW side): `dir = +1`

Wrong sign produces a sawblade. Verify convexity: the involute
midpoint angle should be farther from the tooth centerline than the
straight-line midpoint between base and tip.

**Finding the tip point:** Do NOT compute `tTip = acos(baseR/outsideR)`
and trust it. Sample the involute past the expected tip, walk segments
to find the one crossing the outside circle, use
`lineCircleIntersection` on that segment.

**Base angle computation:**
```
rightBase = toothCenterAngle + halfToothAngle + inv(pressureAngle)
leftBase  = toothCenterAngle - halfToothAngle - inv(pressureAngle)
```
where `inv(a) = tan(a) - a` and `halfToothAngle = pi*module / (4*pitchR)`.

### S-bend construction

Two tangent arcs of equal radius connecting points on parallel lines:

- Arc 1 center directly above/below point A at distance R
- Arc 2 center directly above/below point B at distance R
- Tangency condition: dist(c1, c2) = 2R
- Solve: R = (dx^2 + dy^2) / (2 * lineSpacing)
- Inflection point = midpoint of the two centers

### Regular polygon by circle-stepping

Compass-only construction, no trig:

```js
const vertices = [v0];
for (let i = 0; i < N - 1; i++) {
  const prev = i > 0 ? vertices[i-1] : null;
  const hits = cad.circleCircleIntersection(vertices[i], chord, center, R);
  const next = prev
    ? hits.sort((a,b) =>
        Math.hypot(b[0]-prev[0], b[1]-prev[1]) -
        Math.hypot(a[0]-prev[0], a[1]-prev[1])
      )[0]
    : hits.sort((a,b) => b[1] - a[1])[0];
  vertices.push(next);
}
cad.addPolyline(vertices, { closed: true });
```

For hexagons, chord = R. For other N-gons: chord = 2*R*sin(pi/N).

### Pipe wall offset around bends

- Straight section: offset by +/-wall_thickness parallel to centerline
- Arc section: concentric arcs at bend_radius +/- wall_thickness
- End caps: short lines connecting inner and outer walls

### Block orientation

Before setting rotation on `cad.place()`, inspect the block's geometry
to understand its natural orientation at rotation=0. Then:
- Face east: rotation=0
- Face north: rotation=90
- Face west: rotation=180
- Face south: rotation=270

Always check the block geometry, don't assume.

### Mirror across an arbitrary angle

To mirror a point across a line through the origin at angle th:
```js
x_ = x * Math.cos(2*th) + y * Math.sin(2*th)
y_ = x * Math.sin(2*th) - y * Math.cos(2*th)
```

## Known API gaps

- **Entity snap**: snap placement to existing entity endpoints/midpoints
- **Spatial index** (rstar) for fast query_near, query_bounds on large drawings
- **Line weight**: no per-entity or per-layer line weight yet
