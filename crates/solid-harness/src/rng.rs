//! A seeded random number generator (SplitMix64, Steele, Lea and Flood
//! 2014). No dependency: the harness only needs reproducible, well-mixed
//! samples, not cryptographic quality. Every sampling site seeds from the
//! case name, so adding a case does not move another case's samples.

#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// A generator seeded from a name (FNV-1a) and a salt.
    pub fn named(name: &str, salt: u64) -> Rng {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        Rng(h ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.unit() * n as f64) as usize % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reproducible_and_spread() {
        let mut a = Rng::named("x", 1);
        let mut b = Rng::named("x", 1);
        let xs: Vec<f64> = (0..1000).map(|_| a.unit()).collect();
        let ys: Vec<f64> = (0..1000).map(|_| b.unit()).collect();
        assert_eq!(xs, ys);
        let mean = xs.iter().sum::<f64>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05);
        assert!(xs.iter().all(|x| (0.0..1.0).contains(x)));
        assert_ne!(Rng::named("x", 2).unit(), Rng::named("x", 1).unit());
    }
}
