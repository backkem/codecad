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

**No kernel adapter may evaluate a feature before its reference meaning
is written down as one exact definition.** Property-only checks are
allowed only where an exact definition is impossible, and the definition
must say so. Sweep and loft get their definitions when they are first
implemented.

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
bump is accepted by re-running the scoreboard.

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
  geometry access and layer 3, layers 8 to 10, a second kernel, curved
  features. The enclosure is wide where boxes fit the solid badly
  (rotated unions, small curved parts: up to 65 % relative width at the
  default budget), so it catches missing or extra features there, not
  small shifts.

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

Open:

- Nothing at the moment.
