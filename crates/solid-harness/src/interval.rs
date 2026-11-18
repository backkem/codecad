//! Outward-rounded interval arithmetic on `f64`, for the reference.
//!
//! An [`Iv`] `[lo, hi]` encloses a real number that the reference computes
//! in exact arithmetic. Every operation returns an interval that contains
//! the exact result for every pair of reals in its operands, so rounding
//! can widen an answer but never flip it.
//!
//! Directed rounding without changing the FPU mode: `+`, `-`, `*`, `/` and
//! `sqrt` are computed rounded to nearest, then the exact rounding error is
//! recovered with an error-free transform (TwoSum for sums, a fused
//! multiply-add for products, quotients and square roots; IEEE 754 makes
//! these exact barring overflow and underflow). The sign of the error says
//! on which side the exact value lies, and only that side is moved one ulp
//! with `f64::next_up` / `f64::next_down`. Results near the underflow range
//! (magnitude below [`TINY`]) or non-finite are widened unconditionally,
//! because the error-free transforms are not exact there.
//!
//! Elementary functions (`sin`, `cos`) are not correctly rounded by the
//! standard library. [`Iv::sin`] and [`Iv::cos`] assume the platform result
//! is within 4 ulp of the exact value (every libm Rust ships with is within
//! 1 ulp) and widen by a relative 2^-48 plus an absolute 2^-1000. That
//! assumption is the one part of the reference's rounding argument that is
//! not proved here.

use std::ops::{Add, Div, Mul, Neg, Sub};

/// Below this magnitude the error-free transforms may lose exactness
/// (subnormal products and quotients); results there are widened
/// unconditionally.
const TINY: f64 = 1e-280;

/// A closed interval of reals, `lo <= hi`. May be unbounded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iv {
    pub lo: f64,
    pub hi: f64,
}

fn widen_dn(x: f64) -> f64 {
    if x.is_nan() {
        f64::NEG_INFINITY
    } else {
        x.next_down()
    }
}

fn widen_up(x: f64) -> f64 {
    if x.is_nan() {
        f64::INFINITY
    } else {
        x.next_up()
    }
}

/// `(a + b)` rounded down and up.
fn add_round(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    if !s.is_finite() || !a.is_finite() || !b.is_finite() {
        return (widen_dn(s), widen_up(s));
    }
    // TwoSum (Knuth): s + err == a + b exactly.
    let bb = s - a;
    let err = (a - (s - bb)) + (b - bb);
    if err > 0.0 {
        (s, s.next_up())
    } else if err < 0.0 {
        (s.next_down(), s)
    } else {
        (s, s)
    }
}

/// `(a * b)` rounded down and up.
fn mul_round(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    if !p.is_finite() || p.abs() < TINY {
        if a == 0.0 || b == 0.0 {
            return (0.0, 0.0);
        }
        return (widen_dn(p), widen_up(p));
    }
    // a * b == p + err exactly.
    let err = a.mul_add(b, -p);
    if err > 0.0 {
        (p, p.next_up())
    } else if err < 0.0 {
        (p.next_down(), p)
    } else {
        (p, p)
    }
}

/// `(a / b)` rounded down and up, `b != 0`.
fn div_round(a: f64, b: f64) -> (f64, f64) {
    let q = a / b;
    if !q.is_finite() || q.abs() < TINY {
        if a == 0.0 && b != 0.0 {
            return (0.0, 0.0);
        }
        return (widen_dn(q), widen_up(q));
    }
    // a - q * b exactly; the exact quotient is q + r / b.
    let r = -q.mul_add(b, -a);
    let s = r * b.signum();
    if s > 0.0 {
        (q, q.next_up())
    } else if s < 0.0 {
        (q.next_down(), q)
    } else {
        (q, q)
    }
}

/// `sqrt(x)` rounded down and up, `x >= 0`.
fn sqrt_round(x: f64) -> (f64, f64) {
    let r = x.sqrt();
    if !r.is_finite() || r < TINY {
        return (widen_dn(r).max(0.0), widen_up(r));
    }
    // x - r^2 exactly; positive means the exact root is above r.
    let res = -r.mul_add(r, -x);
    if res > 0.0 {
        (r, r.next_up())
    } else if res < 0.0 {
        (r.next_down(), r)
    } else {
        (r, r)
    }
}

impl Iv {
    pub const ZERO: Iv = Iv { lo: 0.0, hi: 0.0 };
    pub const ENTIRE: Iv = Iv {
        lo: f64::NEG_INFINITY,
        hi: f64::INFINITY,
    };

    /// The degenerate interval holding one float exactly.
    pub const fn point(x: f64) -> Iv {
        Iv { lo: x, hi: x }
    }

    /// `[lo, hi]`; the caller guarantees `lo <= hi`.
    pub fn new(lo: f64, hi: f64) -> Iv {
        debug_assert!(lo <= hi || lo.is_nan() || hi.is_nan(), "{lo} > {hi}");
        Iv { lo, hi }
    }

    /// The smallest interval holding both.
    pub fn hull(self, o: Iv) -> Iv {
        Iv::new(self.lo.min(o.lo), self.hi.max(o.hi))
    }

    pub fn mid(self) -> f64 {
        if self.lo.is_finite() && self.hi.is_finite() {
            0.5 * self.lo + 0.5 * self.hi
        } else {
            0.0
        }
    }

    pub fn width(self) -> f64 {
        self.hi - self.lo
    }

    pub fn contains_zero(self) -> bool {
        self.lo <= 0.0 && self.hi >= 0.0
    }

    /// Certainly positive / negative.
    pub fn pos(self) -> bool {
        self.lo > 0.0
    }
    pub fn neg_certain(self) -> bool {
        self.hi < 0.0
    }

    /// The sign if it is certain: `Some(true)` positive, `Some(false)`
    /// negative, `None` if the interval holds zero.
    pub fn sign(self) -> Option<bool> {
        if self.lo > 0.0 {
            Some(true)
        } else if self.hi < 0.0 {
            Some(false)
        } else {
            None
        }
    }

    pub fn min(self, o: Iv) -> Iv {
        Iv::new(self.lo.min(o.lo), self.hi.min(o.hi))
    }

    pub fn max(self, o: Iv) -> Iv {
        Iv::new(self.lo.max(o.lo), self.hi.max(o.hi))
    }

    pub fn abs(self) -> Iv {
        if self.lo >= 0.0 {
            self
        } else if self.hi <= 0.0 {
            -self
        } else {
            Iv::new(0.0, (-self.lo).max(self.hi))
        }
    }

    /// `self^2`, tighter than `self * self` when the interval holds zero.
    pub fn sqr(self) -> Iv {
        let a = self.abs();
        Iv::new(mul_round(a.lo, a.lo).0, mul_round(a.hi, a.hi).1)
    }

    /// Square root; the negative part of the interval is dropped (the
    /// callers take roots of sums of squares, which are never negative in
    /// exact arithmetic).
    pub fn sqrt(self) -> Iv {
        let lo = if self.lo <= 0.0 {
            0.0
        } else {
            sqrt_round(self.lo).0
        };
        let hi = if self.hi <= 0.0 {
            0.0
        } else {
            sqrt_round(self.hi).1
        };
        Iv::new(lo, hi)
    }

    /// The sine of a float, widened per the module note.
    pub fn sin_of(x: f64) -> Iv {
        widen_elementary(x.sin())
    }

    /// The cosine of a float, widened per the module note.
    pub fn cos_of(x: f64) -> Iv {
        widen_elementary(x.cos())
    }

    /// `x` lies in the interval.
    pub fn contains(self, x: f64) -> bool {
        self.lo <= x && x <= self.hi
    }

    /// pi: the float `PI` is the nearest float below it.
    pub fn pi() -> Iv {
        Iv::new(std::f64::consts::PI, std::f64::consts::PI.next_up())
    }

    /// The sine over the interval: the end values widened by the width
    /// (`|sin'| <= 1`, so every value is within the width of an end's).
    pub fn sin(self) -> Iv {
        self.lipschitz(Iv::sin_of)
    }

    /// The cosine over the interval, as [`Iv::sin`].
    pub fn cos(self) -> Iv {
        self.lipschitz(Iv::cos_of)
    }

    fn lipschitz(self, f: fn(f64) -> Iv) -> Iv {
        let ends = f(self.lo).hull(f(self.hi));
        let w = (Iv::point(self.hi) - Iv::point(self.lo)).hi;
        let r = ends + Iv::new(-w, w);
        Iv::new(r.lo.max(-1.0), r.hi.min(1.0))
    }
}

fn widen_elementary(v: f64) -> Iv {
    let e = v.abs() * 2f64.powi(-48) + 2f64.powi(-1000);
    let (lo, _) = add_round(v, -e);
    let (_, hi) = add_round(v, e);
    Iv::new(lo.max(-1.0), hi.min(1.0))
}

impl From<f64> for Iv {
    fn from(x: f64) -> Iv {
        Iv::point(x)
    }
}

impl Neg for Iv {
    type Output = Iv;
    fn neg(self) -> Iv {
        Iv::new(-self.hi, -self.lo)
    }
}

impl Add for Iv {
    type Output = Iv;
    fn add(self, o: Iv) -> Iv {
        Iv::new(add_round(self.lo, o.lo).0, add_round(self.hi, o.hi).1)
    }
}

impl Sub for Iv {
    type Output = Iv;
    fn sub(self, o: Iv) -> Iv {
        self + (-o)
    }
}

impl Mul for Iv {
    type Output = Iv;
    fn mul(self, o: Iv) -> Iv {
        let c = [
            mul_round(self.lo, o.lo),
            mul_round(self.lo, o.hi),
            mul_round(self.hi, o.lo),
            mul_round(self.hi, o.hi),
        ];
        let lo = c.iter().map(|x| x.0).fold(f64::INFINITY, f64::min);
        let hi = c.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
        if lo.is_nan() || hi.is_nan() {
            return Iv::ENTIRE;
        }
        Iv::new(lo, hi)
    }
}

impl Div for Iv {
    type Output = Iv;
    fn div(self, o: Iv) -> Iv {
        if o.contains_zero() {
            return Iv::ENTIRE;
        }
        let c = [
            div_round(self.lo, o.lo),
            div_round(self.lo, o.hi),
            div_round(self.hi, o.lo),
            div_round(self.hi, o.hi),
        ];
        let lo = c.iter().map(|x| x.0).fold(f64::INFINITY, f64::min);
        let hi = c.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
        if lo.is_nan() || hi.is_nan() {
            return Iv::ENTIRE;
        }
        Iv::new(lo, hi)
    }
}

impl Add<f64> for Iv {
    type Output = Iv;
    fn add(self, o: f64) -> Iv {
        self + Iv::point(o)
    }
}

impl Sub<f64> for Iv {
    type Output = Iv;
    fn sub(self, o: f64) -> Iv {
        self - Iv::point(o)
    }
}

impl Mul<f64> for Iv {
    type Output = Iv;
    fn mul(self, o: f64) -> Iv {
        self * Iv::point(o)
    }
}

/// A 2D point or box with interval coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iv2 {
    pub x: Iv,
    pub y: Iv,
}

impl Iv2 {
    pub fn new(x: Iv, y: Iv) -> Iv2 {
        Iv2 { x, y }
    }
    pub fn point(x: f64, y: f64) -> Iv2 {
        Iv2::new(Iv::point(x), Iv::point(y))
    }
    pub fn dot(self, o: Iv2) -> Iv {
        self.x * o.x + self.y * o.y
    }
    /// `self.x * o.y - self.y * o.x`.
    pub fn cross(self, o: Iv2) -> Iv {
        self.x * o.y - self.y * o.x
    }
    pub fn norm2(self) -> Iv {
        self.x.sqr() + self.y.sqr()
    }
    pub fn norm(self) -> Iv {
        self.norm2().sqrt()
    }
    pub fn scale(self, s: Iv) -> Iv2 {
        Iv2::new(self.x * s, self.y * s)
    }
    pub fn mid(self) -> (f64, f64) {
        (self.x.mid(), self.y.mid())
    }
    pub fn hull(self, o: Iv2) -> Iv2 {
        Iv2::new(self.x.hull(o.x), self.y.hull(o.y))
    }
}

impl Add for Iv2 {
    type Output = Iv2;
    fn add(self, o: Iv2) -> Iv2 {
        Iv2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Iv2 {
    type Output = Iv2;
    fn sub(self, o: Iv2) -> Iv2 {
        Iv2::new(self.x - o.x, self.y - o.y)
    }
}

/// A 3D point or box with interval coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Iv3 {
    pub x: Iv,
    pub y: Iv,
    pub z: Iv,
}

impl Iv3 {
    pub fn new(x: Iv, y: Iv, z: Iv) -> Iv3 {
        Iv3 { x, y, z }
    }
    pub fn point(x: f64, y: f64, z: f64) -> Iv3 {
        Iv3::new(Iv::point(x), Iv::point(y), Iv::point(z))
    }
    pub fn dot(self, o: Iv3) -> Iv {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Iv3) -> Iv3 {
        Iv3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn norm2(self) -> Iv {
        self.x.sqr() + self.y.sqr() + self.z.sqr()
    }
    pub fn norm(self) -> Iv {
        self.norm2().sqrt()
    }
    pub fn scale(self, s: Iv) -> Iv3 {
        Iv3::new(self.x * s, self.y * s, self.z * s)
    }
    pub fn mid(self) -> [f64; 3] {
        [self.x.mid(), self.y.mid(), self.z.mid()]
    }
}

impl Add for Iv3 {
    type Output = Iv3;
    fn add(self, o: Iv3) -> Iv3 {
        Iv3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Iv3 {
    type Output = Iv3;
    fn sub(self, o: Iv3) -> Iv3 {
        Iv3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Neg for Iv3 {
    type Output = Iv3;
    fn neg(self) -> Iv3 {
        Iv3::new(-self.x, -self.y, -self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_enclose_the_exact_value() {
        // 0.1 + 0.2 is not representable; the interval must hold the exact
        // sum of the two doubles, which lies strictly between neighbours.
        let s = Iv::point(0.1) + Iv::point(0.2);
        assert!(s.lo < s.hi, "{s:?}");
        assert!(s.width() <= 2.0 * f64::EPSILON);
        // Exact sums stay degenerate.
        let e = Iv::point(1.5) + Iv::point(2.25);
        assert_eq!(e, Iv::point(3.75));
        // -z over [0, 2.25] keeps an exact zero bound.
        let z = -Iv::new(0.0, 2.25);
        assert_eq!(z.hi, 0.0);
    }

    #[test]
    fn products_quotients_roots_enclose() {
        let third = Iv::point(1.0) / Iv::point(3.0);
        assert!(third.lo < third.hi);
        let back = third * Iv::point(3.0);
        assert!(back.contains(1.0), "{back:?}");
        let r = Iv::point(2.0).sqrt();
        assert!(r.lo < r.hi);
        assert!((r.sqr()).contains(2.0));
        assert_eq!(Iv::point(9.0).sqrt(), Iv::point(3.0));
        let m = Iv::new(-2.0, 3.0) * Iv::new(-5.0, 1.0);
        assert_eq!(m, Iv::new(-15.0, 10.0));
        assert_eq!(Iv::new(-2.0, 3.0).sqr(), Iv::new(0.0, 9.0));
        assert_eq!(Iv::point(1.0) / Iv::new(-1.0, 1.0), Iv::ENTIRE);
    }

    #[test]
    fn trig_is_widened_and_clamped() {
        let s = Iv::sin_of(std::f64::consts::FRAC_PI_2);
        assert!(s.hi <= 1.0 && s.contains(1.0));
        let c = Iv::cos_of(std::f64::consts::FRAC_PI_2);
        // The float pi/2 is not pi/2, so the exact cosine is about 6e-17.
        assert!(c.contains(6.123233995736766e-17));
    }
}
