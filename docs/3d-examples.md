# 3D examples: the target set

Ten example models that the 3D work is aimed at, the way `examples/01` to
`12` are the targets for 2D. Eight are useful CAD cases from different
trades; two are there to show what the renderer can do. Each one is
rendered in both modes:

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

Both lists end with the maintainer looking at a contact sheet of all ten
and accepting it. The numbered points are checked by tests where a number
exists (silhouette area, edge pixels where the geometry says an edge is,
byte equality, time), and by eye where it does not.

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

## Order of work

1. **Now (after the headless render and the curved features land):** a
   gallery in the test harness with the examples that today's features
   cover: 3 (bookshelf) and 4 (enclosure) in full, 1 (flange) and 2
   (bracket) without their rounds if those are not ready. A contact
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
   all ten, with materials. Examples 9 and 10 are finished here. The
   reference scene for the look gate is example 3.

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
