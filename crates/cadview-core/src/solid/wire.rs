//! Kernel answers as plain, serializable data.
//!
//! The kernel's result types ([`MassProps`], [`TightBox`], [`Validity`],
//! [`TriMesh`], [`Closest`], [`Interference`], [`EvalError`]) are not
//! serializable themselves. Two things need them as bytes: a prebaked scene
//! (`baked.rs`: the answers a viewer needs, written once by a peer that has
//! a kernel) and a kernel that runs in a separately loaded module
//! (`remote.rs`). Both use the types here.
//!
//! Every conversion destructures its source completely, so a field added to
//! a kernel type fails to compile here instead of being dropped silently.

use serde::{Deserialize, Serialize};

use super::kernel::{
    Aabb3, Closest, EvalError, Interference, MassProps, TightBox, TriMesh, Validity,
};
use super::types::{DefId, FeatId, Point3};

/// A serializable mirror of a kernel answer. (Plain methods, not `From`:
/// `From` impls between these and the kernel types would make `?` and
/// `.into()` ambiguous in code that converts kernel errors.)
pub trait Wire: Sized {
    type Kernel;
    fn of(k: Self::Kernel) -> Self;
    fn get(self) -> Self::Kernel;
}

/// [`TightBox`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireBox {
    pub min: Point3,
    pub max: Point3,
    pub tol: f64,
}

impl Wire for WireBox {
    type Kernel = TightBox;
    fn of(b: TightBox) -> Self {
        let TightBox {
            aabb: Aabb3 { min, max },
            tol,
        } = b;
        WireBox { min, max, tol }
    }

    fn get(self) -> TightBox {
        let b = self;
        TightBox {
            aabb: Aabb3::new(b.min, b.max),
            tol: b.tol,
        }
    }
}

/// [`MassProps`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireMass {
    pub volume: f64,
    pub centroid: Option<Point3>,
    pub exact: bool,
}

impl Wire for WireMass {
    type Kernel = MassProps;
    fn of(m: MassProps) -> Self {
        let MassProps {
            volume,
            centroid,
            exact,
        } = m;
        WireMass {
            volume,
            centroid,
            exact,
        }
    }

    fn get(self) -> MassProps {
        let m = self;
        MassProps {
            volume: m.volume,
            centroid: m.centroid,
            exact: m.exact,
        }
    }
}

/// [`Validity`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireValidity {
    pub valid: bool,
    pub solids: u64,
    pub faces: u64,
    pub problems: Vec<String>,
}

impl Wire for WireValidity {
    type Kernel = Validity;
    fn of(v: Validity) -> Self {
        let Validity {
            valid,
            solids,
            faces,
            problems,
        } = v;
        WireValidity {
            valid,
            solids: solids as u64,
            faces: faces as u64,
            problems,
        }
    }

    fn get(self) -> Validity {
        let v = self;
        Validity {
            valid: v.valid,
            solids: v.solids as usize,
            faces: v.faces as usize,
            problems: v.problems,
        }
    }
}

/// [`TriMesh`], positions in full precision.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WireMesh {
    pub positions: Vec<Point3>,
    pub triangles: Vec<[u32; 3]>,
}

impl Wire for WireMesh {
    type Kernel = TriMesh;
    fn of(m: TriMesh) -> Self {
        let TriMesh {
            positions,
            triangles,
        } = m;
        WireMesh {
            positions,
            triangles,
        }
    }

    fn get(self) -> TriMesh {
        let m = self;
        TriMesh {
            positions: m.positions,
            triangles: m.triangles,
        }
    }
}

/// [`Closest`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WireClosest {
    pub distance: f64,
    pub a: Point3,
    pub b: Point3,
}

impl Wire for WireClosest {
    type Kernel = Closest;
    fn of(c: Closest) -> Self {
        let Closest { distance, a, b } = c;
        WireClosest { distance, a, b }
    }

    fn get(self) -> Closest {
        let c = self;
        Closest {
            distance: c.distance,
            a: c.a,
            b: c.b,
        }
    }
}

/// [`Interference`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum WireInterference {
    Apart(WireClosest),
    Touching {
        at: Point3,
    },
    Overlapping {
        volume: f64,
        a_in_b: bool,
        b_in_a: bool,
    },
}

impl Wire for WireInterference {
    type Kernel = Interference;
    fn of(i: Interference) -> Self {
        match i {
            Interference::Apart(c) => WireInterference::Apart(WireClosest::of(c)),
            Interference::Touching { at } => WireInterference::Touching { at },
            Interference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
            } => WireInterference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
            },
        }
    }

    fn get(self) -> Interference {
        let i = self;
        match i {
            WireInterference::Apart(c) => Interference::Apart(c.get()),
            WireInterference::Touching { at } => Interference::Touching { at },
            WireInterference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
            } => Interference::Overlapping {
                volume,
                a_in_b,
                b_in_a,
            },
        }
    }
}

/// [`EvalError`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WireError {
    Unsupported {
        feature: Option<FeatId>,
        what: String,
    },
    Invalid {
        feature: Option<FeatId>,
        reason: String,
    },
    Selector {
        feature: FeatId,
        reason: String,
    },
    UnknownTool {
        feature: FeatId,
        def: DefId,
    },
    ToolCycle {
        feature: FeatId,
        def: DefId,
    },
    Kernel {
        feature: Option<FeatId>,
        message: String,
    },
}

impl Wire for WireError {
    type Kernel = EvalError;
    fn of(e: EvalError) -> Self {
        match e {
            EvalError::Unsupported { feature, what } => WireError::Unsupported { feature, what },
            EvalError::Invalid { feature, reason } => WireError::Invalid { feature, reason },
            EvalError::Selector { feature, reason } => WireError::Selector { feature, reason },
            EvalError::UnknownTool { feature, def } => WireError::UnknownTool { feature, def },
            EvalError::ToolCycle { feature, def } => WireError::ToolCycle { feature, def },
            EvalError::Kernel { feature, message } => WireError::Kernel { feature, message },
        }
    }

    fn get(self) -> EvalError {
        let e = self;
        match e {
            WireError::Unsupported { feature, what } => EvalError::Unsupported { feature, what },
            WireError::Invalid { feature, reason } => EvalError::Invalid { feature, reason },
            WireError::Selector { feature, reason } => EvalError::Selector { feature, reason },
            WireError::UnknownTool { feature, def } => EvalError::UnknownTool { feature, def },
            WireError::ToolCycle { feature, def } => EvalError::ToolCycle { feature, def },
            WireError::Kernel { feature, message } => EvalError::Kernel { feature, message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round<W>(v: W::Kernel) -> W::Kernel
    where
        W: Wire + Serialize + for<'de> Deserialize<'de>,
    {
        let w = W::of(v);
        let bytes = bincode::serialize(&w).unwrap();
        let back: W = bincode::deserialize(&bytes).unwrap();
        back.get()
    }

    #[test]
    fn every_kernel_answer_round_trips_bit_for_bit() {
        let p = |x: f64| Point3::new(x, -x / 3.0, x * 1e-17);
        let b = TightBox {
            aabb: Aabb3::new(p(0.1), p(7.3)),
            tol: 1e-9,
        };
        assert_eq!(round::<WireBox>(b), b);
        let m = MassProps {
            volume: 3_195_386.283_185_307,
            centroid: Some(p(1.0 / 7.0)),
            exact: true,
        };
        assert_eq!(round::<WireMass>(m), m);
        let v = Validity {
            valid: false,
            solids: 2,
            faces: 33,
            problems: vec!["open shell".into()],
        };
        assert_eq!(round::<WireValidity>(v.clone()), v);
        let t = TriMesh {
            positions: vec![p(0.0), p(1.0), p(2.0)],
            triangles: vec![[0, 1, 2]],
        };
        assert_eq!(round::<WireMesh>(t.clone()), t);
        for i in [
            Interference::Apart(Closest {
                distance: 3.5,
                a: p(1.0),
                b: p(2.0),
            }),
            Interference::Touching { at: p(4.0) },
            Interference::Overlapping {
                volume: 10584.000000000004,
                a_in_b: false,
                b_in_a: true,
            },
        ] {
            assert_eq!(round::<WireInterference>(i), i);
        }
        for e in [
            EvalError::unsupported(Some(FeatId(3)), "tessellation"),
            EvalError::invalid(None, "open profile"),
            EvalError::Selector {
                feature: FeatId(1),
                reason: "no edge".into(),
            },
            EvalError::UnknownTool {
                feature: FeatId(2),
                def: DefId(9),
            },
            EvalError::ToolCycle {
                feature: FeatId(2),
                def: DefId(9),
            },
            EvalError::Kernel {
                feature: None,
                message: "refused".into(),
            },
        ] {
            assert_eq!(round::<WireError>(e.clone()), e);
        }
    }
}
