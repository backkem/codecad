// The site's examples: one entry per example page. Paths are relative to the
// site root, where `just build-site` copies examples/ (2D drawings) and
// examples/3d/ (models). An entry with `model` has a Model tab (a prebaked
// scene, its working and presentation renders); one with `drawing` has a
// Drawing tab (a DWG); every entry has a Script tab (`scripts`, plus the
// libraries they load, found by reading them).

const d2 = (id, title, desc, text) => ({
  id,
  title,
  desc,
  text,
  drawing: { dwg: `examples/${id}.dwg`, png: `examples/${id}.png` },
  scripts: [`examples/${id}.js`],
});

const d3 = (id, title, desc, text, extra = {}) => ({
  id,
  title,
  desc,
  text,
  model: {
    scene: `examples/3d/${id}.bake3d`,
    working: `examples/3d/${id}.png`,
    render: `examples/3d/${id}-render.png`,
  },
  scripts: [`examples/3d/${id}.js`],
  ...extra,
});

export const EXAMPLES = [
  // ── Models ─────────────────────────────────────────────────────────
  d3(
    "11-play-tower",
    "Play Tower",
    "142 parts, checked by rules, with its part list sheet",
    "A garden play set of our own design: a platform on four posts, a cabin under a gabled roof, a railing, stairs, a sandbox, a wave slide and two swings. 142 parts in 42 definitions, built once by a library. The script checks the model with rules written in script: no two parts overlap, every part rests on what carries it, nothing in each swing's free space, no gap that can trap a head or a finger, and a guard on every platform edge except at the stairs and the slide. The limits are illustrative, not a safety standard. A second script draws the part list: sawn timber by section and length, a cutting plan, bought items and fixings. A third draws the construction guide: one drawing per assembly step, made from projected views of the model.",
    {
      drawing: {
        dwg: "examples/3d/11-play-tower-partlist.dwg",
        png: "examples/3d/11-play-tower-partlist.png",
        title: "Part list",
      },
      scripts: [
        "examples/3d/11-play-tower.js",
        "examples/3d/11-play-tower-partlist.js",
        "examples/3d/11-play-tower-guide.js",
      ],
    },
  ),
  d3(
    "12-projected-views",
    "Projected Views",
    "A pillow block and its shop drawing from exact projections",
    "A cast pillow block: a base with rounded corners and two counterbored bolt holes, an upright with a semicircular top, a chamfered bore and an oil hole. Front, top and left views in first-angle order and an isometric view come from cad.project as exact lines, arcs, circles and ellipses, with hidden edges dashed on their own layer. They are laid out on an A3 sheet with a title block and dimensions.",
    {
      model: {
        scene: "examples/3d/12-projected-views.bake3d",
        working: null,
        render: null,
      },
      drawing: {
        dwg: "examples/3d/12-projected-views.dwg",
        png: "examples/3d/12-projected-views.png",
        title: "Shop drawing",
      },
    },
  ),
  d3(
    "01-pipe-flange",
    "Pipe Flange",
    "DN50 weld-neck flange: one revolve, chamfers and holes",
    "A DN50 weld-neck flange: a disc with a raised face, a hub tapering to the weld end, a bore, four bolt holes and 1 mm chamfers. One revolve of exact lines, then chamfers and holes. The script checks the volume against its closed form.",
  ),
  d3(
    "02-mounting-bracket",
    "Mounting Bracket",
    "6 mm L-bracket with a gusset, counterbores and slots",
    "An L-bracket from 6 mm plate with an R6 inside bend, a gusset, two counterbored holes, two slots and R3 corners. One extrude of a side profile with exact bend arcs; the slots are cuts of lines and half circles.",
  ),
  d3(
    "03-bookshelf",
    "Bookshelf",
    "Twelve parts with dados and a rabbet, cut by the parts",
    "800 x 1800 x 300: two sides with dados for five shelves, a top, a back panel in a rabbet and three rails, twelve parts. The joints are cut with the parts themselves; the shelves share one definition. The script checks that no two parts overlap and that every shelf touches both sides.",
  ),
  d3(
    "04-enclosure",
    "Electronics Enclosure",
    "Two-part case with bosses, standoffs and a board",
    "A 100 x 60 x 25 case in two halves with 2 mm walls, corner bosses with screw holes, a board on four standoffs with its larger components as blocks, and cut-outs for a connector and a jack.",
  ),
  d3(
    "05-piston-and-rod",
    "Piston and Rod",
    "Ring grooves, a hollow pin and a rod, in quarter cutaway",
    "A Ø80 piston with three ring grooves, pin bosses with blind pin bores, a hollow pin, and a rod 140 between centres whose outline is two circles joined by tangent lines.",
  ),
  d3(
    "06-spur-gears",
    "Spur Gear Pair",
    "Module 2, 20 and 40 teeth, one arc per involute flank",
    "Module 2, 20 and 40 teeth, meshed at 60 centre distance. Each gear is one extrude of its whole outline, with one arc per involute flank, a hub, a bore with a keyway and a chamfer.",
  ),
  d3(
    "07-pipe-spool",
    "Pipe Spool",
    "Swept line with two elbows, three flanges and supports",
    "A Ø60.3 x 3.9 line with two long-radius elbows, swept as one ring along lines and arcs, three flanges from the pipe flange example (one definition), a stub and two saddle supports.",
  ),
  d3(
    "08-turned-stool",
    "Turned Stool",
    "Three splayed turned legs and tapered stretchers",
    "A round seat with rounded edges on three turned legs splayed 8°, with beads and a cove, and three tapered stretchers. Legs and stretchers are placed by turning copies about the axis.",
  ),
  d3(
    "09-room",
    "Room",
    "The floor plan's living room in 3D, with furniture",
    "The living room of the 2D floor plan in 3D: walls with windows and a door, an oak floor and a ceiling, with the bookshelf against the north wall and the stool in the sunlight from the west window. The script checks that the stool stands in the sun patch it computes.",
  ),
  d3(
    "10-still-life",
    "Still Life",
    "Glass, bowl, flask and mill: revolves for the path tracer",
    "A wine glass, a glazed bowl, a steel flask with its cap and a walnut pepper mill on an oak table. Every object is one revolve of exact lines and arcs, checked against its Pappus volume.",
  ),

  // ── Drawings ───────────────────────────────────────────────────────
  d2(
    "01-flange-plate",
    "Flange Plate",
    "DN80 PN16 flange, front view and section A-A",
    "Plate flange DN80 PN16 (EN 1092-1 type 01) as a two-view drawing: front view with the bolt pattern on its pitch circle, and full section A-A with the holes revolved into the cutting plane. Scale 1:2.",
  ),
  d2(
    "02-fibonacci-spiral",
    "Fibonacci Spiral",
    "Square tiling, spiral and golden-ratio construction",
    "A 144 x 89 rectangle tiled with Fibonacci squares. A quarter arc in each square forms the spiral, and two construction diagonals meet at its pole. Next to it, the compass construction of the golden rectangle from a square.",
  ),
  d2(
    "03-roof-truss",
    "Howe Roof Truss",
    "Timber truss with trimmed joints and member schedule",
    "Howe timber truss with a 12 m span, 3 m rise and 6 panels. Members are drawn at their real depth around the setting-out lines, with nail plates, wall bearings, chained dimensions, the pitch angle and a member schedule. Scale 1:50.",
  ),
  d2(
    "04-motor-schematic",
    "Motor Starter",
    "IEC direct-on-line starter, power and control",
    "IEC 60617 schematic of a 3-phase direct-on-line starter: power and control circuits from a symbol block library, terminal numbers, device tags, a parts list and the sequence of operation.",
  ),
  d2(
    "05-bicycle-frame",
    "Bicycle Frame",
    "Road frame set out from stack, reach and angles",
    "Road frame (size 56) set out from the bottom bracket by stack and reach, head and seat angles, BB drop, chainstay length and fork rake. The tube silhouette is fused from the tube strips. Scale 1:8.",
  ),
  d2(
    "06-piston-section",
    "Piston Section",
    "Half-section with ring-groove detail at 3:1",
    "Piston and connecting rod as a half-section assembly at 1:1. The rod shank is shown with a conventional break, and DETAIL A shows the ring grooves at 3:1.",
  ),
  d2(
    "07-floor-plan",
    "Floor Plan",
    "Apartment plan with doors, areas and room schedule",
    "One-bedroom apartment with a study at 1:50. Walls are solids fused with a union, with door and window openings cut out. Rooms and their areas come from interior minus walls. A door block with handing, furniture, chained dimensions, a north arrow and a room schedule.",
  ),
  d2(
    "08-distribution-board",
    "Distribution Board",
    "Single-line diagram: main switch, RCDs, 10 circuits",
    "Single-line diagram of a domestic distribution board: meter, a 63 A main switch, two RCD groups feeding ten MCB circuits, and a type 2 SPD. The legend is drawn from the same block library.",
  ),
  d2(
    "09-spur-gears",
    "Spur Gears",
    "Meshing involute pair with gear data table",
    "A meshing pair of true involute spur gears (m3, 18/30 teeth, 20°) at the standard 72 mm centre distance, with pitch and base circles, the line of action, bores with keyways and a gear data table.",
  ),
  d2(
    "10-cam-follower",
    "Cam and Follower",
    "Cycloidal cam profile with displacement diagram",
    "Disc cam with an in-line roller follower, generated from its motion law: cycloidal rise and return with dwells. The profile is the roller envelope of the pitch curve, with a displacement diagram and a motion table.",
  ),
  d2(
    "11-pid",
    "P&ID",
    "Tank and pump with level and flow control loops",
    "A small P&ID: a feed tank with level control and a pump delivering through a check valve and an isolation valve, with a flow loop on to an exchanger. ISA 5.1 bubbles come from one template block.",
  ),
  d2(
    "12-site-plan",
    "Site Plan",
    "Boundary from bearings, setbacks, parking, contours",
    "Site plan at 1:500. The boundary is plotted from a surveyor's traverse, and the closing leg is computed. The building line is the boundary offset by the setbacks, with the footprint, parking bays, contours, trees, a north arrow and a scale bar.",
  ),
];

/** The entry for `id`, or undefined. */
export const exampleById = (id) => EXAMPLES.find((e) => e.id === id);
