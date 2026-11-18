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
| 9 | `09-room.js` | 25 | 07's living room (5000 x 3200, walls 300 and 100, two windows, a door), typed in: a script under `examples/3d` cannot read `examples/07-floor-plan.js` or its DWG (the sandbox reads below the script's folder only). Heights are the script's. Windows are open, no glass. The bookshelf and stool come from `lib/bookshelf.js` and `lib/stool.js`; the stool's untrimmed feet are lifted so their lowest edges touch the floor. Checks: wall volumes with openings, nothing of the room overlaps the furniture, the bookshelf's back on the wall, the stool in the sun patch the script computes. |
| 10 | `10-still-life.js` | 11 | A wine glass, a glazed bowl, a steel flask and cap, a walnut pepper mill and its finial on an oak table. The kernel refuses an arc whose circle crosses the axis, so the glass bowl has a flat floor and a straight tapered wall, the bowl a flat inside floor, the flask a quarter-circle shoulder, and the mill no convex swell (a cove between beads); the script checks every arc against the rule. No knurled band. |
| 11 | `11-play-tower.js`, `11-play-tower-partlist.js` | 142 | The whole set, built once by `lib/playtower.js`: the frame and roof, a cabin, a railing, stairs with risers and handrails, a sandbox, a wave slide and two swings. Rules over set queries (`lib/rules.js`) and a part list with a cutting plan (`lib/partlist.js`). Simplified: roof boards butted, not lapped (every contact is then a face the overlap check reads as a touch); bought parts are plausible solids, not a maker's; fixings are counted, not modelled; no construction guide yet. See "Example 11" below. |
| 12 | `12-projected-views.js` | 1 | A pillow block drawn on an A3 sheet: front, top and left views in first angle and an isometric view from `cad.project` (rung 3D3a), dimensions through `toSheet`, PNG and PDF. The oil hole is 8 mm off the bore's axis: on it, arris 0.11.1 refuses the cut. The play tower's construction guide can now be drawn from such views (`lib/views.js`); it is not written yet. |

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
| 11 | stained timber, green-stained roof, yellow plastic slide, rubber seats, rope, galvanised hangers, sand | 2.41 s | On grass in daylight (`environment: "sky"`, a green ground). Not yet the box picture; see "Example 11" below. |

Second slice (denoiser on): the times above are for raw 1024 samples;
denoised, 256 samples already look converged on 01, 03 and 06, and the
default stays 1024. The studio floor now meets its backdrop without a line.

| # | Materials | Trace | What still looks wrong |
|---|---|---|---|
| 9 | white paint, oak floor, the bookshelf and stool in oak; sky with a low western sun, two portals, exposure +1.7 | 17 s, 512 spp (interior) | Reads as a sunlit room: a sun patch with the stool's shadow, bounce light on the walls, sky and ground through the windows. The walls are plain; no glass, frames or skirting; the ground outside is a flat colour (the sky's haze softens its horizon). Faint blotches remain on the shaded walls (light bounced off the sun patch, found by chance). |
| 10 | glass, blue glaze, stainless, black plastic, walnut, brass, oak; a dimmed studio, exposure +0.3 | 5.3 s, 1024 spp (camera) | Glass refracts the flask behind it and casts a light shadow; the steel reflects the bowl and the glass; contact shadows under each object. The wood grain is regular stripes; the flask reads as polished chrome more than brushed steel; a little grain is left where the glass's inner floor is seen through two walls; the glass is empty and its shadow has no caustic. |
| 11 | stained timber, green roof, yellow slide; sky, a lawn ground (mottle), eye height 1.6 m, perspective (view "eye") | about 3 s, 1024 spp | Reads as a garden play set seen by a person standing near it: soft lawn patches, hazed horizon, sun shadows. No grass blades or context (fence, trees), so the ground is a soft coloured plane; the haze greys the middle distance a little. |

### Example 11: the play tower (2026-11-18)

Our own design. `lib/playtower.js` builds it; `11-play-tower.js` checks
and renders it; `11-play-tower-partlist.js` draws its part list. Every
part carries `member`, `assembly`, `material`, and `section` (sawn timber)
or `bought: true` and `item`. Angled members carry `cut`, their end cuts.

**The model: 142 parts in 42 definitions.**

| Assembly | Parts |
|---|---|
| Tower | 4 posts 90x90, 4 rims and 3 joists 45x145, 11 deck boards 28x130 (front and back notched round the posts) |
| Swing frame | head rail and swing beam 90x140, two A-frame legs 90x90 cut level, a tie 45x95 |
| Stairs | 2 stringers 45x195, 7 treads 28x220, 6 risers 22x145 (closed risers, an 8 mm drain gap at the foot), 4 stair posts 90x90, 2 handrails 45x95 cut plumb |
| Roof | 2 wall plates, 8 rafters with a birdsmouth, a ridge board, 14 roof boards and 2 ridge caps, stained green |
| Cabin | 34 wall boards 22x145 on three sides, window openings 500 x 445 at the back and on the right, a door the width of the stairs; 6 studs beside the openings |
| Railing | a rail post, a top rail 600 above the deck, 4 balusters 22x145 |
| Sandbox | 8 boards 28x130 in two rows, sand 200 deep (bought, by volume) |
| Slide (bought) | a trough with lips swept along lines and tangent arcs: flat on the deck, a chute at 40°, a wave crest at 15°, a second chute, a level run-out 230 above the ground; 3025 long, 2653 out from the deck; a foot under the exit |
| Swings (bought) | 2 seats 430 x 170, 4 ropes Ø16, 4 hangers under the beam |

**The rules**, script code in `lib/rules.js` over `overlapping3d` and
`pairsWithin3d`. The limits are illustrative, chosen to be plausible for a
garden play set, and are not taken from a safety standard. Each rule names
the parts when it fails; each was checked to fail on a model broken on
purpose ("top rail p_141 rests on post, needs a rail post", "head gap 94
between baluster p_143 and baluster p_144", "finger gap 18 between slide
p_169 and rail post p_139", "bench p_186 is inside the free space of swing
1", "edge front has no guard 600 high from 90 to 864"). Times are the
release build on one machine.

| Rule | Limit | Result | Time |
|---|---|---|---|
| No two parts overlap | touching is fine | none | 1.2 s |
| Every part rests on what carries it | a table: rims on two posts, a riser on its tread and both stringers, a rope on its hanger and its seat, ...; a member with no entry fails | 436 contacts, all carried | 1.7 s |
| Every post, leg and stair post stands on the ground | foot at z = 0 | all | |
| Free space round each swing | a ring sector about the rope tops, 57.5° each way (the seat's half depth plus 1800 along its arc), 250 beside the seat, 1100 above it: nothing in it but that swing | clear | 1.1 s |
| Run-out beyond the slide's exit | 2000 long, 500 wider than the slide each side, 1800 high: empty | clear | (above) |
| Head traps where a child climbs and leans | no gap from 89 to 230 | none open; 26 gaps in range closed by a part between | 1.2 s |
| Finger traps where hands go | no gap from 8 to 25 | none open; 10 closed | 1.6 s |
| Guards | every deck edge higher than 1000 has something reaching 600 above the deck, except at the stairs and the slide | all four edges | 0.4 s |

How the rules work, and what they found while the model was built:

- A free-space zone, a guard probe along an edge and a rod across a gap
  are temporary parts with the attribute `rules.probe`: added, asked about
  with one `overlapping3d`, removed.
- A gap is the distance between two parts (`pairsWithin3d`), counted only
  where the parts face each other across it and only if a rod through the
  middle of the facing area enters no other part. That rod found the open
  risers (153 between treads, in the head range), which is why the stairs
  have risers; the risers then close those gaps. The finger rule found a
  22 mm slot between the upper stair posts and the sandbox frame; the
  sandbox sides moved in line with the posts' inner faces.
- The model script runs in about 12 s: build 0.3 s, checks and rules 7.3
  s, renders 2.9 s (the presentation trace 2.4 s).

**The part list** (`lib/partlist.js`, generic over the attributes above).
A sawn piece's cut length is its own box (the part's local box) along its
longest side, rounded up to the mm: for an angled member, long point to
long point, with the angles from `cut`. The two other sides are checked
against `section`. The cutting plan is first-fit decreasing into the
longest stock length (2400 to 4800 in steps of 600) with a 3 mm kerf,
each length then cut down to the shortest stock that holds its pieces.
Result: 129 pieces in 8 sections, 157.3 m, from 37 stock lengths (166.2
m), 5.4 % offcut. Bought: the slide and its foot, 2 seats, 4 hangers, 7.46
m of rope, 0.32 m³ of sand. Fixings from rules of thumb per joint (39
rules, from "2 coach bolts per post to rim" to "2 screws per roof board
per rafter") and per part (a ground anchor per post): 926 for 436 joints;
a joint no rule names fails the part list script. The data also holds an
assembly order (12 steps, each with its parts and the fixings it uses)
for the construction guide, which waits for projected views.

**Still simplified, and what is missing.**

- Bought parts are plausible solids: the slide is one swept trough (no
  handles, no moulded ribs), ropes are straight rods, a hanger is a plate.
- The gap rule judges openings between two parts. It does not know how
  deep an opening is (a niche backed by a wall counts like a hole through
  it) or openings bounded by three or more parts. A core query that says
  which parts a set of segments (or thin probes) enter would replace the
  temporary rods; the real question, the largest probe that passes an
  opening, has no query at all.
- The slide is slow to query: about 0.1 to 0.3 s per pair against a
  board, about 1 s against a far part inside its box (the sandbox, 1.2 m
  under it), and 2 to 3 s between two swept sections, so the slide is one
  piece and its seat on the deck is short enough that its box clears the
  sandbox. Per-face boxes in the broad phase would prune far pairs.
- The presentation render is not yet the picture on the box. The views
  are the named isometric corners, so the camera looks down at the grass:
  no horizon, no sky in the picture, and the first swing hides behind the
  cabin from every corner. The ground is one flat colour, not grass. Both
  need the renderer (a camera with an eye, a target and a field of view; a
  ground material), not the model.

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
