# 3D test harness: design

Status: accepted 2026-11-18, after 3D0a. Being built slice by slice
(section 9); nothing below is implemented unless a slice note says so.

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
function `inside(p)` and where possible a distance bound. It is slow and
simple, and it shares no code with any kernel. It is the oracle.

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
2. **Boundary shell test.** Sample points on every face of the kernel's
   result. Step a small distance inside and outside along the normal and
   ask the reference. Every inside step must be inside and every outside
   step outside. This is far sharper than comparing volumes, and it
   localises a failure to a face.
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
  thinner than the sample spacing can pass. Where the reference has a
  distance bound, a grid certifies a whole region, so this becomes a cost
  limit, not a limit of principle.
- **Tolerance.** Two faces 1e-12 apart cannot be judged in floating
  point. The harness has an explicit band, and cases inside it are
  undecided, not correct.
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
- **Fuzzing in the normal test run is short and seeded** (a fixed number
  of cases). Longer runs are opt-in through an environment variable.
- **The reference starts with plain floating point.** Interval bounds are
  added when the first sliver-sized bug shows the need.

Open:

- The exact definitions of sweep and loft in section 3. They are settled
  when those features are first evaluated.
- Whether longer fuzzing runs happen on a schedule in CI.
