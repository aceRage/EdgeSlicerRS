//! The random numbers the reference draws its samples with: the 64-bit
//! Mersenne Twister of the C++ standard library (`std::mt19937_64`), and
//! what libc++ makes of its output (`std::uniform_real_distribution`).
//! The engine is laid down by the standard; the distribution is not, and
//! this is libc++'s, which the reference is built with on macOS, so that
//! the samples drawn here are the ones it draws there.

const SIZE: usize = 312;
const MIDDLE: usize = 156;

/// `std::mt19937_64`.
pub struct Mt64 {
    state: [u64; SIZE],
    index: usize,
}

impl Mt64 {
    pub fn new(seed: u64) -> Self {
        let mut state = [0u64; SIZE];
        state[0] = seed;
        for at in 1..SIZE {
            let previous = state[at - 1];
            state[at] = 6_364_136_223_846_793_005u64
                .wrapping_mul(previous ^ (previous >> 62))
                .wrapping_add(at as u64);
        }
        Self { state, index: SIZE }
    }

    pub fn next_u64(&mut self) -> u64 {
        if self.index >= SIZE {
            for at in 0..SIZE {
                let mixed = (self.state[at] & 0xFFFF_FFFF_8000_0000)
                    | (self.state[(at + 1) % SIZE] & 0x7FFF_FFFF);
                let mut twisted = mixed >> 1;
                if mixed & 1 != 0 {
                    twisted ^= 0xB502_6F5A_A966_19E9;
                }
                self.state[at] = self.state[(at + MIDDLE) % SIZE] ^ twisted;
            }
            self.index = 0;
        }
        let mut x = self.state[self.index];
        self.index += 1;
        x ^= (x >> 29) & 0x5555_5555_5555_5555;
        x ^= (x << 17) & 0x71D6_7FFF_EDA6_0000;
        x ^= (x << 37) & 0xFFF7_EEE0_0000_0000;
        x ^ (x >> 43)
    }

    /// A number from 0 up to 1 (`std::uniform_real_distribution<double>`
    /// with its default range, by libc++'s `generate_canonical`: one
    /// draw, over two to the 64th).
    pub fn unit(&mut self) -> f64 {
        self.next_u64() as f64 / 18_446_744_073_709_551_616.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_is_the_standards() {
        // The C++ standard gives the 10000th draw of a default-seeded
        // mt19937_64.
        let mut engine = Mt64::new(5489);
        let mut last = 0;
        for _ in 0..10_000 {
            last = engine.next_u64();
        }
        assert_eq!(last, 9_981_545_732_273_789_042);
    }

    #[test]
    fn the_distribution_is_libcxxs() {
        // What a program built with libc++ prints for the reference's
        // seed: three draws of uniform_real_distribution<double>.
        let mut engine = Mt64::new(27_644_437);
        let drawn = [engine.unit(), engine.unit(), engine.unit()];
        assert_eq!(
            drawn,
            [
                0.936_277_605_160_022_3,
                0.615_367_913_087_001_5,
                0.640_337_473_642_608
            ]
        );
        // No number is 1 or more.
        assert!((0..1000).all(|_| (0.0..1.0).contains(&engine.unit())));
    }
}
