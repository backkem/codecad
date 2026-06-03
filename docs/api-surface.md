# CodeCAD JS API surface

General-purpose JavaScript API for AI agents (and humans) to inspect and
edit 2D CAD drawings. Multi-document: multiple drawings open in tabs,
each a separate session. The `cad` global is the sole entry point.

Not tied to any domain. Works for floor plans, electrical schematics,
site plans, mechanical drawings, whatever is in the DWG. Domain-specific
abstractions (rooms, circuits, nets) are user-built on top.

Two driving scenarios guide the design:
1. **Floor plan**: navigate a .dwg building plan, place sockets/switches/lights
2. **Electrical schematic**: draw line diagrams similar to QElectroTech

---

## Session management

All `cad.*` drawing/query calls target whichever session is set via
`cad.useSession()`. Agent workflow: select session once, then draw.

```js
// Open and target a document
cad.sessions.create("floor-plan.dwg")  // creates session + opens server sync
cad.useSession("floor-plan.dwg")       // all cad.* now targets this session

// Draw on it
cad.addLine([0,0], [100,0])
cad.addCircle([50,50], 30)

// Save to server disk
await cad.saveDwg("output.dwg")

// Switch to another document
cad.useSession("electrical.dwg")
cad.entities()  // returns entities from the electrical drawing

// Check which session is active
cad.currentSession()  // "electrical.dwg"
```

### Session API reference

| Method | Description |
|--------|-------------|
| `cad.useSession(id)` | Set which session `cad.*` targets. Errors if not found. |
| `cad.currentSession()` | Returns current target session ID, or null. |
| `cad.sessions.create(id)` | Create session + open server sync stream. |
| `cad.sessions.destroy(id)` | Close sync stream + drop session. |
| `cad.sessions.list()` | Returns `[{id, entity_count, js_target, visible}]`. |
| `cad.sessions.loadDwgBytes(id, bytes)` | Load DWG from Uint8Array into session. |
| `cad.viewport.start(canvasId, sessionId)` | Start eframe renderer on canvas. |
| `cad.viewport.stop(canvasId)` | Stop renderer. |
| `cad.api.listDocuments()` | `GET /api/documents`, returns available files. |

### Server-side script execution

Run JS scripts on the server in a Wasmtime WASM sandbox. Same `cad.*`
API as the browser, plus file I/O. Mutations batch into a single Yrs
update, synced to connected browsers in real time.

```js
// From browser console (WebTransport RPC)
await cad.exec("design/electrical/place-electrical.js")
await cad.runScript("cad.addCircle([0,0], 50); return cad.describe()")
```

```bash
# From CLI / MCP (HTTP)
curl -X POST http://localhost:8765/api/run \
  -H "Authorization: Bearer cadview-local-dev" \
  -d '{"exec": "design/electrical/place-electrical.js"}'
```

| Method | Description |
|--------|-------------|
| `cad.exec(path)` | Run .js file from disk (RPC to server sandbox) |
| `cad.runScript(code)` | Run inline JS code (RPC to server sandbox) |
| `cad.readFile(path)` | Read file from disk (server sandbox only) |
| `POST /api/run` | HTTP endpoint: `{program}` or `{exec}` field |

Scripts in the sandbox see all `cad.*` methods. `readFile` paths are
relative to the script's directory and sandboxed to the project root.
`console.log` output is captured in the response `stdout` field.

---

## Design principles

1. **Structured-first, screenshots second.** The agent reads the drawing
   through structured queries (`cad.entities()`, `cad.near()`). Screenshots
   are for visual verification, not primary navigation. (Borrowed from
   browser-use's "DOM-first" philosophy.)

2. **Everything is an array you can filter.** Queries return plain JS
   arrays. `.filter()`, `.find()`, `.map()`, `.sort()`, `.reduce()`,
   `.flatMap()` all work. No magic wrappers, no query DSL. The agent
   already knows arrays.

3. **Meters in, meters out.** All coordinates in meters. Internal storage
   is mm. The agent never sees mm.

4. **IDs are stable strings.** Every entity gets a string ID on creation
   that survives edits. The agent references entities by ID.

5. **Snap by default.** Placed geometry snaps to a 50mm grid. Override
   per-call with `{ snap: false }` or `{ snap: 0.1 }` (10cm grid).

6. **Mutations return the result.** Every edit returns the created or
   modified entity, giving the agent immediate feedback.

7. **Batch by design.** One script does a full batch of work. No
   round-trip overhead.

---

## Core API: observe

### Scene summary

```js
cad.describe()
// {
//   bounds: { min: [0, 0], max: [18.5, 14.2] },
//   entities: 4335,
//   layers: ["S_WALL", "C_DOORS", "C_WINDOWS", "F_FURNITURE", ...],
//   counts: { line: 3800, arc: 320, circle: 45, polyline: 33, block: 170 }
// }
```

First call in any session. Cheap overview.

### Entity traversal

The central query. Returns every entity as a plain array. All the power
comes from standard JS array methods.

```js
// All entities
cad.entities()

// Entity shape (common to all types):
// {
//   id: "e_1234",
//   type: "line",          // "line" | "arc" | "circle" | "polyline" | "block"
//   layer: "S_WALL",
//   color: [255, 255, 255],
//   bounds: { min: [0, 0], max: [5.0, 0] },
//
//   // type-specific fields:
//   // line:     start, end
//   // arc:      center, radius, startAngle, endAngle
//   // circle:   center, radius
//   // polyline: points, closed
//   // block:    name, insertPoint, rotation, scale
// }
```

All coordinates in meters. All angles in degrees.

#### Filtering patterns

```js
// By layer
cad.entities().filter(e => e.layer === "S_WALL")

// By type
cad.entities().filter(e => e.type === "circle")

// By layer prefix
cad.entities().filter(e => e.layer.startsWith("E_"))

// Compound
cad.entities().filter(e => e.type === "block" && e.layer === "C_DOORS")

// Spatial: inside a bounding box
cad.entities().filter(e => cad.overlaps(e, { min: [5, 3], max: [11, 7] }))

// Spatial: within distance of a point
cad.entities().filter(e => cad.distanceTo(e, [8, 5]) < 2.0)

// Closest entity of a type to a point
cad.entities()
  .filter(e => e.type === "line" && e.layer === "S_WALL")
  .map(e => ({ ...e, dist: cad.distanceTo(e, [8, 5]) }))
  .sort((a, b) => a.dist - b.dist)
  [0]

// Count per layer
cad.entities().reduce((acc, e) => {
  acc[e.layer] = (acc[e.layer] || 0) + 1
  return acc
}, {})

// Group blocks by name
cad.entities()
  .filter(e => e.type === "block")
  .reduce((acc, e) => {
    acc[e.name] = (acc[e.name] || [])
    acc[e.name].push(e)
    return acc
  }, {})

// All unique block names in the drawing
[...new Set(cad.entities().filter(e => e.type === "block").map(e => e.name))]

// Lines longer than 5m
cad.entities()
  .filter(e => e.type === "line")
  .filter(e => cad.length(e.id) > 5.0)

// Entities that share an endpoint with entity e_42
const ep = cad.entity("e_42").end
cad.entities().filter(e =>
  e.type === "line" &&
  (cad.distance(e.start, ep) < 0.001 || cad.distance(e.end, ep) < 0.001)
)
```

### Single entity lookup

```js
cad.entity("e_42")      // by ID, returns entity or null
```

### Accelerated spatial queries

R-tree backed, O(log n). Use these for large drawings instead of
filtering the full entity array:

```js
cad.near([8.0, 5.0], 2.0)                     // within 2m of point
// [{ entity, distance }, ...]                  // sorted by distance

// Entities whose bounding box intersects the rectangle (partial overlap)
cad.inBounds({ min: [5, 3], max: [11, 7] })
// entity array

// Entities fully contained within the rectangle
cad.inBounds({ min: [5, 3], max: [11, 7] }, "contain")
// entity array

// Both modes:
//   "intersect" (default) - any overlap, even partial
//   "contain"             - entity bounds fully inside query rect
```

### Topology queries

```js
// Entities sharing an endpoint with e_42 (within snap tolerance)
cad.connectedTo("e_42")
// entity array - lines/arcs whose start or end touches e_42's start or end

// Sub-entities of a block insert (the flattened children)
cad.children("e_42")
// entity array - all entities that were created by flattening this block
// Useful for: finding door swing arcs, schematic pin circles, etc.
```

### Measurements

Deterministic tools. The agent never does coordinate math.

```js
// Distance
cad.distance([5, 3], [11, 3])      // point-point -> 6.0
cad.distance("e_42", "e_87")       // entity-entity (closest approach)
cad.distance([8, 5], "e_42")       // point-entity

// Length of a line, arc, polyline
cad.length("e_42")                  // -> 6.4

// Angle between two lines (at their intersection)
cad.angle("e_42", "e_87")          // -> 90.0

// Area of a closed polyline
cad.area("e_100")                   // -> 46.6

// Points on geometry
cad.midpoint("e_42")               // -> [8.2, 7.2]
cad.startOf("e_42")                // -> [5.0, 7.2]
cad.endOf("e_42")                  // -> [11.4, 7.2]
```

### Geometric helpers

Compute derived points without creating entities. Every helper is
**polymorphic**: accepts entity IDs or raw point pairs interchangeably.
This is critical: user code constantly works with derived segments
(wall edges from polyline vertices, schematic grid points) that aren't
standalone entities.

```js
// Interpolate (0.0 = start, 1.0 = end)
cad.lerp("e_42", 0.5)                   // midpoint of entity
cad.lerp([5, 3], [11, 7], 0.5)          // midpoint of two points

// Point at distance along a segment
cad.along("e_42", 2.5)                  // 2.5m from entity start
cad.along([5, 7.2], [11.4, 7.2], 2.5)   // 2.5m along a point pair

// Direction angle of a segment (degrees, 0=east, 90=north)
cad.direction("e_42")                    // -> 0.0 (east-pointing line)
cad.direction([5, 3], [5, 7])            // -> 90.0 (north)

// Normal (perpendicular unit vector, left-hand side)
cad.normal("e_42")                       // -> [0, -1]
cad.normal([5, 7.2], [11.4, 7.2])       // -> [0, -1] (south of east wall)

// Perpendicular foot: project point onto line/segment
cad.projectOnto([8, 5], "e_42")         // -> [8.0, 7.2]
cad.projectOnto([8, 5], [5, 7.2], [11.4, 7.2])  // same, raw points

// Intersection of two lines/arcs
cad.intersection("e_42", "e_87")        // -> [11.4, 7.2] or null

// Parallel offset (returns point pair, not a new entity)
cad.parallel("e_42", 0.2)              // line 0.2m offset
cad.parallel([5, 7.2], [11.4, 7.2], 0.2)

// Offset a point by dx, dy
cad.offset([5, 3], 0.3, 0)             // -> [5.3, 3.0]

// Rotate a point around a center (pure computation, no mutation)
cad.rotatePoint([5, 0], 90, [0, 0])    // -> [0, 5]

// Point-to-segment distance
cad.distanceToSegment([8, 5], [5, 7.2], [11.4, 7.2])  // -> 2.2

// Point-in-polygon test
cad.pointInPolygon([8, 5], [[5,3], [11,3], [11,7], [5,7]])  // -> true

// Centroid of a point array
cad.centroid([[0,0], [10,0], [10,8], [0,8]])  // -> [5, 4]

// Bounding box of a point array
cad.boundsOf([[3,1], [7,5], [2,9]])    // -> { min: [2,1], max: [7,9] }

// Decompose a polyline into edge segments
cad.segments("e_100")
// [{ start: [0,0], end: [5,0] }, { start: [5,0], end: [5,4] }, ...]
```

### Layers

```js
cad.layers()
// [{ name: "S_WALL", color: [255,255,255], visible: true, locked: false }, ...]

cad.layers().filter(l => l.name.startsWith("E_"))
cad.layers().find(l => l.name === "S_WALL")
```

### Screenshots

Visual verification only.

```js
cad.screenshot()                                // full plan
cad.screenshot({ center: [8, 5], zoom: 2.0 })  // custom viewport
cad.screenshot({ bounds: { min: [5,3], max: [11,7] } })  // fit to rect
cad.screenshot({ highlight: ["e_42", "e_87"] }) // color specific entities
cad.screenshot({
  layers: ["S_WALL", "E_POWR"],                 // only show these
  highlight: cad.near([8, 5], 1.0).map(r => r.entity.id)
})
```

---

## Core API: edit

### Raw geometry creation

```js
cad.addLine([5, 3], [5, 7.2], { layer: "E_POWR" })
cad.addArc([2.5, 0], 0.45, { from: 180, to: 270 })
cad.addCircle([8, 5], 0.025, { layer: "E_LITE" })
cad.addPolyline([[0,0], [5,0], [5,4], [0,4]], { closed: true, layer: "S_WALL" })
cad.addText("Label", [8.2, 5.1], { size: 0.15, layer: "A_TEXT" })
```

All return the created entity (with `id`).

Common options on every creation call:
- `layer`: string (default: current layer)
- `color`: [r,g,b] (default: ByLayer)
- `snap`: boolean or grid size (default: true / 0.05)

### Block definitions

Define reusable symbols. A block is a named collection of geometry at a
local origin. Instances share the definition.

```js
cad.defineBlock("SOCKET_DOUBLE", [
  { type: "line", start: [0, 0], end: [0.014, 0] },
  { type: "arc", center: [0.020, 0], radius: 0.006, from: 90, to: 270 },
  { type: "arc", center: [0.028, 0], radius: 0.006, from: 90, to: 270 },
], {
  insertPoint: [0, 0],          // where it attaches (wall contact)
  defaultLayer: "E_POWR",
})
```

Or load block definitions from a library file:

```js
cad.loadBlocks("electrical-arei.json")
cad.loadBlocks("furniture-eu.json")
```

### Placing blocks

```js
cad.place("SOCKET_DOUBLE", { at: [8.0, 3.0], rotation: 90 })
// Returns: { id: "e_4336", type: "block", name: "SOCKET_DOUBLE",
//            insertPoint: [8.0, 3.0], rotation: 90, layer: "E_POWR" }

cad.place("LIGHT_CEILING", { at: [8.2, 5.1] })

cad.place("RESISTOR", { at: [4.0, 2.0], rotation: 0, layer: "SCH_COMP" })
```

Options:
- `at`: [x, y] in meters (required)
- `rotation`: degrees, default 0
- `scale`: [sx, sy] or number, default 1
- `layer`: override the block's default layer
- `snap`: override snap setting

The API is intentionally minimal: point + rotation. Domain-specific
helpers (place on wall, place at schematic grid position) are user-built.

### Mutations

Every mutation accepts a **target**: a single entity ID, an array of
IDs, or an array of entity objects (as returned by `.filter()`). This
means any ad-hoc selection flows directly into a mutation.

```js
// Single entity
cad.remove("e_42")

// Array of IDs
cad.remove(["e_42", "e_43", "e_44"])

// Ephemeral selection from a filter chain
cad.remove(cad.entities().filter(e => e.layer === "E_POWR"))

// Move everything on a layer
cad.move(cad.entities().filter(e => e.layer === "E_POWR"), 0.5, 0)

// Rotate a selection around a point
const sel = cad.entities().filter(e => e.type === "block" && e.name === "MCB_1P")
cad.rotate(sel, 90, [5, 5])

// Change layer of a filtered set
cad.setLayer(
  cad.entities().filter(e => e.layer === "OLD_LAYER"),
  "NEW_LAYER"
)

// Mirror a selection across a line
const furniture = cad.entities().filter(e => e.layer === "F_FURNITURE")
cad.mirror(furniture, [0, 5], [10, 5])
```

Full mutation API:

```js
cad.remove(target)

cad.move(target, dx, dy)               // relative
cad.move(target, { to: [x, y] })       // absolute (single entity only)

cad.copy(target, dx, dy)               // relative -> new entities
cad.copy(target, { to: [x, y] })       // absolute -> new entity

cad.rotate(target, degrees)            // around each entity's center
cad.rotate(target, degrees, [cx, cy])  // around explicit point

cad.mirror(target, [x1, y1], [x2, y2])  // across line

cad.setLayer(target, "LAYER_NAME")
cad.setColor(target, [r, g, b])
```

Returns: for single target, the updated entity. For array target, array
of updated entities.

### Undo

```js
cad.undo()                // last operation
cad.undo(5)               // last 5
cad.redo()
cad.checkpoint("before electrical")   // named restore point
cad.restore("before electrical")
```

### Layer management

```js
cad.addLayer("E_ALARM", { color: [255, 0, 255] })
cad.setLayerVisible("F_FURNITURE", false)
cad.setLayerLocked("S_WALL", true)
```

---

## Layer hierarchy as semantics

Layers encode meaning. The naming convention creates a semantic tree:
the agent reads layer names to understand what things are. This works
for any domain, not just architecture.

### Prefix conventions

For floor plans (Belgian DWG convention):

| Prefix | Meaning | Examples |
|---|---|---|
| `S_` | Structure | `S_WALL`, `S_COLO` |
| `C_` | Circulation/openings | `C_DOORS`, `C_WINDOWS` |
| `F_` | Furniture/finishes | `F_FURNITURE`, `F_SANITAIR` |
| `A_` | Annotation | `A_DIMS`, `A_TEXT` |
| `E_` | Electrical (our overlay) | `E_POWR`, `E_SWCH`, `E_LITE`, `E_DATA` |

For schematics:

| Prefix | Meaning | Examples |
|---|---|---|
| `SCH_` | Schematic | `SCH_COMP`, `SCH_WIRE`, `SCH_JUNC` |
| `SCH_LABEL` | Labels | `SCH_LABEL_COMP`, `SCH_LABEL_NET` |
| `SCH_BUS` | Bus lines | |

### Hidden semantic layers (agent mnemonics)

Layers prefixed with `_` are **off by default** in the viewer. They
exist as queryable metadata the human can toggle on for visualization.
They're the agent's internal annotations made visible.

The specific `_` layers are domain-dependent. Examples:

**Floor plan:**

| Layer | What's on it |
|---|---|
| `_ROOM` | Closed polylines per detected room, fill-colored. |
| `_ROOM_LABEL` | Room name text at centroids. |
| `_WALL_SEG` | Wall segment ID labels ("kitchen/north"). |
| `_DOOR_SWING` | Arc showing swing direction + hinge dot. |
| `_CLEARANCE` | Rectangles for minimum clearances. |

**Schematic:**

| Layer | What's on it |
|---|---|
| `_NET` | Colored polylines tracing each electrical net. |
| `_NET_LABEL` | Net name text at wire midpoints. |
| `_PIN` | Visible dots on component connection points. |
| `_REFDES` | Component reference designators (R1, C3, Q2). |

These are created by user scripts, not by the core API. The `_` prefix
is a convention, not enforced.

```js
// Toggle them for debugging
cad.setLayerVisible("_ROOM", true)
cad.screenshot()

// Query them
cad.entities().filter(e => e.layer === "_NET")
```

### Layer as filter axis

Prefix matching is the primary discovery mechanism:

```js
cad.entities().filter(e => e.layer.startsWith("E_"))    // all electrical
cad.entities().filter(e => e.layer.startsWith("S_"))    // all structural
cad.entities().filter(e => e.layer.startsWith("SCH_"))  // all schematic
cad.entities().filter(e => e.layer.startsWith("_"))     // all semantic
```

---

## Exercise A: floor plan helpers from core primitives

Build `rooms()`, `room("kitchen")`, wall/door models, and placement
helpers using only the core API. Everything below is user code.

### Geometry utilities

Small pure functions the domain helpers depend on:

```js
function centroid(points) {
  const n = points.length
  return [
    points.reduce((s, p) => s + p[0], 0) / n,
    points.reduce((s, p) => s + p[1], 0) / n,
  ]
}

function compassDir(from, to) {
  const dx = to[0] - from[0], dy = to[1] - from[1]
  const angle = Math.atan2(dy, dx) * 180 / Math.PI  // -180..180
  if (angle > -45 && angle <= 45)   return "east"
  if (angle > 45 && angle <= 135)   return "north"   // Y-up in DWG
  if (angle > -135 && angle <= -45) return "south"
  return "west"
}

// Point-to-segment distance
function ptSegDist(pt, a, b) {
  const dx = b[0] - a[0], dy = b[1] - a[1]
  const len2 = dx * dx + dy * dy
  if (len2 === 0) return cad.distance(pt, a)
  const t = Math.max(0, Math.min(1, ((pt[0]-a[0])*dx + (pt[1]-a[1])*dy) / len2))
  return cad.distance(pt, [a[0] + t*dx, a[1] + t*dy])
}

// Rotate point around center by degrees
function rotatePt(pt, deg, center) {
  const r = deg * Math.PI / 180
  const dx = pt[0] - center[0], dy = pt[1] - center[1]
  return [
    center[0] + dx * Math.cos(r) - dy * Math.sin(r),
    center[1] + dx * Math.sin(r) + dy * Math.cos(r),
  ]
}
```

### Room detection

```js
// Strategy 1: DWG has closed polylines on S_WALL (ideal case)
function detectRoomsFromPolylines() {
  return cad.entities()
    .filter(e => e.type === "polyline" && e.closed && e.layer === "S_WALL")
    .map(poly => ({
      polyId: poly.id,
      points: poly.points,
      area: cad.area(poly.id),
      center: centroid(poly.points),
      bounds: poly.bounds,
    }))
}

// Strategy 2: build from individual wall lines (common in DWGs)
function detectRoomsFromLines() {
  const walls = cad.entities()
    .filter(e => e.type === "line" && e.layer === "S_WALL")

  // Build adjacency graph: endpoints within 0.01m tolerance are connected
  const TOL = 0.01
  const graph = new Map()  // point-key -> [entity ids that touch it]

  function ptKey(p) { return `${Math.round(p[0]/TOL)},${Math.round(p[1]/TOL)}` }

  for (const w of walls) {
    const sk = ptKey(w.start), ek = ptKey(w.end)
    if (!graph.has(sk)) graph.set(sk, [])
    if (!graph.has(ek)) graph.set(ek, [])
    graph.get(sk).push({ id: w.id, otherKey: ek, start: w.start, end: w.end })
    graph.get(ek).push({ id: w.id, otherKey: sk, start: w.end, end: w.start })
  }

  // Find minimal cycles (faces) in the planar graph
  // ... standard face-finding algorithm: at each node, sort edges by
  // angle, traverse always turning right (or left) to find closed loops ...
  // This is the hard part. Omitted for brevity, but the key insight is
  // that the core API provides all the geometry; the topology is user code.

  return cycles.map(loop => ({
    points: loop,
    area: polyArea(loop),
    center: centroid(loop),
    bounds: boundsOf(loop),
  }))
}
```

### Room name table

```js
// User-maintained JSON mapping detected polygons to names
const ROOM_NAMES = {
  "kitchen":        { label_nl: "Keuken",     usage: "kitchen",     tier: "high" },
  "living":         { label_nl: "Leefruimte", usage: "living",      tier: "high" },
  "hallway_ground": { label_nl: "Inkomhal",   usage: "circulation", tier: "standard" },
  "wc_ground":      { label_nl: "WC",         usage: "wet",         tier: "standard" },
  // ...
}

// Match detected polygons to names by centroid proximity
function nameRooms(detectedRooms, nameTable) {
  // Simple: user provides centroid hints per room
  const hints = {
    "kitchen": [8.2, 5.1],
    "living":  [3.5, 5.0],
    // ...
  }

  return detectedRooms.map(room => {
    const match = Object.entries(hints)
      .map(([name, hint]) => ({ name, dist: cad.distance(room.center, hint) }))
      .sort((a, b) => a.dist - b.dist)[0]

    return {
      ...room,
      name: match.name,
      ...nameTable[match.name],
    }
  })
}
```

### Wall segments and doors

```js
function wallSegments(room) {
  const pts = room.points
  return pts.map((p, i) => {
    const next = pts[(i + 1) % pts.length]
    return {
      roomName: room.name,
      index: i,
      start: p,
      end: next,
      length: cad.distance(p, next),
      direction: compassDir(p, next),
      id: `${room.name}/${compassDir(p, next)}`,  // "kitchen/north"
    }
  })
}

function doorsForRoom(room) {
  const walls = wallSegments(room)
  const doorBlocks = cad.entities()
    .filter(e => e.type === "block" && e.layer === "C_DOORS")

  return doorBlocks
    .map(door => {
      // Which wall is this door on?
      const wall = walls
        .map(w => ({ ...w, dist: ptSegDist(door.insertPoint, w.start, w.end) }))
        .sort((a, b) => a.dist - b.dist)[0]

      if (wall.dist > 0.5) return null  // not close to any wall of this room

      // Position along wall (meters from start)
      const dx = wall.end[0] - wall.start[0], dy = wall.end[1] - wall.start[1]
      const len = wall.length
      const t = ((door.insertPoint[0]-wall.start[0])*dx + (door.insertPoint[1]-wall.start[1])*dy) / (len*len)
      const position = t * len

      // Swing direction from arc sub-entities
      // Door blocks contain an arc whose sweep tells us the swing
      const arcs = cad.entities()
        .filter(e => e.type === "arc")
        .filter(e => cad.distance(e.center, door.insertPoint) < 0.05)
      const swing = arcs.length > 0 ? (arcs[0].endAngle > arcs[0].startAngle ? "left" : "right") : "unknown"

      return {
        id: door.id,
        blockName: door.name,
        wall: wall,
        position: position,
        width: 0.9,  // derive from block geometry or metadata
        swing: swing,
        insertPoint: door.insertPoint,
      }
    })
    .filter(Boolean)
}
```

### The `rooms()` / `room("kitchen")` interface

Now assemble into the convenience API that the previous version had
baked in:

```js
// Build the model once
const _detected = detectRoomsFromPolylines()  // or detectRoomsFromLines()
const _named = nameRooms(_detected, ROOM_NAMES)
const _rooms = _named.map(r => ({
  ...r,
  walls: wallSegments(r),
  doors: doorsForRoom(r),
  windows: windowsForRoom(r),  // similar to doorsForRoom, on C_WINDOWS
}))

// Public helpers
function rooms() { return _rooms }

function room(name) {
  return _rooms.find(r => r.name === name) || null
}

// Derived queries
function adjacent(roomName) {
  const r = room(roomName)
  return [...new Set(r.doors.map(d => {
    // The "other room" is the room on the other side of this door
    return _rooms
      .filter(other => other.name !== roomName)
      .find(other => doorsForRoom(other).some(od => od.id === d.id))
      ?.name
  }).filter(Boolean))]
}

function entitiesInRoom(roomName) {
  const r = room(roomName)
  return cad.inBounds(r.bounds, "contain")
    .filter(e => pointInPolygon(entityCenter(e), r.points))
}

function entitiesOnWall(wallId) {
  const [roomName, dir] = wallId.split("/")
  const wall = room(roomName)?.walls.find(w => w.direction === dir)
  if (!wall) return []
  return cad.entities().filter(e =>
    ptSegDist(entityCenter(e), wall.start, wall.end) < 0.15
  )
}
```

### Placing on a wall

```js
function placeOnWall(blockName, wall, distAlongWall, opts = {}) {
  // Compute point at distance along the wall
  const t = distAlongWall / wall.length
  const pt = [
    wall.start[0] + t * (wall.end[0] - wall.start[0]),
    wall.start[1] + t * (wall.end[1] - wall.start[1]),
  ]

  // Rotation: block should face into the room (perpendicular to wall)
  const wallAngle = Math.atan2(
    wall.end[1] - wall.start[1],
    wall.end[0] - wall.start[0],
  ) * 180 / Math.PI
  // +90 rotates the block to face "left" of the wall direction (into room)
  const rotation = wallAngle + 90

  return cad.place(blockName, { at: pt, rotation, ...opts })
}

function placeNearDoor(blockName, door, side, offset, opts = {}) {
  // side: "hinge" | "open" | "left" | "right"
  const wall = door.wall
  let pos = door.position

  if (side === "hinge" || side === "left") {
    pos -= (door.width / 2 + offset)
  } else {
    pos += (door.width / 2 + offset)
  }

  // Clamp to wall bounds
  pos = Math.max(0.05, Math.min(wall.length - 0.05, pos))

  return placeOnWall(blockName, wall, pos, opts)
}

function placeNearCorner(blockName, wall, whichEnd, offset, opts = {}) {
  const dist = whichEnd === "start" ? offset : wall.length - offset
  return placeOnWall(blockName, wall, dist, opts)
}

function distributeInRoom(blockName, room, count) {
  // Simple grid distribution inside room bounds
  const b = room.bounds
  const cols = Math.ceil(Math.sqrt(count * (b.max[0]-b.min[0]) / (b.max[1]-b.min[1])))
  const rows = Math.ceil(count / cols)
  const dx = (b.max[0] - b.min[0]) / (cols + 1)
  const dy = (b.max[1] - b.min[1]) / (rows + 1)

  const placed = []
  let n = 0
  for (let r = 1; r <= rows && n < count; r++) {
    for (let c = 1; c <= cols && n < count; c++) {
      const pt = [b.min[0] + c * dx, b.min[1] + r * dy]
      // Only place if inside room polygon (handles L-shapes etc.)
      if (pointInPolygon(pt, room.points)) {
        placed.push(cad.place(blockName, { at: pt }))
        n++
      }
    }
  }
  return placed
}
```

### Full example: wire up the kitchen

```js
const k = room("kitchen")

// 1. Socket near each door, hinge side
k.doors.forEach(d =>
  placeNearDoor("SOCKET_DOUBLE", d, "hinge", 0.3)
)

// 2. Switch near each door, open side
k.doors.forEach(d =>
  placeNearDoor("SWITCH_SINGLE", d, "open", 0.15)
)

// 3. Ceiling lights: 1 per 6 sqm, minimum 2
distributeInRoom("LIGHT_CEILING", k, Math.max(2, Math.ceil(k.area / 6)))

// 4. Corner sockets on walls without doors
k.walls
  .filter(w => !k.doors.some(d => d.wall.id === w.id))
  .filter(w => w.length > 1.0)
  .forEach(w => {
    placeNearCorner("SOCKET_DOUBLE", w, "start", 0.3)
    if (w.length > 3.0)
      placeNearCorner("SOCKET_DOUBLE", w, "end", 0.3)
  })

// 5. Verify
const placed = entitiesInRoom("kitchen").filter(e => e.layer.startsWith("E_"))
console.log(`kitchen: ${placed.length} electrical entities placed`)

// 6. See the result
cad.screenshot({
  bounds: k.bounds,
  highlight: placed.map(e => e.id)
})
```

### Batch all rooms

```js
for (const r of rooms()) {
  cad.checkpoint(`before-${r.name}`)

  r.doors.forEach(d => placeNearDoor("SOCKET_DOUBLE", d, "hinge", 0.3))
  r.doors.forEach(d => placeNearDoor("SWITCH_SINGLE", d, "open", 0.15))
  distributeInRoom("LIGHT_CEILING", r, Math.max(1, Math.ceil(r.area / 8)))

  r.walls
    .filter(w => !r.doors.some(d => d.wall.id === w.id) && w.length > 1.0)
    .forEach(w => placeNearCorner("SOCKET_DOUBLE", w, "start", 0.3))

  const count = entitiesInRoom(r.name).filter(e => e.layer.startsWith("E_")).length
  console.log(`${r.name}: ${count} electrical entities`)
}
```

### Semantic overlay layers (written by user code)

```js
// Persist the room model as visible geometry on hidden layers
for (const r of rooms()) {
  cad.addPolyline(r.points, { closed: true, layer: "_ROOM", color: [100,150,200] })
  cad.addText(r.name, r.center, { size: 0.3, layer: "_ROOM_LABEL" })

  for (const w of r.walls) {
    const mid = cad.midpoint(w.start, w.end)  // point-point midpoint (not entity)
    cad.addText(w.id, mid, { size: 0.12, layer: "_WALL_SEG" })
  }
}

cad.setLayerVisible("_ROOM", false)
cad.setLayerVisible("_ROOM_LABEL", false)
cad.setLayerVisible("_WALL_SEG", false)
```

---

## Exercise B: electrical schematic (QElectroTech style)

Same core API, entirely different domain. Single-line diagram of the
distribution board, circuits, and protection devices.

### Grid and coordinate helpers

Schematics live on a logical grid. Components snap to grid crossings,
wires run along grid lines.

```js
const GRID = 0.0025  // 2.5mm per grid unit (in meters, the API's unit)

function gpt(gx, gy) { return [gx * GRID, gy * GRID] }

// Shorthand: move in grid units from a point
function right(pt, n) { return [pt[0] + n * GRID, pt[1]] }
function up(pt, n)    { return [pt[0], pt[1] + n * GRID] }
function left(pt, n)  { return right(pt, -n) }
function down(pt, n)  { return up(pt, -n) }
```

### Block definitions with pin metadata

Blocks carry pin positions as a convention: the block definition
includes circle entities on a special sub-layer `_PIN` at each
connection point.

```js
cad.defineBlock("MCB_1P", [
  // Body: rectangle
  { type: "polyline", points: [gpt(0,1), gpt(4,1), gpt(4,7), gpt(0,7)],
    closed: true },
  // Symbol inside
  { type: "line", start: gpt(2, 2), end: gpt(2, 6) },
  { type: "arc", center: gpt(2, 4), radius: GRID * 1.5, from: 0, to: 180 },
  // Pin markers (on _PIN layer, tiny circles the system can find)
  { type: "circle", center: gpt(2, 0), radius: GRID * 0.3, layer: "_PIN" },  // in
  { type: "circle", center: gpt(2, 8), radius: GRID * 0.3, layer: "_PIN" },  // out
], { insertPoint: gpt(0, 0), defaultLayer: "SCH_COMP" })

cad.defineBlock("RCD_2P", [
  { type: "polyline", points: [gpt(0,1), gpt(8,1), gpt(8,7), gpt(0,7)], closed: true },
  { type: "circle", center: gpt(2, 0), radius: GRID * 0.3, layer: "_PIN" },  // L in
  { type: "circle", center: gpt(6, 0), radius: GRID * 0.3, layer: "_PIN" },  // N in
  { type: "circle", center: gpt(2, 8), radius: GRID * 0.3, layer: "_PIN" },  // L out
  { type: "circle", center: gpt(6, 8), radius: GRID * 0.3, layer: "_PIN" },  // N out
], { insertPoint: gpt(0, 0), defaultLayer: "SCH_COMP" })

// Or load a library
// cad.loadBlocks("iec-panel-symbols.json")
```

### Pin discovery

Generic: find pins on any placed component block by looking for `_PIN`
circles near it.

```js
function pins(component) {
  const c = cad.entity(component.id)
  // Find _PIN circles close to this block insert
  return cad.near(c.insertPoint, 0.05)
    .map(r => r.entity)
    .filter(e => e.layer === "_PIN" && e.type === "circle")
    .map(e => e.center)
    .sort((a, b) => a[1] - b[1])  // sort bottom to top (Y-up)
}

function pinIn(component, index = 0) {
  return pins(component)[index]
}

function pinOut(component, index) {
  const p = pins(component)
  return p[index !== undefined ? index : p.length - 1]
}
```

### Wiring helpers

```js
function wire(...points) {
  // Polyline through all points on wire layer
  return cad.addPolyline(points, { layer: "SCH_WIRE" })
}

// L-shaped wire: horizontal then vertical (or vice versa)
function wireHV(from, to) {
  const corner = [to[0], from[1]]
  return wire(from, corner, to)
}

function wireVH(from, to) {
  const corner = [from[0], to[1]]
  return wire(from, corner, to)
}

function junction(pt) {
  return cad.addCircle(pt, GRID * 0.6, { layer: "SCH_JUNC", color: [0,0,0] })
}

function busBar(from, to) {
  return cad.addLine(from, to, { layer: "SCH_BUS" })
}
```

### Labeling

```js
function refdes(component, text) {
  const c = cad.entity(component.id)
  const pos = [c.insertPoint[0] + 5 * GRID, c.insertPoint[1] + 4 * GRID]
  return cad.addText(text, pos, { size: GRID * 3, layer: "SCH_LABEL_COMP" })
}

function netLabel(pt, name) {
  const pos = [pt[0] + GRID, pt[1] + GRID]
  return cad.addText(name, pos, { size: GRID * 2.5, layer: "SCH_LABEL_NET" })
}
```

### Net tracing (read-back)

Find all entities connected in an electrical net:

```js
function traceNet(startPt, visited = new Set()) {
  const key = `${startPt[0].toFixed(6)},${startPt[1].toFixed(6)}`
  if (visited.has(key)) return []
  visited.add(key)

  // Find wires touching this point
  const touching = cad.near(startPt, GRID * 0.5)
    .map(r => r.entity)
    .filter(e => e.layer === "SCH_WIRE" || e.layer === "SCH_BUS")

  const result = [...touching]

  for (const w of touching) {
    // Follow the wire to its other end(s)
    if (w.type === "line") {
      const otherEnd = cad.distance(w.start, startPt) < GRID * 0.5 ? w.end : w.start
      result.push(...traceNet(otherEnd, visited))
    }
    if (w.type === "polyline") {
      for (const pt of w.points) {
        if (cad.distance(pt, startPt) > GRID * 0.5) {
          result.push(...traceNet(pt, visited))
        }
      }
    }
  }

  return result
}

// Find all components on a net
function componentsOnNet(startPt) {
  const netEntities = traceNet(startPt)
  const netPoints = netEntities.flatMap(e =>
    e.type === "line" ? [e.start, e.end] : e.points || [e.center]
  )

  return cad.entities()
    .filter(e => e.type === "block" && e.layer === "SCH_COMP")
    .filter(comp =>
      pins({ id: comp.id }).some(pin =>
        netPoints.some(np => cad.distance(pin, np) < GRID * 0.5)
      )
    )
}
```

### Full example: distribution board

```js
cad.addLayer("SCH_COMP", { color: [0, 0, 0] })
cad.addLayer("SCH_WIRE", { color: [0, 0, 200] })
cad.addLayer("SCH_BUS",  { color: [200, 0, 0] })
cad.addLayer("SCH_JUNC", { color: [0, 0, 0] })
cad.addLayer("SCH_LABEL_COMP", { color: [100, 100, 100] })
cad.addLayer("SCH_LABEL_NET",  { color: [0, 150, 0] })
cad.addLayer("_PIN", { color: [255, 0, 0] })
cad.setLayerVisible("_PIN", false)

// Main breaker at top
const main = cad.place("MCB_3P", { at: gpt(20, 80) })
refdes(main, "Q0")

// 4 RCD groups
const rcdSpacing = 20  // grid units between RCDs
const circuits = [
  { rcd: "Q1", mcbs: ["kitchen", "living", "dining"] },
  { rcd: "Q2", mcbs: ["bed1", "bed2", "bed3", "bed4"] },
  { rcd: "Q3", mcbs: ["bath1", "bath2", "laundry", "wc"] },
  { rcd: "Q4", mcbs: ["garage", "outdoor", "technical"] },
]

circuits.forEach((group, gi) => {
  const rcdPos = gpt(10 + gi * rcdSpacing, 60)
  const rcd = cad.place("RCD_2P", { at: rcdPos })
  refdes(rcd, group.rcd)

  // Wire main to RCD
  wireVH(pinOut(main, gi), pinIn(rcd))

  // MCBs under this RCD
  group.mcbs.forEach((name, mi) => {
    const mcbPos = down(rcdPos, 20 + mi * 12)
    const mcb = cad.place("MCB_1P", { at: mcbPos })
    refdes(mcb, `${group.rcd}.${mi + 1}`)
    netLabel(pinOut(mcb), name.toUpperCase())

    // Wire RCD -> MCB
    wireVH(pinOut(rcd), pinIn(mcb))

    // Junction at branch point (if more than one MCB)
    if (mi > 0) {
      const prevMcb = down(rcdPos, 20 + (mi - 1) * 12)
      junction([prevMcb[0] + 2 * GRID, rcdPos[1] - 12 * GRID])
    }
  })
})

// Verify
const compCount = cad.entities().filter(e => e.layer === "SCH_COMP").length
const wireCount = cad.entities().filter(e => e.layer === "SCH_WIRE").length
console.log(`Panel: ${compCount} components, ${wireCount} wires`)

cad.screenshot()
```

### Semantic overlay (schematic)

```js
// Build net overlays: trace each net, draw it on _NET in a unique color
const netColors = [[255,0,0], [0,200,0], [0,0,255], [200,200,0], [200,0,200]]

const mcbs = cad.entities()
  .filter(e => e.type === "block" && e.name === "MCB_1P")

mcbs.forEach((mcb, i) => {
  const outPin = pinOut({ id: mcb.id })
  const netEnts = traceNet(outPin)
  const color = netColors[i % netColors.length]

  netEnts.forEach(e => {
    // Draw a colored copy on the _NET layer
    if (e.type === "line") cad.addLine(e.start, e.end, { layer: "_NET", color })
    if (e.type === "polyline") cad.addPolyline(e.points, { layer: "_NET", color })
  })
})

cad.setLayerVisible("_NET", false)  // hidden by default
```

---

## Execution model

### Sandbox

WASM sandbox. `cad` is the only external global. Standard JS
(ES2023+) available. No filesystem, no network, no `eval`.

### Script lifecycle

```
Agent writes JS
    │
    ▼
Sandbox executes against Document
    │
    ├─ Reads:  cad.entities(), cad.near(), cad.distance(), etc.
    ├─ Writes: cad.place(), cad.addLine(), cad.remove(), etc.
    ├─ Visual: cad.screenshot()
    └─ Output: console.log(), return value
```

Document persists across script executions. Scripts are stateless.

### Delta tracking

Every mutation is recorded:

```json
{
  "added": [{ "id": "e_4336", "type": "block", ... }],
  "removed": ["e_42"],
  "modified": [{ "id": "e_100", "field": "end", "old": [5,0], "new": [6,0] }]
}
```

Feeds: WebTransport push to browsers, visual diff overlay (red=removed,
green=added), undo stack.

---

## Rust backing: what cadview-core needs

### Entity identity

```rust
struct EntityId(u64);

struct DrawEntity {
    id: EntityId,
    layer: String,
    color: Color,
    shape: Shape,
}
```

### Extended Shape enum

```rust
enum Shape {
    Line(Line),
    Arc(BezPath),
    Circle(Circle),
    Polyline { points: Vec<Point>, closed: bool },
    BlockInsert { name: String, position: Point, rotation: f64, scale: (f64, f64) },
    Text { content: String, position: Point, height: f64 },
}
```

### Block definitions

```rust
struct BlockDef {
    name: String,
    entities: Vec<(Shape, String, Color)>,  // shape, layer, color
    insert_point: Point,
    default_layer: String,
}
```

### Spatial index + topology

R-tree (rstar crate) over entity bounding boxes, plus endpoint index:

```rust
impl Document {
    fn query_near(&self, point: Point, radius: f64) -> Vec<(EntityId, f64)>;
    fn query_bounds(&self, rect: Rect, mode: BoundsMode) -> Vec<EntityId>;
    fn connected_to(&self, id: EntityId, tolerance: f64) -> Vec<EntityId>;
    fn children(&self, block_insert_id: EntityId) -> Vec<EntityId>;
}

enum BoundsMode { Intersect, Contain }
```

### Geometry helpers (pure functions, no mutation)

```rust
fn lerp(a: Point, b: Point, t: f64) -> Point;
fn along(a: Point, b: Point, distance: f64) -> Point;
fn direction(a: Point, b: Point) -> f64;   // degrees
fn normal(a: Point, b: Point) -> (f64, f64);  // unit vector
fn project_onto(pt: Point, seg_a: Point, seg_b: Point) -> Point;
fn distance_to_segment(pt: Point, seg_a: Point, seg_b: Point) -> f64;
fn point_in_polygon(pt: Point, polygon: &[Point]) -> bool;
fn centroid(polygon: &[Point]) -> Point;
fn bounds_of(points: &[Point]) -> Rect;
fn rotate_point(pt: Point, degrees: f64, center: Point) -> Point;
fn segments(polyline_id: EntityId) -> Vec<(Point, Point)>;
fn intersection(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point>;
```

### Mutations

```rust
impl Document {
    fn add_entity(&mut self, shape: Shape, layer: &str, color: Color) -> EntityId;
    fn remove_entity(&mut self, id: EntityId) -> Option<DrawEntity>;
    fn move_entity(&mut self, id: EntityId, dx: f64, dy: f64);
    fn add_layer(&mut self, name: &str, color: Color);
    fn place_block(&mut self, name: &str, pos: Point, rot: f64, scale: (f64, f64)) -> EntityId;
    fn define_block(&mut self, def: BlockDef);
}
```

### Serialization

```rust
impl Document {
    fn to_json(&self) -> String;
    fn delta_json(&self, since: u64) -> String;
}
```

---

## Feedback from exercises

Issues discovered by building the user-code exercises against the core
API. Items marked **resolved** have been folded back into the core
design above.

### Resolved

- **Geometric helpers were entity-ID-only.** User code constantly works
  with raw point pairs (wall edges from polyline vertices, schematic grid
  points). All helpers now accept both entity IDs and point pairs.
- **No point-to-segment distance.** Added `cad.distanceToSegment()`.
  Used in door-to-wall association, entity-on-wall detection.
- **No point-in-polygon.** Added `cad.pointInPolygon()`. Needed for
  L-shaped rooms, any non-rectangular region containment.
- **No direction angle.** Added `cad.direction()`. Was computing
  `Math.atan2() * 180 / Math.PI` repeatedly in user code.
- **No polyline decomposition.** Added `cad.segments()`. Extracting edge
  pairs from polyline vertices required index arithmetic with modulo.
- **No endpoint connectivity.** Added `cad.connectedTo()`. Room
  detection and net tracing both need "entities sharing an endpoint."
- **No block sub-entity access.** Added `cad.children()`. Door swing
  detection and pin discovery both search for sub-entities near a block
  insert point.
- **No pure point rotation.** Added `cad.rotatePoint()`. Pin computation
  in schematics needed rotation without mutating entities.
- **No centroid/boundsOf for point arrays.** Added both. Needed for
  every derived polygon (rooms, regions).

### Open questions

- **Block pin metadata**: Exercise B uses a `_PIN` layer convention.
  Works, but structured pin metadata (name, direction, electrical type)
  on block definitions would enable richer automation (auto-wiring,
  ERC checks). Add a `pins` field to `cad.defineBlock()` options?
- **Multi-page**: DWGs can have multiple model spaces / layouts. API
  assumes one. Need `cad.pages()` / `cad.setPage()`?
- **Coordinate system**: meters work for buildings. For schematics at
  2.5mm grid, the `gpt()` helper works but 0.0025 is unwieldy.
  Consider `cad.setSnap(0.0025)` at minimum. Full unit scale
  (`cad.setUnit("mm")`) is heavier, may not be worth it.
- **Performance**: `cad.entities()` returns a full copy. For 5000+
  entity drawings, a filtered-at-source variant
  (`cad.entities({ layer: "S_WALL" })`) could avoid copying. Or is
  `cad.inBounds()` + array filter sufficient?
- **Entity snapping**: `cad.place()` could support
  `{ snapTo: "e_42", snapMode: "endpoint" }`. The exercises compute
  snap points manually via `cad.startOf()`, `cad.endOf()`.
- **Room detection quality**: Exercise A's topology-based room detection
  is hard to get right (T-junctions, wall thickness, tolerance). A core
  `cad.findClosedRegions({ layer: "S_WALL" })` would help, but it's a
  complex algorithm. Keep user-side for now, promote if it stabilizes.
