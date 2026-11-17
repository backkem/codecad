# Circuit boards in CodeCAD: direction

Status: a direction, written 2026-11-18. Nothing here is planned on a
ladder or built. It records how circuit board design would fit the
architecture, so that work on the 2D API, the 3D ladder and the code ABI
does not close the door on it.

## The shape of the problem

A circuit board is a layered 2D design that lives inside a 3D product,
tied together by a list of electrical connections. Those are three
different things, and each has one natural home.

| What | Where it lives | Why |
|---|---|---|
| Copper, solder mask, silkscreen, drills, board outline | The 2D document, one layer per board layer | It is flat geometry, and everything a fabricator receives is 2D per layer. The existing exact 2D model (lines, arcs, bulge polylines, fills) already fits. |
| The board as an object, component bodies, the enclosure | 3D parts and assets | Fit and clearance are 3D questions. |
| The netlist and the design rules | The script, or data given to the script | It is meaning, not geometry (see `architecture.md`, "Semantics live in code"). |

Two consequences follow.

**2D is the source of truth for the board.** The board as a solid is
derived: an extruded outline minus its holes, the simplest kind of part
the 3D ladder has. Copper is never modelled as solids. If it has to be
seen in 3D it is extruded as a display mesh on demand, not sent through
the solid kernel.

**The core does not learn electronics.** A circuit board layer is a
library on top of the generic API, the way `examples/lib/drafting.js`
sits on the 2D API today.

## Where 2D and 3D meet

- **The enclosure drives the board.** Project the enclosure's interior,
  its standoffs and its connector openings onto the board plane to get
  the outline, the mounting holes and the keep-out areas. Change the
  case and the board follows. This is the 3D to 2D projection the ladder
  plans for shop drawings (3D3a), used in the other direction of the
  design flow.
- **One placement, two representations.** A component is a 2D footprint
  (pads, courtyard, silkscreen) paired with a 3D body. The 3D design
  already pairs an asset with a plan symbol for furniture
  (`AssetRef.plan_symbol` in `3d-design.md` section 4.1). A footprint is
  the same idea, and a block definition is its natural 2D form.
- **Height and clearance.** Component bodies against the lid, connectors
  against panel cut-outs, tall parts against 2D zones that carry a
  maximum height. These are the interference and distance checks of rung
  3D1b.
- **Drawings from the model.** Assembly and fabrication drawings are
  projected views with dimensions, the same route as shop drawings.
- **Renders for review.** `cad.render` lets a script or an agent look at
  one layer or the assembled board without a browser.

## What the core would need

All of it generic, none of it about electronics:

1. **Attributes on entities and block definitions.** A track has to say
   which net it belongs to, a pad which pin it is. The core stores the
   key and value pairs, keeps them through save, sync and undo, and
   filters on them, without interpreting them.
2. **Set-level geometric queries.** Every pair from set A and set B
   closer than a distance. Every overlap between two selections. These
   are what clearance and short-circuit checks reduce to, and they have
   to run in the core to stay fast on boards with thousands of tracks.
3. **Polygon union and offset on selections,** for copper pours and
   their clearances. The core already has polygon booleans
   (`region.rs`); exact offsets are on the API ladder (A2).
4. **Per-layer export.** Fabrication formats are 2D per layer. An
   exporter is a consumer of the document like the PDF and PNG paths.

Items 1 and 2 are recorded as open questions in `api-surface.md`.

## What a board library would provide

- A netlist model and footprints, defined in script or loaded from data.
- Placement of a footprint and its 3D body in one call.
- Design rule checks written against the generic queries: clearance
  between different nets, track width, annular rings, board edge
  clearance, maximum height zones.
- A connectivity check that is independent of the script's own
  bookkeeping: derive which copper touches which, through which vias,
  from the finished geometry, and compare it with the intended netlist.
  A mismatch names the nets and the place. This is the same "derive,
  then compare" discipline as the 3D test harness.
- Schematic drawing. The repo already has schematic examples and
  electrical symbol import to build on.

## Choices to keep open

- **Interoperate before inventing.** Reading and writing existing
  footprint, board and fabrication formats comes before any format of
  our own. Check the licence of every library and data set first.
- **Interactive routing is out of scope for this direction.** The split
  above holds while code is the editor. Dragging a track in the viewer
  would change geometry without the netlist's knowledge, so an
  interactive router would need the document to know about connectivity.
  That is a different product decision and it is not taken here.
- **Units and scale.** Boards work in fractions of a millimetre, while
  the API speaks metres (see the open question in `api-surface.md`).
