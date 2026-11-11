//! Coherent noise: a port of the parts of libnoise the reference's fuzzy
//! skin uses.
//!
//! libnoise is by Jason Bevins, copyright 2003-2005, and is licensed
//! LGPL-2.1-or-later. Ported from its `src/noisegen.cpp` and
//! `src/noisegen.h` (`GradientCoherentNoise3D`, `GradientNoise3D`,
//! `ValueNoise3D`, `IntValueNoise3D`, `MakeInt32Range`), `src/interp.h`
//! (`LinearInterp`, `SCurve3`), `src/module/perlin.cpp`,
//! `src/module/billow.cpp`, `src/module/ridgedmulti.cpp`,
//! `src/module/voronoi.cpp` and `src/vectortable.h` (in `vectortable.rs`).
//!
//! The modules are built the way the reference's `get_noise_module`
//! (Feature/FuzzySkin/FuzzySkin.cpp) builds them: it sets the frequency
//! (one over the noise scale), the octave count and the persistence, and
//! leaves everything else at libnoise's defaults (seed 0, lacunarity 2,
//! standard quality, Voronoi displacement 1 with distance off).
//!
//! The arithmetic is kept as the C++ does it. libnoise hashes lattice
//! points with `int` products that overflow by design; they are wrapping
//! `i32` operations here, and shifts of negative values are arithmetic, as
//! every compiler the reference builds with makes them. No multiply-add is
//! fused, so a build of the C++ that does fuse them (clang on arm64) can
//! differ from this in the last bits.

use crate::vectortable::RANDOM_VECTORS;

// noisegen.cpp, NOISE_VERSION 2.
const X_NOISE_GEN: i32 = 1619;
const Y_NOISE_GEN: i32 = 31337;
const Z_NOISE_GEN: i32 = 6971;
const SEED_NOISE_GEN: i32 = 1013;
const SHIFT_NOISE_GEN: u32 = 8;

/// `DEFAULT_PERLIN_LACUNARITY`, `DEFAULT_BILLOW_LACUNARITY` and
/// `DEFAULT_RIDGED_LACUNARITY`: how much finer each octave is than the last.
const LACUNARITY: f64 = 2.0;
/// `PERLIN_MAX_OCTAVE`, `BILLOW_MAX_OCTAVE` and `RIDGED_MAX_OCTAVE`.
pub(crate) const MAX_OCTAVES: i32 = 30;
/// The seed every module starts with (`DEFAULT_PERLIN_SEED` and the rest).
const SEED: i32 = 0;

/// interp.h, `LinearInterp`.
fn linear_interp(n0: f64, n1: f64, a: f64) -> f64 {
    ((1.0 - a) * n0) + (a * n1)
}

/// interp.h, `SCurve3`: the cubic S-curve of the standard noise quality
/// (`QUALITY_STD`, the default of every module).
fn s_curve3(a: f64) -> f64 {
    a * a * (3.0 - 2.0 * a)
}

/// noisegen.h, `MakeInt32Range`: folds a coordinate into what an `int`
/// holds. `%` on `f64` is C's `fmod`.
fn make_int32_range(n: f64) -> f64 {
    if n >= 1073741824.0 {
        (2.0 * (n % 1073741824.0)) - 1073741824.0
    } else if n <= -1073741824.0 {
        (2.0 * (n % 1073741824.0)) + 1073741824.0
    } else {
        n
    }
}

/// The corner below a coordinate, as libnoise finds it:
/// `(x > 0.0 ? (int)x : (int)x - 1)`. Zero and whole negative numbers go
/// one further down than a floor would, which is libnoise's behaviour. The
/// cast truncates as C's does for everything an `int` holds; beyond that
/// C++ leaves the result undefined and this saturates.
fn lattice_floor(x: f64) -> i32 {
    if x > 0.0 {
        x as i32
    } else {
        (x as i32).wrapping_sub(1)
    }
}

/// noisegen.cpp, `GradientNoise3D`.
fn gradient_noise_3d(fx: f64, fy: f64, fz: f64, ix: i32, iy: i32, iz: i32, seed: i32) -> f64 {
    // The C++ masks the sum with 0xffffffff, which keeps all 32 bits.
    let mut vector_index = X_NOISE_GEN
        .wrapping_mul(ix)
        .wrapping_add(Y_NOISE_GEN.wrapping_mul(iy))
        .wrapping_add(Z_NOISE_GEN.wrapping_mul(iz))
        .wrapping_add(SEED_NOISE_GEN.wrapping_mul(seed));
    vector_index ^= vector_index >> SHIFT_NOISE_GEN;
    vector_index &= 0xff;
    let row = (vector_index as usize) << 2;

    let xv_gradient = RANDOM_VECTORS[row];
    let yv_gradient = RANDOM_VECTORS[row + 1];
    let zv_gradient = RANDOM_VECTORS[row + 2];

    let xv_point = fx - f64::from(ix);
    let yv_point = fy - f64::from(iy);
    let zv_point = fz - f64::from(iz);

    ((xv_gradient * xv_point) + (yv_gradient * yv_point) + (zv_gradient * zv_point)) * 2.12
}

/// noisegen.cpp, `GradientCoherentNoise3D` at `QUALITY_STD`.
pub(crate) fn gradient_coherent_noise_3d(x: f64, y: f64, z: f64, seed: i32) -> f64 {
    let x0 = lattice_floor(x);
    let x1 = x0.wrapping_add(1);
    let y0 = lattice_floor(y);
    let y1 = y0.wrapping_add(1);
    let z0 = lattice_floor(z);
    let z1 = z0.wrapping_add(1);

    let xs = s_curve3(x - f64::from(x0));
    let ys = s_curve3(y - f64::from(y0));
    let zs = s_curve3(z - f64::from(z0));

    let n0 = gradient_noise_3d(x, y, z, x0, y0, z0, seed);
    let n1 = gradient_noise_3d(x, y, z, x1, y0, z0, seed);
    let ix0 = linear_interp(n0, n1, xs);
    let n0 = gradient_noise_3d(x, y, z, x0, y1, z0, seed);
    let n1 = gradient_noise_3d(x, y, z, x1, y1, z0, seed);
    let ix1 = linear_interp(n0, n1, xs);
    let iy0 = linear_interp(ix0, ix1, ys);
    let n0 = gradient_noise_3d(x, y, z, x0, y0, z1, seed);
    let n1 = gradient_noise_3d(x, y, z, x1, y0, z1, seed);
    let ix0 = linear_interp(n0, n1, xs);
    let n0 = gradient_noise_3d(x, y, z, x0, y1, z1, seed);
    let n1 = gradient_noise_3d(x, y, z, x1, y1, z1, seed);
    let ix1 = linear_interp(n0, n1, xs);
    let iy1 = linear_interp(ix0, ix1, ys);

    linear_interp(iy0, iy1, zs)
}

/// What `IntValueNoise3D` does with the sign bit of its result.
///
/// The C++ ends with `& 0x7fffffff`, on a product that overflows `int` by
/// design. Overflow of a signed `int` is undefined in C++, and a compiler
/// that reasons from that (clang and GCC at -O2) proves the product is
/// never negative and drops the mask. The reference's own macOS build of
/// libnoise does: its `IntValueNoise3D` returns negative numbers, and its
/// `ValueNoise3D` converts them as unsigned, so it ranges over (-3, 1]
/// where libnoise documents [-1, 1]. A compiler that wraps (MSVC) keeps
/// the mask. So the reference's Voronoi noise depends on what built it.
///
/// Different on purpose: this engine takes the code as written
/// ([`ValueHash::AsWritten`]), which is what libnoise documents and gives
/// the same result everywhere. The other reading is kept so the tests can
/// hold every step of the port against the real library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueHash {
    /// The mask applies: 0 to 2147483647.
    AsWritten,
    /// The mask is optimised away and the result read as unsigned, as in
    /// the reference's clang build.
    #[cfg_attr(not(test), allow(dead_code))]
    MaskDropped,
}

/// noisegen.cpp, `IntValueNoise3D`, before its last mask.
fn int_value_noise_3d_unmasked(x: i32, y: i32, z: i32, seed: i32) -> i32 {
    let mut n = X_NOISE_GEN
        .wrapping_mul(x)
        .wrapping_add(Y_NOISE_GEN.wrapping_mul(y))
        .wrapping_add(Z_NOISE_GEN.wrapping_mul(z))
        .wrapping_add(SEED_NOISE_GEN.wrapping_mul(seed))
        & 0x7fff_ffff;
    n = (n >> 13) ^ n;
    n.wrapping_mul(
        n.wrapping_mul(n)
            .wrapping_mul(60493)
            .wrapping_add(19_990_303),
    )
    .wrapping_add(1_376_312_589)
}

/// noisegen.cpp, `IntValueNoise3D`: 0 to 2147483647.
pub(crate) fn int_value_noise_3d(x: i32, y: i32, z: i32, seed: i32) -> i32 {
    int_value_noise_3d_unmasked(x, y, z, seed) & 0x7fff_ffff
}

/// noisegen.cpp, `ValueNoise3D`: -1 to 1.
pub(crate) fn value_noise_3d(x: i32, y: i32, z: i32, seed: i32, hash: ValueHash) -> f64 {
    let n = match hash {
        ValueHash::AsWritten => f64::from(int_value_noise_3d(x, y, z, seed)),
        ValueHash::MaskDropped => f64::from(int_value_noise_3d_unmasked(x, y, z, seed) as u32),
    };
    1.0 - (n / 1073741824.0)
}

/// One of libnoise's noise modules, set up as the reference sets it up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Module {
    Perlin {
        frequency: f64,
        octaves: i32,
        persistence: f64,
    },
    Billow {
        frequency: f64,
        octaves: i32,
        persistence: f64,
    },
    RidgedMulti {
        frequency: f64,
        octaves: i32,
    },
    Voronoi {
        frequency: f64,
    },
}

impl Module {
    /// `Perlin::GetValue`, `Billow::GetValue`, `RidgedMulti::GetValue` or
    /// `Voronoi::GetValue`.
    pub(crate) fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        match *self {
            Module::Perlin {
                frequency,
                octaves,
                persistence,
            } => perlin(x, y, z, frequency, octaves, persistence),
            Module::Billow {
                frequency,
                octaves,
                persistence,
            } => billow(x, y, z, frequency, octaves, persistence),
            Module::RidgedMulti { frequency, octaves } => ridged_multi(x, y, z, frequency, octaves),
            Module::Voronoi { frequency } => voronoi(x, y, z, frequency, ValueHash::AsWritten),
        }
    }
}

/// module/perlin.cpp, `Perlin::GetValue`.
fn perlin(
    mut x: f64,
    mut y: f64,
    mut z: f64,
    frequency: f64,
    octaves: i32,
    persistence: f64,
) -> f64 {
    let mut value = 0.0;
    let mut cur_persistence = 1.0;

    x *= frequency;
    y *= frequency;
    z *= frequency;

    for cur_octave in 0..octaves {
        let nx = make_int32_range(x);
        let ny = make_int32_range(y);
        let nz = make_int32_range(z);

        // `(m_seed + curOctave) & 0xffffffff`: the mask keeps every bit.
        let seed = SEED.wrapping_add(cur_octave);
        let signal = gradient_coherent_noise_3d(nx, ny, nz, seed);
        value += signal * cur_persistence;

        x *= LACUNARITY;
        y *= LACUNARITY;
        z *= LACUNARITY;
        cur_persistence *= persistence;
    }

    value
}

/// module/billow.cpp, `Billow::GetValue`.
fn billow(
    mut x: f64,
    mut y: f64,
    mut z: f64,
    frequency: f64,
    octaves: i32,
    persistence: f64,
) -> f64 {
    let mut value = 0.0;
    let mut cur_persistence = 1.0;

    x *= frequency;
    y *= frequency;
    z *= frequency;

    for cur_octave in 0..octaves {
        let nx = make_int32_range(x);
        let ny = make_int32_range(y);
        let nz = make_int32_range(z);

        let seed = SEED.wrapping_add(cur_octave);
        let signal = gradient_coherent_noise_3d(nx, ny, nz, seed);
        let signal = 2.0 * signal.abs() - 1.0;
        value += signal * cur_persistence;

        x *= LACUNARITY;
        y *= LACUNARITY;
        z *= LACUNARITY;
        cur_persistence *= persistence;
    }
    value += 0.5;

    value
}

/// module/ridgedmulti.cpp, `RidgedMulti::GetValue`, with the weights of
/// `RidgedMulti::CalcSpectralWeights` worked out as it goes.
fn ridged_multi(mut x: f64, mut y: f64, mut z: f64, frequency: f64, octaves: i32) -> f64 {
    x *= frequency;
    y *= frequency;
    z *= frequency;

    let mut value = 0.0;
    let mut weight = 1.0;

    let offset = 1.0;
    let gain = 2.0;

    // CalcSpectralWeights: weight i is pow(lacunarity^i, -h) with h = 1,
    // and lacunarity^i is built by repeated multiplication. With a
    // lacunarity of 2 every one is a power of two, so the division here
    // is exact and equals what `pow` returns.
    let mut spectral_frequency = 1.0;

    for cur_octave in 0..octaves {
        let nx = make_int32_range(x);
        let ny = make_int32_range(y);
        let nz = make_int32_range(z);

        let seed = SEED.wrapping_add(cur_octave) & 0x7fff_ffff;
        let mut signal = gradient_coherent_noise_3d(nx, ny, nz, seed);

        signal = signal.abs();
        signal = offset - signal;
        signal *= signal;
        signal *= weight;

        weight = (signal * gain).clamp(0.0, 1.0);

        let spectral_weight = 1.0 / spectral_frequency;
        value += signal * spectral_weight;
        spectral_frequency *= LACUNARITY;

        x *= LACUNARITY;
        y *= LACUNARITY;
        z *= LACUNARITY;
    }

    (value * 1.25) - 1.0
}

/// module/voronoi.cpp, `Voronoi::GetValue`, with the distance term off
/// (libnoise's default) and a displacement of 1, as the reference sets it.
fn voronoi(mut x: f64, mut y: f64, mut z: f64, frequency: f64, hash: ValueHash) -> f64 {
    const DISPLACEMENT: f64 = 1.0;

    x *= frequency;
    y *= frequency;
    z *= frequency;

    let x_int = lattice_floor(x);
    let y_int = lattice_floor(y);
    let z_int = lattice_floor(z);

    let mut min_dist = 2147483647.0;
    let mut x_candidate = 0.0;
    let mut y_candidate = 0.0;
    let mut z_candidate = 0.0;

    // Each unit cube holds one seed point; look through the cubes around
    // for the nearest.
    for dz in -2..=2 {
        let z_cur = z_int.wrapping_add(dz);
        for dy in -2..=2 {
            let y_cur = y_int.wrapping_add(dy);
            for dx in -2..=2 {
                let x_cur = x_int.wrapping_add(dx);
                let x_pos = f64::from(x_cur) + value_noise_3d(x_cur, y_cur, z_cur, SEED, hash);
                let y_pos = f64::from(y_cur)
                    + value_noise_3d(x_cur, y_cur, z_cur, SEED.wrapping_add(1), hash);
                let z_pos = f64::from(z_cur)
                    + value_noise_3d(x_cur, y_cur, z_cur, SEED.wrapping_add(2), hash);
                let x_dist = x_pos - x;
                let y_dist = y_pos - y;
                let z_dist = z_pos - z;
                let dist = x_dist * x_dist + y_dist * y_dist + z_dist * z_dist;

                if dist < min_dist {
                    min_dist = dist;
                    x_candidate = x_pos;
                    y_candidate = y_pos;
                    z_candidate = z_pos;
                }
            }
        }
    }

    // `m_enableDistance` is false, so the distance contributes nothing.
    let value = 0.0;

    value
        + (DISPLACEMENT
            * value_noise_3d(
                x_candidate.floor() as i32,
                y_candidate.floor() as i32,
                z_candidate.floor() as i32,
                SEED,
                hash,
            ))
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("noise_reference.rs");

    /// How far a value may be from the C++ library's. The port fuses no
    /// multiply-add; the library the values came from (clang, arm64) does.
    const TOLERANCE: f64 = 1e-12;

    /// The value of a module as the library the samples came from gives
    /// it: Voronoi with the mask of `IntValueNoise3D` dropped.
    fn library_value(
        kind: u8,
        scale: f64,
        octaves: i32,
        persistence: f64,
        at: (f64, f64, f64),
    ) -> f64 {
        let frequency = 1.0 / scale;
        let (x, y, z) = at;
        match kind {
            b'P' => Module::Perlin {
                frequency,
                octaves,
                persistence,
            }
            .value(x, y, z),
            b'B' => Module::Billow {
                frequency,
                octaves,
                persistence,
            }
            .value(x, y, z),
            b'R' => Module::RidgedMulti { frequency, octaves }.value(x, y, z),
            b'V' => voronoi(x, y, z, frequency, ValueHash::MaskDropped),
            other => panic!("unknown module {other}"),
        }
    }

    #[test]
    fn modules_match_the_cpp_library() {
        assert!(MODULE_SAMPLES.len() >= 40);
        for kind in *b"PBRV" {
            assert!(MODULE_SAMPLES.iter().any(|sample| sample.0 == kind));
        }
        for &(kind, scale, octaves, persistence, x, y, z, expected) in MODULE_SAMPLES {
            let got = library_value(kind, scale, octaves, persistence, (x, y, z));
            assert!(
                (got - expected).abs() <= TOLERANCE,
                "{} scale {scale} octaves {octaves} persistence {persistence} at ({x}, {y}, {z}): {got} vs {expected}",
                kind as char
            );
        }
    }

    #[test]
    fn gradient_noise_matches_the_cpp_library() {
        assert!(GRADIENT_SAMPLES.len() >= 10);
        for &(x, y, z, seed, expected) in GRADIENT_SAMPLES {
            let got = gradient_coherent_noise_3d(x, y, z, seed);
            assert!(
                (got - expected).abs() <= TOLERANCE,
                "({x}, {y}, {z}) seed {seed}: {got} vs {expected}"
            );
        }
    }

    #[test]
    fn value_noise_matches_the_cpp_library() {
        assert!(VALUE_SAMPLES.len() >= 10);
        let mut differ = 0;
        for &(x, y, z, seed, int_expected, expected) in VALUE_SAMPLES {
            // The library's build drops the last mask (see `ValueHash`).
            assert_eq!(int_value_noise_3d_unmasked(x, y, z, seed), int_expected);
            // One exact conversion, division and subtraction: no room
            // for a difference.
            assert_eq!(
                value_noise_3d(x, y, z, seed, ValueHash::MaskDropped),
                expected
            );

            // As written, the same bits without the sign.
            let masked = int_value_noise_3d(x, y, z, seed);
            assert_eq!(masked, int_expected & 0x7fff_ffff);
            let as_written = value_noise_3d(x, y, z, seed, ValueHash::AsWritten);
            assert!((-1.0..=1.0).contains(&as_written), "{as_written}");
            assert_eq!(as_written, 1.0 - f64::from(masked) / 1073741824.0);
            if int_expected < 0 {
                differ += 1;
                assert_eq!(as_written, expected + 2.0);
            } else {
                assert_eq!(as_written, expected);
            }
        }
        // The samples do show the two readings apart.
        assert!(differ > 0);
    }

    #[test]
    fn voronoi_as_written_stays_in_its_documented_range() {
        let mut apart = 0;
        for &(kind, scale, _, _, x, y, z, library) in MODULE_SAMPLES {
            if kind != b'V' {
                continue;
            }
            let frequency = 1.0 / scale;
            let value = Module::Voronoi { frequency }.value(x, y, z);
            assert!((-1.0..=1.0).contains(&value), "{value}");
            if (value - library).abs() > TOLERANCE {
                apart += 1;
            }
        }
        // The library's build leaves that range; this port does not follow it.
        assert!(apart > 0);
    }

    #[test]
    fn coordinates_fold_into_int_range() {
        assert_eq!(make_int32_range(12.5), 12.5);
        assert_eq!(make_int32_range(1073741824.0), -1073741824.0);
        assert_eq!(make_int32_range(1073741825.5), 3.0 - 1073741824.0);
        assert_eq!(make_int32_range(-1073741825.5), -3.0 + 1073741824.0);
    }

    #[test]
    fn lattice_floor_is_libnoises() {
        assert_eq!(lattice_floor(2.7), 2);
        assert_eq!(lattice_floor(-2.7), -3);
        // Not a true floor at zero and at whole negative numbers.
        assert_eq!(lattice_floor(0.0), -1);
        assert_eq!(lattice_floor(-2.0), -3);
    }

    #[test]
    fn noise_stays_in_range_and_is_repeatable() {
        let perlin = Module::Perlin {
            frequency: 1.0,
            octaves: 1,
            persistence: 0.5,
        };
        let voronoi = Module::Voronoi { frequency: 1.0 };
        for i in 0..200 {
            let (x, y, z) = (
                f64::from(i) * 0.37 - 20.0,
                f64::from(i) * -0.11,
                f64::from(i % 7) * 0.2,
            );
            let p = perlin.value(x, y, z);
            assert!((-1.0..=1.0).contains(&p), "{p}");
            assert_eq!(p, perlin.value(x, y, z));
            let v = voronoi.value(x, y, z);
            assert!((-1.0..=1.0).contains(&v), "{v}");
        }
    }
}
