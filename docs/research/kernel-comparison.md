# Kernel comparison: arris 0.11.1 and cad-kernel 0.15.0

Evidence from the solid harness (docs/3d-test-harness.md, slice 4) on the
two candidate kernels behind `SolidKernel`. It does not choose a kernel;
it states what was measured, how, and what it does and does not support.
The decision taken afterwards (arris for now) is recorded in 3d-design.md
section 5.

Everything below was run on one Windows x64 machine (24 hardware
threads), release builds unless a line says otherwise, on 2026-11-18. The
commands are at the end of each section.

## 1. Summary

- **Slice-1 catalogue (153 rows): no wrong case on either kernel.** Every
  closed form matches within the layer's 1e-9 relative (the prismatic
  corpus to 5e-16 on both). arris refuses
  `A - A` (7 parts) where cad-kernel returns an empty solid; cad-kernel's
  adapter refuses a tight box around its exact B-spline ellipses (2
  parts). The planar contact sweeps are undecided on both.
- **Fuzzing, 4000 cases per kernel in the region near contacts and far
  from the origin** (final run): arris 55.9 % correct, 43.4 % refused,
  0.71 % wrong, no hang; cad-kernel 78.1 % correct, 15.0 % refused, 6.2 %
  wrong, 0.68 % hang (the 7 timed were all in its mesher), 1 panic. Of
  cad-kernel's 246 wrong cases 166 are open meshes where the enclosure
  and the shell test find nothing wrong.
- **Both kernels have recorded wrong cases** in the catalogue now (11 on
  arris, 14 on cad-kernel), all fuzzer findings: volume precision far
  from the origin (both; up to 91 % on cad-kernel), faces within 1e-9 to
  1e-7 mm merged (both, identically), results their own checkers reject
  (both), open meshes (both), wrong shells (both), a panic (cad-kernel).
- **Where they disagree, the reference usually cannot say who is right:**
  209 of 260 disagreements are on small, thin or turned parts where the
  enclosure is wide and the 1e-3 mm shell step too coarse. Where it can
  decide, cad-kernel is wrong 37 times, arris 5, both 9.
- **Capabilities for the ladder differ:** arris has general sweeps,
  sections, edge projection, exact ray hits, provenance and a centroid;
  cad-kernel has whole-solid offsets, shells and thickening, a fast
  helical sweep, and the 12-edge rounded board, but no section,
  projection, ray, edge provenance or centroid.
- **Footprint:** cad-kernel is one crate with no dependencies (one
  `Cargo.lock` entry, 60 k lines) and starts threads natively; arris is
  eight crates plus nalgebra (17 entries, 116 k lines) and starts none.
  Both are `MIT OR Apache-2.0`, pure Rust with no `unsafe`, build for
  wasm32, and are very young: arris two months and 14 releases,
  cad-kernel 17 days and 18 releases, describing itself as "not yet
  shaped for general use".
- **In our own code** the fuzzer found a bug in the new cad-kernel adapter
  (a chamfer skipped a picked edge that an earlier flat had cut away);
  it now refuses. The arris adapter needed no change.

## 2. The kernels

| | arris | cad-kernel |
|---|---|---|
| Version pinned | `=0.11.1` (latest on crates.io) | `=0.15.0` (latest on crates.io) |
| Licence | `MIT OR Apache-2.0` (every crate) | `MIT OR Apache-2.0`; its `util::fmath` follows the FreeBSD/musl algorithms (MIT) |
| Repository | github.com/arris-labs/arris | github.com/bmander/geomsolver (`rust/cad-kernel/`, synced to its own repository) |
| Stated origin | "Open CASCADE is used solely as the differential test oracle ... Its source is read, never copied" (PROVENANCE.md) | "clean-room: Open CASCADE's behaviour and conventions were studied, never its code" (README); built inside geomsolver, a constraint solver, "held to Open CASCADE as an oracle" |
| Status it states | pre-1.0, API breaks between minor versions | "extracted, not yet shaped for general use"; a 0.x minor may break the API |
| Age, cadence | first release 2026-09-17; 14 releases in 2 months | first release 2026-11-01; 18 releases in 17 days (0.14.0 on 11-14, 0.15.0 on 11-17) |
| Crates, lines of Rust | 8 crates, 116 298 lines in `src/` (geom 45 361, ops 39 104, io 12 231, topo 7 085, check 5 292, mesh 3 919, math 3 186) | 1 crate, 59 703 lines in `src/` |
| Packages added to `Cargo.lock` | 17 (the 8 arris crates, nalgebra 0.35, simba, wide, safe_arch, robust, four glam versions locked but not built; all MIT, Apache-2.0, BSD or Zlib) | 1 (`cad-kernel` itself; it has no dependencies) |
| `unsafe` | none (`forbid(unsafe_code)`) | none (no occurrence in `src/`) |
| Threads, process state | none with default features off; no process-wide state | starts OS threads natively (`std::thread::scope` in `util::par`, one per core, per parallel map); one thread on wasm32. Process-wide: a mutex-guarded cache of B-spline net indexes (cleared at 256 entries), two environment switches read once (`SOLVENT_BREP_DEBUG`, `SOLVENT_BREP_TIME`), thread-local caches |
| wasm32 | builds (`cargo build -p arris --target wasm32-unknown-unknown --release`, 42.8 s); its README says wasm32 output is byte-identical to native | builds (`cargo build -p cad-kernel --target wasm32-unknown-unknown --release`, 46.0 s); its CI builds wasm32 and its README says native and wasm builds give identical bits |
| First build (this machine) | 25 s first `cargo check` of core with arris, 46 s first debug test build (3D0a note) | 30.3 s debug, 45.8 s release, 46.0 s release for wasm32 (one crate) |

No package other than `cad-kernel` entered `Cargo.lock` with this work
(806 entries before, 807 after).

## 3. How each feature maps

Both adapters implement the same subset and refuse the rest with a typed
`EvalError` (`Unsupported` for revolve, sweep, loft, fillet, tapered
extrude, asymmetric chamfer, `ToolRef::Part`, spline profile segments).

| Feature | arris (`src/kernels/arris.rs`) | cad-kernel (`src/kernels/cad_kernel.rs`) |
|---|---|---|
| Extrude | `geom::Profile` (lines, arcs, ellipses, circles), `ops::extrude` | `build::Profile` in 3D (lines, circular arcs on a frame in the plane, elliptic arcs as exact rational quadratic B-splines), `build::prism` from `min(0, d)` to `max(0, d)` |
| Hole | `ops::primitive_cylinder` from the entry point, `ops::cut`; through: past the part's tight box | `build::prism` of `build::disc` from the entry point, `boolean::combined(.., Cut, 0)`; through: past `Brep::bounds` |
| Boolean, inline tool | tool evaluated into the same model, `ops::transform`, `fuse`/`cut`/`common` | tool evaluated on its own, `Brep::moved`, `combined` with `Union`/`Cut`/`Common` at the kernel's own tolerance (1e-9 of the operands' size). Not `combined_written`, which first moves a tool's planar faces that are "all but flush" onto the solid's |
| Chamfer (`d1 == d2`) | `ops::chamfer` on the edges `select.rs` picks (mitred corners) | no multi-edge chamfer in the kernel: per picked edge, `fillet::flat_along` (legs `d`, `d`) and a cut, one edge after another; refused when one flat cuts away another picked edge, or the edge is concave |
| Volume | `ops::measure::mass_properties` (flux over exact faces) | `props::volume` (Green's theorem over exact faces) |
| Centroid | from `mass_properties` | none in the kernel: `MassProps::centroid` is `None`, and the harness's centroid comparisons are not applicable |
| Box | computed in the adapter from exact vertices, edge curves, sphere and torus extremes; NURBS refused | `Brep::bounds` of the moved B-rep, accepted only when every edge is a line, circle or ellipse and every face a plane, cylinder or cone (elsewhere it is only conservative); otherwise refused |
| Validity | `check::check` at `Level::Full`, `check::lumps` | `Brep::check` at ten times the kernel's tolerance; solids are the face components with positive volume |
| Tessellation | `mesh::tessellate(chord)` | `mesh::mesh(chord, 0.5 rad)` |

Faces of every cad-kernel primitive are named under a key of their own
(`p<n>.<face>`), as its own front end names them: its flat and fillet
code reads two faces of one name as pieces of one face and refused the
chamfered slab ("two canals meeting in a mitre") before this.

The only change outside the harness: `MassProps::centroid` in
`cadview-core/src/solid/kernel.rs` is now `Option<Point3>`.

## 4. Scoreboards

The catalogue is 113 parts (88 from slice 1, 25 fuzzer findings), 59
relations and 6 sweeps: 178 rows per kernel. The committed files are
`crates/solid-harness/scoreboards/arris.txt` and `cad-kernel.txt`.

| Layer | arris 0.11.1 | cad-kernel 0.15.0 |
|---|---|---|
| closed (30 parts with a closed form) | 30 correct | 28 correct, 2 refused |
| enclosure | 96 correct, 12 refused, 3 WRONG | 104 correct, 1 refused, 6 WRONG |
| shell | 96 correct, 14 refused, 1 WRONG | 100 correct, 1 refused, 3 WRONG |
| mesh | 95 correct, 15 refused, 3 WRONG | 99 correct, 1 refused, 6 WRONG |
| valid (the kernel's own check) | 96 correct, 12 refused, 5 WRONG | 109 correct, 1 refused, 3 WRONG |
| equiv | 12 correct | 12 correct |
| algebra | 40 correct, 7 refused | 41 correct, 6 refused |
| continuity | 1 correct, 2 refused, 3 undecided | 2 correct, 1 refused, 3 undecided |
| **overall** | **143 correct, 21 refused, 3 undecided, 11 WRONG** | **151 correct, 10 refused, 3 undecided, 14 WRONG** |

On the 153 rows from before the fuzzer, neither kernel has a wrong case:
arris 134 correct, 16 refused, 3 undecided; cad-kernel 141 correct, 9
refused, 3 undecided. Every wrong case is a fuzzer finding.

**Not correct on arris** (each fuzzer recipe is in `src/cases.rs`, "Fuzzer
findings"; the full recipe is one line there):

- refused, `A - A` on 7 parts ("the result has no thickness"), and the 7
  `A - A = empty` relations that use them.
- refused, sweeps "hole breaking out" and "two holes through tangency"
  (7 steps each within 1e-7 of tangency); undecided, the three planar
  contact sweeps (section 6).
- WRONG, small box far from the origin (s1 i34): a 0.0135 x 0.3 x 50
  box at x = 2 285 813 mm; volume 0.2026571097, 1.9e-9 above the
  certified enclosure [0.2026558918, 0.2026571077] (9.6e-9 relative).
- WRONG, tiny box with a face 1e-9 off (s4 i20): 6.000000030e-5 against
  at most 6.0000000000e-5 (5e-9 relative; cad-kernel too).
- WRONG, intersection 1e-7 above a face (s3 i114): exact 263.9999868,
  arris 263.9999956 (cad-kernel the same).
- WRONG (own check rejects the result): through hole tangent to a side
  face (s1 i196, "loop 0 intersects itself"); pin 1.1e-6 inside a side
  face (s2 i463, "loops 0 and 1 intersect"); 0.1 x 0.001 x 0.0001 notch
  1e-7 inside a face (s4 i733, "pcurves jump 1e-7"); a hole whose entry
  disc lies on the bottom face pointing out (s3 i353); a cylinder
  tangent to two sides inside a box corner, fused (s14 i325, "loops 0
  and 1 intersect"). In a debug build arris panics on these instead
  ("kernel bug: an operation's output fails the checker").
- WRONG (open mesh): box cut 1e-6 off a cylinder's tangent line (s2
  i179, 6 bad directed edges at every chord); cylinder cut by a box
  turned 1e-4 rad (s12 i128, 2 at chord 0.1).
- WRONG (shell and open mesh): a prism with collinear vertices notched
  1e-7 off its end, 3.1e6 mm from the origin (s3 i966, 46 steps on the
  wrong side).
- refused, 5 other findings ("the faces touch along a curve interior to
  both result faces"; "kernel bug: the face arrangement is not a
  subdivision").

**Not correct on cad-kernel:**

- refused, `ellipse` and `half ellipse` on the closed-form layer: the
  adapter refuses a tight box around a B-spline edge (the elliptic arcs
  are exact rational B-splines; the kernel's box there is the control
  hull), and the 6 algebra relations on the ellipse that compare boxes.
  Their volumes, shells and meshes are correct.
- refused, sweep "two holes through tangency" (one step, `s = 1e-6`);
  undecided, the three planar contact sweeps (section 6).
- refused, heptagon chamfered on both caps (s4 i218): the adapter
  refuses (section 3; this was an adapter bug before).
- WRONG, open mesh (2 bad directed edges at every chord): two near
  tangent blind holes breaking out of a face (s1 i24) and exactly
  tangent ones (s4 i0). arris refuses both.
- WRONG, volume outside the enclosure: tiny box with a face 1e-9 off (s4
  i20), intersection 1e-7 above a face (s3 i114), both as arris; a 0.02
  thick plate intersected 1e-3 up, 7.3e4 mm away (s2 i101, 3.8190000324
  against at most 3.8190000000, 8.5e-9 relative); a 0.2 x 0.02 x 0.2
  corner box 1e6 mm away (s13 i455, 8.0035e-4 against at most
  8.0000e-4: 4.4e-4 relative); a thin rod with a hole along its corner,
  7e6 mm away (s1 i857, volume 0.00501 where its own mesh and arris say
  0.0529: 91 % low).
- WRONG, volume against its own mesh beyond the chord's bound: turned
  thin intersection 1e-7 off a side (s3 i547, 3.7255 against mesh
  3.7479); turned plate at the top of a tall box (s11 i337, 4.7853
  against mesh 4.7305 and arris 4.7304: 1.1e-2 relative).
- WRONG, shell: blind hole leaving a 1e-7 floor (s2 i152, 226 steps on
  the wrong side); side hole along a thin prism (s11 i61, 26 steps;
  arris refuses).
- WRONG (own check rejects the result): end of a long bar with a side
  hole (s2 i300, a vertex 1e-6 off its curve); small washer and turned
  cylinder (s4 i801, a pcurve 1.2e-9 off its B-spline edge).
- WRONG, panic: a cylinder turned 1e-9 rad fused to a box (s2 i730,
  "a vertex at [-5e-8, 0, -5.0e12]", `tolerant.rs:90`), in release and
  debug builds.

## 5. Where the kernels disagree

On the 88 catalogue parts that predate the fuzzer (`examples/compare.rs`)
the two kernels agree on volume (1e-9 relative) and sampled membership
wherever both answer. They differ only on `A - A` (7 parts): arris
refuses ("the result has no thickness"), cad-kernel returns an empty
B-rep of volume 0. Both are acceptable; the reference's enclosure for
those parts is exactly 0.

Under fuzzing they disagree far more often; section 7 has the numbers and
who the reference says is wrong.

## 6. The hard region: the continuity sweeps

Each kernel's volume minus the exact volume of the recipe at each step
(`examples/compare.rs`; the exact volumes are closed forms: circular
segments and lenses for the cylinders). A contact sits at `s = 0`.
Values of a few 1e-10 are rounding of a 3e6 mm^3 volume.

**Box tool through a flush top face** (600 x 300 x 18 board, 20 x 20
tool, `V = V0 - 400 max(s, 0)`):

| s | arris | cad-kernel |
|---|---|---|
| -1 to 0 | +4.7e-10 | -1.4e-9 |
| 1e-9 | +4.0e-7 | +4.0e-7 |
| 1e-8 | +4.0e-6 | +4.0e-6 |
| 1e-7 | +4.7e-10 | +4.0e-5 |
| 1e-6 | 0 | +2.7e-4 |
| 1e-5 to 1 | below 1e-9 | below 2e-9 |

Both snap a cut of 1e-9 and 1e-8 mm to "no cut" (+400 s). arris cuts
from 1e-7 on (its linear tolerance is 1e-7 mm); cad-kernel snaps 1e-7 too
(its tolerance here is 1e-9 of the operands' size, 6.7e-7 mm), and at
1e-6 it is neither snapped nor exact: it removes a third of the slab.

**Dado tool through a flush end face** (`V = V0 - 1800 (18 - max(s, 0))`):

| s | arris | cad-kernel |
|---|---|---|
| -1 to -1e-5 | below 1e-9 | below 2e-9 |
| -1e-6 | +9.3e-10 | -2.4e-3 |
| -1e-7 | -2.1e-4 | -3.0e-4 |
| -1e-8 | -2.1e-5 | -3.0e-5 |
| -1e-9 | -2.1e-6 | -3.0e-6 |
| 0 | +9.3e-10 | 0 |
| 1e-9 | +3.0e-7 | +1.2e-6 |
| 1e-8 | +3.0e-6 | +1.2e-5 |
| 1e-7 | +3.0e-5 | +1.2e-4 |
| 1e-6 | +4.7e-10 | +1.2e-3 |
| 1e-5 to 1 | below 2e-9 | below 2e-9 |

Within 1e-7 of the contact both kernels are off by a constant times `s`
(arris 2100 s and 300 s; cad-kernel 3000 s and 1200 s): they move the
end face onto the board's. cad-kernel does so up to 1e-6, beyond the
harness's 1e-6 mm band edge (at `s = -1e-6` its volume is 2.4e-3 low).

**Cylinder tool through tangency with a face**: both within 3e-9
everywhere except cad-kernel at `s = 1e-6` (+2.5e-8) and 1e-7 (+2.8e-9).

**Hole breaking out through a face** (internal tangency at 0): arris
refuses every step from -1e-8 to 1e-7 (7 steps); cad-kernel answers all
21 steps within 3e-9 of the exact volume.

**Two holes through tangency**: arris refuses -1e-7 to 1e-7 (7 steps);
cad-kernel answers 20 of 21 steps within 2e-9 and refuses `s = 1e-6`
("a shell of the arrangement is kept on one side of a plane and dropped
on another").

**Fused box through face contact** (`V = V0 + 4000 - 400 max(-s, 0)`):
arris is exact at `|s| >= 1e-7` and off by 267 |s| (overlap side) and
133 s (gap side) for `|s| <= 1e-8`; cad-kernel is off by 267 |s| from
`s = -1e-6` to -1e-9 and by 133 s from 1e-9 to 1e-7, exact at 0 and
from `|s| >= 1e-5` (and at `s = 1e-6`).

In the harness's terms (layer 7, band 1e-6 mm times the part's area) all
of this is undecided for both kernels, except the refusals. The pattern:
arris refuses near tangent cylinders and snaps planar contacts within
1e-7 mm; cad-kernel answers near tangent cylinders exactly, snaps planar
contacts within its size-relative tolerance, and in some sweeps returns
intermediate volumes at 1e-6 mm.

## 7. Fuzzing

The fuzzer (`examples/fuzz.rs`, `src/fuzz.rs`) draws recipes from a base
(box, convex prism, cylinder, washer) and up to five operations (boolean
with a box or cylinder tool, hole with or without counterbore, a second
hole tangent to the first, cap chamfer), biased toward flush and
coplanar faces, shared edges, tangent and breaking-out holes, holes
ending on the far face, tools 0, 1e-12, 1e-9, 1e-7, 1e-6, 1e-5 or 1e-3
mm off a face, tools a thousandth of the base, tiny, quarter-turn and
generic rotations, and parts moved 1e4 to 1e7 mm away. Each case is
judged without a closed form (enclosure with a 10 000-cell budget, shell
test, mesh invariants, the kernel's own check) and the two kernels are
compared (volume to 1e-9 relative, sampled membership of their meshes);
a disagreement that neither kernel's layers decide gets an enclosure
with forty times the budget. A case that gives no answer in 120 s is
killed and recorded as a hang. Every case is reproducible from (seed,
index).

**The fuzzer finds planted bugs.** Against broken adapters (seeds 101 and
102, 300 cases each): arris with every hole 0.02 mm wider, 112 of 297
judged cases wrong (all on the shell test); cad-kernel with tool
positions snapped to 0.01 mm, 143 of 298 wrong (shell 122, enclosure 12,
the rest the kernel's own open meshes).

**Final run** (both adapters as committed; seeds 11 to 14, 1000 cases
each, four processes in parallel, about 30 minutes per seed):

| | arris 0.11.1 | cad-kernel 0.15.0 |
|---|---|---|
| cases | 4000 | 4000 |
| not defined by the reference (skipped) | 34 | 34 |
| correct | 2215 (55.9 %) | 3098 (78.1 %) |
| refused | 1723 (43.4 %) | 595 (15.0 %) |
| WRONG | 28 (0.71 %) | 246 (6.2 %) |
| no answer in 120 s | 0 | 27 (0.68 %) |
| panics (counted in WRONG) | 0 | 1 |
| evaluation per seed: median, p95, max | 6.7 to 8.0, 147 to 172, up to 4351 ms | 5.6 to 6.5, 32 to 38, up to 1247 ms |

(Percentages of the 3966 defined cases. The evaluation times are with
four fuzzers sharing the machine, and exclude the hangs, which are in
tessellation.)

Wrong cases by the first layer that says so: arris enclosure 14,
validity 11, mesh 3; cad-kernel mesh 173 (166 open meshes, 7 mesh volumes
off the stated volume by more than the chord's bound), enclosure 37,
shell 20, validity 15, panic 1.

The most frequent refusals: arris "the faces touch along a curve
interior to both result faces" (945), "the result has no material" (370,
an empty intersection), "kernel bug: the face arrangement is not a
subdivision" (224 in four variants), "the result's shells would touch
along an edge or at a vertex" (49), "no closed form for plane against
plane or line" (55); cad-kernel "a shell of the arrangement is kept on
one side of a plane (or cylinder) and dropped on another" (175), "a
plane's pieces do not close" (64), its mesher's "a face's boundary passes
through one of its points" (57), "a plane has a hole in no piece" (36),
"a piece of a plane touches a plane along no face of either" (69), the
adapter's "a through hole points away from the part" (33) and chamfer
refusals (11). Both kernels label some refusals as their own bugs
("kernel bug: ..." in arris, "Failed: ..." in cad-kernel); they are typed
errors, so the harness counts them as safe.

**The two kernels against each other** (final run): both answered on 2078
cases, agreeing on 1818 and disagreeing on 260; one answered on 1500
(almost always cad-kernel); neither on 361. Of the 260 disagreements the
reference finds cad-kernel wrong on 37, arris on 5 (one of them only with
the finer enclosure), both on 9, and **cannot decide 209**: 60 differ by
less than 1e-8 relative, 94 by 1e-8 to 1e-6, 47 by 1e-6 to 1e-3, 8 by
more (up to 0.29 relative, on a 4e-4 mm^3 part whose enclosure is [0,
1.4e-3]). These are small, thin or turned parts where the enclosure is
wide (relative widths 1e-3 to unbounded) and the shell test's 1e-3 mm
step cannot see the difference. The harness cannot say which kernel is
right on them.

**Hangs.** All 27 (and 11 in the first run) are cad-kernel, and in every
one timed so far (7 of them) evaluation and volume take under 20 ms and
the tessellation at chord 0.1 does not finish in 150 s (one finished
after 67 s with a refusal). The smallest recipe found (seed 4, index
195, shrunk with each try in a child process given 30 s): a 13 x 0.1 x 9
box with one blind hole, diameter 0.0415, 6 deep, its axis at x =
12.979259, so its side passes 1.5e-7 mm inside the x = 13 face:

```
box 13.0 0.1 9.0 hole 12.979259244243075 0.03 9.0 0.0 0.0 -1.0 0.04148151151185186 6.0 nocb
```

The harness has no time limit on a kernel call, so hangs are not in the
catalogue (they would hang the gate); see docs/3d-test-harness.md,
section 11.

**First run** (seeds 1 to 4, 4000 cases, before the chamfer fix): arris
2216 correct, 1741 refused, 11 wrong, no hang; cad-kernel 3151 correct,
586 refused, 220 wrong, 11 hangs, 1 panic. It found the adapter bug of
section 3 (the heptagon), most of the findings in section 4, and a
second cad-kernel panic class only in the final run (seed 12, index 799,
"a vertex at [167, 2.2e12, 8.9e12]").

**Distinct findings in the catalogue** (section 4 has the numbers): 25
recipes, each shrunk (operations dropped, numbers rounded while the
failure persists). Shrinking was stopped before it ended for some jobs
of the final run; those findings were not added. Kinds:

- arris: volume precision far from the origin; faces within 1e-7 mm
  merged (volume off by up to 3e-13 mm^3 on a 6e-5 mm^3 part, 8.8e-6 on
  264); results its own checker rejects at tangencies and near-coincident
  faces (5 recipes); open meshes on slivers (2); a shell-test failure on
  a far-away prism with collinear vertices.
- cad-kernel: open meshes at tangent cylinders (166 of the final run's
  246 wrong cases); hangs in the mesher (27); volumes far from the origin
  wrong by up to 91 %; volumes off its own mesh by 1e-2 relative on
  turned thin intersections; faces within its tolerance merged; wrong
  shells at a 1e-7 floor and a thin side hole; results its own checker
  rejects (2 recipes); a panic on a cylinder turned 1e-9 rad.
- Both: faces 1e-9 to 1e-7 mm apart merged the same way.

## 8. Speed

Release build, idle machine, `examples/timings.rs`: every catalogue part
evaluated three times (fastest kept), then tessellated once at each
harness chord. cad-kernel runs its parallel steps on every core (up to
24 threads here); arris runs on one thread. These are wall-clock times.

| | arris 0.11.1 | cad-kernel 0.15.0 |
|---|---|---|
| parts evaluated (of 113) | 101 (12 refused) | 111 (1 refused, 1 panic) |
| evaluation, total / median / p95 / max | 1159 / 1.59 / 56.9 / 81.4 ms | 495 / 2.14 / 20.1 / 29.3 ms |
| slowest | unions and splits with the counterbore (57 to 81 ms) | the chamfers (27 to 29 ms: one flat and one cut per edge) |
| **on the 100 parts both evaluate** | | |
| evaluation | 1158.5 ms | 417.2 ms |
| tessellation, chord 0.1 | 21.1 ms | 69.5 ms |
| chord 0.01 | 38.0 ms | 100.3 ms |
| chord 0.001 | 93.7 ms | 235.2 ms |
| chord 0.0001 | 246.0 ms (145 110 triangles over 99 parts) | 928.2 ms (202 310 over 111 parts) |

Single operations from the capability tests (release): the smoke board
plain / rounded 21.3 / 25.9 ms (arris) and 4.2 / 5.8 ms (cad-kernel); its
tessellation at chord 0.001 40.7 ms, 32 592 triangles (arris) and 21.2
ms, 30 964 triangles (cad-kernel); the section at z = 9 23.2 ms (arris
`ops::section`) and 4.4 ms (cad-kernel, a Boolean); one helical sweep
393.8 ms (arris) and 0.6 ms (cad-kernel).

The harness itself: the scoreboard's kernel run is 7.2 s (arris) and 6.5
s (cad-kernel) in release, 10.9 s and 17.6 s in a debug build (the arris
crates are optimised in dev builds by the workspace profile; cad-kernel
is not, and is fast enough unoptimised that no override was added). The
enclosures take 11 s and are shared.

First build on this machine: cad-kernel 30.3 s debug, 45.8 s release,
46.0 s release for wasm32 (one crate, no dependencies); arris 25 s for the
first `cargo check` of core with it and 46 s for the first debug test
build (3D0a note), 42.8 s release for wasm32 (eight crates plus
nalgebra and its dependencies).

## 9. Panics

The harness catches a panic (also one raised on a cad-kernel worker
thread, which `std::thread::scope` passes on) and counts it as wrong.

- **cad-kernel, release builds:** 2 panics in 8000 fuzz cases, both
  `tolerant.rs:90` "a vertex at [...]" with coordinates near 1e12: a
  cylinder turned 1e-9 rad fused to a box (seed 2, index 730; in the
  catalogue) and a case of seed 12 (index 799). The catalogue case
  panics the same way in a debug build. No other panic in either build.
- **arris, release builds:** no panic in 8000 fuzz cases. It returned a
  solid its own checker rejects in 11 of the final run's cases (and 4
  of the first run's).
- **arris, debug builds:** arris runs its checker on every operation's
  input and output in debug builds and panics when its own output fails
  ("kernel bug: an operation's output fails the checker",
  `arris-ops/src/lib.rs:110`). The catalogue cases it returns invalid
  solids for in release panic there instead; the scoreboard records
  them as wrong in both builds (docs/3d-test-harness.md, slice 4).

So "no panic" for cad-kernel holds in neither build; for arris it holds in
release builds and not in debug builds, by its own design.

## 10. Capabilities for the rungs ahead

Each cell is backed by a test that checks a closed form
(`cargo test -p solid-harness --release --lib forward_look -- --nocapture`;
the arris tests are in `kernels::arris::forward_look`, the cad-kernel
ones in `kernels::cad_kernel::forward_look`) or by a pointer into the
kernel's source.

| Capability | arris 0.11.1 | cad-kernel 0.15.0 |
|---|---|---|
| Revolve | yes: `ops::revolve`; rectangle turned a whole turn, 6 pi to 1e-16 | yes: `build::revolve`; same tube, 6 pi to 2e-16 |
| Sweep | yes along a general path: `ops::sweep` with a frame law (line, arc, helix segments); a disc swept one helical turn, 2.5 pi^2 to 1.4e-11 (393.8 ms) | along a constant twist only (a line or a screw): `build::carried`, `build::canal` (a ball along a helix); same helix, 2.5 pi^2 to 1.4e-16 (0.6 ms). No sweep along a general path (README: "What comes next"; brep-bench: "solid sweeps along general motions" refused) |
| Loft | yes: `ops::loft`, ruled or smooth through sections; square frustum, 28 to 6e-16 | yes, from B-spline sheets: `build::lofted` takes rings of nets between two planes (no section-to-section loft that builds the sheets for you); bilinear sheets of the same frustum, 28 to 4e-16 |
| Fillets | yes: `ops::fillet`; the 12-edge rounded smoke board, 33 faces, valid, 3 188 376.635132 (25.9 ms) | `rounds::rounded` for straight edges between planes where every vertex has all three edges rounded (and `fillet::roll` for single runs): the 12-edge rounded smoke board, 33 faces, valid, 3 188 376.635132 (5.8 ms). README: "fillets and chamfers where three edges meet or a face is consumed" are refused in other cases |
| Chamfers | yes: `ops::chamfer`, mitred corners | per edge only (`fillet::flat_along`, `corners::chamfer` for 2D regions); the adapter composes edges and refuses where that would be wrong (section 3) |
| Offsets, shells | `ops::offset_faces` (move faces), `ops::shell` (hollow, inward or outward); cube shell 424 exact, face offset 10 exact | `offset::grown` and `offset::shrunk` (whole solid, round or sharp joins), `offset::shelled`, `offset::thickened`; box grown 2 with round joins to Steiner's closed form within 3e-16, cube shell 424 exact |
| Exact volume | yes (flux over exact faces); centroid too | yes (Green's theorem over exact faces); no centroid or other moments |
| Tessellation | `mesh::tessellate` to a chord; rounded board closed at 0.1 to 0.001, volume error 1.2e-4 to 1.7e-6 | `mesh::mesh` to a measured sag; closed at 0.1 to 0.001, error 2.0e-4 to 3.1e-6 (larger than arris's at the same chord; both well inside the chord's Steiner bound) |
| Planar sections | yes: `ops::section`; rounded board at z = 9, area 179 913.734517543 (exact) | no section operation; a Boolean with a half-space box gives the cut face exactly: same area, 179 913.734517543 |
| Projection, hidden lines | edges project to exact 2D curves (`query::project_to_plane`), exact ray hits (`query::ray_fire`); no visibility or silhouettes | none: no projection, ray or hidden-line API (the words appear only in its CNC sweep modules); point classification exists (`query::Located::solid_place`: in, out, on, tested) |
| Entity provenance | every operation returns a provenance record naming edges by `(role, segment)` through later operations; matches `select.rs` on a dado across a cap | faces carry names given at construction (`Profile::names`, caps `near`/`far`), kept by Booleans (both pieces of a top face cut by a dado keep its name); no edge naming, no operation record |
| wasm32 build | builds (section 2) | builds (section 2) |

## 11. What the evidence supports, and what it does not

It supports:

- For the prismatic features the ladder uses now, both kernels meet every
  closed form, the certified enclosure, the shell test, the mesh
  invariants, the equivalent constructions and the algebra on the
  slice-1 catalogue.
- Near contacts, tangencies and far from the origin, arris is the more
  conservative kernel: it refuses about three times as often as
  cad-kernel and is wrong about nine times less often. Its wrong answers
  are mostly invalid solids its own checker would have caught (a panic
  in a debug build), sub-tolerance merges and precision loss far from
  the origin.
- cad-kernel answers more of the hard cases (holes breaking out, tangent
  holes, `A - A`) and evaluates Booleans faster, but its mesher is the
  weak part: open meshes are two thirds of its wrong cases and every hang
  timed was in the mesher, and its volume can be badly wrong far from the origin (91 % in
  one case) and off its own mesh by 1e-2 on turned thin intersections.
- Neither kernel can be shipped on the "wrong must be zero" rule today:
  each has recorded wrong cases.

It does not support:

- Rates outside this generator. The fuzzer over-represents offsets of
  1e-12 to 1e-3 mm, tiny tools and parts 1e4 to 1e7 mm away; most real
  parts have none of these, and the percentages above say nothing about
  them.
- A ranking on the 209 undecided disagreements, or on any difference
  below the enclosure's width (often 1e-3 relative or more on turned,
  thin or tiny parts) or the shell step (1e-3 mm).
- A judgement of sub-tolerance merges. The enclosure has no positional
  band, so a face moved 5e-10 mm is wrong there; whether that should be
  undecided is open (docs/3d-test-harness.md, section 11). Both kernels
  have such cases.
- Anything about curved features in the harness's sense: revolve, sweep,
  loft, fillets and offsets were checked against one closed form each
  (section 10), not judged by the layers or fuzzed.
- Wasm behaviour: both kernels build for wasm32; neither was run there,
  and no wasm size was measured.
- Bounded run time: the harness has no time limit, and cad-kernel's
  mesher hangs on 0.7 % of fuzz cases. arris was never seen to hang, which
  is not a bound either.
- Speed beyond one machine: cad-kernel's times use up to 24 threads,
  arris's one.

The trade-off as measured: arris offers more of what the next rungs need
(sections, projection, rays, provenance, centroids, general sweeps and
lofts, multi-edge chamfers) and fails less often, at the cost of more
refusals near contacts, a larger dependency set and debug-build panics.
cad-kernel offers a much smaller footprint, faster Booleans, broader
offsets and fewer refusals, at the cost of a higher wrong rate (mostly
its mesher), mesher hangs, panics, no centroid, section, projection or
edge provenance, native threads, and a project 17 days old that calls
itself not yet shaped for general use. Both are young enough that their
next releases may change any of this; the scoreboards and the fuzzer
re-measure a version bump.

## Commands

```bash
# Scoreboards (both kernels) and mutants
cargo test -p solid-harness --release --test scoreboard -- --nocapture
cargo test -p solid-harness --release --test mutants -- --nocapture
# Capability tests
cargo test -p solid-harness --release --lib forward_look -- --nocapture
# Catalogue cross-check and sweep tables
cargo run -p solid-harness --release --example compare
# Timings
cargo run -p solid-harness --release --example timings
# Fuzzing (each seed 1000 cases; a planted bug first)
cargo run -p solid-harness --release --example fuzz -- run --seed 101 --cases 300 --kernels broken-arris:hole
cargo run -p solid-harness --release --example fuzz -- run --seed 11 --cases 1000 --kernels arris,cad-kernel
```
