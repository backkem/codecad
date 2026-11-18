# 3D examples: the target set

Eleven example models that the 3D work is aimed at, the way `examples/01`
to `12` are the targets for 2D. Eight are useful CAD cases from different
trades, two are there to show what the renderer can do, and one, the play
tower, is the flagship that uses everything at once. Each one is rendered
in both modes:

- **Working render**: fast, rasterized, for checking a model while
  building it.
- **Presentation render**: path traced, with materials and lighting, for
  showing the result.

The models are ordinary scripts (the domain lives in the script, see
`docs/architecture.md`, "Semantics live in code"). Until the 3D script API
exists they live as feature-tree recipes in the test harness, where they
also serve as extra cases for the kernel scoreboard.

Dimensions are in mm. Z is up.

## What "satisfactory" means

**Working render** (every example, 960x640, isometric plus the views
listed):

1. Edges are drawn. Silhouettes and sharp edges show as thin dark lines
   over the shading, as in any CAD viewport. Smooth faces show no facet
   lines.
2. Parts can be told apart: one colour per part or per material.
3. Depth reads without effort: holes, pockets and gaps between parts are
   visibly darker than the faces around them (lighting from a key
   direction plus a fill, and a soft contact shadow on the ground).
4. Curved faces look curved at the default tolerance, with no cracks
   between faces.
5. Two runs give the same bytes, and a warm render takes under one second.
6. Where an example lists a cutaway or an exploded view, the hidden
   feature it is meant to show is visible.

**Presentation render** (every example):

1. Materials read as what they are: wood shows grain, brushed metal shows
   its direction, glass refracts, paint is matte or glossy as specified.
2. The model stands on a ground or in a room, lit by an environment, with
   soft shadows.
3. The image is converged: no visible noise at the target sample count
   after denoising.
4. The reference scene passes the look gate against Blender (rung 3D5b).

Both lists end with the maintainer looking at a contact sheet of all eleven
and accepting it. The numbered points are checked by tests where a number
exists (silhouette area, edge pixels where the geometry says an edge is,
byte equality, time), and by eye where it does not.

## Status (2026-11-18)

Nine of the eleven exist as scripts in `examples/3d/` (see its
`index.md`), with working renders. Each script checks its parts against
values it computes itself (exact volumes from closed forms, boxes, part
counts) and fails if one does not hold; a server test runs them all. The
recipes went straight to scripts; the harness does not hold them as
scoreboard cases yet.

| # | Script | Parts | What is simplified, and why |
|---|---|---|---|
| 1 | `01-pipe-flange.js` | 1 | Bore Ø52.5, not Ø60.3 (that would leave no wall at the weld end). |
| 2 | `02-mounting-bracket.js` | 1 | Nothing. |
| 3 | `03-bookshelf.js` | 12 | The list names nine parts for "twelve"; a plinth, a low rail and a hanging rail make twelve. Checks between parts (3D1b): no two overlap, every shelf touches both sides, the shelf gap is the opening. |
| 4 | `04-enclosure.js` | 9 | The board's corners are notched to clear the corner bosses. |
| 5 | `05-piston-and-rod.js` | 3 | Square pin bosses and blind pin bores that stop short of the skirt: where two cylinders meet, the kernel adapter could not give the part's box. Since 3D1b it can (tight to 1e-9 mm); the example is not changed back yet. |
| 6 | `06-spur-gears.js` | 2 | One arc per flank, no root fillets. No mesh interference check. |
| 7 | `07-pipe-spool.js` | 7 | The third flange is the mating flange of the next spool, on a stub. No gaskets, bolts or welds. |
| 8 | `08-turned-stool.js` | 7 | Beads are half circles (a flatter bead's circle crosses the leg axis: refused as a spindle torus). Joints are not cut: tenons overlap the seat and legs; feet and shoulders stay square to the 8° leg. |
| 9 | not started | | Waits for the presentation render. |
| 10 | not started | | Waits for the presentation render. |
| 11 | `11-play-tower-frame.js` | 63 | The timber frame and a gabled timber roof: posts, platform frame and deck, swing beam on a head rail, A-frame, stairs; wall plates, rafters, ridge board, roof boards and ridge caps. The roof boards are butted with 5 mm gaps, not lapped: chosen so every contact is a face the overlap check reads as a touch (lapped boards would meet along lines; not tried). No cabin, railings, sandbox, slide, swings, fixings or clearance rules yet. Checks between parts (3D1b): no two overlap, the swing beam rests on the head rail and both legs, every rafter rests on a plate and the ridge, the roof clears the swing beam by 328.8. |

### Presentation renders (2026-11-18, rung 3D5a first slice)

Each script gives its parts a `material` attribute, and
`examples/3d/lib/checks.js` path traces the isometric view (and any view
with a `present` option) with the presets of `lib/materials.js`, to
`target/3d-views/NN-name-render.png` (not committed yet). All nine stand on
a light floor in the default studio light (a grey dome and a 16° soft key
light), with soft shadows and bounce light. Times are tracing at 960 x 640
and 1024 spp on one discrete GPU. Against the presentation list: point 2
holds for all; point 3 holds except where noted (no denoiser yet); point 4
(the look gate) is not run.

| # | Materials | Trace | What still looks wrong |
|---|---|---|---|
| 1 | turned steel | 1.47 s | Fine grain left inside the bore and the bolt holes (glossy metal seen in metal). |
| 2 | brushed steel | 1.16 s | Reads as metal. One streak direction for the whole part (its longest), not one per leg as on a bent plate; faint at this size. |
| 3 | oak, birch ply | 2.02 s | Reads as furniture. Grain is procedural rings, softer than real oak figure. |
| 4 | grey plastic, green board, black parts, aluminium capacitors | 1.67 s, 2.37 s lid lifted | Reads right; the components are blocks (the model's simplification). |
| 5 | turned aluminium, steel rod, turned steel pin | 1.27 s | Reads as metal; the piston's turning marks are faint at this size. |
| 6 | turned steel | 1.77 s | Concentric streaks show on the faces; some speckle remains where the streaks are finer than a pixel. |
| 7 | steel pipe, red gloss paint, turned steel flanges | 1.21 s | The pipe's streaks run along the whole spool's longest direction, not around the bends. |
| 8 | oak | 2.18 s | Reads as a wooden stool; grain runs along each leg and stretcher. |
| 11 | stained timber | 1.81 s | Studio light, not the grass and daylight of the box picture (the script can pass `environment: "sky"` and a green ground). |

No example uses glass yet; glass is checked by the tests (a slab's
displacement, energy in the furnace).

## The examples

### Useful cases

| # | Example | What it is | Features it needs | Views |
|---|---|---|---|---|
| 1 | **Pipe flange** | DN50 weld-neck flange: disc Ø165 x 18, raised face Ø102 x 2, bore Ø60.3, hub tapering to Ø60.3 x 45 high, four Ø18 bolt holes on a Ø125 circle, 1 mm chamfers. The 3D twin of `01-flange-plate`. | revolve (or extrude and holes for a first version), hole pattern, chamfer | iso, half cutaway |
| 2 | **Mounting bracket** | L-bracket from 6 mm plate: legs 80 x 60 and 60 x 60, a triangular gusset, two Ø9 counterbored holes in the base, two 9 x 20 slots in the upright, R6 inside bend, R3 outer corners. | extrude, slot (extrude with arcs), counterbore, fillet | iso, front |
| 3 | **Bookshelf** | The worked example of 3d-design.md section 10.1: 800 x 1800 x 300, two 18 mm sides with 6 mm dados, five shelves, a top, a 6 mm back panel in a rabbet. Twelve parts. | extrude, boolean, copy and move, interference check | iso, exploded, front |
| 4 | **Electronics enclosure** | 100 x 60 x 25 two-part case with 2 mm walls, four corner bosses with Ø2.5 holes, a 90 x 50 x 1.6 board on four standoffs, cut-outs for a 9 x 3.5 connector and a Ø8 jack, the board's larger components as blocks. Ties to `docs/pcb-direction.md`. | shell (or pocket), boss pattern, cut-outs, multi-part assembly | iso closed, iso with lid lifted, top |
| 5 | **Piston and connecting rod** | Ø80 piston with three ring grooves and a Ø22 pin bore, a rod 140 between centres with a Ø52 big end, the pin. The 3D twin of `06-piston-section`. | revolve with grooves, boolean, extrude with arcs, fillet | iso, quarter cutaway |
| 6 | **Spur gear pair** | Module 2, 20 and 40 teeth, 15 wide, hubs with keyways, meshed at 60 centre distance. Tooth flanks as arcs fitted to the involute. The 3D twin of `09-spur-gears`. | extrude of a long exact profile, pattern, keyway, chamfer | iso, front |
| 7 | **Pipe spool** | Ø60.3 x 3.9 pipe run with two 90° long-radius elbows and three flanges from example 1, on two supports. The 3D side of `11-pid`. | sweep along lines and arcs, hollow section, reuse of a part | iso, top |
| 8 | **Turned stool** | Round seat Ø320 x 30 with a rounded edge, three turned legs with beads and coves splayed at 8°, three tapered stretchers. | revolve of arcs and lines, loft, fillet, placed copies | iso, front |

### Show pieces

| # | Example | What it is | What it shows |
|---|---|---|---|
| 9 | **Room** | The bookshelf and the stool placed in a room extruded from the walls of `07-floor-plan`, with a window, daylight through it, a wood floor and painted walls. | 2D and 3D together; a model in its setting; daylight, soft shadows and bounce light in the presentation render |
| 10 | **Still life** | A wine glass, a ceramic bowl, a steel thermos and a wooden pepper mill on a table: all revolved profiles, the mill with a knurled band. | Glass, glazed ceramic, brushed steel and wood in one image: refraction, reflections, contact shadows |

### Flagship

**11. Play tower with slide and swing.** A garden play set of our own
design, the kind sold as a kit: something you would want to buy, except
that here you get the recipe.

- **The model.** A 1500 x 1500 platform 1450 above the ground on four
  90 x 90 posts. On it a small cabin with board walls, a window opening
  and a gabled roof of overlapping boards, next to an open balcony with
  a railing. Stairs with handrails up one side, a sandbox framed in
  boards underneath, a 2900 long wave slide off the front, and a swing
  beam 2300 above the ground running out to an A-frame, with two swings.
  About 200 parts: sawn timber in a handful of stock sections, plus
  bought items (the slide, swing seats, ropes, hooks, bolts).
- **Sanity checks, written in the script.** No two parts overlap. Every
  joint has the fixings its parts need. The clearances a play set must
  keep are script rules over generic queries: free space around the
  swings and the slide exit, gaps that cannot trap a head or a finger,
  railing heights for the fall height. A rule that fails names the
  parts.
- **Renders.** A working render for building it, with an exploded view.
  A presentation render on grass in daylight, with stained timber, the
  green roof panels and the plastic slide: the picture on the box.
- **The 2D side.** A part list a person can shop and saw from: timber
  grouped by section and length with a cutting plan per stock length,
  and the hardware and bought items counted. Then a construction guide:
  numbered steps, each a drawing of what is added in that step, with
  part labels and the fixings to use.

It needs every rung of the ladder (many placed parts, angled members,
bought parts as assets, projected views, dimensions, a script library
for the part list), so it is finished last. Its plain timber frame
(posts, platform, swing beam and A-frame) joins the gallery early, as
the many-parts case for the working renderer.

## Order of work

1. **Now (after the headless render and the curved features land):** a
   gallery in the test harness with the examples that today's features
   cover: 3 (bookshelf) and 4 (enclosure) in full, 1 (flange) and 2
   (bracket) without their rounds if those are not ready, and the timber
   frame of 11 (play tower). A contact
   sheet of working renders. The renderer is then improved against the
   list above until the sheet is accepted: edges over shading first,
   then per-part colours, lighting, the ground shadow, cutaway and
   exploded views.
2. **With the curved features (rung 3D0b):** examples 5 to 8 and the
   shapes of 10 join the gallery and the contact sheet.
3. **With the script API (rungs 3D1a, 3D1b):** the recipes become
   `examples/3d/NN-name.js` scripts, with an export step that writes
   their PNGs, like the 2D examples. The harness keeps the parts as
   scoreboard cases.
4. **With the path tracer (rungs 3D5a, 3D5b):** presentation renders of
   all eleven, with materials. Examples 9 and 10 are finished here. The
   reference scene for the look gate is example 3.
5. **With views, dimensions and script libraries (rungs 3D3a, 3D3b,
   A7):** the play tower's part list and construction guide, which
   complete example 11.

## Notes

- The examples span mechanical, woodworking, electronics, process and
  building work on purpose: each new trade is a check that the core
  stayed generic.
- Example 6 fits arcs to the involute because the reference semantics has
  no spline profile yet. If spline profiles get a certified reference,
  the flanks become exact involutes.
- Every part of every example goes through the harness (closed form
  where one exists, otherwise enclosure, shell test, mesh invariants and
  the two kernels against each other). An example the kernel refuses or
  gets wrong is a finding, not something to work around in the recipe.
