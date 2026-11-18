# 3D test harness: design

Status: accepted 2026-11-18, after 3D0a. Being built slice by slice
(section 9); nothing below is implemented unless the "Slices built" note
in section 9 says so.

## 1. Why

CodeCAD's 3D parts are evaluated by a solid kernel behind the
`SolidKernel` trait (`cadview-core/src/solid/`). Every candidate kernel is
large (200 000 to 500 000 lines), young, written mostly by one person, and
none has been read end to end by anyone. One candidate changed its licence
within weeks of being chosen. Whether we ship on a third-party kernel or
write our own, the kernel's code cannot be the thing we trust.

What we trust instead is evidence that we produce ourselves, from first
principles, with no other kernel as the source of truth. This harness is
that evidence. It decides which kernel ships, guards every kernel bump,
and is the specification an in-house kernel would have to meet.

3D0a already has the first piece: a corpus of eight joinery solids with
hand-derived volumes and bounding boxes. That only covers shapes somebody
derived a formula for. The harness generalises it.

## 2. Scope

Broad on purpose. The harness must work for any solid a kernel can
produce, not only prismatic parts: curved faces, revolves, sweeps, lofts,
roundovers, offsets, freeform surfaces. It is organised by **kind of
evidence**, not by feature, so a new feature needs no new machinery, only
a definition (section 3).

Out of scope: strength simulation. Verifying a solver uses the same ideas
(closed-form cases, convergence, symmetry, conservation) but is a separate
harness.

## 3. The reference meaning of a feature

Every feature in the tree gets a second definition, as a set of points,
that involves no boundary representation at all:

| Feature | Point `p` is inside when |
|---|---|
| Extrude | its projection lies inside the profile and its height is between the caps (with taper: inside the profile scaled for that height) |
| Revolve | its radius and height, taken about the axis, lie inside the profile, and its angle is within the sweep |
| Hole | it is not within the hole's radius of the axis, over the hole's depth |
| Boolean | the operands' answers combined with and, or, and-not |
| Offset, shell | its distance to the solid is below the offset |
| Roundover | it is in the solid opened (convex edges) or closed (concave edges) by a ball of the given radius |
| Sweep | it lies inside the profile placed at the nearest frame along the path |
| Loft | it lies inside the section interpolated for its position between the sections |

Evaluated recursively over the feature tree this gives, for any part, a
signed function `f`: negative inside, positive outside, and `|f(p)|` a
guaranteed lower bound on the distance from `p` to the boundary. Each
primitive contributes an exact signed distance (1-Lipschitz, exact sign);
booleans combine them with `min` and `max`, which keep the sign exact and
the magnitude a valid lower bound. The reference is **certified**: it is
evaluated in outward-rounded interval arithmetic, so rounding can widen
an answer but never flip it. For a point it answers inside, outside or
"boundary band" (the interval holds zero), together with the distance
bound; the harness reports the band as undecided, never as correct. It
is slow and simple, and it shares no code with any kernel. It is the
oracle.

The rows the harness implements, written as one exact definition each
in `solid-harness/src/reference/` (doc comments are the specification):

| Feature | Exact definition |
|---|---|
| Extrude (no taper) | `(u, v)` in the profile region (non-zero winding of simple, disjoint loops, holes inside the outer loop) and `w` between `0` and the signed distance |
| Hole | removes `rho < d/2` for `t` in `[0, depth]` from the entry point along the direction (`t >= 0` for through), plus `rho < cbore/2` for `t` in `[0, cbore depth]`; nothing behind the entry point |
| Boolean, inline tool | the tool's reference moved by the placement (normalised quaternion), combined by union, difference or intersection |
| Chamfer, equal distances | defined only on the cap edges of a first, adding extrude over a convex polygon, before any other feature: keeps `s + t >= d`, `s` the distance inside from the edge line and `t` the depth from the cap; mitres at convex corners; every other case is "not defined" |
| Revolve | the profile on one side of its axis (it may touch it); a point is in the full revolve when its meridian point (axial coordinate, distance from the axis) is in the profile region; below a whole turn, cut to the angles `[0, theta]` from the profile's half-plane, right-handed about the axis (a wedge: two half-spaces through the axis, intersected up to 180 degrees, united above) |
| Loft | defined for two sections on planes with the same axes, each one strictly convex polygon of lines, side `i` of one exactly parallel to and in the direction of side `i` of the other: the convex hull of the two polygons (every side face is then a plane); every other loft is "not defined" |
| Sweep | along lines and arcs that start at the profile plane's origin along its normal, tangent-continuous: the section moves rigidly with no twist, sliding along a line and turning about an arc's axis; the solid is the union of the pieces (a prism per line, a revolve of the section by the arc's angle per arc) |
| Fillet (rolling ball) | defined only for all edges, directly after a first, adding extrude of a strictly convex polygon: the prism opened by a ball of radius `r`, which for a convex prism is the set of points within `r` of the prism shrunk by `r` (signed distance `sd(P') - r`, exact); every other round is "not defined" |

**No kernel adapter may evaluate a feature before its reference meaning
is written down as one exact definition.** Property-only checks are
allowed only where an exact definition is impossible, and the definition
must say so. Where a feature's definition covers only some recipes (loft,
fillet), the rest are "not defined": those parts get n/a on the reference
layers and are judged by closed forms, the mesh layer, the kernel's
validity, equivalent constructions and the two kernels against each
other.

Two consequences:

- The table is also a **specification**. Writing "a roundover is the solid
  opened by a ball" settles what the feature means before any kernel is
  asked. Sweep and loft need the most care: their meaning has to be pinned
  down here, and an operation with several valid answers (section 8) can
  only be checked for properties.
- Shapes with no recipe (imported or freeform geometry) have no reference
  of their own. Operations on them are still checkable: the result of a
  boolean must equal the combination of its operands, with operand
  membership taken from a fine mesh and ray casting.

## 4. Evidence layers

Each layer is independent of the others and of the kernel under test.

1. **Closed forms.** Volume, area, centroid and bounding box derived by
   hand, matched to 1e-9. Beyond the 3D0a corpus: Pappus's theorems give
   exact volume and area for any revolve, the prismatoid formula for ruled
   lofts, the Steinmetz solid for two crossing holes, and standard results
   for cones, spheres, tori and edge rounds.

   1b. **Certified volume enclosure.** No closed form needed: subdivide
   the reference's bounding box adaptively. A cell whose interval is
   `<= 0` (or `>= 0`) everywhere, or whose centre value exceeds its
   half-diagonal, is wholly inside (or outside): that is what the distance
   bound certifies. This gives guaranteed `V_lo <= V <= V_hi`, refined to
   a stated relative width or budget. A kernel volume outside the
   enclosure is wrong, and every hand-derived closed form must fall inside
   it (that tests the reference). Volume evidence therefore never rests on
   sampling alone.
2. **Boundary shell test.** Sample points on every face of the kernel's
   result. Step a small distance inside and outside along the normal and
   ask the reference. Every inside step must be certified inside and every
   outside step certified outside. The step is chosen from the mesh
   tolerance so that a correct kernel never leaves a step in the band.
   This is far sharper than comparing volumes, and it localises a failure
   to a face.
3. **Entity consistency.** Every edge curve lies on both of its faces,
   every vertex on its edges, every loop closes, orientation is
   consistent, and faces that should meet tangentially (roundovers) do.
   Checked by us from the kernel's geometry, without the kernel's own
   checker.
4. **Integral identities.** The mesh volume by the divergence theorem
   converges to the kernel's stated volume as the tolerance shrinks. The
   total vector area of a closed surface is zero. The Euler characteristic
   matches the number of through holes the recipe makes.
5. **Equivalent constructions.** The same solid built two ways must agree
   in volume, bounding box and sampled membership: a box and an extruded
   rectangle, a straight sweep and an extrude, a loft of identical
   sections and an extrude, a tapered extrude and a loft, a revolve of a
   rectangle and a tube.
6. **Algebra and motion.** Volume of a union is the sum minus the
   intersection. `A - A` is empty and `A + A` is `A`. De Morgan,
   associativity and absorption hold. Results do not change under rigid
   motion, mirroring, operand order or a change of units. Splitting by a
   plane preserves volume.
7. **Continuity sweeps.** Slide a tool through a tangency or a flush
   contact in small steps. Volume must change continuously and in the
   right direction. A jump is a topology bug. This needs no closed form
   and it targets the region where kernels break.
8. **Sections and projection.** The area of a section equals the
   reference sampled in the cutting plane. Hidden-line output is checked
   two ways: each visible curve against a ray cast to the eye, and the
   whole view against a rasterised image of the mesh (the PNG renderer and
   `pngdiff` already exist).
9. **Determinism.** The same input gives the same bytes, natively and on
   wasm32.
10. **Fuzzing.** Random feature trees, biased toward coincident faces,
    shared edges, tangent holes, tiny features and huge coordinates. A
    failing case is shrunk to a minimal recipe and added to the corpus.
    Fuzzing is a development-time tool: it does not run in CI, and what it
    finds enters the gate as fixed corpus cases.

## 5. Outcomes and the scoreboard

Every case on every kernel ends in exactly one of:

- **correct**: all applicable layers pass;
- **refused**: the kernel returned a typed error. Acceptable: a kernel
  that says "I cannot do this" is safe;
- **undecided**: the evidence falls inside the declared tolerance band
  (section 8);
- **wrong**: the kernel returned a solid that fails a layer, or panicked.

The scoreboard is a committed file per kernel listing every case and its
outcome. It works as a ratchet: **wrong must be zero**, and a case may
only move toward correct. CI fails on any regression. A kernel version
bump is accepted by re-running the scoreboard. (Since slice 4, a
candidate kernel's scoreboard may record a fuzzer finding against it as
wrong: the gate keeps it visible and fails on any wrong it does not
record. Wrong must be zero for the kernel that ships.)

## 6. What the trait needs

The harness sees a kernel only through `SolidKernel`. Today that is
`evaluate`, `mass`, `bbox` and `check`. The layers need more, each as an
optional capability a kernel declares:

| Capability | Used by |
|---|---|
| tessellate to a tolerance | integral identities, raster comparison |
| enumerate faces, edges, vertices and their adjacency | entity consistency, Euler characteristic |
| evaluate a face surface or an edge curve at a parameter | boundary shell test, entity consistency |
| classify a point as inside, on or outside | algebra checks without a mesh |
| planar section, projection | sections and projection |

A missing capability makes the layers that need it **not applicable** for
that kernel. It is never counted as a pass, and the scoreboard shows it.

Rung 3D1b added four optional capabilities that no layer uses, so the
scoreboards do not see them: `evaluate_from` (continue from an evaluated
prefix; default: from scratch), `bbox_within` (a box tight to a stated
tolerance where an exact one does not exist; default: the exact `bbox`,
which the layers keep demanding), `distances` and `interferences` (between
placed solids). Their evidence is `tests/capabilities.rs`, over the whole
catalogue on arris: every evaluated prefix continued gives the
from-scratch volume, centroid, box and mesh, or the same refusal;
`bbox_within` equals the exact box with tolerance 0 wherever that exists,
and elsewhere lies within its tolerance of the closed-form box; a solid
against itself shares its whole volume, and moved past its own box it is
apart by at least the gap, with closest points that far apart.

## 7. Testing the harness

A harness that reports no failures is either looking at a correct kernel
or not looking hard enough. Before trusting it:

- **Broken adapters.** Wrap a real kernel so that it returns a subtly
  wrong solid: a face moved by one tolerance, a hole of the wrong radius,
  a missing face, a flipped orientation, a boolean that ignores one
  operand. The harness must flag every one. Each layer needs at least one
  mutation that only it can catch.
- **Two unrelated kernels.** Run more than one implementation. Where they
  disagree, the reference decides. Where the harness passes both on
  everything, raise the fuzzing bias until something breaks.
- **Fixed seeds.** All sampling is seeded, so a failure reproduces.

## 8. Limits

The approach has no limit on the class of shape. Its limits are these:

- **Sampling finds bugs, it does not prove their absence.** A sliver
  thinner than the sample spacing can pass the shell test. The reference
  has a distance bound, so the volume enclosure certifies whole regions
  instead of sampling them: missing or extra material larger than the
  enclosure's width cannot pass. The width is a cost limit (boundary area
  times the smallest cell the budget reaches), not a limit of principle.
- **Tolerance.** Two faces 1e-12 apart cannot be judged in floating
  point. The reference never guesses: an interval that holds zero is the
  boundary band, and the harness reports it as undecided, not correct.
  Relational checks on kernel numbers (volumes, boxes, continuity) have
  explicit, justified bands, and evidence inside them is undecided.
- **Operations with several valid answers.** Corner treatment in
  roundovers, loft interpolation and smooth blends. Unless section 3 fixes
  one definition, only properties can be checked: tangency, radius,
  passing through the sections.
- **Shared mathematics.** Where the reference must use the same formula
  as the kernel (evaluating a spline surface), both can be wrong together.
  The mitigations are a second, differently structured implementation and
  the comparison between kernels.
- **Coverage is statistical.** Fuzzing explores an infinite space. Only
  exact arithmetic gives guarantees, and that is practical only for planes
  and simple curved surfaces.
- **Field-defined shapes** (topology optimisation, lattices). Their
  natural form is a function of position, which is what the reference
  already is. The harness can check a mesh against the field directly.
  Representing such shapes with exact surfaces is the kernel's limit, not
  the harness's.

## 9. Build order

Each slice lands with its broken-adapter tests and a scoreboard.

1. **Reference semantics for the features 3D0a evaluates** (extrude, hole,
   boolean, chamfer), the outcome model, the scoreboard file, and the
   layers that need no new capability: closed forms, algebra and motion,
   continuity sweeps, equivalent constructions. Run on the reference
   backend.
2. **Tessellation and point classification** as capabilities, then the
   integral identities and the boundary shell test.
3. **Topology and geometry access**, then entity consistency.
4. **A second kernel** behind the trait, and the first fuzzing runs.
5. **Curved features** as they arrive on the ladder: revolve, roundover,
   sweep, loft, each starting with its line in the section 3 table.
6. **Sections and projection** with 3D3a and 3D4a.

After slice 4 the scoreboards for the candidates exist, and the kernel
decision in `3d-design.md` section 5 can be made with them.

### Slices built (2026-11-18)

Slice 1 and the tessellation half of slice 2, in the crate
`crates/solid-harness` (`publish = false`; no shipped crate depends on
it, and cadview-core links no kernel any more).

- **Kernel boundary.** The arris adapter moved from core to
  `solid-harness/src/kernels/arris.rs`. `SolidKernel` gained one optional
  capability, `tessellate(brep, chord) -> TriMesh` (positions and
  outward triangles), whose default returns `Unsupported`; the harness
  reads that as not applicable. Point classification is not a capability
  yet.
- **Reference** (`src/reference/`): the definitions in the section 3
  table, certified in interval arithmetic (`src/interval.rs`, no
  dependency). Convex polygons and circles have closed-form distances;
  other profile loops use segment distances plus a winding number.
- **Layers built:** 1 closed forms, 1b certified enclosure, 2 shell test,
  4 mesh invariants (closed and manifold, zero vector area, Euler
  characteristic, mesh volume within the chord's Steiner tube of the
  stated volume at chords 1e-1 to 1e-4), 5 equivalent constructions, 6
  algebra and motion (`A + A`, `A n A`, `A - A`, operand order, union
  identity, rigid motion, mirroring, plane split), 7 continuity sweeps.
  Outcomes and the ratchet are in `src/outcome.rs` and
  `src/scoreboard.rs`; arris's scoreboard is `scoreboards/arris.txt`.
- **Catalogue:** 88 solids (the 9 corpus parts, 8 more with or without
  closed forms, 15 more for 12 equivalent pairs, 56 derived for 47
  algebra and motion relations), 59 relations and 6 sweeps.
- **What it found on arris 0.11.1:** no wrong case. 134 cases correct, 16
  refused (`A - A` is refused as "no thickness" on all 7 bases; the hole
  breakout and two-tangent-holes sweeps refuse steps within 1e-7 of
  tangency), 3 undecided (three flush-contact sweeps whose volume moves
  within the 1e-6 mm band at steps of 1e-7 and below, the kernel's own
  tolerance). The shell test certified every step it took on every
  part: no step was left in the band.
- **What it found in our own code:** the arris adapter drilled holes from
  `max(dia, 1)` mm behind the entry point, which opens a hole whose entry
  lies inside the material to the surface. Writing the hole's exact
  meaning down made that visible (case "hole entering below the
  surface", wrong on three layers); the adapter now drills from the
  entry point.
- **Broken adapters** (`src/kernels/broken.rs`, `tests/mutants.rs`): 11
  mutants (tool shifted 0.01 mm, hole 0.02 mm wider, last feature
  dropped, last op swapped, closed cavities dropped, tools snapped to a
  0.01 mm grid, box ignoring rotation, volume scaled by 1 + 1e-6, and a
  dropped, flipped or moved mesh triangle or vertex). All are caught.
  Only the mesh layer catches a dropped triangle and only the algebra
  layer the box that ignores rotation; no mutant is caught by the shell
  test alone or by the enclosure alone, because the relational layers
  compare centroids and the Euler characteristic sees a lost cavity.
- **Not built yet:** point classification as a capability, topology and
  geometry access and layer 3, layers 8 and 9, curved features. The
  enclosure is wide where boxes fit the solid badly (rotated unions,
  small curved parts: up to 65 % relative width at the default budget),
  so it catches missing or extra features there, not small shifts.

### Slice 4 built (2026-11-18): a second kernel and fuzzing

- **Second kernel: cad-kernel `=0.15.0`** (crates.io, from the geomsolver
  repository; `MIT OR Apache-2.0`, no dependencies, one entry in
  `Cargo.lock`), adapter `src/kernels/cad_kernel.rs`, the only file that
  names it. Same feature subset as arris. It computes no centroid, so
  `MassProps::centroid` became an `Option` (core's `kernel.rs`) and the
  centroid comparison of layers 5 and 6 is not applicable for it. Its
  chamfer is built from the kernel's per-edge flat (`fillet::flat_along`)
  and a cut, edge by edge, and refuses where one flat cuts away another
  picked edge (no multi-edge chamfer exists in the kernel).
- **Scoreboards for every kernel.** `tests/scoreboard.rs` runs every
  kernel that has a file in `scoreboards/` and fails on a file no test
  runs. The kernel's own validity check became a column of its own
  (`valid`) on every part; before, only parts with a closed form had it.
- **Recorded findings.** A wrong verdict that the committed file records
  as wrong is kept and printed, not failed: the fuzzer's findings enter
  the catalogue as named cases (`cases.rs`, "Fuzzer findings") and their
  wrong verdicts stay visible in the scoreboard until the kernel is
  fixed. Any wrong verdict the file does not record, and any move away
  from correct, still fails. A case recorded wrong that is still wrong
  stays the recorded finding even if other columns say so: arris returns
  a solid its own checker rejects in a release build and panics on the
  same recipe in a debug build (its checker runs on every operation
  there). A kernel that ships must have no recorded wrong case. (This
  replaces "wrong must be zero" in section 5 for candidate kernels.)
- **Fuzzer** (layer 10): `src/fuzz.rs` (generator, judge, shrinker,
  printer) and the driver `examples/fuzz.rs`, a development-time tool,
  never run by `cargo test` or CI. A case is reproducible from (seed,
  index). Judging needs no closed form: enclosure, shell test, mesh
  invariants, the kernel's validity, and between two kernels their
  volumes and sampled membership; a disagreement neither kernel's layers
  decide gets a forty times finer enclosure. Batches run in child
  processes so a hang costs one case. Planted bugs are found (a hole
  0.02 mm too wide on arris: 112 of 297 cases wrong; tool positions
  snapped to 0.01 mm on cad-kernel: 143 of 298).
- **What it found** is in `docs/research/kernel-comparison.md`, with the
  smallest recipes: 25 of them are now catalogue parts (113 in all), on
  which arris has 11 recorded wrong cases and cad-kernel 14. Neither has
  a wrong case on the 153 rows from slice 1. In our own code: the cad-kernel adapter skipped a
  picked edge that an earlier flat had cut away, which is wrong at a
  convex corner that turns less than the chamfer (now a refusal).
- **Limits found.** The harness has no time limit on a kernel call, so a
  case on which a kernel hangs (cad-kernel's mesher, several found)
  cannot enter the catalogue; the fuzzer records it. The enclosure has
  no positional band: a kernel that moves a face by less than its own
  tolerance (well under the 1e-6 mm band of section 8) is wrong there
  when the volume moves by more than 1e-9 relative. Whether that should
  be undecided, as it is in the continuity layer, is open (section 11).

### After the kernel decision (2026-11-18): the arris adapter refuses what arris itself rejects

The ladder continues on arris (3d-design.md section 5), so its recorded
wrong cases have to go before it ships. Eight of the eleven were results
arris can detect itself; the adapter now turns them into refusals:

- **The kernel's checker, on every operation.** arris is built with its
  `paranoid` feature, so a release build checks each operation's output
  and returns an error for one its checker rejects. A debug build panics
  there instead; the adapter catches that one panic (by its message) and
  returns the same refusal, so both builds give the same scoreboard. The
  whole result must also pass the full check level. Five cases moved
  from wrong to refused.
- **Closed meshes only.** The adapter refuses a tessellation that is not
  a closed, consistently oriented surface. Three cases moved from wrong
  to refused (two slivers, one prism far from the origin).
- **Cost:** none measurable on the catalogue (kernel run 7.9 s against
  7.2 s). No row from before the fuzzer changed.
- **Left: three wrong cases, all on the enclosure layer**, each a
  position error under 1e-6 mm: two faces merged at 1e-9 and 1e-7 mm,
  and rounding on a small box 2.3e6 mm from the origin. They are the
  open question below (a positional band for the enclosure), not
  something the adapter can detect.

### Slice 5 built (2026-11-18, rung 3D0b): curved features and a positional band

- **A positional band for the enclosure** (closes the open question of
  section 11). A volume outside the certified enclosure (plus the `1e-9`
  relative slack) by at most `A * BAND` is now undecided, further out is
  wrong. `BAND = 1e-6` mm is the band of section 8 and of the continuity
  layer. `A` is a certified upper bound on the boundary's area from the
  recipe alone, never from a kernel: the boundary of a Boolean result lies
  in the union of its operands' boundaries, so the sum over the reference's
  leaves bounds it, each leaf's area by a closed form rounded up (a
  prism's caps and side; a cylinder's side over the part of its axis the
  part's box spans; a solid of revolution's surface by Pappus; a chamfer's
  plane by the box), and every convex leaf at most the area of the part's
  box (a point of a convex leaf's boundary in the box is on the boundary
  of their intersection, and surface area grows with convex sets). The
  argument for the band: a boundary moved by at most `BAND` along its
  normal changes the volume by at most `A * BAND` to first order. Over
  the 135 catalogue parts with an enclosure the band is a median `5e-7` of
  the volume and at most `4e-4` for parts with a volume, except a sliver
  of `8e-7` mm^3 (fuzzer case s4 i801), where it is 33 times the volume:
  a part thinner than the band cannot be judged by volume at all. The
  scoreboard run prints the table (`band_table`).
- **What moved:** three cases on arris (all its recorded wrong cases:
  face merges at 1e-9 and 1e-7 mm, rounding 2.3e6 mm from the origin) and
  three on cad-kernel, from wrong to undecided. No other row changed.
  One cad-kernel case stays wrong: a corner box far from the origin whose
  volume is off by 3.5e-7 mm^3 against a band of 1.9e-7. arris now has no
  recorded wrong case.
- **Curved features in both adapters.** arris: `ops::revolve`,
  `ops::loft`, `ops::sweep` (rotation-minimising frame) and `ops::fillet`.
  cad-kernel: `build::revolve`, a ruled loft of two line sections as
  bilinear sheets (`build::lofted`), a sweep of lines and arcs as the union
  of a prism per line and a revolve per arc (the kernel has no general
  sweep), and `rounds::rounded`. Its adapter now computes a tight box for
  sphere, torus, bilinear-sheet and degree-one B-spline geometry too (the
  candidate extremes, kept when the kernel's own classification puts them
  on the boundary). Tapered extrudes stay unsupported in both.
- **A curved corpus** in core (`corpus/curved.rs`): a slab with all 12
  edges rounded, a tapered square bar (loft), a revolved profile with a
  bead and a cove, and a round bar swept along a line, an arc and a line.
  Both kernels build all four as valid single solids with closed meshes,
  every volume within 2e-15 relative of the hand-derived closed form and
  every box within 3e-14 mm (gate: 1e-6).
- **Reference meanings** (section 3) for revolve (full turn and wedge),
  loft (two parallel convex sections with parallel sides: a convex
  polytope), sweep (lines and arcs: the union of prisms and revolves) and
  fillet (all edges of a convex prism: the prism opened by a ball). All
  are certified: sign-exact, 1-Lipschitz, interval-evaluated. Lofts and
  rounds outside those cases are "not defined" (see section 11).
- **Catalogue:** 217 rows per kernel (137 parts, 73 relations, 7 sweeps):
  the 4 corpus parts, 2 wedge revolves, 4 equivalent pairs (revolved
  rectangle against extruded annulus, loft of equal sections against an
  extrude, sweep along a line against an extrude, sweep along a quarter
  arc against a quarter revolve), motions of five curved parts, mirrors
  of two revolves, `A + A`, `A n A`, `A - A` of the revolved profile, and
  one continuity sweep through a curved tangency (a box face sliding onto
  a revolved cylinder).
- **Mutants:** 14 (three new: every fillet radius 1e-3 mm larger, every
  revolve axis moved 0.01 mm, the last loft section scaled by 1.001), all
  caught, the band in place. The fillet radius is caught only by the
  closed form (the rounded slab, 2.6e-6 relative): the surface moves by
  at most 4e-4 mm, under the shell test's 1e-3 step, and the enclosure is
  too wide on a rounded part. The axis shift is caught by the closed form,
  the shell test, the mesh layer (the shifted axis opens a tube: Euler
  characteristic 0) and an equivalent pair; the loft scale by the closed
  form, the enclosure, the shell test and the loft-against-extrude pair.
  Every earlier mutant is still caught; the mesh layer alone still
  catches a dropped triangle and the algebra layer alone the box that
  ignores rotation, and "volume scaled by 1 + 1e-6" is still wrong on
  the enclosure layer on 14 parts (closed form 44). The
  run now takes 54 minutes in release, most of it tessellating curved
  parts at chord 1e-4.
- **What it found on arris 0.11.1:** nothing wrong. Every curved row is
  correct on every layer; `A - A` of the revolved profile is refused, as
  for the other bases.
- **What it found on cad-kernel 0.15.0:** its mesher breaks the chord
  contract on torus faces at the finest chord. On a quarter torus the
  worst triangle centroid lies 9.5e-3, 9.9e-4 and 1.3e-2 mm off the exact
  torus at chords 1e-2, 1e-3 and 1e-4 (the test
  `kernels::cad_kernel::forward_look::torus_mesh_against_its_chord`):
  finer than 1e-3 it gets worse, not better. The shell test (steps of
  1e-3 off that mesh) and the mesh layer's volume convergence catch it on
  10 rows (every part with a torus face, and the parts derived from them).
  They are recorded as findings against cad-kernel. Its B-reps and
  volumes are right (closed form and enclosure correct on all of them).
- **What it found in our own code:** no bug in the adapters or the
  reference. One property to know: a point on a revolve's axis is in the
  boundary band when the profile runs along the axis (the axis segment is
  part of the meridian's boundary), so the enclosure never certifies the
  thin column of cells along it.

### After slice 5 (2026-11-18): the adapter ships, the gates are fast again

- **The arris adapter moved to a shipped crate**, `crates/cadview-solid`
  (MIT, compiles for wasm32, the only crate that names arris). Its
  behaviour is unchanged: the exact pin, default features off,
  `paranoid`, the refusal of results arris's checker rejects (the debug
  panic turned into the same refusal), the refusal of open meshes. The
  closed-mesh check it uses is now kernel-neutral code in cadview-core
  (`solid/mesh.rs`, `topology`), and the harness's mesh layer uses the
  same function. The harness tests the adapter from cadview-solid. Both
  scoreboard files came out byte-identical after the move. A new test
  (`shipped_kernels_record_no_wrong_case`) fails if the committed
  scoreboard of a shipped kernel (arris) records any wrong case.
  cad-kernel and the broken wrappers stay in the harness.
- **Where the time went** (release, one thread, arris): 307 s for the
  catalogue. Tessellation at chord 1e-4 took 164 s, 131 s of it on the
  revolved profile with a bead and a cove and its four derived copies
  (22 to 30 s each, 2 million triangles); chord 1e-3 took 7.5 s, the
  coarser chords under 1 s. The shell test took 65 s, 57 s of it on two
  turned copies (the swept bar moved, 38 s; the rounded slab moved,
  18 s): the mesh grid kept a long triangle in every cell of its box, and
  turned against the axes that box is most of the part. The mesh layer
  took 17 s, the relations 45 s (winding numbers on the 1e-3 meshes),
  evaluation 4.5 s, the sweeps 1.8 s, and the enclosures 15 s per test.
  cad-kernel: 178 s, of it tessellation 37 s (its mesher has its own
  threads), shell 61 s on the same two parts, mesh 18 s. The mutant gate
  ran the whole arris catalogue 15 times (once unbroken, once per
  mutant), most of it tessellating parts the mutation does not touch.
- **What changed.** Nothing is dropped or coarsened: the same parts,
  layers, chords, samples and tolerances.
  - *Parallel runs.* Parts, relations, sweep steps and enclosures run on
    `std::thread::scope` threads (`run::par_map`, one per hardware thread,
    `SOLID_HARNESS_THREADS` to change it, `1` for serial), the slow parts
    started first, the results in catalogue order. Each item is a pure
    function of its input: arris keeps no process-wide state and starts
    no threads; every sample is seeded from the case name. cad-kernel
    keeps per-thread warm starts and a process-wide index of B-spline
    nets, both keyed by address; by its own documentation they change how
    fast it answers, not what (it clears the warm starts at each of its
    own parallel items and Booleans). Checked: both scoreboard files come
    out byte-identical with 24 threads, with 5 and serially.
  - *The mesh grid* keeps a triangle only in the cells it comes near (the
    cell's circumscribed ball meets the triangle, with slack for
    rounding), so a query sees the same triangles that matter and gives
    the same distance, bit for bit (a test compares it with a scan of
    every triangle). The shell test went from 65 s to 15 s on one thread.
  - *Mutants run only what they change* (`Broken::run_against`). A part
    whose mutated recipe equals the original, tools included, gets the
    same input on the inner kernel, so its B-rep is the unbroken run's.
    When the wrapper changes no output for it, its run and verdicts are
    taken over. When the wrapper changes the mass or a mesh, the B-rep and
    meshes are taken over, mass, box and check are asked again through
    the wrapper, each mesh goes through the wrapper's mesh mutation (what
    its `tessellate` does to the inner kernel's mesh), and the verdicts
    are computed again. A relation is taken over when every part it names
    was, and the wrapper does not change the box a motion asks for; a
    sweep when every step's recipe is unchanged and the wrapper does not
    change volumes.
- **Evidence unchanged.** The mutant table is the same before and after:
  every mutant caught by the same layers on the same number of cases.
- **Timings** on a 24-thread machine (release, then debug):

  | Gate | Before | After |
  |---|---|---|
  | scoreboard, both kernels, release | 262 s | 50 s (arris 50 s, cad-kernel 25 s) |
  | mutants, release | 54 min (66 min in a run beside other builds) | 4.1 to 4.4 min |
  | scoreboard, debug | not measured | 99 s |
  | mutants, debug | not measured | 8.4 min |
  | lib tests, debug | | 23 s |
  | corpus, debug | | 1.2 s |

  The slowest single part now sets the pace: the revolved profile
  intersected with itself takes about 30 s to tessellate at 1e-4 on one
  thread, and arris tessellates on one thread.

## 10. Effect on the ladder

- 3D0a's gate (the closed-form corpus) stays and becomes layer 1.
- 3D0b's "second kernel as a differential check" becomes slice 4 here.
  The second kernel no longer has to be Truck.
- Later rungs that add a feature add its reference meaning and its cases
  to the harness as part of their gate.

## 11. Decisions and open questions

Decided:

- **The harness is its own crate** that depends on core, not part of
  core's tests. It holds the reference, the layers, the scoreboards and
  the adapters for the candidate kernels, so core links no kernel at all
  until one is chosen. It is a test tool and is never a dependency of a
  shipped crate.
- **Fuzzing is a development-time tool only.** It does not run in CI; a
  case it finds is shrunk and added to the corpus, which is what the gate
  runs.
- **The reference is certified from the start.** It returns inside,
  outside or boundary band with a guaranteed distance lower bound, built
  from sign-exact 1-Lipschitz functions and evaluated in outward-rounded
  interval arithmetic. A band answer is undecided, never correct. (This
  replaces "plain floating point first, intervals later".)
- **The certified volume enclosure is a layer** (1b), on every part that
  has a reference.
- **No adapter evaluates a feature without its exact reference
  definition.** Property-only checks only where an exact definition is
  impossible, stated as such. Sweep and loft get their definitions when
  they are first implemented.

- **A positional band for the enclosure** (3D0b): a volume outside the
  certified enclosure by at most `A * 1e-6` mm, `A` a certified area
  bound from the recipe, is undecided; further out is wrong (layer 1b
  above, slice 5).
- **Curved features get a reference only where it is exact** (3D0b):
  revolve (any profile on one side of its axis, any angle), sweep along
  lines and arcs (any profile), loft only between two parallel convex
  polygons with parallel sides, fillet only on all edges of a convex
  extrude directly after it. Everything else is "not defined", not
  approximated.

Open:

- **What the curved references do not cover yet**, and what each would
  need:
  - *Fillet on a subset of edges, or after other features.* The rolling
    ball's point set is still "not reached by a ball of radius `r` inside
    the solid near a selected edge", but near the ends of a selected
    chain and at corners where selected and unselected edges meet, the
    kernels' blends are a choice (section 8), not a consequence of the
    definition. An exact reference needs a definition of those ends
    first; the opening of the whole solid is exact only when every edge
    is rounded.
  - *Concave rounds* (the closing by a ball): exact as a set, but the
    signed distance of a closing needs the distance to the complement's
    opening, which has no closed form over our leaves yet.
  - *Lofts between non-parallel sides, more than two sections, or curved
    sections.* The ruled faces are then bilinear (or smooth) sheets; the
    exact distance to them needs a certified distance to a bilinear patch
    (a degree-4 problem per point), which is not written.
  - *Sweeps along helices or splines*, and sweeps whose arcs pass the
    section across their axis: the frame is then not a sequence of rigid
    slides and turns.
  - *Tapered extrudes* need the offset of a profile (exact for polygons,
    arcs offset to arcs), not written.
- **The enclosure is weak on curved parts** at the default budget: its
  relative width is 0.03 to 0.8 on the curved catalogue parts (a box
  subdivision fits round faces badly, and cells along a revolve axis
  never certify), so it catches a missing or extra feature there, not a
  small error. The shell test and the closed forms carry those parts.
- **A time limit on kernel calls**, so that a hang can be a recorded
  outcome in the catalogue rather than only a fuzzer note.
