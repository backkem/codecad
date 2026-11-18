//! Broken adapters (design section 7): a wrapper around a real kernel that
//! makes its result subtly wrong while the harness still sees, and builds
//! its reference from, the original recipe. Every mutant must be flagged
//! as wrong by some layer; `tests/mutants.rs` checks that and reports
//! which layers caught each one.

use cadview_core::solid::{
    Aabb3, DefId, EvalError, Evaluated, Feature, MassProps, Op, PartDef, Placement, Point3,
    SolidKernel, ToolRef, ToolResolver, TriMesh, Validity, Vec3,
};

use crate::cases::Tools;

/// One way to be wrong.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mutation {
    /// Every boolean tool moved by this distance (mm) along +X.
    ShiftTool(f64),
    /// Every hole's diameter grown by this much (mm).
    HoleDiameter(f64),
    /// The last feature of every part is ignored.
    DropLastFeature,
    /// The last feature with an op uses another one (subtract becomes add,
    /// add and intersect become subtract).
    SwapLastOp,
    /// Subtracting booleans whose tool lies strictly inside the part so
    /// far (closed cavities) are ignored: a plausible kernel bug that
    /// leaves no wrong surface behind, only a missing one.
    DropCavities,
    /// Every boolean tool translation rounded to a grid of this size (mm):
    /// right for round numbers, wrong near contacts.
    SnapTools(f64),
    /// `bbox` ignores the rotation of the frame it is asked for.
    BboxIgnoresRotation,
    /// `mass` reports the volume times this factor.
    MassScale(f64),
    /// The middle triangle of every mesh is missing.
    DropTriangle,
    /// The middle triangle of every mesh is reversed.
    FlipTriangle,
    /// The first vertex of every mesh is moved this far (mm) along
    /// (1, 1, 1) / sqrt(3).
    MoveVertex(f64),
}

impl Mutation {
    pub fn label(&self) -> String {
        match self {
            Mutation::ShiftTool(d) => format!("tool shifted {d} mm"),
            Mutation::HoleDiameter(d) => format!("hole diameter +{d} mm"),
            Mutation::DropLastFeature => "last feature dropped".into(),
            Mutation::SwapLastOp => "last op swapped".into(),
            Mutation::DropCavities => "closed cavities dropped".into(),
            Mutation::SnapTools(g) => format!("tool positions snapped to {g} mm"),
            Mutation::BboxIgnoresRotation => "bbox ignores frame rotation".into(),
            Mutation::MassScale(f) => format!("volume scaled by {f}"),
            Mutation::DropTriangle => "mesh: triangle dropped".into(),
            Mutation::FlipTriangle => "mesh: triangle flipped".into(),
            Mutation::MoveVertex(d) => format!("mesh: vertex moved {d} mm"),
        }
    }
}

/// A kernel with one mutation.
pub struct Broken<K> {
    pub inner: K,
    pub mutation: Mutation,
}

fn map_tools(def: &PartDef, f: &dyn Fn(Placement) -> Placement) -> PartDef {
    let mut d = def.clone();
    for feat in &mut d.features {
        if let Feature::Boolean {
            tool: ToolRef::Def(_, p),
            ..
        } = feat
        {
            *p = f(*p);
        }
    }
    d
}

/// Every tool definition reachable from `def`.
fn reachable(def: &PartDef, tools: &dyn ToolResolver) -> Vec<PartDef> {
    let mut out: Vec<PartDef> = Vec::new();
    let mut todo = vec![def.clone()];
    while let Some(d) = todo.pop() {
        for f in &d.features {
            if let Feature::Boolean {
                tool: ToolRef::Def(id, _),
                ..
            } = f
            {
                if !out.iter().any(|t| t.id == *id) {
                    if let Some(t) = tools.def(*id) {
                        out.push(t.clone());
                        todo.push(t.clone());
                    }
                }
            }
        }
    }
    out
}

impl<K: SolidKernel> Broken<K> {
    /// The mutated recipe and tools.
    fn mutate(&self, def: &PartDef, tools: &dyn ToolResolver) -> (PartDef, Tools) {
        let all = reachable(def, tools);
        let per_def = |d: &PartDef| -> PartDef {
            match self.mutation {
                Mutation::ShiftTool(s) => map_tools(d, &|p| Placement {
                    translation: p.translation + Vec3::new(s, 0.0, 0.0),
                    ..p
                }),
                Mutation::SnapTools(g) => map_tools(d, &|p| Placement {
                    translation: Vec3::new(
                        (p.translation.x / g).round() * g,
                        (p.translation.y / g).round() * g,
                        (p.translation.z / g).round() * g,
                    ),
                    ..p
                }),
                Mutation::HoleDiameter(e) => {
                    let mut d = d.clone();
                    for f in &mut d.features {
                        if let Feature::Hole { dia, .. } = f {
                            *dia += e;
                        }
                    }
                    d
                }
                _ => d.clone(),
            }
        };
        let mut top = per_def(def);
        let tools = Tools(all.iter().map(per_def).collect());
        match self.mutation {
            Mutation::DropLastFeature if top.features.len() > 1 => {
                top.features.pop();
            }
            Mutation::SwapLastOp => {
                for f in top.features.iter_mut().rev() {
                    let op = match f {
                        Feature::Extrude { op, .. } | Feature::Boolean { op, .. } => op,
                        _ => continue,
                    };
                    *op = match *op {
                        Op::Subtract => Op::Add,
                        Op::Add | Op::Intersect => Op::Subtract,
                    };
                    break;
                }
            }
            Mutation::DropCavities => {
                let mut keep = Vec::new();
                for (i, f) in top.features.iter().enumerate() {
                    let cavity = match f {
                        Feature::Boolean {
                            op: Op::Subtract,
                            tool: ToolRef::Def(id, p),
                            ..
                        } => self.strictly_inside(&top, i, *id, p, &tools),
                        _ => false,
                    };
                    if !cavity {
                        keep.push(f.clone());
                    }
                }
                top.features = keep;
            }
            _ => {}
        }
        (top, tools)
    }

    /// Whether tool `id` placed by `p` lies strictly inside the box of the
    /// part's first `i` features (as the inner kernel evaluates them).
    fn strictly_inside(
        &self,
        def: &PartDef,
        i: usize,
        id: DefId,
        p: &Placement,
        tools: &Tools,
    ) -> bool {
        let prefix = PartDef {
            features: def.features[..i].to_vec(),
            ..def.clone()
        };
        let (Some(tool), Ok(part)) = (tools.def(id), self.inner.evaluate(&prefix, tools)) else {
            return false;
        };
        let Ok(t) = self.inner.evaluate(tool, tools) else {
            return false;
        };
        let (Ok(pb), Ok(tb)) = (
            self.inner.bbox(&part.brep, &Placement::IDENTITY),
            self.inner.bbox(&t.brep, p),
        ) else {
            return false;
        };
        let m = 1e-6;
        tb.min.x > pb.min.x + m
            && tb.min.y > pb.min.y + m
            && tb.min.z > pb.min.z + m
            && tb.max.x < pb.max.x - m
            && tb.max.y < pb.max.y - m
            && tb.max.z < pb.max.z - m
    }
}

impl<K: SolidKernel> SolidKernel for Broken<K> {
    type Brep = K::Brep;

    fn name(&self) -> String {
        format!("{} with {}", self.inner.name(), self.mutation.label())
    }

    fn evaluate(
        &self,
        def: &PartDef,
        tools: &dyn ToolResolver,
    ) -> Result<Evaluated<K::Brep>, EvalError> {
        let (d, t) = self.mutate(def, tools);
        self.inner.evaluate(&d, &t)
    }

    fn mass(&self, b: &K::Brep) -> Result<MassProps, EvalError> {
        let mut m = self.inner.mass(b)?;
        if let Mutation::MassScale(f) = self.mutation {
            m.volume *= f;
        }
        Ok(m)
    }

    fn bbox(&self, b: &K::Brep, frame: &Placement) -> Result<Aabb3, EvalError> {
        if self.mutation == Mutation::BboxIgnoresRotation && !frame.is_identity() {
            let bb = self.inner.bbox(b, &Placement::IDENTITY)?;
            let t = frame.translation;
            let mv = |p: Point3| Point3::new(p.x + t.x, p.y + t.y, p.z + t.z);
            return Ok(Aabb3::new(mv(bb.min), mv(bb.max)));
        }
        self.inner.bbox(b, frame)
    }

    fn check(&self, b: &K::Brep) -> Result<Validity, EvalError> {
        self.inner.check(b)
    }

    fn tessellate(&self, b: &K::Brep, chord: f64) -> Result<TriMesh, EvalError> {
        let mut m = self.inner.tessellate(b, chord)?;
        let mid = m.triangles.len() / 2;
        match self.mutation {
            Mutation::DropTriangle if !m.triangles.is_empty() => {
                m.triangles.remove(mid);
            }
            Mutation::FlipTriangle if !m.triangles.is_empty() => {
                m.triangles[mid].swap(1, 2);
            }
            Mutation::MoveVertex(d) if !m.positions.is_empty() => {
                let s = d / 3f64.sqrt();
                let p = m.positions[0];
                m.positions[0] = Point3::new(p.x + s, p.y + s, p.z + s);
            }
            _ => {}
        }
        Ok(m)
    }
}
