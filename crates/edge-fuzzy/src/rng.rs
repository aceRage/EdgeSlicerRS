//! The random numbers behind the jitter.
//!
//! Different from the reference on purpose. Its `random_value()`
//! (FuzzySkin.cpp) draws from a thread-local `std::mt19937` seeded from the
//! hardware, so the same slice comes out differently on every run. This
//! engine must give the same output for the same input on any thread, so
//! every `fuzzy_polyline` / `fuzzy_extrusion_line` call gets a generator
//! of its own, seeded from what the call is given. The inputs are the ones
//! the reference's test-only `TestSeedScope` feeds its `std::seed_seq`:
//! the bits of `slice_z`, the low 32 bits of the first point's x and y,
//! and the point count. (The reference also mixes in the seed a test
//! asked for; there is none here.)
//!
//! The generator is xoshiro256++ (Blackman and Vigna, public domain),
//! its state filled by splitmix64 as its authors recommend. The numbers
//! are not the reference's, which no two of its runs share either.

use edge_geom::Point2;

/// One step of splitmix64: advances `state` and returns the next output.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The generator of one fuzz call.
#[derive(Clone, Debug)]
pub(crate) struct CallRng {
    state: [u64; 4],
}

impl CallRng {
    /// Seeds from a call's input, as `TestSeedScope` picks it.
    pub(crate) fn for_call(slice_z: f64, first: Point2, count: usize) -> Self {
        let z = slice_z.to_bits();
        // The seven words of the reference's seed sequence, less the two
        // of the test seed: uint32_t(z), uint32_t(z >> 32),
        // uint32_t(first.x()), uint32_t(first.y()), uint32_t(n).
        let words = [
            z as u32,
            (z >> 32) as u32,
            first.x as u32,
            first.y as u32,
            count as u32,
        ];
        // Absorb the words one at a time, scrambling in between, so that
        // no two of them can cancel.
        let mut hash = 0x6564_6765_2d66_757a_u64;
        for word in words {
            hash ^= u64::from(word);
            hash = splitmix64(&mut hash);
        }
        let mut state = [0; 4];
        for slot in &mut state {
            *slot = splitmix64(&mut hash);
        }
        // xoshiro must not start from all zeros; splitmix64 cannot hand
        // out four zeros in a row, but the guard costs nothing.
        if state == [0; 4] {
            state[0] = 1;
        }
        Self { state }
    }

    /// xoshiro256++.
    fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// A number in [0, 1), every one a multiple of 2^-53: the reference's
    /// `random_value()`.
    pub(crate) fn value(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(z: f64, x: i64, y: i64, n: usize) -> CallRng {
        CallRng::for_call(z, Point2 { x, y }, n)
    }

    #[test]
    fn xoshiro_matches_its_reference_output() {
        // First outputs of xoshiro256++ from the state {1, 2, 3, 4},
        // worked out by hand from the published algorithm.
        let mut generator = CallRng {
            state: [1, 2, 3, 4],
        };
        assert_eq!(generator.next_u64(), 41_943_041);
        assert_eq!(generator.next_u64(), 58_720_359);
    }

    #[test]
    fn values_are_in_the_unit_interval_and_spread_over_it() {
        let mut generator = rng(0.2, 0, 0, 4);
        let values: Vec<f64> = (0..10_000).map(|_| generator.value()).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
        let low = values.iter().filter(|&&v| v < 0.1).count();
        assert!((800..1200).contains(&low), "{low}");
    }

    #[test]
    fn same_call_same_numbers_and_any_input_changes_them() {
        let draw =
            |mut generator: CallRng| -> Vec<u64> { (0..4).map(|_| generator.next_u64()).collect() };
        let base = draw(rng(0.2, 1000, 2000, 4));
        assert_eq!(base, draw(rng(0.2, 1000, 2000, 4)));
        assert_ne!(base, draw(rng(0.4, 1000, 2000, 4)));
        assert_ne!(base, draw(rng(0.2, 1001, 2000, 4)));
        assert_ne!(base, draw(rng(0.2, 1000, 2001, 4)));
        assert_ne!(base, draw(rng(0.2, 1000, 2000, 5)));
        // x and y are not interchangeable.
        assert_ne!(base, draw(rng(0.2, 2000, 1000, 4)));
    }
}
