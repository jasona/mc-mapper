//! The game's random sources: Xoroshiro128++ with its seed mixing, and the positional factory
//! that derives an independent stream from a name (MD5 of the name, xored into the factory seed).
//! Bit-for-bit ports of `XoroshiroRandomSource`, `Xoroshiro128PlusPlus` and `RandomSupport`.

use md5::{Digest, Md5};

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15; // -7046029254386353131
const SILVER: u64 = 0x6A09_E667_F3BC_C909; // 7640891576956012809

pub fn mix_stafford13(z: u64) -> u64 {
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// `RandomSupport.upgradeSeedTo128bit`: the world seed spread over 128 bits.
pub fn upgrade_seed(seed: i64) -> (u64, u64) {
    let lo = (seed as u64) ^ SILVER;
    let hi = lo.wrapping_add(GOLDEN);
    (mix_stafford13(lo), mix_stafford13(hi))
}

/// `RandomSupport.seedFromHashOf`: the MD5 of the name as two big-endian longs.
pub fn seed_from_hash_of(name: &str) -> (u64, u64) {
    let h = Md5::digest(name.as_bytes());
    let lo = u64::from_be_bytes(h[0..8].try_into().unwrap());
    let hi = u64::from_be_bytes(h[8..16].try_into().unwrap());
    (lo, hi)
}

#[derive(Clone, Debug)]
pub struct Xoroshiro {
    lo: u64,
    hi: u64,
}

impl Xoroshiro {
    /// From a world seed, mixed (`new XoroshiroRandomSource(long)`).
    pub fn from_seed(seed: i64) -> Xoroshiro {
        let (lo, hi) = upgrade_seed(seed);
        Xoroshiro::from_parts(lo, hi)
    }

    /// From 128 raw bits (`new XoroshiroRandomSource(long, long)`).
    pub fn from_parts(lo: u64, hi: u64) -> Xoroshiro {
        if lo == 0 && hi == 0 {
            Xoroshiro { lo: GOLDEN, hi: SILVER }
        } else {
            Xoroshiro { lo, hi }
        }
    }

    pub fn next_long(&mut self) -> u64 {
        let (s0, mut s1) = (self.lo, self.hi);
        let r = s0.wrapping_add(s1).rotate_left(17).wrapping_add(s0);
        s1 ^= s0;
        self.lo = s0.rotate_left(49) ^ s1 ^ (s1 << 21);
        self.hi = s1.rotate_left(28);
        r
    }

    pub fn next_int(&mut self) -> i32 {
        self.next_long() as i32
    }

    /// `nextInt(bound)`: Lemire's method, as the game does it.
    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        assert!(bound > 0);
        let b = bound as u64;
        let mut m = (self.next_int() as u32 as u64) * b;
        let mut low = m & 0xFFFF_FFFF;
        if low < b {
            let t = (bound.wrapping_neg() as u32) % (bound as u32);
            while low < t as u64 {
                m = (self.next_int() as u32 as u64) * b;
                low = m & 0xFFFF_FFFF;
            }
        }
        (m >> 32) as i32
    }

    pub fn next_double(&mut self) -> f64 {
        (self.next_long() >> 11) as f64 * 1.110_223_024_625_156_5e-16
    }

    pub fn fork_positional(&mut self) -> Positional {
        let lo = self.next_long();
        let hi = self.next_long();
        Positional { lo, hi }
    }
}

/// `XoroshiroPositionalRandomFactory`.
#[derive(Clone, Copy, Debug)]
pub struct Positional {
    pub lo: u64,
    pub hi: u64,
}

impl Positional {
    pub fn from_hash_of(&self, name: &str) -> Xoroshiro {
        let (a, b) = seed_from_hash_of(name);
        Xoroshiro::from_parts(a ^ self.lo, b ^ self.hi)
    }
}

/// The factory every Overworld noise is seeded from: `RandomState.random`.
pub fn world_positional(seed: i64) -> Positional {
    Xoroshiro::from_seed(seed).fork_positional()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_seed_of_a_known_string() {
        // MD5("") = d41d8cd98f00b204e9800998ecf8427e
        assert_eq!(seed_from_hash_of(""), (0xd41d8cd98f00b204, 0xe9800998ecf8427e));
    }

    #[test]
    fn bounded_ints_stay_in_range() {
        let mut r = Xoroshiro::from_seed(1);
        for b in [1, 2, 3, 7, 256, 1 << 30] {
            for _ in 0..100 {
                let v = r.next_int_bounded(b);
                assert!((0..b).contains(&v));
            }
        }
    }
}
