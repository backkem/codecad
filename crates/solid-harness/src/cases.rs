//! The case catalogue: every solid the harness builds, the relations
//! between them, and the continuity sweeps.
//!
//! A [`PartCase`] is one recipe (with its tool definitions) and, where
//! somebody derived it by hand, its closed-form volume and bounding box and
//! the Euler characteristic its recipe implies. Derived solids (moved,
//! mirrored, combined) carry no closed form: their evidence is relational.
//! Every part runs on the part layers (closed form, enclosure, shell,
//! mesh); [`Relation`]s run on layers 5 and 6; [`Sweep`]s on layer 7.

use std::f64::consts::{PI, TAU};

use cadview_core::solid::corpus::{curved, prismatic};
use cadview_core::solid::{
    Aabb3, CapSide, DefId, Depth, EdgeSel, FeatId, Feature, Op, PartDef, Path3, Placement, Plane3,
    Point3, Profile2d, Seg2, Seg3, ToolRef, ToolResolver, Vec3,
};
use cadview_core::types::{LwVertex, Shape as Shape2d};
use kurbo::{Line, Point};

/// One solid to build.
#[derive(Debug, Clone)]
pub struct PartCase {
    pub name: String,
    pub def: PartDef,
    pub tools: Tools,
    /// Hand-derived volume (mm^3), if any.
    pub volume: Option<f64>,
    /// Hand-derived bounding box, if any.
    pub bbox: Option<Aabb3>,
    /// The Euler characteristic of the boundary the recipe implies
    /// (2 per closed shell, minus 2 per through hole), if derived.
    pub euler: Option<i64>,
}

/// Tool definitions, as a resolver.
#[derive(Debug, Clone, Default)]
pub struct Tools(pub Vec<PartDef>);

impl ToolResolver for Tools {
    fn def(&self, id: DefId) -> Option<&PartDef> {
        self.0.iter().find(|d| d.id == id)
    }
}

impl Tools {
    pub fn as_slice(&self) -> &Tools {
        self
    }
}

/// A relation between parts (by name) that the specification implies.
#[derive(Debug, Clone)]
pub enum Relation {
    /// Layer 5: the same solid built two ways.
    Equivalent { name: String, a: String, b: String },
    /// Layer 6: two recipes the algebra says are equal (`A + A = A`,
    /// `A n A = A`, operand order).
    Same { name: String, a: String, b: String },
    /// Layer 6: `V(A u B) + V(A n B) = V(A) + V(B)`.
    UnionIdentity {
        name: String,
        a: String,
        b: String,
        union: String,
        inter: String,
    },
    /// Layer 6: `A - A` is empty.
    Empty {
        name: String,
        part: String,
        of: String,
    },
    /// Layer 6: `moved` is `base` moved by `motion`.
    Motion {
        name: String,
        base: String,
        moved: String,
        motion: Placement,
    },
    /// Layer 6: `mirrored` is `base` mirrored in the plane `x = x0`.
    Mirror {
        name: String,
        base: String,
        mirrored: String,
        x0: f64,
    },
    /// Layer 6: `whole` split by a plane into two pieces.
    Split {
        name: String,
        whole: String,
        pieces: [String; 2],
    },
}

impl Relation {
    pub fn name(&self) -> &str {
        match self {
            Relation::Equivalent { name, .. }
            | Relation::Same { name, .. }
            | Relation::UnionIdentity { name, .. }
            | Relation::Empty { name, .. }
            | Relation::Motion { name, .. }
            | Relation::Mirror { name, .. }
            | Relation::Split { name, .. } => name,
        }
    }
}

/// Layer 7: one parameter sweep through a contact.
#[derive(Debug, Clone)]
pub struct Sweep {
    pub name: String,
    /// Parameter values, increasing, and the recipe at each.
    pub steps: Vec<(f64, PartCase)>,
    /// A bound on `|dV/ds|` derived from the geometry (mm^2).
    pub slope: f64,
    /// `+1`: volume never decreases as `s` grows; `-1`: never increases.
    pub direction: f64,
    /// A bound on the part's surface area (mm^2), for the band.
    pub area: f64,
}

/// Everything the harness runs.
#[derive(Debug, Clone)]
pub struct Catalogue {
    pub parts: Vec<PartCase>,
    pub relations: Vec<Relation>,
    pub sweeps: Vec<Sweep>,
}

impl Catalogue {
    pub fn part(&self, name: &str) -> Option<&PartCase> {
        self.parts.iter().find(|p| p.name == name)
    }
}

// ── Builders ──────────────────────────────────────────────────────────

pub fn def(id: u64, name: &str, features: Vec<Feature>) -> PartDef {
    PartDef {
        id: DefId(id),
        name: name.into(),
        features,
    }
}

pub fn extrude(id: u32, profile: Profile2d, plane: Plane3, distance: f64, op: Op) -> Feature {
    Feature::Extrude {
        id: FeatId(id),
        profile,
        plane,
        distance,
        taper_deg: 0.0,
        op,
    }
}

/// The box `[x0, x1] x [y0, y1] x [z0, z1]` as a rectangle on XY at `z0`
/// extruded up.
pub fn cuboid(id: u32, lo: [f64; 3], hi: [f64; 3], op: Op) -> Feature {
    extrude(
        id,
        Profile2d::rect(lo[0], lo[1], hi[0], hi[1]),
        Plane3::XY.offset(lo[2]),
        hi[2] - lo[2],
        op,
    )
}

pub fn boolean(id: u32, op: Op, tool: u64, placement: Placement) -> Feature {
    Feature::Boolean {
        id: FeatId(id),
        op,
        tool: ToolRef::Def(DefId(tool), placement),
    }
}

pub fn hole(id: u32, at: [f64; 3], dir: [f64; 3], dia: f64, depth: Depth) -> Feature {
    Feature::Hole {
        id: FeatId(id),
        at: Point3::new(at[0], at[1], at[2]),
        dir: Vec3::new(dir[0], dir[1], dir[2]),
        dia,
        depth,
        cbore: None,
    }
}

pub fn chamfer_cap(id: u32, feature: u32, side: CapSide, d: f64) -> Feature {
    Feature::Chamfer {
        id: FeatId(id),
        edges: EdgeSel::Cap {
            feature: FeatId(feature),
            side,
            segs: None,
        },
        d1: d,
        d2: d,
    }
}

pub fn aabb(lo: [f64; 3], hi: [f64; 3]) -> Aabb3 {
    Aabb3::new(
        Point3::new(lo[0], lo[1], lo[2]),
        Point3::new(hi[0], hi[1], hi[2]),
    )
}

fn part(name: &str, d: PartDef, tools: Vec<PartDef>) -> PartCase {
    PartCase {
        name: name.into(),
        def: d,
        tools: Tools(tools),
        volume: None,
        bbox: None,
        euler: None,
    }
}

impl PartCase {
    fn closed(mut self, volume: f64, bbox: Aabb3) -> Self {
        self.volume = Some(volume);
        self.bbox = Some(bbox);
        self
    }
    fn euler(mut self, chi: i64) -> Self {
        self.euler = Some(chi);
        self
    }
}

// ── Recipe transforms (motion and mirror) ─────────────────────────────

type Q = [f64; 4];

fn qmul(a: Q, b: Q) -> Q {
    let [aw, ax, ay, az] = a;
    let [bw, bx, by, bz] = b;
    [
        aw * bw - ax * bx - ay * by - az * bz,
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
    ]
}

/// Rotates `v` by the unit quaternion `q`.
pub fn qrot(q: Q, v: Vec3) -> Vec3 {
    let [w, x, y, z] = q;
    let u = Vec3::new(x, y, z);
    let t = u.cross(v).scale(2.0);
    v + t.scale(w) + u.cross(t)
}

/// A rigid motion `p -> Q p + m` applied to a whole recipe: planes,
/// points and directions move, tool placements compose; tool definitions
/// are untouched (they live in their own frame).
pub fn moved_def(d: &PartDef, m: &Placement) -> PartDef {
    let q = m.rotation;
    let pt = |p: Point3| {
        let r = qrot(q, Vec3::new(p.x, p.y, p.z)) + m.translation;
        Point3::new(r.x, r.y, r.z)
    };
    let moved_plane = |plane: &Plane3| Plane3 {
        origin: pt(plane.origin),
        x_axis: qrot(q, plane.x_axis),
        normal: qrot(q, plane.normal),
    };
    let features = d
        .features
        .iter()
        .map(|f| match f.clone() {
            Feature::Extrude {
                id,
                profile,
                plane,
                distance,
                taper_deg,
                op,
            } => Feature::Extrude {
                id,
                profile,
                plane: Plane3 {
                    origin: pt(plane.origin),
                    x_axis: qrot(q, plane.x_axis),
                    normal: qrot(q, plane.normal),
                },
                distance,
                taper_deg,
                op,
            },
            Feature::Hole {
                id,
                at,
                dir,
                dia,
                depth,
                cbore,
            } => Feature::Hole {
                id,
                at: pt(at),
                dir: qrot(q, dir),
                dia,
                depth,
                cbore,
            },
            Feature::Boolean {
                id,
                op,
                tool: ToolRef::Def(t, p),
            } => Feature::Boolean {
                id,
                op,
                tool: ToolRef::Def(
                    t,
                    Placement {
                        rotation: qmul(q, p.rotation),
                        translation: qrot(q, p.translation) + m.translation,
                    },
                ),
            },
            Feature::Chamfer {
                id,
                edges: EdgeSel::Near { points },
                d1,
                d2,
            } => Feature::Chamfer {
                id,
                edges: EdgeSel::Near {
                    points: points.into_iter().map(pt).collect(),
                },
                d1,
                d2,
            },
            Feature::Revolve {
                id,
                profile,
                plane,
                axis,
                angle_deg,
                op,
            } => Feature::Revolve {
                id,
                profile,
                plane: moved_plane(&plane),
                axis,
                angle_deg,
                op,
            },
            Feature::Sweep {
                id,
                profile,
                plane,
                path,
                op,
            } => Feature::Sweep {
                id,
                profile,
                plane: moved_plane(&plane),
                path: Path3 {
                    segs: path
                        .segs
                        .iter()
                        .map(|s| match *s {
                            Seg3::Line { from, to } => Seg3::Line {
                                from: pt(from),
                                to: pt(to),
                            },
                            Seg3::Arc { from, via, to } => Seg3::Arc {
                                from: pt(from),
                                via: pt(via),
                                to: pt(to),
                            },
                        })
                        .collect(),
                },
                op,
            },
            Feature::Loft {
                id,
                sections,
                ruled,
                op,
            } => Feature::Loft {
                id,
                sections: sections
                    .into_iter()
                    .map(|(p, pl)| (p, moved_plane(&pl)))
                    .collect(),
                ruled,
                op,
            },
            other => other,
        })
        .collect();
    PartDef {
        id: d.id,
        name: d.name.clone(),
        features,
    }
}

/// Mirrors a recipe in the plane `x = x0` (`x -> 2 x0 - x`). With `L` the
/// linear reflection `diag(-1, 1, 1)`, a sketch plane `(o, x, n)` becomes
/// `(M o, L x, -L n)` with the same profile and the distance negated
/// (`L x`, `L y` keep the profile's coordinates and `-L n` makes the frame
/// right-handed again); a hole's point and direction reflect; a tool
/// definition is mirrored through the origin (`x0 = 0`, ids offset by
/// `id_shift`) and its placement `(R, t)` becomes `(L R L, L t + 2 x0 X)`,
/// whose quaternion is `(w, x, -y, -z)` for `R = (w, x, y, z)`.
pub fn mirrored_def(d: &PartDef, x0: f64, id_shift: u64) -> PartDef {
    let l = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
    let pt = |p: Point3| Point3::new(2.0 * x0 - p.x, p.y, p.z);
    let features = d
        .features
        .iter()
        .map(|f| match f.clone() {
            Feature::Extrude {
                id,
                profile,
                plane,
                distance,
                taper_deg,
                op,
            } => Feature::Extrude {
                id,
                profile,
                plane: Plane3 {
                    origin: pt(plane.origin),
                    x_axis: l(plane.x_axis),
                    normal: -l(plane.normal),
                },
                distance: -distance,
                taper_deg,
                op,
            },
            Feature::Hole {
                id,
                at,
                dir,
                dia,
                depth,
                cbore,
            } => Feature::Hole {
                id,
                at: pt(at),
                dir: l(dir),
                dia,
                depth,
                cbore,
            },
            Feature::Boolean {
                id,
                op,
                tool: ToolRef::Def(t, p),
            } => {
                let [w, x, y, z] = p.rotation;
                Feature::Boolean {
                    id,
                    op,
                    tool: ToolRef::Def(
                        DefId(t.0 + id_shift),
                        Placement {
                            rotation: [w, x, -y, -z],
                            translation: l(p.translation) + Vec3::new(2.0 * x0, 0.0, 0.0),
                        },
                    ),
                }
            }
            Feature::Chamfer {
                id,
                edges: EdgeSel::Near { points },
                d1,
                d2,
            } => Feature::Chamfer {
                id,
                edges: EdgeSel::Near {
                    points: points.into_iter().map(pt).collect(),
                },
                d1,
                d2,
            },
            // The plane as for an extrude keeps the profile's coordinates.
            // A reflection turns the other way round its axis, so the
            // mirrored revolve runs about the reversed axis.
            Feature::Revolve {
                id,
                profile,
                plane,
                axis,
                angle_deg,
                op,
            } => Feature::Revolve {
                id,
                profile,
                plane: Plane3 {
                    origin: pt(plane.origin),
                    x_axis: l(plane.x_axis),
                    normal: -l(plane.normal),
                },
                axis: Line::new(axis.p1, axis.p0),
                angle_deg,
                op,
            },
            other => other,
        })
        .collect();
    PartDef {
        id: DefId(d.id.0 + id_shift),
        name: d.name.clone(),
        features,
    }
}

fn mirrored_case(base: &PartCase, x0: f64) -> PartCase {
    const SHIFT: u64 = 100_000;
    let tools = base
        .tools
        .0
        .iter()
        .map(|t| mirrored_def(t, 0.0, SHIFT))
        .collect();
    let mut d = mirrored_def(&base.def, x0, SHIFT);
    d.id = base.def.id; // the part's own id does not matter; keep it
    part(&format!("{} mirrored", base.name), d, tools)
}

fn moved_case(base: &PartCase, m: &Placement) -> PartCase {
    part(
        &format!("{} moved", base.name),
        moved_def(&base.def, m),
        base.tools.0.clone(),
    )
}

/// `base` as a tool definition with id `id` (its own tools kept).
fn as_tool(base: &PartCase, id: u64) -> (PartDef, Vec<PartDef>) {
    let mut d = base.def.clone();
    d.id = DefId(id);
    (d, base.tools.0.clone())
}

/// A part made only of booleans of placed tool definitions:
/// `first` added, then `(op, tool, placement)` in order.
fn assembly(
    name: &str,
    first: (u64, Placement),
    rest: &[(Op, u64, Placement)],
    tools: Vec<PartDef>,
) -> PartCase {
    let mut features = vec![boolean(1, Op::Add, first.0, first.1)];
    for (i, (op, t, p)) in rest.iter().enumerate() {
        features.push(boolean(2 + i as u32, *op, *t, *p));
    }
    part(name, def(9_000, name, features), tools)
}

fn merge_tools(lists: Vec<Vec<PartDef>>) -> Vec<PartDef> {
    let mut out: Vec<PartDef> = Vec::new();
    for l in lists {
        for d in l {
            if !out.iter().any(|o| o.id == d.id) {
                out.push(d);
            }
        }
    }
    out
}

// ── The catalogue ─────────────────────────────────────────────────────

/// The corpus parts as cases, with their closed forms and Euler
/// characteristics (2 for one shell with no through hole, 0 with one).
fn corpus_parts() -> Vec<PartCase> {
    let mut out = Vec::new();
    for entry in prismatic::entries() {
        for p in &entry.parts {
            let chi = match p.def.name.as_str() {
                "board with hole" | "block with counterbore" => 0,
                _ => 2,
            };
            out.push(
                part(&p.def.name, p.def.clone(), entry.tools.clone())
                    .closed(p.volume, p.bbox)
                    .euler(chi),
            );
        }
    }
    out
}

/// The curved corpus parts (rung 3D0b) as cases, with their closed forms.
/// Each is one closed shell with no through hole: Euler characteristic 2.
fn curved_corpus_parts() -> Vec<PartCase> {
    let mut out = Vec::new();
    for entry in curved::entries() {
        for p in &entry.parts {
            out.push(
                part(&p.def.name, p.def.clone(), entry.tools.clone())
                    .closed(p.volume, p.bbox)
                    .euler(2),
            );
        }
    }
    out
}

/// A rectangle `rho` in `[r0, r1]`, `z` in `[0, h]` on the XZ plane,
/// revolved about the z axis by `deg` degrees (right-handed, from +X
/// towards +Y).
fn revolved_rect(id: u64, name: &str, r0: f64, r1: f64, h: f64, deg: f64) -> PartCase {
    part(
        name,
        def(
            id,
            name,
            vec![Feature::Revolve {
                id: FeatId(1),
                profile: Profile2d::rect(r0, 0.0, r1, h),
                plane: Plane3::XZ,
                axis: Line::new((0.0, 0.0), (0.0, 1.0)),
                angle_deg: deg,
                op: Op::Add,
            }],
        ),
        vec![],
    )
}

/// Curved parts beyond the corpus, each with a closed form:
///
/// - a quarter and a three-quarter revolve of the rectangle `rho` in
///   `[10, 20]`, `z` in `[0, 5]`: a whole turn is `pi (400 - 100) 5 =
///   1 500 pi`, so `375 pi` and `1 125 pi`. From +X turning towards +Y,
///   the quarter spans `x, y` in `[0, 20]`; the three-quarter turn
///   reaches -X at 180 degrees and -Y at 270: `x, y` in `[-20, 20]`.
fn curved_extra() -> Vec<PartCase> {
    vec![
        revolved_rect(41, "quarter revolve of a rectangle", 10.0, 20.0, 5.0, 90.0)
            .closed(375.0 * PI, aabb([0.0, 0.0, 0.0], [20.0, 20.0, 5.0]))
            .euler(2),
        revolved_rect(
            42,
            "three-quarter revolve of a rectangle",
            10.0,
            20.0,
            5.0,
            270.0,
        )
        .closed(1125.0 * PI, aabb([-20.0, -20.0, 0.0], [20.0, 20.0, 5.0]))
        .euler(2),
    ]
}

/// Pairs of recipes for the same curved solid (layer 5).
fn curved_equivalences() -> (Vec<PartCase>, Vec<Relation>) {
    let mut parts = Vec::new();
    let mut rel = Vec::new();
    let mut pair = |name: &str, a: PartCase, b: PartCase| {
        rel.push(Relation::Equivalent {
            name: name.into(),
            a: a.name.clone(),
            b: b.name.clone(),
        });
        parts.push(a);
        parts.push(b);
    };

    // A tube: the rectangle rho in [4, 10], z in [0, 3] turned a whole
    // turn, and the annulus extruded 3: V = pi (100 - 16) 3 = 252 pi. One
    // shell with one through hole.
    let tube_v = 252.0 * PI;
    let tube_b = aabb([-10.0, -10.0, 0.0], [10.0, 10.0, 3.0]);
    let annulus = Profile2d::circle(Point::new(0.0, 0.0), 10.0)
        .with_hole(vec![Seg2::Circle(kurbo::Circle::new((0.0, 0.0), 4.0))]);
    pair(
        "tube: revolved rectangle = extruded annulus",
        revolved_rect(43, "revolved rectangle tube", 4.0, 10.0, 3.0, 360.0)
            .closed(tube_v, tube_b)
            .euler(0),
        part(
            "extruded annulus",
            def(
                44,
                "extruded annulus",
                vec![extrude(1, annulus, Plane3::XY, 3.0, Op::Add)],
            ),
            vec![],
        )
        .closed(tube_v, tube_b)
        .euler(0),
    );

    // A square bar: a loft between two equal 40 x 40 squares 300 apart,
    // and the square extruded 300: V = 480 000.
    let bar_v = 480_000.0;
    let bar_b = aabb([-20.0, -20.0, 0.0], [20.0, 20.0, 300.0]);
    let sq = Profile2d::rect(-20.0, -20.0, 20.0, 20.0);
    pair(
        "square bar: loft of equal sections = extrude",
        part(
            "loft of equal squares",
            def(
                45,
                "loft of equal squares",
                vec![Feature::Loft {
                    id: FeatId(1),
                    sections: vec![
                        (sq.clone(), Plane3::XY),
                        (sq.clone(), Plane3::XY.offset(300.0)),
                    ],
                    ruled: true,
                    op: Op::Add,
                }],
            ),
            vec![],
        )
        .closed(bar_v, bar_b)
        .euler(2),
        part(
            "extruded square bar",
            def(
                46,
                "extruded square bar",
                vec![extrude(1, sq, Plane3::XY, 300.0, Op::Add)],
            ),
            vec![],
        )
        .closed(bar_v, bar_b)
        .euler(2),
    );

    // A round bar: a disc r = 6 on the YZ plane swept 100 along +X, and
    // the disc extruded 100: V = 3 600 pi.
    let rod_v = 3600.0 * PI;
    let rod_b = aabb([0.0, -6.0, -6.0], [100.0, 6.0, 6.0]);
    let disc = Profile2d::circle(Point::new(0.0, 0.0), 6.0);
    pair(
        "round bar: sweep along a line = extrude",
        part(
            "disc swept along a line",
            def(
                47,
                "disc swept along a line",
                vec![Feature::Sweep {
                    id: FeatId(1),
                    profile: disc.clone(),
                    plane: Plane3::YZ,
                    path: Path3 {
                        segs: vec![Seg3::Line {
                            from: Point3::ORIGIN,
                            to: Point3::new(100.0, 0.0, 0.0),
                        }],
                    },
                    op: Op::Add,
                }],
            ),
            vec![],
        )
        .closed(rod_v, rod_b)
        .euler(2),
        part(
            "extruded disc",
            def(
                48,
                "extruded disc",
                vec![extrude(1, disc.clone(), Plane3::YZ, 100.0, Op::Add)],
            ),
            vec![],
        )
        .closed(rod_v, rod_b)
        .euler(2),
    );

    // A quarter torus: the same disc swept along a quarter circle of
    // radius 40 about the axis through (0, 40, 0) along +Z, from the
    // origin to (40, 40, 0), and the disc revolved 90 degrees about that
    // axis (in the YZ plane's coordinates the line u = 40, pointing +Z).
    // Pappus: V = pi 36 * 40 pi / 2 = 720 pi^2. The start disc lies in
    // x = 0 and the end disc in y = 40; the outside of the bend reaches
    // x = 46: box [0, -6, -6] to [46, 40, 6].
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let qt_v = 720.0 * PI * PI;
    let qt_b = aabb([0.0, -6.0, -6.0], [46.0, 40.0, 6.0]);
    pair(
        "quarter torus: sweep along an arc = revolve by its angle",
        part(
            "disc swept along a quarter arc",
            def(
                49,
                "disc swept along a quarter arc",
                vec![Feature::Sweep {
                    id: FeatId(1),
                    profile: disc.clone(),
                    plane: Plane3::YZ,
                    path: Path3 {
                        segs: vec![Seg3::Arc {
                            from: Point3::ORIGIN,
                            via: Point3::new(40.0 * s, 40.0 - 40.0 * s, 0.0),
                            to: Point3::new(40.0, 40.0, 0.0),
                        }],
                    },
                    op: Op::Add,
                }],
            ),
            vec![],
        )
        .closed(qt_v, qt_b)
        .euler(2),
        part(
            "disc revolved a quarter turn",
            def(
                50,
                "disc revolved a quarter turn",
                vec![Feature::Revolve {
                    id: FeatId(1),
                    profile: disc,
                    plane: Plane3::YZ,
                    axis: Line::new((40.0, 0.0), (40.0, 1.0)),
                    angle_deg: 90.0,
                    op: Op::Add,
                }],
            ),
            vec![],
        )
        .closed(qt_v, qt_b)
        .euler(2),
    );
    (parts, rel)
}

/// Motion and mirroring of curved parts, and `A + A`, `A n A`, `A - A`
/// on one of them (layer 6).
fn curved_algebra(all: &[PartCase]) -> (Vec<PartCase>, Vec<Relation>) {
    let named = |n: &str| all.iter().find(|p| p.name == n).cloned().expect(n);
    let mut parts = Vec::new();
    let mut rel = Vec::new();
    let m = generic_motion();
    for bn in [
        "rounded slab",
        "tapered square bar",
        "revolved profile with bead and cove",
        "swept round bar",
        "three-quarter revolve of a rectangle",
    ] {
        let b = named(bn);
        let mut mv = moved_case(&b, &m);
        mv.euler = b.euler;
        rel.push(Relation::Motion {
            name: format!("motion: {bn}"),
            base: b.name.clone(),
            moved: mv.name.clone(),
            motion: m,
        });
        parts.push(mv);
    }
    for bn in [
        "revolved profile with bead and cove",
        "three-quarter revolve of a rectangle",
    ] {
        let b = named(bn);
        let x0 = 37.0;
        let mut mr = mirrored_case(&b, x0);
        mr.euler = b.euler;
        rel.push(Relation::Mirror {
            name: format!("mirror: {bn}"),
            base: b.name.clone(),
            mirrored: mr.name.clone(),
            x0,
        });
        parts.push(mr);
    }
    let bn = "revolved profile with bead and cove";
    let b = named(bn);
    let tid = 51_000;
    let (t, tt) = as_tool(&b, tid);
    let tools = merge_tools(vec![vec![t], tt]);
    let id = Placement::IDENTITY;
    let mut aa = assembly(
        &format!("{bn} + itself"),
        (tid, id),
        &[(Op::Add, tid, id)],
        tools.clone(),
    );
    aa.euler = b.euler;
    let mut ai = assembly(
        &format!("{bn} n itself"),
        (tid, id),
        &[(Op::Intersect, tid, id)],
        tools.clone(),
    );
    ai.euler = b.euler;
    let am = assembly(
        &format!("{bn} - itself"),
        (tid, id),
        &[(Op::Subtract, tid, id)],
        tools,
    );
    rel.push(Relation::Same {
        name: format!("A + A = A: {bn}"),
        a: b.name.clone(),
        b: aa.name.clone(),
    });
    rel.push(Relation::Same {
        name: format!("A n A = A: {bn}"),
        a: b.name.clone(),
        b: ai.name.clone(),
    });
    rel.push(Relation::Empty {
        name: format!("A - A = empty: {bn}"),
        part: am.name.clone(),
        of: b.name.clone(),
    });
    parts.extend([aa, ai, am]);
    (parts, rel)
}

/// A stadium 40 long, radius 5, from a bulge polyline, extruded 10:
/// V = (40 * 10 + 25 pi) * 10 = 4000 + 250 pi; the caps reach x = -5 and
/// x = 45.
fn slot() -> PartCase {
    let vx = |x, y, bulge| LwVertex {
        point: Point::new(x, y),
        bulge,
    };
    let shape = Shape2d::LwPolyline {
        vertices: vec![
            vx(0.0, 0.0, 0.0),
            vx(40.0, 0.0, 1.0),
            vx(40.0, 10.0, 0.0),
            vx(0.0, 10.0, 1.0),
        ],
        closed: true,
    };
    let outer = Profile2d::loop_from_shape(&shape).expect("closed polyline");
    part(
        "slot",
        def(
            21,
            "slot",
            vec![extrude(
                1,
                Profile2d {
                    outer,
                    holes: vec![],
                },
                Plane3::XY,
                10.0,
                Op::Add,
            )],
        ),
        vec![],
    )
    .closed(
        4000.0 + 250.0 * PI,
        aabb([-5.0, 0.0, 0.0], [45.0, 10.0, 10.0]),
    )
    .euler(2)
}

/// A washer: circle r = 10 with a hole r = 4, extruded -3:
/// V = pi (100 - 16) 3 = 252 pi.
fn washer() -> PartCase {
    let p = Profile2d::circle(Point::new(0.0, 0.0), 10.0)
        .with_hole(vec![Seg2::Circle(kurbo::Circle::new((0.0, 0.0), 4.0))]);
    part(
        "washer",
        def(22, "washer", vec![extrude(1, p, Plane3::XY, -3.0, Op::Add)]),
        vec![],
    )
    .closed(252.0 * PI, aabb([-10.0, -10.0, -3.0], [10.0, 10.0, 0.0]))
    .euler(0)
}

/// A full ellipse a = 8, b = 3, major axis at 30 degrees, extruded 5:
/// V = pi a b h = 120 pi. Its box half-widths are sqrt(a^2 cos^2 + b^2
/// sin^2) = sqrt(48 + 2.25) along x and sqrt(a^2 sin^2 + b^2 cos^2) =
/// sqrt(16 + 6.75) along y.
fn ellipse() -> PartCase {
    let e = Shape2d::Ellipse {
        center: Point::new(0.0, 0.0),
        major_axis: (8.0 * (PI / 6.0).cos(), 8.0 * (PI / 6.0).sin()),
        minor_ratio: 3.0 / 8.0,
        start_param: 0.0,
        end_param: TAU,
    };
    let outer = Profile2d::loop_from_shape(&e).expect("full ellipse");
    let (hx, hy) = (50.25f64.sqrt(), 22.75f64.sqrt());
    part(
        "ellipse",
        def(
            23,
            "ellipse",
            vec![extrude(
                1,
                Profile2d {
                    outer,
                    holes: vec![],
                },
                Plane3::XY,
                5.0,
                Op::Add,
            )],
        ),
        vec![],
    )
    .closed(120.0 * PI, aabb([-hx, -hy, 0.0], [hx, hy, 5.0]))
    .euler(2)
}

/// A D shape: half an ellipse (a = 12 along x, b = 6) closed by its
/// diameter, extruded 4: V = pi a b / 2 * 4 = 144 pi; box x in [-12, 12],
/// y in [0, 6], z in [0, 4]. An elliptic arc and a line in one loop.
fn half_ellipse() -> PartCase {
    let p = Profile2d {
        outer: vec![
            Seg2::Line(Line::new((-12.0, 0.0), (12.0, 0.0))),
            Seg2::Arc(kurbo::Arc {
                center: Point::new(0.0, 0.0),
                radii: kurbo::Vec2::new(12.0, 6.0),
                start_angle: 0.0,
                sweep_angle: PI,
                x_rotation: 0.0,
            }),
        ],
        holes: vec![],
    };
    part(
        "half ellipse",
        def(
            24,
            "half ellipse",
            vec![extrude(1, p, Plane3::XY, 4.0, Op::Add)],
        ),
        vec![],
    )
    .closed(144.0 * PI, aabb([-12.0, 0.0, 0.0], [12.0, 6.0, 4.0]))
    .euler(2)
}

/// One vertical edge of a 10 x 10 x 10 cube chamfered 1 by the Near
/// selector: V = 1000 - 1 * 1 / 2 * 10 = 995. The reference has no
/// meaning for a Near selector: the reference layers are n/a.
fn near_chamfer() -> PartCase {
    part(
        "cube with one chamfered edge",
        def(
            25,
            "cube with one chamfered edge",
            vec![
                cuboid(1, [0.0; 3], [10.0; 3], Op::Add),
                Feature::Chamfer {
                    id: FeatId(2),
                    edges: EdgeSel::Near {
                        points: vec![Point3::new(10.2, 10.1, 5.0)],
                    },
                    d1: 1.0,
                    d2: 1.0,
                },
            ],
        ),
        vec![],
    )
    .closed(995.0, aabb([0.0; 3], [10.0; 3]))
    .euler(2)
}

/// A chamfer on an L profile: a concave corner, which the reference does
/// not define. Kept to show the reference layers turning n/a.
fn l_chamfer() -> PartCase {
    let l = Profile2d::polygon(&[
        Point::new(0.0, 0.0),
        Point::new(20.0, 0.0),
        Point::new(20.0, 10.0),
        Point::new(10.0, 10.0),
        Point::new(10.0, 20.0),
        Point::new(0.0, 20.0),
    ]);
    part(
        "L with chamfered top",
        def(
            26,
            "L with chamfered top",
            vec![
                extrude(1, l, Plane3::XY, 5.0, Op::Add),
                chamfer_cap(2, 1, CapSide::End, 1.0),
            ],
        ),
        vec![],
    )
    .euler(2)
}

/// A 60 mm cube with a closed internal cavity: the intersection of two
/// boxes 20 on a side, turned 30 degrees about different axes, centred at
/// the cube's centre. Nobody derived the cavity's volume by hand: this
/// part has no closed form, so its volume evidence is the enclosure. The
/// boundary is two shells (Euler characteristic 4).
fn cavity() -> PartCase {
    let a = def(
        41,
        "cavity a",
        vec![cuboid(1, [-10.0; 3], [10.0; 3], Op::Add)],
    );
    let b = def(
        42,
        "cavity b",
        vec![cuboid(1, [-10.0; 3], [10.0; 3], Op::Add)],
    );
    let cav = def(
        43,
        "cavity",
        vec![
            boolean(
                1,
                Op::Add,
                41,
                Placement::axis_angle(Vec3::Z, PI / 6.0, Vec3::new(0.0, 0.0, 0.0)),
            ),
            boolean(
                2,
                Op::Intersect,
                42,
                Placement::axis_angle(Vec3::X, PI / 6.0, Vec3::new(0.0, 0.0, 0.0)),
            ),
        ],
    );
    part(
        "cube with internal cavity",
        def(
            27,
            "cube with internal cavity",
            vec![
                cuboid(1, [0.0; 3], [60.0; 3], Op::Add),
                boolean(
                    2,
                    Op::Subtract,
                    43,
                    Placement::translation(30.0, 30.0, 30.0),
                ),
            ],
        ),
        vec![a, b, cav],
    )
    .euler(4)
}

/// A blind hole whose entry point lies inside the material, 5 below the
/// top of a 50 x 50 x 20 block: Ø6, 10 deep, at (25, 25, 15) pointing
/// down. By the reference meaning the hole removes only `t` in [0, 10]
/// from its entry point, z in [5, 15]: a closed internal cavity, V =
/// 50 000 - 90 pi, two shells.
fn hole_below_surface() -> PartCase {
    part(
        "hole entering below the surface",
        def(
            28,
            "hole entering below the surface",
            vec![
                cuboid(1, [0.0; 3], [50.0, 50.0, 20.0], Op::Add),
                hole(
                    2,
                    [25.0, 25.0, 15.0],
                    [0.0, 0.0, -1.0],
                    6.0,
                    Depth::Blind(10.0),
                ),
            ],
        ),
        vec![],
    )
    .closed(50_000.0 - 90.0 * PI, aabb([0.0; 3], [50.0, 50.0, 20.0]))
    .euler(4)
}

// ── Fuzzer findings ───────────────────────────────────────────────────
//
// Each case below is a failure the fuzzer (`examples/fuzz.rs`) found, as
// the shrinker printed it, so the gate keeps running it. The doc comment
// says on which kernel it failed, how, and with what numbers. None has a
// closed form: the enclosure, the shell test and the mesh layer judge it.

/// Found by the fuzzer (seed 1, index 34) on arris 0.11.1, shrunk: a
/// 187 x 9 x 50 box at x = 2 285 813 mm intersected with a 0.0135 x 0.3
/// x 88 tool at its corner, a 0.0135 x 0.3 x 50 box (about 0.2 mm^3) far
/// from the origin. arris's volume, 0.2026571097, lies 1.9e-9 (9.6e-9
/// relative) above the certified enclosure [0.2026558918,
/// 0.2026571077]: its flux integral loses precision far from the origin
/// (the case before shrinking was 8.2e-7 relative off). cad-kernel is
/// correct.
fn fuzz_far_small_box() -> PartCase {
    let name = "fuzz: small box far from the origin (s1 i34)";
    let tools = vec![def(
        1000,
        "tool",
        vec![Feature::Extrude {
            id: FeatId(1),
            profile: Profile2d {
                outer: vec![
                    Seg2::Line(Line::new((0.0, 0.0), (0.013510473847674346, 0.0))),
                    Seg2::Line(Line::new(
                        (0.013510473847674346, 0.0),
                        (0.013510473847674346, 0.3),
                    )),
                    Seg2::Line(Line::new((0.013510473847674346, 0.3), (0.0, 0.3))),
                    Seg2::Line(Line::new((0.0, 0.3), (0.0, 0.0))),
                ],
                holes: vec![],
            },
            plane: Plane3::XY,
            distance: 88.0,
            taper_deg: 0.0,
            op: Op::Add,
        }],
    )];
    part(
        name,
        def(
            7000,
            name,
            vec![
                extrude(
                    1,
                    Profile2d::rect(0.0, 0.0, 187.0, 9.0),
                    Plane3::XY.at(Point3::new(2285813.0, 0.0, 0.0)),
                    50.0,
                    Op::Add,
                ),
                boolean(
                    2,
                    Op::Intersect,
                    1000,
                    Placement::translation(2285813.0, 0.0, 0.0),
                ),
            ],
        ),
        tools,
    )
    .euler(2)
}

/// Found by the fuzzer (seed 1, index 24) on cad-kernel 0.15.0, shrunk: a
/// 65 x 65 x 147 block with two blind holes, diameter 58.5, depth 111,
/// from the top, centred 1e-6 inside the x = 0 face and 58.500001 along
/// it (1e-6 apart, so not touching), both breaking out of the y = 65 face.
/// cad-kernel returns a valid B-rep at a volume inside the enclosure,
/// but its mesh is open at every chord (2 bad directed edges). arris
/// refuses it ("the faces touch along a curve interior to both result
/// faces").
fn fuzz_tangent_holes_breaking_out() -> PartCase {
    let name = "fuzz: two near-tangent blind holes breaking out (s1 i24)";
    part(
        name,
        def(
            7001,
            name,
            vec![
                cuboid(1, [0.0; 3], [65.0, 65.0, 147.0], Op::Add),
                hole(
                    2,
                    [1e-6, 61.074071, 147.0],
                    [0.0, 0.0, -1.0],
                    58.5,
                    Depth::Blind(111.0),
                ),
                hole(
                    3,
                    [58.500001, 61.074071, 147.0],
                    [0.0, 0.0, -1.0],
                    58.5,
                    Depth::Blind(111.0),
                ),
            ],
        ),
        vec![],
    )
}

/// A fuzzer finding in its compact form: the recipe as the shrinker
/// printed it (`fuzz::Recipe::encode`), built by the same code the fuzzer
/// judged it with.
fn fuzz_case(name: &str, recipe: &str) -> PartCase {
    crate::fuzz::Recipe::decode(recipe)
        .unwrap_or_else(|| panic!("{name}: not a fuzz recipe"))
        .case(name)
}

/// Every fuzzer finding, as catalogue parts.
fn fuzz_findings() -> Vec<PartCase> {
    let mut out = vec![fuzz_far_small_box(), fuzz_tangent_holes_breaking_out()];
    out.extend(FUZZ_FINDINGS.iter().map(|(n, r)| fuzz_case(n, r)));
    out
}

/// The other findings, shrunk, in compact form (`fuzz_case`). Each line
/// says which kernel the fuzzer found wrong (or refusing, or disagreeing)
/// and how; docs/research/kernel-comparison.md has the numbers.
const FUZZ_FINDINGS: &[(&str, &str)] = &[
    // Both kernels: a 0.02 x 0.1 x 0.03 box whose y faces are 1e-9 mm off
    // the base's; both volumes are 5e-9 and 3e-9 relative above the
    // enclosure (a face moved by about 5e-10 mm).
    (
        "fuzz: tiny box with a face 1e-9 off (s4 i20)",
        "box 24.0 79.0 37.0 tool Intersect b 0.02 0.1 0.03 0.0 0.0 0.0 0.0 0.0 1e-9 0.0",
    ),
    // arris: a through hole tangent to a side face (centre exactly one
    // radius in) through a box and a fused bar; arris's own check finds
    // the result invalid ("loop 0 intersects itself").
    (
        "fuzz: through hole tangent to a side face (s1 i196)",
        "box 17.0 17.0 7.0 tool Add b 80.0 7.0 4.0 0.0 0.0 0.0 0.0 0.0 0.0 6.557502359971457 \
         hole 28.0 7.728844132020787 6.557502359971457 0.0 0.0 -1.0 15.457688264041574 through nocb",
    ),
    // arris: a pin of radius 0.0113 fused 1.1e-6 inside a side face of a
    // plate; arris's own check finds the result invalid ("loops 0 and 1
    // intersect").
    (
        "fuzz: thin pin almost tangent to a side face (s2 i463)",
        "box 113.0 1108.1386016658328 1.0 tool Add c 0.011269122502257131 104.0 \
         0.0 0.0 0.0 0.0 7.0 1108.1273326433304 0.0",
    ),
    // arris: a 0.1 x 0.001 x 0.0001 notch 1e-7 inside a face; arris's own
    // check finds the result invalid (pcurves jump 1e-7).
    (
        "fuzz: tiny notch 1e-7 inside a face (s4 i733)",
        "box 47.0 2.0 156.0 tool Subtract b 0.1 0.001 0.0001 0.0 0.0 0.0 0.0 0.0 1e-7 0.0",
    ),
    // arris: a blind hole whose entry disc lies on the bottom face and
    // points out of the part (it removes nothing), half out of a side,
    // far from the origin; arris's own check finds the result invalid.
    (
        "fuzz: hole pointing out of the bottom face (s3 i353)",
        "box 319.8958400652496 17.0 49.0 hole 312.44490874650796 0.0 0.0 0.0 0.0 -1.0 \
         14.901862837483371 30.0 nocb motion 0.0 0.0 0.0 0.0 364002.0 0.0 0.0",
    ),
    // arris: a cylinder cut by a box whose face is 1e-6 mm off the
    // cylinder's tangent line; one shell step lands outside the reference.
    (
        "fuzz: box cut 1e-6 off a cylinder's tangent line (s2 i179)",
        "cyl 11.0 3238.0 tool Subtract b 67.0 191.0 96.0 0.0 0.0 0.0 0.0 0.0 1e-6 1136.0",
    ),
    // arris: a prism with collinear vertices, notched 1e-7 off its end,
    // far from the origin; 39 shell steps land outside the reference.
    (
        "fuzz: notched prism with collinear vertices far away (s3 i966)",
        "prism 7 127.0 0.0 87.0 1.0 35.0 1.0 0.0 0.0 18.0 0.0 88.0 0.0 118.0 0.0 966.0 \
         tool Subtract b 149.0 6.0 33.0 0.0 0.0 0.0 0.0 1e-7 -2.0 273.0 \
         motion 0.0 0.0 0.0 0.0 3103784.0 0.0 0.0",
    ),
    // cad-kernel: two blind holes exactly tangent, both breaking out of a
    // side; the mesh is open at every chord.
    (
        "fuzz: two tangent blind holes breaking out (s4 i0)",
        "box 5.0 5.0 170.0 hole 1.7098512661973304 3.183616 170.0 0.0 0.0 -1.0 4.5 11.0 nocb \
         hole 6.20985126619733 3.183616 170.0 0.0 0.0 -1.0 4.5 11.0 nocb",
    ),
    // Both kernels: a box intersected with a tool whose bottom is 1e-7 mm
    // above the base's (exact volume 6 x 22 x (2 - 1e-7) = 263.9999868);
    // both return 263.9999956, as if that face had moved 6.7e-8 mm down,
    // within each kernel's own tolerance.
    (
        "fuzz: intersection 1e-7 above a face (s3 i114)",
        "box 162.0 22.0 2.0 tool Intersect b 6.0 150.0 67.0 0.0 0.0 0.0 0.0 0.0 0.0 1e-7",
    ),
    // cad-kernel: a 0.02 thick plate intersected with a tool 0.001 above
    // it, far from the origin; the volume is outside the enclosure.
    (
        "fuzz: thin plate intersection 1e-3 up, far away (s2 i101)",
        "box 882.0 3.0 0.02 tool Intersect b 67.0 136.0 0.02 0.0 0.0 0.0 0.0 0.0 0.0 0.001 \
         motion 0.0 0.0 0.0 0.0 73264.0 0.0 18316.0",
    ),
    // cad-kernel: a 0.03 square rod with a 0.026 hole along its corner,
    // far from the origin; the mesh volume is half the stated volume.
    (
        "fuzz: hole along the corner of a thin rod far away (s1 i857)",
        "box 0.03 0.03 69.0 hole 0.0 0.0 69.0 0.0 0.0 -1.0 0.026 69.0 nocb \
         motion 0.0 0.0 0.0 0.0 6285032.0 -3142516.0 0.0",
    ),
    // cad-kernel: a 1 mm slab intersected with a turned 0.3 mm plate 1e-7
    // off its side, away from the origin; the mesh volume is off the
    // stated one by more than the chord's bound.
    (
        "fuzz: turned thin intersection 1e-7 off a side (s3 i547)",
        "box 1.0 117.0 168.0 tool Intersect b 0.3 128.0 43.0 0.0 0.0 0.0 2.0 0.5 1e-7 0.0 \
         motion 0.0 0.0 0.0 0.0 15653.0 0.0 0.0",
    ),
    // cad-kernel: panics ("a vertex at [1.7e12, ...]") fusing a cylinder
    // turned 1e-9 rad onto a tall box.
    (
        "fuzz: cylinder turned 1e-9 rad fused to a box (s2 i730)",
        "box 134.0 20.0 3754.0 tool Add c 100.0 2.0 -0.02 0.0 -1.0 -1e-9 0.0 0.0 0.0",
    ),
    // cad-kernel: a blind hole at a corner ending 1e-7 above the bottom
    // face; shell steps land outside the reference.
    (
        "fuzz: blind hole leaving a 1e-7 floor (s2 i152)",
        "box 2.0 2.0 64.0 hole 0.0 0.0 64.0 0.0 0.0 -1.0 2.0 63.9999999 nocb",
    ),
    // The cad-kernel adapter: both caps of a heptagon chamfered 0.3. The
    // flat on one edge cut away the next picked edge, which the adapter
    // then skipped (0.3 mm^3 too much material). The adapter now refuses.
    (
        "fuzz: heptagon chamfered on both caps (s4 i218)",
        "prism 7 1.4 10.0 1.0 12.0 0.0 11.0 0.0 4.0 0.2 0.0 1.0 0.0 2.0 4.0 198.0 chamfer 0.3 Both",
    ),
    // cad-kernel: its own check finds the result invalid (a vertex 1.6e-6
    // off its edge's curve).
    (
        "fuzz: intersection at the end of a long bar with a side hole (s2 i300)",
        "box 1839.742847 10.0 15.0 tool Intersect b 31.0 0.3 177.0 0.0 0.0 0.0 0.0 1808.742846 0.0 0.0 \
         hole 0.0 0.0 0.0 1.0 0.0 0.0 9.0 through nocb",
    ),
    // cad-kernel: its own check finds the result invalid (a pcurve 1.6e-9
    // off its B-spline edge) for a small washer cut by a turned cylinder.
    (
        "fuzz: small washer and turned cylinder (s4 i801)",
        "washer 0.1 0.09 22.0 tool Intersect c 0.03 0.04 0.0 -1.0 0.0 1.0 0.2 0.0 0.0",
    ),
    // cad-kernel: a 0.2 x 0.02 x 0.2 corner of a box 1e6 mm from the
    // origin; its volume is 4.4e-4 relative above the certified enclosure
    // (8.0035e-4 against at most 8.0000e-4). arris is correct.
    (
        "fuzz: tiny corner box far from the origin (s13 i455)",
        "box 27.0 136.0 95.0 tool Intersect b 0.2 0.02 0.2 0.0 0.0 0.0 0.0 0.0 0.0 0.0 \
         motion 0.0 0.0 0.0 0.0 0.0 -900032.0 450016.0",
    ),
    // arris: a cylinder inside a box, tangent to two of its sides, its
    // base on the box's: the union is the box, and arris returns a solid
    // its own check rejects ("loops 0 and 1 intersect") and cannot mesh.
    (
        "fuzz: cylinder tangent inside a box corner, fused (s14 i325)",
        "cyl 0.3 1.0 tool Add b 19.0 34.0 29.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0",
    ),
    // cad-kernel: a tall box intersected with a small plate turned 0.4
    // rad, 1e-5 off a side, at the top; its volume is 1.1e-2 relative off
    // its own mesh's (4.7853 against 4.7305; arris says 4.7304).
    (
        "fuzz: turned plate intersected at the top of a tall box (s11 i337)",
        "box 118.0 20.0 3482.0 tool Intersect b 1.0 10.0 2.0 1.0 0.0 0.0 0.4 1e-5 0.0 3480.0",
    ),
    // arris: a cylinder cut by a box turned 1e-4 rad; the mesh at chord
    // 0.1 is open (2 bad directed edges). cad-kernel is correct.
    (
        "fuzz: cylinder cut by a box turned 1e-4 rad (s12 i128)",
        "cyl 52.0 23.0 tool Subtract b 672.0 14.0 21.0 0.0 0.0 -1.0 -0.0001 0.0 0.0 0.0",
    ),
    // cad-kernel: a thin prism (0.1 wide at one end) with a side hole
    // along it; 29 shell steps land outside the reference. arris refuses.
    (
        "fuzz: side hole along a thin prism (s11 i61)",
        "prism 6 137.0 0.0 0.0 0.101768 0.0 0.1 0.0 0.0 70.0 0.0 136.0 0.0 1725.0 \
         hole 0.0 0.055972 0.0 1.0 0.0 0.0 0.091591 61.0 nocb",
    ),
    // Both: a box intersected with a cylinder far from the origin; the
    // volumes differ by more than 1e-9 relative and the fuzz enclosure
    // could not say which is right.
    (
        "fuzz: cylinder intersection far from the origin (s2 i11)",
        "box 182.0 1.0 55.0 tool Intersect c 67.0 72.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0 \
         motion 0.0 0.0 0.0 0.0 3633639.0 0.0 0.0",
    ),
];

/// Pairs of recipes for the same solid (layer 5), with their parts.
fn equivalences(corpus: &[PartCase]) -> (Vec<PartCase>, Vec<Relation>) {
    let named = |n: &str| corpus.iter().find(|p| p.name == n).cloned().expect(n);
    let mut parts = Vec::new();
    let mut rel = Vec::new();
    let mut pair = |name: &str, a: PartCase, b: PartCase, parts: &mut Vec<PartCase>| {
        rel.push(Relation::Equivalent {
            name: name.into(),
            a: a.name.clone(),
            b: b.name.clone(),
        });
        for p in [a, b] {
            if !parts.iter().any(|q: &PartCase| q.name == p.name)
                && !corpus.iter().any(|q| q.name == p.name)
            {
                parts.push(p);
            }
        }
    };

    // A box as one extrude, and as two halves fused on a shared face.
    let bx = |name: &str, f: Vec<Feature>| {
        part(name, def(30, name, f), vec![])
            .closed(120_000.0, aabb([0.0; 3], [100.0, 60.0, 20.0]))
            .euler(2)
    };
    let one = bx(
        "box",
        vec![cuboid(1, [0.0; 3], [100.0, 60.0, 20.0], Op::Add)],
    );
    let two = bx(
        "box from two halves",
        vec![
            cuboid(1, [0.0; 3], [50.0, 60.0, 20.0], Op::Add),
            cuboid(2, [50.0, 0.0, 0.0], [100.0, 60.0, 20.0], Op::Add),
        ],
    );
    pair(
        "box: one extrude = two fused halves",
        one.clone(),
        two,
        &mut parts,
    );

    // The same box extruded down from z = 20, and from a flipped plane.
    let down = bx(
        "box extruded down",
        vec![extrude(
            1,
            Profile2d::rect(0.0, 0.0, 100.0, 60.0),
            Plane3::XY.offset(20.0),
            -20.0,
            Op::Add,
        )],
    );
    // Normal -Z, x_axis X: y = n x x = -Y, so (u, v) = (x, -y).
    let flipped = bx(
        "box on a flipped plane",
        vec![extrude(
            1,
            Profile2d::rect(0.0, -60.0, 100.0, 0.0),
            Plane3 {
                origin: Point3::new(0.0, 0.0, 20.0),
                x_axis: Vec3::X,
                normal: Vec3::new(0.0, 0.0, -1.0),
            },
            20.0,
            Op::Add,
        )],
    );
    pair(
        "box: extrude up = extrude down",
        one.clone(),
        down.clone(),
        &mut parts,
    );
    pair(
        "box: extrude down = flipped plane",
        down,
        flipped,
        &mut parts,
    );

    // A box as the intersection of two longer prisms. The XZ plane has
    // normal -Y and (u, v) = (x, z); at y = 65 extruded 70 along -Y it
    // spans y in [-5, 65].
    let inter = bx(
        "box as an intersection",
        vec![
            cuboid(1, [0.0; 3], [100.0, 60.0, 30.0], Op::Add),
            extrude(
                2,
                Profile2d::rect(-5.0, 0.0, 105.0, 20.0),
                Plane3::XZ.at(Point3::new(0.0, 65.0, 0.0)),
                70.0,
                Op::Intersect,
            ),
        ],
    );
    pair(
        "box: one extrude = intersection of prisms",
        one,
        inter,
        &mut parts,
    );

    // A through hole as a Hole, and as a boolean with a cylinder tool.
    let hole_tool = def(
        131,
        "drill",
        vec![extrude(
            1,
            Profile2d::circle(Point::new(0.0, 0.0), 5.0),
            Plane3::XY,
            30.0,
            Op::Add,
        )],
    );
    let hv = 200.0 * 100.0 * 18.0 - 450.0 * PI;
    let hb = aabb([0.0; 3], [200.0, 100.0, 18.0]);
    let by_hole = part(
        "plate with hole",
        def(
            32,
            "plate with hole",
            vec![
                cuboid(1, [0.0; 3], [200.0, 100.0, 18.0], Op::Add),
                hole(
                    2,
                    [50.0, 50.0, 18.0],
                    [0.0, 0.0, -1.0],
                    10.0,
                    Depth::Through,
                ),
            ],
        ),
        vec![],
    )
    .closed(hv, hb)
    .euler(0);
    let by_tool = part(
        "plate with drilled tool",
        def(
            33,
            "plate with drilled tool",
            vec![
                cuboid(1, [0.0; 3], [200.0, 100.0, 18.0], Op::Add),
                boolean(
                    2,
                    Op::Subtract,
                    131,
                    Placement::translation(50.0, 50.0, -6.0),
                ),
            ],
        ),
        vec![hole_tool.clone()],
    )
    .closed(hv, hb)
    .euler(0);
    pair(
        "through hole: hole = cylinder tool",
        by_hole,
        by_tool,
        &mut parts,
    );

    // The counterbore as a hole, and as two cylinder booleans.
    let cb = named("block with counterbore");
    let small = def(
        134,
        "small drill",
        vec![extrude(
            1,
            Profile2d::circle(Point::new(0.0, 0.0), 4.5),
            Plane3::XY,
            32.0,
            Op::Add,
        )],
    );
    let big = def(
        135,
        "counterbore drill",
        vec![extrude(
            1,
            Profile2d::circle(Point::new(0.0, 0.0), 7.5),
            Plane3::XY,
            10.0,
            Op::Add,
        )],
    );
    let cb2 = part(
        "block with two cylinder cuts",
        def(
            36,
            "block with two cylinder cuts",
            vec![
                cuboid(1, [0.0; 3], [100.0, 100.0, 30.0], Op::Add),
                boolean(
                    2,
                    Op::Subtract,
                    134,
                    Placement::translation(50.0, 50.0, -1.0),
                ),
                boolean(
                    3,
                    Op::Subtract,
                    135,
                    Placement::translation(50.0, 50.0, 21.0),
                ),
            ],
        ),
        vec![small, big],
    )
    .closed(cb.volume.unwrap(), cb.bbox.unwrap())
    .euler(0);
    pair(
        "counterbore: hole = two cylinder tools",
        cb,
        cb2,
        &mut parts,
    );

    // The rabbet as a subtracted extrude, and as an L section run along X.
    let rb = named("back with rabbet");
    let l_section = Profile2d::polygon(&[
        Point::new(0.0, 0.0),
        Point::new(300.0, 0.0),
        Point::new(300.0, 9.0),
        Point::new(288.0, 9.0),
        Point::new(288.0, 18.0),
        Point::new(0.0, 18.0),
    ]);
    let rb2 = part(
        "rabbet as an L section",
        def(
            37,
            "rabbet as an L section",
            vec![extrude(1, l_section, Plane3::YZ, 600.0, Op::Add)],
        ),
        vec![],
    )
    .closed(rb.volume.unwrap(), rb.bbox.unwrap())
    .euler(2);
    pair(
        "rabbet: subtracted extrude = L section",
        rb,
        rb2,
        &mut parts,
    );

    // The dado as a boolean tool, and as a subtracted extrude.
    let dd = named("side with dado");
    let dd2 = part(
        "dado as a subtracted extrude",
        def(
            38,
            "dado as a subtracted extrude",
            vec![
                cuboid(1, [0.0; 3], [600.0, 300.0, 18.0], Op::Add),
                cuboid(2, [291.0, -5.0, 12.0], [309.0, 305.0, 22.0], Op::Subtract),
            ],
        ),
        vec![],
    )
    .closed(dd.volume.unwrap(), dd.bbox.unwrap())
    .euler(2);
    pair(
        "dado: boolean tool = subtracted extrude",
        dd,
        dd2,
        &mut parts,
    );

    // The mortise as a subtracted extrude, and as a boolean tool.
    let mt = named("stile with mortise");
    let pocket = def(
        139,
        "pocket",
        vec![cuboid(1, [0.0; 3], [50.0, 15.0, 30.0], Op::Add)],
    );
    let mt2 = part(
        "mortise as a boolean tool",
        def(
            40,
            "mortise as a boolean tool",
            vec![
                extrude(
                    1,
                    Profile2d::rect(0.0, 0.0, 45.0, 45.0),
                    Plane3::YZ,
                    400.0,
                    Op::Add,
                ),
                boolean(
                    2,
                    Op::Subtract,
                    139,
                    Placement::translation(175.0, 15.0, 15.0),
                ),
            ],
        ),
        vec![pocket],
    )
    .closed(mt.volume.unwrap(), mt.bbox.unwrap())
    .euler(2);
    pair(
        "mortise: subtracted extrude = boolean tool",
        mt,
        mt2,
        &mut parts,
    );

    // The washer as a profile with a hole, and as a subtracted cylinder
    // (coplanar caps).
    let w = washer();
    let w2 = part(
        "washer as a subtracted cylinder",
        def(
            44,
            "washer as a subtracted cylinder",
            vec![
                extrude(
                    1,
                    Profile2d::circle(Point::new(0.0, 0.0), 10.0),
                    Plane3::XY,
                    -3.0,
                    Op::Add,
                ),
                extrude(
                    2,
                    Profile2d::circle(Point::new(0.0, 0.0), 4.0),
                    Plane3::XY,
                    -3.0,
                    Op::Subtract,
                ),
            ],
        ),
        vec![],
    )
    .closed(w.volume.unwrap(), w.bbox.unwrap())
    .euler(0);
    pair("washer: hole loop = subtracted cylinder", w, w2, &mut parts);

    // The slot from a bulge polyline, and as a rectangle plus two
    // cylinders (tangent contacts, coplanar caps).
    let s = slot();
    let cyl = |id: u32, x: f64| {
        extrude(
            id,
            Profile2d::circle(Point::new(x, 5.0), 5.0),
            Plane3::XY,
            10.0,
            Op::Add,
        )
    };
    let s2 = part(
        "slot as a box and two cylinders",
        def(
            45,
            "slot as a box and two cylinders",
            vec![
                cuboid(1, [0.0; 3], [40.0, 10.0, 10.0], Op::Add),
                cyl(2, 0.0),
                cyl(3, 40.0),
            ],
        ),
        vec![],
    )
    .closed(s.volume.unwrap(), s.bbox.unwrap())
    .euler(2);
    pair("slot: bulges = box and cylinders", s, s2, &mut parts);

    // Chamfering both caps at once, and one cap after the other:
    // each cap removes (L + W) d^2 - 4 d^3 / 3 (the chamfered slab's
    // closed form), so V = 100 * 60 * 20 - 2 (160 * 4 - 32 / 3).
    let cv = 120_000.0 - 2.0 * (160.0 * 4.0 - 32.0 / 3.0);
    let cbx = aabb([0.0; 3], [100.0, 60.0, 20.0]);
    let both = part(
        "box chamfered on both caps",
        def(
            46,
            "box chamfered on both caps",
            vec![
                cuboid(1, [0.0; 3], [100.0, 60.0, 20.0], Op::Add),
                chamfer_cap(2, 1, CapSide::Both, 2.0),
            ],
        ),
        vec![],
    )
    .closed(cv, cbx)
    .euler(2);
    let seq = part(
        "box chamfered cap by cap",
        def(
            47,
            "box chamfered cap by cap",
            vec![
                cuboid(1, [0.0; 3], [100.0, 60.0, 20.0], Op::Add),
                chamfer_cap(2, 1, CapSide::Start, 2.0),
                chamfer_cap(3, 1, CapSide::End, 2.0),
            ],
        ),
        vec![],
    )
    .closed(cv, cbx)
    .euler(2);
    pair(
        "chamfer: both caps = one cap then the other",
        both,
        seq,
        &mut parts,
    );

    (parts, rel)
}

/// A generic rotation (about (1, 2, 3) by 0.7 rad) and translation.
pub fn generic_motion() -> Placement {
    Placement::axis_angle(Vec3::new(1.0, 2.0, 3.0), 0.7, Vec3::new(13.0, -7.0, 5.0))
}

/// Algebra and motion cases (layer 6) on a set of bases.
fn algebra(all: &[PartCase]) -> (Vec<PartCase>, Vec<Relation>) {
    let named = |n: &str| all.iter().find(|p| p.name == n).cloned().expect(n);
    let bases = [
        "board with hole",
        "side with dado",
        "rail with half-lap",
        "block with counterbore",
        "chamfered slab",
        "slot",
        "ellipse",
    ];
    let mut parts = Vec::new();
    let mut rel = Vec::new();
    let m = generic_motion();
    for (k, bn) in bases.iter().enumerate() {
        let b = named(bn);
        // Motion and mirror of the whole recipe.
        let mv = moved_case(&b, &m);
        rel.push(Relation::Motion {
            name: format!("motion: {bn}"),
            base: b.name.clone(),
            moved: mv.name.clone(),
            motion: m,
        });
        let mut mv = mv;
        mv.euler = b.euler;
        parts.push(mv);
        let x0 = 37.0;
        let mut mr = mirrored_case(&b, x0);
        mr.euler = b.euler;
        rel.push(Relation::Mirror {
            name: format!("mirror: {bn}"),
            base: b.name.clone(),
            mirrored: mr.name.clone(),
            x0,
        });
        parts.push(mr);
        // A + A, A n A, A - A as booleans of the part as a tool.
        let tid = 50_000 + k as u64;
        let (t, tt) = as_tool(&b, tid);
        let tools = merge_tools(vec![vec![t], tt]);
        let id = Placement::IDENTITY;
        let mut aa = assembly(
            &format!("{bn} + itself"),
            (tid, id),
            &[(Op::Add, tid, id)],
            tools.clone(),
        );
        aa.euler = b.euler;
        let mut ai = assembly(
            &format!("{bn} n itself"),
            (tid, id),
            &[(Op::Intersect, tid, id)],
            tools.clone(),
        );
        ai.euler = b.euler;
        let am = assembly(
            &format!("{bn} - itself"),
            (tid, id),
            &[(Op::Subtract, tid, id)],
            tools.clone(),
        );
        rel.push(Relation::Same {
            name: format!("A + A = A: {bn}"),
            a: b.name.clone(),
            b: aa.name.clone(),
        });
        rel.push(Relation::Same {
            name: format!("A n A = A: {bn}"),
            a: b.name.clone(),
            b: ai.name.clone(),
        });
        rel.push(Relation::Empty {
            name: format!("A - A = empty: {bn}"),
            part: am.name.clone(),
            of: b.name.clone(),
        });
        parts.extend([aa, ai, am]);
    }

    // Union identity and operand order on overlapping pairs.
    let pairs: [(&str, &str, Placement); 3] = [
        (
            "board with hole",
            "block with counterbore",
            Placement::axis_angle(Vec3::Z, PI / 6.0, Vec3::new(20.0, -30.0, 5.0)),
        ),
        (
            "slot",
            "ellipse",
            Placement::axis_angle(Vec3::Z, 0.4, Vec3::new(40.0, 5.0, 3.0)),
        ),
        (
            "side with dado",
            "rail with half-lap",
            Placement::axis_angle(
                Vec3::new(0.0, 0.0, 1.0),
                PI / 2.0,
                Vec3::new(330.0, -20.0, 9.0),
            ),
        ),
    ];
    for (k, (an, bn, pb)) in pairs.iter().enumerate() {
        let (a, b) = (named(an), named(bn));
        let (ta, ida) = (as_tool(&a, 60_000 + 2 * k as u64), 60_000 + 2 * k as u64);
        let (tb, idb) = (as_tool(&b, 60_001 + 2 * k as u64), 60_001 + 2 * k as u64);
        let tools = merge_tools(vec![vec![ta.0, tb.0], ta.1, tb.1]);
        let id = Placement::IDENTITY;
        let tag = format!("{an} and {bn}");
        let u = assembly(
            &format!("{tag}: A u B"),
            (ida, id),
            &[(Op::Add, idb, *pb)],
            tools.clone(),
        );
        let u2 = assembly(
            &format!("{tag}: B u A"),
            (idb, *pb),
            &[(Op::Add, ida, id)],
            tools.clone(),
        );
        let i = assembly(
            &format!("{tag}: A n B"),
            (ida, id),
            &[(Op::Intersect, idb, *pb)],
            tools.clone(),
        );
        let i2 = assembly(
            &format!("{tag}: B n A"),
            (idb, *pb),
            &[(Op::Intersect, ida, id)],
            tools.clone(),
        );
        // B placed on its own, for V(B).
        let bp = assembly(&format!("{tag}: B placed"), (idb, *pb), &[], tools.clone());
        rel.push(Relation::UnionIdentity {
            name: format!("union identity: {tag}"),
            a: a.name.clone(),
            b: bp.name.clone(),
            union: u.name.clone(),
            inter: i.name.clone(),
        });
        rel.push(Relation::Same {
            name: format!("operand order (union): {tag}"),
            a: u.name.clone(),
            b: u2.name.clone(),
        });
        rel.push(Relation::Same {
            name: format!("operand order (intersection): {tag}"),
            a: i.name.clone(),
            b: i2.name.clone(),
        });
        parts.extend([u, u2, i, i2, bp]);
    }

    // Splitting by an oblique plane: a big slab (the half-space side) as a
    // tool, rotated 0.3 rad about (1, 1, 0) and through the part.
    let splits: [(&str, [f64; 3]); 3] = [
        ("board with hole", [60.0, 50.0, 9.0]),
        ("block with counterbore", [50.0, 52.0, 15.0]),
        ("slot", [38.0, 5.0, 5.0]),
    ];
    for (k, (bn, at)) in splits.iter().enumerate() {
        let b = named(bn);
        let tid = 70_000 + 2 * k as u64;
        let hid = tid + 1;
        let (t, tt) = as_tool(&b, tid);
        // A slab 2000 x 2000 x 1000 with its bottom face through `at`.
        let half = def(
            hid,
            "half space",
            vec![cuboid(
                1,
                [-1000.0, -1000.0, 0.0],
                [1000.0, 1000.0, 1000.0],
                Op::Add,
            )],
        );
        let tools = merge_tools(vec![vec![t, half], tt]);
        let pl = Placement::axis_angle(
            Vec3::new(1.0, 1.0, 0.0),
            0.3,
            Vec3::new(at[0], at[1], at[2]),
        );
        let id = Placement::IDENTITY;
        let above = assembly(
            &format!("{bn} split: above"),
            (tid, id),
            &[(Op::Intersect, hid, pl)],
            tools.clone(),
        );
        let below = assembly(
            &format!("{bn} split: below"),
            (tid, id),
            &[(Op::Subtract, hid, pl)],
            tools.clone(),
        );
        rel.push(Relation::Split {
            name: format!("split by a plane: {bn}"),
            whole: b.name.clone(),
            pieces: [above.name.clone(), below.name.clone()],
        });
        parts.extend([above, below]);
    }
    (parts, rel)
}

/// The parameter values of a sweep through a contact at `s = 0`: whole
/// steps, then geometric steps down to 1e-9, zero, and the mirror image.
pub fn sweep_params() -> Vec<f64> {
    let mut neg: Vec<f64> = vec![1.0, 0.1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6, 1e-7, 1e-8, 1e-9];
    let mut out: Vec<f64> = neg.iter().map(|x| -x).collect();
    out.push(0.0);
    neg.reverse();
    out.extend(neg);
    out
}

fn sweeps() -> Vec<Sweep> {
    let params = sweep_params();
    let mut out = Vec::new();
    let mk =
        |name: &str, f: &dyn Fn(f64) -> (Vec<Feature>, Vec<PartDef>)| -> Vec<(f64, PartCase)> {
            params
                .iter()
                .map(|&s| {
                    let (features, tools) = f(s);
                    let n = format!("{name} @ {s:e}");
                    (s, part(&n, def(80, &n, features), tools))
                })
                .collect()
        };

    // A 20 x 20 x 10 box tool cut into the top of a 600 x 300 x 18 board,
    // its bottom at 18 - s: no contact below 0, flush at 0, cutting s
    // deep above. V = V0 - 400 max(s, 0): never increases, slope 400.
    let tool = def(
        81,
        "box tool",
        vec![cuboid(1, [0.0; 3], [20.0, 20.0, 10.0], Op::Add)],
    );
    let t1 = tool.clone();
    out.push(Sweep {
        name: "box tool through a flush top face".into(),
        steps: mk("flush top", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [600.0, 300.0, 18.0], Op::Add),
                    boolean(
                        2,
                        Op::Subtract,
                        81,
                        Placement::translation(100.0, 100.0, 18.0 - s),
                    ),
                ],
                vec![t1.clone()],
            )
        }),
        slope: 400.0,
        direction: -1.0,
        area: 393_600.0,
    });

    // The dado tool sliding along X past the board's end face: it spans x
    // in [582 + s, 600 + s]; its end is flush with the board's at s = 0.
    // Removed = 300 * 6 * (18 - max(s, 0)): never decreases, slope 1800.
    let dado = def(
        82,
        "dado tool",
        vec![cuboid(1, [0.0; 3], [18.0, 310.0, 10.0], Op::Add)],
    );
    out.push(Sweep {
        name: "dado tool through a flush end face".into(),
        steps: mk("flush end", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [600.0, 300.0, 18.0], Op::Add),
                    boolean(
                        2,
                        Op::Subtract,
                        82,
                        Placement::translation(582.0 + s, -5.0, 12.0),
                    ),
                ],
                vec![dado.clone()],
            )
        }),
        slope: 1800.0,
        direction: 1.0,
        area: 393_600.0,
    });

    // A cylinder r = 5 (through, z in [-1, 19]) approaching the side face
    // y = 0 of a 200 x 100 x 18 plate: centre at y = -5 + s, tangent to
    // the face at s = 0. The removed area is a circular segment; its rate
    // is the chord (at most 2r = 10) times the height 18: slope 180.
    let cyl = def(
        83,
        "cylinder tool",
        vec![extrude(
            1,
            Profile2d::circle(Point::new(0.0, 0.0), 5.0),
            Plane3::XY,
            20.0,
            Op::Add,
        )],
    );
    out.push(Sweep {
        name: "cylinder tool through tangency with a face".into(),
        steps: mk("tangent cylinder", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [200.0, 100.0, 18.0], Op::Add),
                    boolean(
                        2,
                        Op::Subtract,
                        83,
                        Placement::translation(100.0, -5.0 + s, -1.0),
                    ),
                ],
                vec![cyl.clone()],
            )
        }),
        slope: 180.0,
        direction: -1.0,
        area: 50_800.0,
    });

    // A through hole r = 5 moving out through the side face y = 0: centre
    // at y = 5 - s, internally tangent at s = 0, breaking out above.
    // Less material is removed as it leaves: never decreases, slope 180.
    out.push(Sweep {
        name: "hole breaking out through a face".into(),
        steps: mk("hole breakout", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [200.0, 100.0, 18.0], Op::Add),
                    hole(
                        2,
                        [100.0, 5.0 - s, 18.0],
                        [0.0, 0.0, -1.0],
                        10.0,
                        Depth::Through,
                    ),
                ],
                vec![],
            )
        }),
        slope: 180.0,
        direction: 1.0,
        area: 50_800.0,
    });

    // Two through holes r = 5, centres 10 - s apart: tangent at s = 0,
    // overlapping above. The overlap grows, so less is removed: never
    // decreases, slope at most the lens chord 10 times 18.
    out.push(Sweep {
        name: "two holes through tangency".into(),
        steps: mk("two holes", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [200.0, 100.0, 18.0], Op::Add),
                    hole(
                        2,
                        [100.0, 50.0, 18.0],
                        [0.0, 0.0, -1.0],
                        10.0,
                        Depth::Through,
                    ),
                    hole(
                        3,
                        [110.0 - s, 50.0, 18.0],
                        [0.0, 0.0, -1.0],
                        10.0,
                        Depth::Through,
                    ),
                ],
                vec![],
            )
        }),
        slope: 180.0,
        direction: 1.0,
        area: 50_800.0,
    });

    // A 20 x 20 x 10 box fused onto a 100 x 100 x 18 block, its bottom at
    // 18 + s: overlapping below 0, face to face at 0, apart (two lumps)
    // above. V = V0 + 4000 - 400 max(-s, 0): never decreases, slope 400.
    let t6 = tool.clone();
    out.push(Sweep {
        name: "fused box through face contact".into(),
        steps: mk("fused box", &|s| {
            (
                vec![
                    cuboid(1, [0.0; 3], [100.0, 100.0, 18.0], Op::Add),
                    boolean(2, Op::Add, 81, Placement::translation(40.0, 40.0, 18.0 + s)),
                ],
                vec![t6.clone()],
            )
        }),
        slope: 400.0,
        direction: 1.0,
        area: 28_000.0,
    });

    // A box tool approaching a cylinder of radius R = 20 and height 30
    // made by a revolve (the rectangle rho in [0, 20], z in [0, 30] turned
    // about the z axis): the tool's face y = 20 - s touches the curved
    // face along a line at s = 0 (a tangency) and cuts a flat above. The
    // removed area grows at the chord 2 sqrt(2 R s - s^2) <= 2 R = 40 per
    // unit of s, times the height 30: never increases, slope 1 200. Area:
    // the cylinder's 2 pi 20 30 + 2 pi 400 < 6 300 plus a flat of at
    // most 40 x 30.
    let flat = def(
        84,
        "flat tool",
        vec![cuboid(1, [0.0; 3], [60.0, 20.0, 40.0], Op::Add)],
    );
    out.push(Sweep {
        name: "box tool through tangency with a revolved cylinder".into(),
        steps: mk("revolved cylinder flat", &|s| {
            (
                vec![
                    Feature::Revolve {
                        id: FeatId(1),
                        profile: Profile2d::rect(0.0, 0.0, 20.0, 30.0),
                        plane: Plane3::XZ,
                        axis: Line::new((0.0, 0.0), (0.0, 1.0)),
                        angle_deg: 360.0,
                        op: Op::Add,
                    },
                    boolean(
                        2,
                        Op::Subtract,
                        84,
                        Placement::translation(-30.0, 20.0 - s, -5.0),
                    ),
                ],
                vec![flat.clone()],
            )
        }),
        slope: 1200.0,
        direction: -1.0,
        area: 8_000.0,
    });
    out
}

/// The whole catalogue.
pub fn catalogue() -> Catalogue {
    let mut parts = corpus_parts();
    parts.extend([
        slot(),
        washer(),
        ellipse(),
        half_ellipse(),
        near_chamfer(),
        l_chamfer(),
        cavity(),
        hole_below_surface(),
    ]);
    let (eq_parts, mut relations) = equivalences(&parts);
    for p in eq_parts {
        if !parts.iter().any(|q| q.name == p.name) {
            parts.push(p);
        }
    }
    let (al_parts, al_rel) = algebra(&parts);
    parts.extend(al_parts);
    relations.extend(al_rel);
    parts.extend(fuzz_findings());
    // Rung 3D0b: the curved features.
    parts.extend(curved_corpus_parts());
    parts.extend(curved_extra());
    let (ce_parts, ce_rel) = curved_equivalences();
    parts.extend(ce_parts);
    relations.extend(ce_rel);
    let (ca_parts, ca_rel) = curved_algebra(&parts);
    parts.extend(ca_parts);
    relations.extend(ca_rel);
    let names: std::collections::HashSet<&str> = parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names.len(), parts.len(), "part names must be unique");
    Catalogue {
        parts,
        relations,
        sweeps: sweeps(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::{Class, Reference};

    #[test]
    fn mirror_quaternion_is_l_r_l() {
        // (w, x, -y, -z) acts as L R L for L = diag(-1, 1, 1).
        let p = Placement::axis_angle(Vec3::new(1.0, 2.0, 3.0), 0.7, Vec3::new(0.0, 0.0, 0.0));
        let [w, x, y, z] = p.rotation;
        let lrl = [w, x, -y, -z];
        let l = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        for v in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(0.3, -2.0, 1.5)] {
            let a = qrot(lrl, v);
            let b = l(qrot(p.rotation, l(v)));
            assert!(
                (a.x - b.x).abs() < 1e-14 && (a.y - b.y).abs() < 1e-14 && (a.z - b.z).abs() < 1e-14
            );
        }
    }

    #[test]
    fn transforms_move_reference_points() {
        let c = catalogue();
        let base = c.part("rail with half-lap").unwrap();
        let rb = Reference::build(&base.def, &base.tools).unwrap();
        let m = generic_motion();
        let mv = moved_case(base, &m);
        let rm = Reference::build(&mv.def, &mv.tools).unwrap();
        let mr = mirrored_case(base, 37.0);
        let rr = Reference::build(&mr.def, &mr.tools).unwrap();
        let mut rng = crate::rng::Rng::named("transforms", 0);
        let mut checked = 0;
        for _ in 0..2000 {
            let p = Point3::new(
                rng.range(-10.0, 410.0),
                rng.range(-10.0, 60.0),
                rng.range(-5.0, 25.0),
            );
            let a = rb.classify(p);
            let pm = {
                let r = qrot(m.rotation, Vec3::new(p.x, p.y, p.z)) + m.translation;
                Point3::new(r.x, r.y, r.z)
            };
            let b = rm.classify(pm);
            let c2 = rr.classify(Point3::new(74.0 - p.x, p.y, p.z));
            let side = |c: Class| match c {
                Class::Inside { .. } => Some(true),
                Class::Outside { .. } => Some(false),
                Class::Band => None,
            };
            if let (Some(x), Some(y), Some(z)) = (side(a), side(b), side(c2)) {
                assert_eq!(x, y, "motion at {p:?}");
                assert_eq!(x, z, "mirror at {p:?}");
                checked += 1;
            }
        }
        assert!(checked > 1900, "{checked}");
    }

    #[test]
    fn equivalent_recipes_have_equal_references() {
        let c = catalogue();
        for r in &c.relations {
            let (a, b) = match r {
                Relation::Equivalent { a, b, .. } | Relation::Same { a, b, .. } => (a, b),
                _ => continue,
            };
            let (pa, pb) = (c.part(a).unwrap(), c.part(b).unwrap());
            let (ra, rb) = match (
                Reference::build(&pa.def, &pa.tools),
                Reference::build(&pb.def, &pb.tools),
            ) {
                (Ok(x), Ok(y)) => (x, y),
                _ => continue,
            };
            let bb = ra.bounds();
            let mut rng = crate::rng::Rng::named(r.name(), 7);
            for _ in 0..500 {
                let p = Point3::new(
                    rng.range(bb.min[0] - 1.0, bb.max[0] + 1.0),
                    rng.range(bb.min[1] - 1.0, bb.max[1] + 1.0),
                    rng.range(bb.min[2] - 1.0, bb.max[2] + 1.0),
                );
                let (x, y) = (ra.classify(p), rb.classify(p));
                let side = |c: Class| match c {
                    Class::Inside { .. } => Some(true),
                    Class::Outside { .. } => Some(false),
                    Class::Band => None,
                };
                if let (Some(x), Some(y)) = (side(x), side(y)) {
                    assert_eq!(x, y, "{}: {p:?}", r.name());
                }
            }
        }
    }
}
