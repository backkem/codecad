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
| `cad.sessions.list()` | Returns `[{id, entity_count, part_count, baked, js_target, visible}]`. |
| `cad.sessions.loadBake(id, bytes, adopt)` | Load a prebaked 3D scene (see "Solid parts in the browser"). |
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
| `cad.use(name)` | Load a script library once, get its exports (see "Script libraries") |
| `cad.render(opts)` | Render the document to a PNG on the server (see below) |
| `POST /api/run` | HTTP endpoint: `{program}` or `{exec}` field |

Scripts in the sandbox see all `cad.*` methods. `readFile` paths are
relative to the script's directory and sandboxed to the project root.
`console.log` output is captured in the response `stdout` field.

### Script libraries

Shared helpers live in a library: a script file in `lib/` next to the
script that uses it. A script loads it with one call:

```js
const draft = await cad.use("drafting");   // runs lib/drafting.js
draft.setup({ scale: 2 });
const { P, dim, text } = draft;
```

A library is a script like any other, so it is written as the body of an
async function and ends by returning what it offers:

```js
// lib/drafting.js
const geom = await cad.use("geom");        // a library loads another the same way

function dim(a, b, off) { /* ... */ }
function leader(tip, knee, label) { /* ... */ }

return { dim, leader };
```

That is all `cad.use(name)` does: read `lib/<name>.js`, run it, and hand
back what it returned. It is the old
`new Function(cad.readFile("lib/drafting.js"))()` with three additions:

- **Once per run.** Later calls for the same name get the same result.
- **Named in errors.** The file name and its own line numbers appear in
  stack traces (`boom@lib/late.js:2:9`).
- **A name, never a path.** A name is letters, digits, `-` and `_`. The
  host builds the path `lib/<name>.js` under the script's folder itself and
  reads it under the same rule as `cad.readFile`. A missing library fails
  with `cad.use: library "nope" not found (no file lib/nope.js)`.

Things to know:

- `cad.use` returns a promise, so `await` it. In the browser the text comes
  from the server (the `loadLibrary` RPC, read from the server's working
  directory); in the sandbox it is the `lib.load` host call. The same
  script and the same library run in both.
- A library runs in strict mode, so it cannot create a global by accident.
  It can still write to `globalThis` on purpose.
- Two libraries that wait for each other never finish loading. The sandbox
  ends such a run with "waiting for something that can never happen"; it
  does not hang.
- A library uses only the public `cad.*` API. Domain libraries are script
  code (see `architecture.md`, "Semantics live in code").

The loader is about twenty lines in `web/src/cad-api.ts`
(`createLibraryLoader`); the host side is `load_library` in
`crates/cadview-server/src/script.rs`.

#### Why not ES modules?

Libraries are scripts, not ES modules, because the sandbox engine cannot
load modules today. Checked by running code in it (codelib's guest:
StarlingMonkey, built with componentize-js):

| Probe | Result |
|---|---|
| `import` or `export` in a script | `SyntaxError`: codelib evaluates every program as the body of an async function, so it is never a module. |
| `await import("lib/m.js")` | The engine's module loader can only read files, and its file table is fixed when the engine snapshot is built. Mounting a folder at run time changes nothing: every import fails with "Bad file descriptor". |
| A hook to hand the engine module source | None: no `ShadowRealm`, `Compartment`, `ModuleSource` or `require`. |

So real modules need a change below codelib, in how the engine is built
and snapshotted. A first version of this rung kept `import`/`export`
syntax by rewriting it on the server; that took a hand-written JavaScript
tokenizer and was dropped for the form above. If the engine gains module
loading, a library changes its last line and the loader is swapped;
scripts keep calling `cad.use`.

### Render to PNG

```js
cad.render({
  out: "render.png",        // relative to the server's working directory
  size: [960, 640],         // pixels (times pixelRatio)
  bounds: { min, max },     // optional; default fits the whole drawing
  hiddenLayers: ["DIM"],    // optional
  backend: "auto",          // "auto" (GPU, else CPU) | "gpu" | "cpu"
  pixelRatio: 1,            // scales size and stroke width together
})
// -> { ok, path, width, height, backend: "gpu" | "cpu", bytes, ms }
```

Same picture as the browser's Vello view (decided by
`cadview_core::render`, drawn by `cadview-render`). In the browser,
`cad.render` flushes pending sync first, like `savePdf`. Unknown fields
are ignored; `spp` has no effect yet. With a `view` (or `mode: "3d"`, or a
document that has solid parts and no 2D entities) it draws the parts in 3D
instead: see "Solid parts", "Render". The same render is available from
the CLI:
`cadview-server drawing.dwg --render out.png --size 960x640
--render-backend auto`.

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

8. **Meaning lives in the script, mechanisms in the core.** The core
   does not know what a room, a net or a part number is. It answers
   generic geometric questions, on whole sets and in one call, and the
   script supplies the meaning. When a rule is too slow in script, move
   the geometric primitive it needs into the core, not the rule. See
   `docs/architecture.md`, "Semantics live in code".

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
//   attrs: { net: "GND" },  // only when the entity has attributes (A8)
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

Built in A8 on the same code as the set queries (see "Attributes and set
queries" below). Before A8 these two were declared in `cad-api.ts` but not
dispatched.

```js
cad.near([8.0, 5.0], 2.0)                     // within 2 of a point
// [{ ...entity, distance }, ...]               // nearest first
// distance is exact for lines, arcs, circles, polylines and fills; for
// text, splines and other shapes a bounding box stands in, marked approx: true

// Entities whose bounding box intersects the rectangle (partial overlap)
cad.inBounds({ min: [5, 3], max: [11, 7] })
// entity array

// Entities fully contained within the rectangle
cad.inBounds({ min: [5, 3], max: [11, 7] }, "contain")
// entity array

// Both modes:
//   "intersect" (default) - any overlap, even partial
//   "contain"             - entity bounds fully inside query rect
// Both take { select } to look at part of the drawing only:
cad.near([8, 5], 2, { select: { layer: "S_WALL" } })
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

## Attributes and set queries

Built in rung A8 (`docs/LADDER-API.md`). Two of the mechanisms named in
`docs/architecture.md`, "Semantics live in code": entities carry the
script's own facts, and the core answers questions about whole sets in one
call. Neither knows any domain. The two worked rules below (clearance
between nets, free space around play equipment) are script code over these
calls.

Coordinates are drawing units, like every `cad_call` (mm for DWG files).

### The attribute model

An attribute is a key and a value that a script attaches to an entity or a
block definition. The core stores it, keeps it through every path below,
matches it in selections and returns it. It never acts on its meaning.

| Choice | Decision | Why |
|---|---|---|
| Value types | A JSON scalar: string, finite number, boolean. | Strings only would make every rule parse numbers (a stock length, a free space, a clearance), and `"1.0"` would not equal `"1"`. Arrays and objects would make equality matching ill-defined, grow sync blobs, and have no form in DWG extended data. Structured data belongs in the script's own tables, keyed by an attribute (an id). |
| Null | Not a value. In an update it removes the key; in a selector it means "absent". | One meaning per position, no "present but null" state. |
| Keys | 1 to 64 ASCII letters, digits and `_ - . : /`. | Safe as DWG strings and JSON keys, readable in other CAD programs. Dots suggest namespaces (`pcb.net`) but the core gives them no meaning. |
| Limits | At most 64 keys per entity, a string value at most 1024 bytes, all keys and values of one entity at most 8192 bytes. | Keeps sync blobs small and DWG extended data far below its 16 KB limit per entity. A failing update changes nothing. |
| Where | `DrawEntity.attrs` (sorted map). `BlockDef.attrs` for the definition, `BlockDef.shape_attrs[i]` for its shape `i`. | A footprint has a part name, a pad has a pin number. The per-shape list is parallel to `shapes`, so the shape tuples used by the renderers did not change. |
| Layers | No attributes. | A layer is already a name the script picks; facts about a layer live in the script's table keyed by that name. Open if a domain needs more. |
| Block children | A child of an insert carries the definition's attributes, then the shape's, then the insert's; a later one wins on equal keys. | The usual CAD rule: the instance overrides the definition. A pad of `R1` then carries `ref: "R1"` and its `pin`. |

Every new call passes the question in `AGENTS.md`: would another domain need
the same thing? A key and value store, equality and presence matching, and
"which pairs are closer than d" serve a floor plan, a circuit board, a
cutting list and a site plan alike. Nothing is named after a trade.

### How attributes survive

| Path | Behaviour |
|---|---|
| Yrs sync | Attributes are part of the entity blob, so a change of attributes alone is a change and syncs. Block definitions are now diffed by content hash (before, only by name, so a redefinition did not sync either). |
| Undo, redo | Part of the snapshot. |
| move, rotate, mirror, trimTo, extendTo, fillet and chamfer on the inputs | Edit in place: attributes stay. |
| copy, array, offset, trim | The new entity gets the source's attributes. |
| fillet arc, chamfer line | A new entity joining two: it gets the attributes both have with equal values. |
| Block insert, `entities({expand: true})`, `children()` | Children carry definition, shape and insert attributes (see above) and a `child` index. |
| JSON | `cad.saveJson(path)` and `cad.loadJson(path)` keep everything (attributes, blocks, layers, linetypes, ids, ByLayer styles), numbers bit for bit. `cad.save(path)` stays a one-way export of the `entities()` view, now with `attrs`. |
| DWG | Entity attributes, including those of shapes inside block definitions, are written as extended data under the registered application `CODECAD` and read back. The attributes of a block definition itself have no place in this export: `saveDwg` reports them in `attrs.droppedBlocks`, and entities with no DWG form in `attrs.droppedEntities`. JSON is the lossless format. |
| PDF, PNG | Ignored. |
| Old data | Documents without the field load with no attributes: sync blobs written before A8 decode through a fallback, JSON files through serde defaults, and DWG files simply have no `CODECAD` extended data. |

DWG layout of the extended data: a 1071 format version (1), then per
attribute a 1000 string (the key) and its value: a 1000 string for ASCII
text up to 255 bytes, a 1002 `{` ... `}` group of 1004 binary chunks
(UTF-8) for any other text, a 1040 real for a number, a 1070 integer (0
or 1) for a boolean. Written in the R2000 layout that `saveDwg` uses.

### Setting and reading attributes

```js
// At creation: every creation call takes attrs (lines, arcs, circles,
// polylines, splines, text, fills, hatches, place)
cad.addPolyline([[0, 0], [10, 0]], { layer: "CU", attrs: { net: "GND", width: 0.25 } })

// Later, on any target (ids, entity objects or a selector). Merge; null
// removes a key; replace: true starts from nothing. All or nothing.
cad.setAttrs({ layer: "CU", has: ["net"] }, { checked: true })   // -> { updated: [...] }
cad.removeAttrs("e_12", ["checked"])
cad.attrs("e_12")                                                // -> { net: "GND", width: 0.25 }

// Block definitions: the definition and each shape
cad.defineBlock("R0603", [
  { type: "circle", center: [-0.8, 0], radius: 0.35, layer: "CU", attrs: { pin: "1" } },
  { type: "circle", center: [0.8, 0], radius: 0.35, layer: "CU", attrs: { pin: "2" } },
], { attrs: { footprint: "R0603" } })
cad.place("R0603", { at: [20, 0], attrs: { ref: "R1", "net.1": "GND", "net.2": "VCC" } })
cad.setBlockAttrs("R0603", { rev: "B" })                  // the definition
cad.setBlockAttrs("R0603", { plated: true }, { shape: 0 }) // shape 0
cad.blockAttrs("R0603")      // -> { attrs: {...}, shapes: [{ pin: "1", plated: true }, { pin: "2" }] }

// Entities report them (absent when empty); describe() counts the keys
cad.entity("e_12").attrs     // { net: "GND", width: 0.25 }
cad.describe().attrs         // { checked: 5, net: 5, ... }
```

### Selections

Every call that takes a target or a set takes the same selection: an id, a
list of ids or entity objects, or a selector object whose conditions must
all hold.

```js
{
  ids: ["e_1", "e_2"],          // only these
  layer: "CU",                  // or ["CU", "SILK"]
  type: "line",                 // or a list; the types entities() reports
  attrs: { net: "GND", via: null },  // equal values (typed); null = key absent
  has: ["net"],                 // keys present, any value
  bounds: { min: [0, 0], max: [50, 30] },  // bounding box overlaps
  expand: true,                 // block inserts as their children, each matched
                                // on its own layer and attributes
}
```

`entities(selector)`, every mutation target (`move`, `copy`, `remove`,
`rotate`, `mirror`, `array`, `setAttrs`, ...) and every set query use it.
`expand` is for queries only; a mutation needs whole entities. An unknown
key is an error, so a typo does not select everything.

### Set queries

Whole sets in one call, with exact geometry: lines, arcs, circles,
polylines with bulges and fill boundaries are used as lines and arcs, never
sampled. Shapes with no exact distance here (splines, ellipses, text, curve
paths) are left out and listed in `skipped` with their id and type, so a
rule never passes silently on geometry it did not see. A static
bounding-volume hierarchy over the line and arc pieces keeps the calls near
n log n.

Every call returns `{ pairs, skipped }` (and `truncated: true` past
`limit`, default 100 000 pairs). A pair names an entity of `a` and one of
`b`, and for a block insert the child (`aChild`, `bChild`): the expanded
child, or for an unexpanded insert the child where the contact is. With `b`
null or left out, pairs are taken within `a`, each once. An entity in both
sets is never paired with itself.

Options shared by the pair queries:

- `differ: ["net"]`: leave out pairs whose values for all these keys are
  equal (a missing key counts as a value). The core compares; it does not
  know what a net is.
- `attrs: true`: add `aAttrs` and `bAttrs` (effective attributes) to each
  pair, so a rule needs no second lookup.
- `limit`: the most pairs returned.

```js
// Every pair closer than a distance (strictly), with the closest points
cad.pairsWithin({ layer: "CU", has: ["net"] }, null, 0.2, { differ: ["net"] })
// { pairs: [{ a, b, distance, pa: [x, y], pb: [x, y] }, ...], skipped: [] }

// Every pair whose outlines meet: crossing or touching points, shared
// stretches (collinear lines, arcs of one circle). areas: closed shapes are
// filled, so one inside another is a pair too, with contains: "a" | "b"
cad.overlapping({ layer: "PLAY" }, null, { areas: true })
// { pairs: [{ a, b, points: [[x, y]], shared: false, contains? }], skipped }

// Which entities lie inside a region: a rectangle, a ring of points, or
// each closed entity of a selection (circle, closed polyline, fill; holes
// count). The region is the union of an entity's closed shapes; open curves
// add nothing and are listed in skipped. Touching the boundary from inside
// counts, unless strict: true.
cad.inside({ layer: "PLAY" }, { layer: "SITE" })
// { pairs: [{ a, b, touches }], skipped }   b is null for a rectangle or ring

// The k nearest entities of b for each entity of a, optionally within a distance
cad.nearest({ attrs: { kind: "socket" } }, { layer: "WALL" }, { k: 1, within: 500 })
// { pairs: [{ a, b, distance, pa, pb }], skipped }
```

Exactness. The distance between two pieces is exact up to floating point:
the minimum is an intersection, an endpoint of one piece against the other,
or an interior point of both where the connecting line is normal to both
(for a line and an arc, on the radius normal to the line; for two arcs, on
the line through both centres). Tests compare it with dense sampling and
the indexed pairs with the naive loop. Because the test is strict, two
tracks drawn exactly 0.2 apart may come out at 0.19999999999999998: pass
`0.2 - 1e-9` when the boundary itself is allowed.

Speed (release build, one machine): 10 000 random lines and arcs, all pairs
closer than 0.2: `pairsWithin` 19 ms through `cad_call` (1 912 pairs),
the naive loop over all 50 million pairs with the same exact distance
12.5 s. The test is `timing_pairs_within_10000_segments` in
`crates/cadview-core/tests/attrs_setquery.rs`.

What the list is, and why. `near` and `inBounds` are the one-point and
one-rectangle versions and run on the same code. The distance between two
given entities is `pairsWithin(id1, id2, 1e12)` (a finite bound; JSON has no Infinity). Intersection points
come with `overlapping`, so there is no separate set intersection call.
Union and offset of a selection (copper pours, keep-out zones) are the open
part: `region.rs` and rung A2.

### Worked rules

Both are script code in `crates/cadview-server/tests/a8/`, run by the
server tests through the same entry `/api/run` uses, before and after a JSON
and a DWG round trip.

A clearance rule between nets (`pcb-clearance.js`). Tracks carry `net`; a
pad takes its net from its footprint's insert (`net.1`, `net.2`), which is
the rule's own convention:

```js
const CLEARANCE = 0.2;
const netOf = (a) => a.net ?? a[`net.${a.pin}`] ?? null;
const copper = { layer: "CU", expand: true };
const foreign = (p) => netOf(p.aAttrs) !== netOf(p.bAttrs);
const tooClose = cad.pairsWithin(copper, null, CLEARANCE, { attrs: true }).pairs.filter(foreign);
const shorts = cad.overlapping(copper, null, { attrs: true }).pairs.filter(foreign);
```

A free-space rule around equipment footprints (`play-free-space.js`):
every item inside the site, no two overlapping, each keeping the free space
its `free` attribute asks for:

```js
const items = { layer: "PLAY", has: ["item"] };
const inSite = new Set(cad.inside(items, { layer: "SITE" }).pairs.map((p) => p.a));
const overlaps = cad.overlapping(items, null, { areas: true }).pairs;
const most = Math.max(...cad.entities(items).map((e) => e.attrs.free));
const tight = cad.pairsWithin(items, null, most, { attrs: true }).pairs
  .filter((p) => p.distance < Math.max(p.aAttrs.free, p.bAttrs.free));
```

---

## Solid parts

Built in rung 3D1a (`docs/LADDER-3D.md`). A script builds exact solid parts,
the document stores and syncs them, and a script asks about them and renders
them. The core speaks a general shape language (`docs/3d-design.md`, section
4.0): no part kinds, no materials with meaning, no grain, no joinery words.
A domain library (woodworking, sheet metal, enclosures) builds its own
vocabulary from these calls and keeps its meaning in attributes.

```js
const side  = cad.extrude({ min: [0, 0], max: [300, 1782] },
                          { plane: "yz", distance: 18, attrs: { part: "side" } });
const shelf = cad.extrude({ min: [12, 6], max: [788, 300] }, { at: [0, 0, 400], distance: 18 });
cad.cut(side, { extrude: { min: [6, 400], max: [300, 418] }, plane: "yz", at: [12, 0, 0], distance: 6 });
cad.hole(shelf, { at: [50, 40, 418], dir: [0, 0, -1], dia: 8, depth: 12 });
const right = cad.copy3d(side, { rotate: { angle: 180, about: [400, 150, 0] } });
cad.parts({ attrs: { part: "side" } });   // ids, attrs, placement, bbox, volume, centroid
cad.render({ view: "iso", out: "shelf.png" });
```

Units are drawing units (mm), angles degrees, and Z is up. The 2D drawing
plane is world XY at z = 0.

### The part model

| Choice | Decision | Why |
|---|---|---|
| What a part is | A **definition** (`PartDef`: an id, a name and an ordered feature list in its own frame) placed by a **part** (a rigid placement and attributes). Features: extrude, revolve, sweep, loft, hole, boolean with a tool, fillet, chamfer. | Shape only. What the shape is for lives in the script. |
| Ids | Parts take numbers from the document's one id counter, the one entities use, and print as `p_12`; definitions print as `d_3`; entities stay `e_7`. | One counter: a number is never both. Separate prefixes: a part id passed where an entity is expected fails instead of picking something else. |
| Definitions and instances | `copy3d` places the same definition again, with no re-evaluation; parts differ in placement and attributes. A modifying call changes the part's **definition**, so every part that shares it changes too, like editing a block definition. `copy3d(p, { unique: true })` gives copies with definitions of their own. | Repeated parts (shelves, pins, legs) are one shape. Editing it once is the common case; a variant is asked for. |
| Coordinates in modifying calls | World coordinates, moved into the part's own frame before they are stored. `frame: "local"` gives them in that frame directly. | A script cuts where it sees the part. Because definitions are shared, a cut through one copy appears at the same place on every copy (in each one's frame). |
| A part as a tool | `cut(a, { part: b })` stores b's definition and b's placement relative to a at the time of the call. Moving b later does not move the cut; editing b's definition does change it. A definition used as a tool stays while any part uses it, directly or as a tool. A part can never reach itself through its tools. | The definition stays self-contained in its own frame, so instances keep sharing it. A live link to b's placement would give each instance a different cut. |
| Profiles | Exact lines and arcs (`Profile2d`), never flattened: vertex lists with bulges (`[x, y, bulge]`), `{center, radius}`, `{min, max}`, a closed 2D entity (closed polyline, circle, full ellipse, closed spline) by id or object, or `{outer, holes}` of any of these (the shape `regionClip` returns; its rings are already polygons). | "Draw the outline in 2D, then extrude it" with no loss. |
| Sketch planes | `"xy"` (u along X, normal +Z), `"xz"` (u along X, v along Z, normal -Y, so right-handed), `"yz"` (u along Y, v along Z, normal +X), or `{origin, x, normal}`; `at` moves the origin. | Z up; the three principal planes cover most parts, a general plane the rest. |
| Attributes | The same `attrs` mechanism as entities: same value types, keys and limits, never interpreted. | One way for a script to mark its facts. |
| Selection | The entity selector's shape: an id, a list, or `{ids, def, attrs, has, bounds}`. `bounds` is a box (`min`, `max` with 3 coordinates) or a plan rectangle (2 coordinates, every height) and needs a kernel. Parts have no layer or type. | A script filters parts the way it filters entities. Layers are 2D grouping; a part's grouping is an attribute. |

### Where the geometry comes from

cadview-core links no kernel. A shell that has one hands each document an
evaluator (`Document::solids`, a `SolidEnv`): the server gives every
document the arris adapter from `crates/cadview-solid`. It is shell state:
never synced, serialized or undone.

- **With a kernel** (server scripts, `/api/run`, the CLI) every new or
  changed definition is evaluated before it is stored, and so is every
  definition that uses it as a tool. A refusal fails the call and leaves the
  document as it was:
  `cut: part p_2 ("base"), feature 7 (extrude): a tapered extrude is not supported by this kernel yet`.
  The message names the call, the part (or `definition d_4` for a new part
  that was refused), the feature number and kind, and the reason. A call
  whose result the kernel builds but cannot measure (its box or volume)
  fails the same way and changes nothing, since the part could be neither
  queried nor rendered.
- **Without a kernel** (the browser today) part calls check the input's
  shape (closed profiles, sane numbers, known ids) and store and sync it.
  Geometry fails with `this build has no solid kernel: ...`: `parts()` and
  `part()` with geometry (default), `bounds` in a selector, and 3D renders.
  `parts({ geometry: false })` lists parts in every build.
- **The evaluation cache** lives in the `SolidEnv`, so in the shell, next to
  the evaluator. It is keyed by content: the bytes of a definition's features
  plus those of every tool definition it reaches. The same features evaluate
  once, whichever part, copy or document holds them; an edit changes the key,
  so a stale result is never used; undo usually finds the old key still
  cached. Meshes are cached per entry and chord. The server shares one cache
  between its documents. The least recently used entries go past 4096.
- **A change continues from what was evaluated before** (rung 3D1b). A
  definition that misses the cache is continued from its longest cached
  prefix of features (that prefix's key is the key of those features and
  the tools they reach), through the kernel's `evaluate_from`. A modifying
  call adds one feature, so it costs that feature's operation, not the
  whole list again. The arris adapter continues on a copy of the prefix's
  model, which holds exactly what a from-scratch run holds at that point,
  so the result is the same B-rep: tests compare volume, centroid, box and
  mesh bit for bit on every prefix of the harness catalogue and on the
  examples. Each call still checks the whole result (the kernel's full
  validity check) and integrates its volume, which grow with the part.

### How parts survive

| Path | Behaviour |
|---|---|
| Yrs sync | Two maps of their own: `partdefs` (one bincode `PartDef` per `d_<n>`) and `parts` (one `Part` per `p_<n>`), diffed by content hash like entities. Concurrent edits to one definition are last-writer-wins on that definition. Meshes and B-reps are never synced; each peer derives them. |
| Undo, redo | Part of the snapshot (`cad.checkpoint()` before, as for 2D). |
| `clear` | Removes parts and definitions too. |
| JSON | `saveJson` / `loadJson` keep definitions and parts exactly (numbers bit for bit). Files from before parts load with none. |
| DWG | `saveDwg` writes the 2D content only and says so: `parts: {dropped, defs, note}` in its result. The 2D content of the DWG is the same with or without parts. |
| PDF, 2D PNG | Ignore parts. |

### Calls

Creating calls take `attrs` and `name` and return the new part. Modifying
calls take one part (an id, a part object, or a selector that picks exactly
one), add a feature to its definition and return the part. With a kernel,
the returned part carries its geometry.

```js
// Create
cad.extrude(profile, { plane, at, distance, taper, attrs, name })
cad.extrude([[0, 0], [100, 0], [100, 40, 1], [0, 40]], { distance: 6 })   // a bulge: an exact arc
cad.revolve(profile, { plane, at, axis: [[u0, v0], [u1, v1]], angle: 360, attrs, name })
cad.revolve({ min: [10, 0], max: [20, 50] }, { plane: "xz", axis: [[0, 0], [0, 1]] })  // a tube
cad.sweep(profile, path, { plane, at, attrs, name })   // path: [{to}, {via, to}, ...] from the plane origin
cad.sweep({ center: [0, 0], radius: 6 }, [{ to: [0, 0, 100] }, { via: [50, 0, 150], to: [100, 0, 100] }])
cad.loft(sections, { ruled, attrs, name })             // sections: [{profile, plane, at}, ...]
cad.loft([{ profile: { min: [-20, -20], max: [20, 20] } },
          { profile: { min: [-12, -12], max: [12, 12] }, at: [0, 0, 300] }], { ruled: true })

// Modify: a tool is {extrude|revolve|sweep|loft: ..., <its options>} or {part}
cad.cut(part, { extrude: { min: [0, 20], max: [100, 30] }, at: [0, 0, 12], distance: 6 })
cad.union(part, { extrude: lip, at: [0, 0, 110], distance: 5 })
cad.intersect(part, { part: other })
cad.hole(part, { at: [20, 10, 18], dir: [0, 0, -1], dia: 8, depth: 12 })   // depth: mm or "through" (default)
cad.hole(part, { at: [60, 10, 18], dia: 5, cbore: [9, 3] })               // counterbore [dia, depth]
cad.fillet3d(part, { edges: { cap: "end" }, radius: 3, fallback: "error" })
cad.chamfer3d(part, { edges: { lateral: [0] }, d: 1 })

// Place
cad.move3d(target, [dx, dy, dz])
cad.rotate3d(target, { angle: 90, axis: [0, 0, 1], about: [0, 0, 0] })
cad.copy3d(part, { move: [0, 0, 300], count: 4 })        // a row of four, sharing the definition
cad.copy3d(part, { rotate: { angle: 90, about: [0, 0, 0] }, count: 3, unique: true })
cad.mirror3d(target, { point: [400, 0, 0], normal: [1, 0, 0] })   // in place -> {mirrored, defsMade, defsRemoved}
cad.mirror3d(part, { normal: [0, 1, 0], copy: true, attrs: { hand: "left" } })  // the image, as a new part
cad.remove3d(target)                                     // -> {removed, defsRemoved}

// Attributes
cad.setPartAttrs({ has: ["part"] }, { checked: true })
cad.removePartAttrs("p_12", ["checked"])

// Query
cad.parts({ attrs: { part: "shelf" } })
// [{ id: "p_12", def: "d_3", name, placement: { translation, rotation: [w, x, y, z] },
//    features: [{ id: 1, kind: "extrude" }, ...], instances: 2, attrs,
//    bbox: { min, max }, localBbox: { min, max }, bboxTol, volume, centroid }]
cad.parts({ geometry: false })     // without box, volume, centroid: works in every build
cad.part("p_12")
cad.partDef("d_3")                 // the whole feature tree, the parts using it, its tools
cad.describe()                     // ... parts, partDefs, partAttrs, solidKernel
```

Edge selectors name edges by meaning, never by kernel topology
(`docs/3d-design.md`, 4.3): `"all"`, `{cap: "start" | "end" | "both",
feature?, segs?}` (the edges of an extrude's caps, by profile segment
index), `{lateral: [segs], feature?}` (edges swept from profile vertices),
or `{near: [[x, y, z], ...]}` (the edge nearest each point, in world
coordinates). `feature` defaults to the definition's first feature; feature
ids are listed in `parts()`.

The `near` rule: a seam is never picked. A seam is where a closed surface
(a cylinder, cone, sphere, torus, a revolve's surface) meets itself: both
sides of it are one face, it is not an edge of the shape, and nothing can
round or bevel it. Of the other edges the nearest wins. Among edges equally
near (to 1e-6 mm), one the point is nearest to inside its length beats one
it is nearest to at an end vertex; then the first in the kernel's order.
So a point on a revolve's sketch at +x, where the seam meets a circular
edge, picks the circle, and a point at a corner prefers an edge it lies
along. Put the point on the edge you mean, away from its ends.

`bbox` is the tight box of the exact solid in the world; `localBbox` the
same in the part's own frame (what a cut list wants). `bboxTol` says how
tight: 0 when both are exact (each side touches the solid). Where two
cylinders meet (a cross bore, a tee, a round boss in a curved wall) the
edge is a NURBS curve the kernel fitted to the intersection, and no exact
box of it exists here; its extremes are then bracketed and `bboxTol` is
1e-9: the box still contains the solid and each side lies at most that far
outside it. Why that is guaranteed: a NURBS curve's weights are positive,
so the curve over any parameter range lies in the box of the control
points of its segment over that range (knot insertion); a piece whose
control box reaches more than the tolerance past the curve points found so
far is halved and its midpoint added, until none does; boxes are padded
for the rounding of knot insertion. The box is tight to the edge curves the
kernel holds; those follow the true intersection to the kernel's linear
tolerance (1e-7 mm). NURBS faces (a smooth loft) still have no box.
`volume` is integrated on the exact faces (no mesh); `centroid` is in the
world, or null where a kernel computes no first moments.

What the kernel (arris 0.11.1) does not do yet, and refuses: tapered
extrudes, asymmetric chamfers, spline profile segments, elliptic segments
in a revolve, and fillets whose corners it has no closed form for. Each is
a refusal with the message above.

`mirror3d` makes mirror images. A reflection is not a rigid motion (it
turns a left hand into a right one), so it cannot be a placement: a
mirrored part places a **mirrored definition**. Every definition is
mirrored the same way, across its own frame's YZ plane, so the mirrored
definition depends only on the original: a part placed by `P` and mirrored
across a world plane `M` places the mirrored definition by `M P S` (`S` the
YZ reflection), which is rigid. Parts that share a definition mirror to
parts that share one mirrored definition, made once (its tools are
mirrored too). The image is a copy, not a live link: later edits of the
original do not reach it. A definition whose features equal the mirrored
ones is reused, so mirroring back finds the original. Feature ids and edge
selectors carry over; a part with a live part link (`ToolRef::Part`) is
refused.

### Checks between parts

Built in rung 3D1b. The questions a rule asks about an assembly (does
anything overlap, is this gap wide enough, is this inside that, how far
apart are these two) are set queries over parts, the 3D twins of the 2D
set queries above. One call answers for whole selections, picked with the
part selector, and returns every pair at once:

```js
// Pairs that share volume (interference), each once; b null: within a
cad.overlapping3d(null)
// { pairs: [{ a, b, volume, depth, aInB, bInA }], skipped: [] }
cad.overlapping3d({ attrs: { part: "shelf" } }, { attrs: { part: "side" } },
                  { tolerance: 1e-6, touching: false, attrs: true, differ: ["group"] })

// Pairs strictly closer than a distance (clearance), with closest points
cad.pairsWithin3d({ has: ["member"] }, null, 50)
// { pairs: [{ a, b, distance, pa: [x, y, z], pb: [x, y, z] }], skipped: [] }

// Parts inside a box (touching its sides counts unless strict), or inside
// the solid of each part of a selection
cad.inside3d({ has: ["component"] }, { min: [0, 0, 0], max: [100, 60, 23] })
// { pairs: [{ a, b: null, touches }], skipped: [] }
cad.inside3d(["p_31"], { attrs: { part: "case" } })   // { pairs: [{ a, b }] }

// The distance between two given parts (measure)
cad.distance3d("p_4", "p_9")    // { a, b, distance, pa, pb }
```

Names. They are the 2D names (`overlapping`, `pairsWithin`, `inside`) with
the `3d` every part call carries where a 2D call of the same name exists
(`move3d`, `rotate3d`, `copy3d`, `mirror3d`). A script that knows the 2D
rules knows these, and the options mean the same: `differ`, `attrs`,
`limit`, `strict`. The ladder's working names `interference` and
`measure3d` were dropped: one names a use (a domain word), the other a
second name for what 2D calls `distance`. `distance3d` is the 2D
`distance` for two parts.

Semantics. As in 2D: with `b` null the pairs are taken within `a`, each
once; a part in both sets is never paired with itself; pairs come in
order of `a`'s, then `b`'s position. `overlapping3d` lists pairs that share
volume. Touching (a shelf in its dado, a board on a joist) is not overlap;
`touching: true` lists those too, with `volume: 0`. Each overlap has a
`depth`: its volume over the largest face of the box both parts' boxes
share, which is the penetration depth for box-shaped overlaps (2 for a
shelf 2 mm too long) and never more than that box's smallest side. An
overlap whose depth is at most `tolerance` (default 1e-6 mm) counts as a
touch: rounding in a rotated placement leaves slivers that thin. `pairsWithin3d` counts
touching and overlapping parts as distance 0. A pair the kernel cannot
answer goes to `skipped` with the reason, so a rule never passes silently
on a pair it did not see.

Exactness. The answers come from the exact solids, never from meshes.
Distances and closest points are arris's `query::distance`: the critical
points of the distance on each pair of face and edge carriers, solved in
closed form for planes, cylinders, cones, spheres and tori, and recursing
onto the boundaries; exact to rounding. Pairs it has no formula for (a
cone against a cone off their common axis, anything against a NURBS
face's interior) are refused, so skipped. The shared volume is the
kernel's `common` of the two solids, measured by the flux integral on its
exact faces. Where arris refuses that common (two faces in one plane can
leave a contact it cannot read, as a lengthened shelf against its side
does), the volume is `V(a) - V(a - b)` (or the same the other way round):
the cut that made the joint, measured the same way; only if both fail is
the pair skipped. Where an edge is a fitted intersection curve (two cylinders
meeting) the answer is to the kernel's linear tolerance (1e-7 mm).
`inside3d` with a box compares tight boxes, which is exact: a solid lies in
a box exactly when its tight box does (to `bboxTol`).

Speed. A broad phase on the parts' world boxes (sweep and prune along x)
picks candidate pairs; only those reach the kernel, and each solid is
copied into the kernel's scratch space once per call. For overlap, only
boxes whose interiors meet are candidates, so touching parts cost a box
test. On a 10 x 10 grid of blocks with three overlaps (release build, one
machine): `overlapping3d` 26 ms, `pairsWithin3d` over 182 pairs 700 ms,
`inside3d` with a box 0.5 ms (`a_hundred_parts_stay_fast` in
`crates/cadview-solid/tests/checks.rs`).

A rule on top, script code over these calls (examples/3d/11):

```js
const clash = cad.overlapping3d(null, undefined, { attrs: true });
run.ok("no two parts overlap", clash.pairs.length === 0 && clash.skipped.length === 0);
const rests = cad.pairsWithin3d(beam, { has: ["member"] }, 1e-6, { attrs: true });
// the beam must rest on the head rail and both legs
```

### Render

`cad.render` draws the parts when it gets a `view` (or `mode: "3d"`, or the
document has parts and no 2D entities). It takes the 2D options (`out`,
`size`, `pixelRatio`, `backend`: "auto" or "gpu") and these:

```js
cad.render({
  out: "renders/assembly.png", size: [960, 640],
  view: "iso",                 // iso (south-east) | iso-sw | iso-ne | iso-nw | front | back | left | right | top | bottom
  projection: "orthographic",  // or "perspective"
  select: { attrs: { part: "shelf" } },        // which parts (default all)
  colorBy: "part",             // "part" | "def" | { attr: "key" } ("#rrggbb" values are colours)
  edges: true, occlusion: true,                // occlusion: true | false | strength 0..1
  ground: true,                // or { height, color }: a contact shadow
  background: "light",         // "light" | "viewer" | [r, g, b] | "#rrggbb"
  clip: { point: [200, 150, 0], normal: [0, -1, 0] },   // cutaway, up to 4 planes
  clipMode: "any",             // remove what is beyond any plane; "all": beyond every plane (two planes: a quarter cutaway)
  explode: 0.8,                // or { factor, axes: "xyz" }
  chord: 0.25,                 // mm; default a quarter pixel, as a power of two
})
// -> { ok, path, width, height, backend: "gpu", bytes, ms, mode: "3d", parts, triangles, chord, kernel }
```

The scene is decided in core (`cadview_core::solid::scene`): which parts,
colours, the tolerance, view, cues, cuts. The colour attribute is a render
option, not part data. Colours follow the document's part order, so a part
keeps its colour in a view that `select`s only some parts. The server draws it with `cadview-render3d` on the
GPU device the 2D renderer uses; there is no CPU path for 3D yet.

`quality: "presentation"` path traces the same scene with the same
options: light from an environment, soft shadows on the ground, bounce
light, physical materials. It draws no edge lines or occlusion.

```js
const mat = await cad.use("materials");      // examples/3d/lib: oak, steel, glass, ...
cad.render({
  view: "iso", ground: true, out: "renders/shelf.png",
  quality: "presentation", spp: 1024, seed: 0,  // same scene, spp and seed: same bytes
  materials: mat.option(),     // { attr: "material", map: { oak: { color, roughness, pattern }, ... }, default? }
  environment: "studio",       // "studio" | "sky" | { preset, zenith, horizon, ground, intensity, sweep, sun: { direction, frame, angle, irradiance, color } | false }
  exposure: 0,                 // stops
  denoise: true,               // edge-aware denoiser (default); false: the raw estimate
  portals: [{ origin: [0, 3300, 900], u: [0, 1600, 0], v: [0, 0, 1200] }],  // openings the sky lights an interior through
})
// -> { ..., quality: "presentation", samples, denoised, trace: | "sky" | { preset, zenith, horizon, ground, intensity, sun: { direction, frame, angle, irradiance, color } | false }
  exposure: 0,                 // stops
})
// -> { ..., quality: "presentation", samples, trace: { passes, triangles, bvhMs, uploadMs, traceMs, pngMs, ... } }
```

`materials` works like `colorBy`: the values of one attribute map to
numbers (`color`, `metallic`, `roughness`, `transmission`, `ior`,
`emission`, `clearcoat`, `clearcoatRoughness`, and a `pattern`: `bands`
for wood grain, `brushed` for streaks along an axis or, `circular`, about
it; `axis` is "long", "short", "symmetry" or a vector in the part's
frame). The core knows no material names. The working render uses only
`color`, `metallic` and `roughness` from it.

A placed camera replaces the named view in both qualities:
`camera: { eye: [x, y, z], target: [x, y, z], fov: 30 }` (mm, vertical
field of view in degrees) is perspective; `height: 800` instead (the
view's height in mm) makes it orthographic; `up` is the world direction
shown as up (default [0, 0, 1]). It is used as given unless `fit: true`
(named views fit unless `fit: false`).

The ground takes a material like a part (`ground: { material: { color,
roughness, pattern } }`; the working render draws its colour). The
`mottle` pattern (`{kind: "mottle", scale, strength, color}`) adds soft
patches about `scale` mm across, so an even surface (a lawn, a floor) is
not flat; `materials.js` has `lawn`. `haze` (mm, default 150 m for "sky")
blends the ground seen far away into the horizon colour, so the sky meets
it without a hard line.

`sweep` (on for "studio") blends the backdrop seen
just above the horizon up from the lit ground into the dome, so a level
view shows no line where the floor meets it. `portals` are rectangles (a
corner and two perpendicular edges): the tracer samples the dome through
them directly, which an interior lit through windows needs to converge;
the result is the same. Shadow rays from the sun pass through glass
(attenuated, not bent).

### Prebaked scenes: `cad.bake3d`

A server script writes what a viewer without a kernel needs: the parts,
their meshes, exact boxes and volumes, and the render options to start
from (design section 14.3).

```js
const mat = await cad.use("materials");
cad.bake3d({ out: "examples/3d/03-bookshelf.bake3d",
             view: "iso", ground: true, materials: mat.option() })
// -> { ok, path, bytes, parts, defs, solids, triangles, chord, coarser, kernel, format, ms }
```

It takes the 3D `cad.render` options (not `size`, `pixelRatio`,
`backend`, `mode`); `chord` defaults to a quarter pixel at 1280 px.
`coarser` counts solids the kernel would only mesh at a coarser chord.
`GET /api/bake3d?doc=<key>[&options=<json>]` (bearer token) returns the
same bytes.

### Solid parts in the browser

The page shows parts in both render modes and, once it has loaded the
kernel module, evaluates them itself with the server's core code.

```js
await cad.loadKernel()          // once: fetches the kernel module (~1.5 MB gzipped)
const p = cad.extrude({ min: [0, 0], max: [100, 50] }, { distance: 18 })
cad.parts()                     // the server's numbers
cad.kernel.status()             // { state: "none" | "not-loaded" | "loaded", kernel, evaluations }
```

Before `cad.loadKernel()`, a call that needs geometry throws "the solid
kernel is not loaded in this page yet: run `await cad.loadKernel()`
once, then call again", and starts the download, so the call works when
tried again. A build without the module (`just build-kernel` not run)
says so from `cad.loadKernel()`.

The 3D view, for a page's own canvas (the app's 2D/3D switch uses it):

| Method | Description |
|--------|-------------|
| `cad.view3d.start(canvas, sessionId, opts)` | Show a session's parts. `opts`: 3D `cad.render` options plus `quality` ("working" \| "presentation") and `spp`. Returns a key. WebGPU only. |
| `cad.view3d.command(key, cmd)` | `{quality, view, projection, fit: true, orbit: [yaw, pitch], spp, traceScale: 1 \| 2, denoise, options}` |
| `cad.view3d.stats(key)` | Frames drawn, samples and target, timings, camera, errors. |
| `cad.view3d.stop(key)` | Stop it. |
| `cad.view3d.devices()` | GPU devices the page created (one is expected). |
| `cad.sessions.loadBake(id, bytes, adopt)` | Load a `.bake3d`; `adopt`: the session takes its parts. |
| `cad.api.fetchBake(docId, opts?)` | The server's bake of a document (`GET /api/bake3d`). |

Where the 3D view gets geometry, in order: the page's kernel when loaded;
a bake the session holds; in the app with a server, the server's bake of
the document (fetched again when its parts change). Presentation mode
draws the working render while the camera moves, then accumulates samples
up to its target and stops; on a slow GPU it traces at half resolution
and sets a target it reaches in about 30 s. Every image it shows goes
through the tracer's denoiser, as `cad.render` does (`denoise: false` in
the options or a command shows the raw estimate).

The embed viewer shows a bake with a Working/Presentation switch, and
never downloads the kernel:

```html
<script type="module" src="embed/codecad-viewer.js"></script>
<codecad-viewer scene="examples/3d/03-bookshelf.bake3d" style="width:960px;height:640px"></codecad-viewer>
```

The app opens one with `?scene=<url>` or a dropped `.bake3d` file.

### Projected views

Built in rung 3D3a. `cad.project` turns the parts into ordinary 2D
drawing entities: one view of them, with visible edges, hidden edges and
silhouettes, as exact lines, arcs, circles and ellipses that the 2D
renderer, DWG and PDF already understand.

```js
const front = cad.project({ view: "front", at: [120, 185], scale: 1 });
// -> { block: "view-front", insert: "e_41", view, from, right, up, origin, at, scale,
//      parts, entities, counts: { line, arc, circle, ellipse, spline, visible, hidden },
//      bounds: { min, max }, open: [], stats, oracle, ms, toSheet(p3) }
cad.project({ from: [1, -2, 1], up: [0, 0, 1], parts: { attrs: { part: "side" } },
              at: [300, 80], scale: 0.5, hidden: false, name: "side detail" });
front.toSheet([0, 0, 50]);                           // a 3D point on the sheet
cad.viewPoints({ view: "front", at: [120, 185], points: [[0, 0, 50]] });  // the same, by options
cad.entities({ expand: true, attrs: { partId: "p_7", edge: "hidden" } }); // its hidden lines
```

| Option | Default | Meaning |
|---|---|---|
| `view` | | A name, as `cad.render` names views: `front`, `back`, `left`, `right`, `top`, `bottom`, `iso` (also `isometric`, `iso-se`), `iso-sw`, `iso-ne`, `iso-nw`. Or give `from`. |
| `from`, `up` | up: world Z (Y when looking along Z) | A general direction toward the eye, and which way is up on the sheet. |
| `parts` | all | The part selector. |
| `origin` | centre of the parts' box | The 3D point drawn at `at`. Views of one set of parts share it, so they line up. |
| `at`, `scale` | `[0, 0]`, 1 | Where the view goes on the sheet, and sheet units per mm. |
| `hidden` | true | Draw hidden edges. |
| `silhouettes` | true | Draw the outlines of curved faces. |
| `smooth` | false | Draw edges where two faces meet tangentially (round boundaries) everywhere. Without it they are drawn only where they lie on the outline of the view. |
| `layers` | `{visible: "visible", hidden: "hidden"}` | The layer of each class: `visible`, `hidden`, `silhouette` (default: the visible layer), `hiddenSilhouette` (default: the hidden layer). |
| `partAttrs` | false | Copy each part's attributes onto its entities too. |
| `name` | `"view-<view>"` | The block's name. |
| `place` | true | Place an insert (or move the one there is). |
| `exact`, `chord` | true, max(0.01, 1e-5 x size) | The visibility oracle: ask the kernel's exact rays where the mesh is unsure; the mesh's chord (mm). |

| Choice | Decision | Why |
|---|---|---|
| What a view is | One **block definition** in true size (view-plane millimetres, the view's origin at the block's (0, 0)) and one **insert** at `at` with `scale`. Projecting again with the same name replaces the block's content and moves its first insert, so what a script placed next to it stays. | A view moves, scales and exports as one thing, and DWG keeps it as a block. The scale is in the insert, so the block's numbers are the part's. |
| Link to the part | Every entity carries `partId` (`"p_7"`), `edge` (`"visible"` or `"hidden"`) and `curve` (`"edge"` or `"silhouette"`), through the attribute mechanism; `partAttrs: true` adds the part's own attributes. | A script dimensions, labels or restyles by part with the selectors it already has (`entities({expand: true, attrs})`, `children(insert)`). |
| Layers | The script names them. Nothing in the core gives a layer name meaning. A layer the call creates gets plain defaults (white; Continuous 0.5 mm for visible and silhouettes, Hidden 0.25 mm for hidden), an existing layer is left as it is. Linetype and weight come from the layer, since block shapes carry none. | No built-in layer semantics. |
| Mapping 3D to the sheet | `toSheet(p)` on the result (plain arithmetic on `origin`, `right`, `up`, `at`, `scale`), or `cad.viewPoints` with the same view options and `points`. | Dimensions and labels go on projected features with the existing 2D calls. |
| Arranging views | Not in the core. First- or third-angle placement is a script library (`examples/3d/lib/views.js`, `arrange`), from `viewPoints` of the parts' box. | It is a drafting convention, not geometry. |

What is exact: a line projects to a line, a circle or an ellipse to an
ellipse (a circle when its plane faces the viewer, a line when the view
runs along its plane), so the entities are LINE, ARC, CIRCLE and ELLIPSE.
Silhouettes come in closed form: two rulings of a cylinder, elliptic
cylinder or cone, the great circle of a sphere, and the four or two
circles of a torus whose axis is along or across the view. A B-spline edge
(where two cylinders meet) is drawn as the conic it lies on in this view
when it lies on one (on its cylinder seen along the axis), and otherwise
as a cubic spline fitted to 0.01 mm. Coincident pieces are merged, and
visible wins over hidden, so a view carries no duplicates.

How visibility is decided: every curve is split where, in the view, it
crosses another projected edge or silhouette, where another curve ends on
it, and where a conic seen edge on folds back: those are the only places
visibility can change between parts that do not intersect. The crossings
are exact (closed form where one curve is straight, Newton's method on the
exact parameters otherwise). Each piece is then asked once, at its middle
(long pieces at three points, and split by bisection where they
disagree): a ray toward the eye against a fine mesh of the parts, which is
a numeric oracle only and is never stored or drawn. Where the mesh cannot
be trusted (a hit within 50 chords, or within two chords of the mesh's
outline, or at a grazing angle), four rays 8 chords to the side decide it
(visible if any escapes), and failing that the kernel's exact ray hits.
Pieces shorter than 1e-6 mm take a neighbour's class.

`open` lists what has no closed form here: torus faces seen obliquely
(their silhouette is not a conic) and B-spline faces; their silhouettes
are missing from the view. Parts must not interpenetrate: an edge that
passes into another part changes visibility where it pierces it, which is
not a crossing in the view.

### A domain library on top

A woodworking library is script code over these calls. It never reaches
the core:

```js
// lib/boards.js
function board(name, [w, l], t, { plane = "xy", at = [0, 0, 0], ...attrs } = {}) {
  return cad.extrude({ min: [0, 0], max: [w, l] },
                     { plane, at, distance: t, name, attrs: { board: name, t, ...attrs } });
}
function dado(part, { plane, at, from, to, depth }) {
  return cad.cut(part, { extrude: { min: from, max: to }, plane, at, distance: depth });
}
const cutList = () => cad.parts({ has: ["board"] }).map((p) => ({
  name: p.attrs.board, size: [0, 1, 2].map((i) => p.localBbox.max[i] - p.localBbox.min[i]),
}));
return { board, dado, cutList };
```

### Not yet

Sections and hatching, dimensions placed automatically, materials, assets and `scene3d` settings, live tool links
(`ToolRef::Part`) and clearances on tool parts are later rungs (3D2b to
3D6b). Between parts: no `nearest3d` yet, and no distance from a bare point
to a part.

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

### Construction operations (implemented)

These are real `cad_call` methods (see `cad.methods()`), implemented in
`cadview-core` (`construct.rs`, `annotate.rs`, `region.rs`, `spline.rs`)
and exposed through `web/src/cad-api.ts`. Entity arguments accept an ID
string or an entity object. Operations that modify an entity keep its ID.

```js
// Intersections and tangents (pure, no entities created)
cad.lineLineIntersection(l1, l2, { segments })   // l = [[x,y],[x,y]] | line id -> [x,y] | null
cad.tangentLines(c1, r1, c2, r2, "outer")        // or "inner" -> [[p1, p2], ...] (0-2 lines)
cad.tangentFromPoint(p, c, r)                    // -> tangent points on the circle (0-2)

// Fillet / chamfer: trims or extends both inputs, keeps their IDs
cad.fillet(a, b, r, { pick: [pa, pb], layer })   // lines, arcs, circles -> {arc, a, b}
cad.chamfer(a, b, d1, d2, { pick })              // two lines -> {line, a, b}

// Trim / extend against boundary entities (lines, arcs, circles, polylines)
cad.trimTo(id, boundary, keepPoint)              // keeps the piece containing keepPoint
cad.extendTo(id, boundary, { end: "start" | "end" | "nearest" })

// Arrays (counts include the original; returns the new copies)
cad.array(target, { polar: { center, count, angle: 360, rotate: true } })
cad.array(target, { rect: { rows, cols, dx, dy } })

cad.clear({ all: true })                         // also clears blocks, resets linetypes
```

**Fillet rules.** The fillet arc is a real `arc` entity tangent to both
inputs. `pick` gives a point on the part of each entity that should
survive (like clicking in AutoCAD); without picks, each entity keeps the
end farther from the other one, which is right for ordinary corners.
Radius 0 trims/extends both to a sharp corner (`arc: null`). Circles are
never trimmed. Errors: `cannot fillet parallel lines`, `no fillet
solution: radius too large for these entities`.

**Annotation.**

```js
cad.addText("A1", at, { height, rotation /*deg*/, align: "center", valign: "middle" })
cad.measure(p1, p2, { offset, dir: "h" | "v" | "aligned", label: "<> mm",
                      decimals, text_height, arrow, shift, layer })
cad.dimRadius(circleOrArc, { angle, label: "R<>" })      // or {center, radius}
cad.dimDiameter(circle, { angle, label: "Ø<>" })
cad.dimAngle(vertex, p1, p2, { radius, label: "<>°" })   // CCW from p1 to p2
```

Dimensions are plain entities on the given layer: lines (an arc for
`dimAngle`), filled arrowheads (`solid_fill`, written to DWG as SOLID)
and one real TEXT entity holding the label (returned as `text`). The
result is `{value, label, ids, text}`. `measure` offset is the signed
distance of the dimension line from the outermost measured point
(positive = left of the dimension direction, e.g. above for `dir: "h"`).
Text alignment is resolved at creation: the stored TEXT is left/baseline
anchored at the computed position.

**Fills, hatches, curves, regions.**

```js
// Rings: [[x,y],...] | {center, radius} | circle / closed polyline id
cad.addFill(boundary, { holes: [ring, ...], color })     // SOLID fill, arcs stay exact
cad.addHatch(boundary, { holes, angle, spacing, pattern: "lines" | "cross" | "solid" })
cad.addSpline(points, { fit: true, degree: 3, closed })  // fit=false: control points
cad.addPolyline([[0,0,0.5], [10,0], [10,10]], { closed }) // [x, y, bulge]
cad.regionClip(subject, clip, "subtract")  // intersect|union|subtract|xor -> [{outer, holes, area}]
```

Hatch lines are anchored at multiples of `spacing` so neighbouring
hatches line up, and holes use an even-odd scanline with half-open
crossing rules (lines through vertices are no longer dropped).
`regionClip` uses the `i_overlay` crate; it is polygonal (circles are
sampled), so use it to compute boundaries, then draw them with
`addPolyline` / `addFill` / `addHatch`.

**Blocks.** `defineBlock` shapes accept `{type: "text", text, at, height,
rotation, align, valign}` and polylines with bulge vertices. `place`
takes `scale` as a number or `[sx, sy]` (or `xScale` / `yScale`).

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
(ES2023+) available. No filesystem and no network: files come in only
through `cad.readFile` and `cad.use`, inside the script's folder.

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
  ERC checks). Add a `pins` field to `cad.defineBlock()` options? By
  principle 8 the generic answer is preferred: free-form attributes on
  entities and block definitions, which the core stores, syncs and
  filters on without interpreting them. Pins, nets and part numbers then
  need no API of their own. **Built in A8** (see "Attributes and set
  queries"): shape attributes in a block definition carry pin metadata.
  Still open: attributes on layers, and DWG storage for the attributes of
  a block definition itself.
- **Set-level queries**: rules over whole drawings (clearance between two
  groups, overlaps between two layers) need queries that take two sets and
  return every matching pair in one call. **Built in A8**: `pairsWithin`,
  `overlapping`, `inside`, `nearest`. Still open: union and offset of a
  selection, and exact distances for splines and ellipses (left out and
  reported in `skipped` today).
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
