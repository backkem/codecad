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

use cadview_core::solid::corpus::prismatic;
use cadview_core::solid::{
    Aabb3, CapSide, DefId, Depth, EdgeSel, FeatId, Feature, Op, PartDef, Placement, Plane3, Point3,
    Profile2d, Seg2, ToolRef, ToolResolver, Vec3,
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
