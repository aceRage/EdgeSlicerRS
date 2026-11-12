//! How much is flushed between two filaments: the project's
//! `flush_volumes_matrix` (mm³ from filament i to filament j at `i * n + j`)
//! and `flush_volumes_vector` (two numbers for each filament), kept the
//! size of the filament list as the reference keeps them, and the volumes a
//! filament added or recoloured is given from the colours (the reference's
//! "Auto-Calc", FlushVolCalc.cpp, Plater.cpp `auto_calc_flushing_volumes`).

use edge_config::Settings;

/// The least flushed from a support filament (FlushVolCalc.cpp
/// `g_min_flush_volume_from_support`).
const FROM_SUPPORT: i32 = 420;
/// What is flushed into a support filament (`g_flush_volume_to_support`).
const TO_SUPPORT: i32 = 230;
/// The most a volume worked out may be (`g_max_flush_volume`).
pub const MAX_FLUSH: i32 = 20000;

/// A colour as the reference's `wxColour` reads it: `#RRGGBB`, or
/// `#RRGGBBAA`; anything else is white.
fn rgba(colour: &str) -> [u8; 4] {
    let hex = colour.trim().trim_start_matches('#');
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    match hex.len() {
        6 | 8 => match (byte(0), byte(2), byte(4)) {
            (Some(r), Some(g), Some(b)) => [
                r,
                g,
                b,
                if hex.len() == 8 {
                    byte(6).unwrap_or(255)
                } else {
                    255
                },
            ],
            _ => [255; 4],
        },
        _ => [255; 4],
    }
}

/// ColorSpaceConvert.cpp `RGB2HSV`, in single precision as there.
fn hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if f64::from(delta.abs()) < 0.001 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let s = if f64::from(max.abs()) < 0.001 {
        0.0
    } else {
        delta / max
    };
    (h, s, max)
}

fn radians(degrees: f32) -> f32 {
    (f64::from(degrees / 180.0) * std::f64::consts::PI) as f32
}

fn luminance(r: f32, g: f32, b: f32) -> f32 {
    (f64::from(r) * 0.3 + f64::from(g) * 0.59 + f64::from(b) * 0.11) as f32
}

/// FlushVolCalculator::calc_flush_vol: what is flushed going from one
/// colour to another, at least `min` (with 60 mm³ more) and at most `max`.
pub fn calc_flush_vol(from: &str, to: &str, min: i32, max: i32) -> i32 {
    let [mut sr, mut sg, mut sb, sa] = rgba(from);
    let [mut dr, mut dg, mut db, da] = rgba(to);
    // Transparent filaments are taken as white.
    if sa == 0 {
        (sr, sg, sb) = (255, 255, 255);
    }
    if da == 0 {
        (dr, dg, db) = (255, 255, 255);
    }
    let unit = |c: u8| f32::from(c) / 255.0;
    let (sr, sg, sb, dr, dg, db) = (unit(sr), unit(sg), unit(sb), unit(dr), unit(dg), unit(db));
    let (h1, s1, v1) = hsv(sr, sg, sb);
    let (h2, s2, v2) = hsv(dr, dg, db);
    // DeltaHS_BBS.
    let (a1, a2) = (radians(h1), radians(h2));
    let dx = a1.cos() * s1 * v1 - a2.cos() * s2 * v2;
    let dy = a1.sin() * s1 * v1 - a2.sin() * s2 * v2;
    let mut hs_dist = (dx * dx + dy * dy).sqrt().min(1.2);
    // A colour change shows more going to a light colour, and from a dark.
    let (from_lumi, to_lumi) = (luminance(sr, sg, sb), luminance(dr, dg, db));
    let lumi_flush = if to_lumi >= from_lumi {
        (to_lumi - from_lumi).powf(0.7) * 560.0
    } else {
        let inter = (0.67 * f64::from(v2) + 0.33 * f64::from(v1)) as f32;
        hs_dist = inter.min(hs_dist);
        (from_lumi - to_lumi) * 80.0
    };
    let hs_flush = 230.0 * hs_dist;
    // The third side of a triangle with these two at 120°.
    let third = (hs_flush * hs_flush + lumi_flush * lumi_flush
        - 2.0 * hs_flush * lumi_flush * radians(120.0).cos())
    .sqrt();
    let volume = third.max(60.0) + min as f32;
    (volume as i32).min(max)
}

/// Plater.cpp `get_min_flush_volumes`: for each filament, the nozzle's
/// volume less what a long retraction when cutting takes back out of it.
pub fn min_flush_volumes(settings: &Settings) -> Vec<i32> {
    let number = |key: &str| settings.texts(key).and_then(|items| items.first().cloned());
    let nozzle_volume = settings
        .text("nozzle_volume")
        .or_else(|| {
            settings
                .texts("nozzle_volume")
                .and_then(|items| items.first().map(String::as_str))
        })
        .and_then(|text| text.trim().parse::<f64>().ok())
        .map_or(0, |volume| volume as i32);
    let level = settings
        .text("enable_long_retraction_when_cut")
        .and_then(|text| text.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let machine_on = number("long_retractions_when_cut").is_some_and(|on| on.trim() == "1");
    let printer_length = number("retraction_distances_when_cut")
        .and_then(|text| text.trim().parse::<f64>().ok())
        .unwrap_or(18.0);
    let count = settings
        .texts("filament_diameter")
        .map_or(1, <[String]>::len);
    let lengths = settings.texts("filament_retraction_distances_when_cut");
    let on = settings.texts("filament_long_retractions_when_cut");
    (0..count)
        .map(|filament| {
            let mut retract = if level != 0 && machine_on {
                printer_length as i32
            } else {
                0
            };
            // 0, 1, or "nil" (the printer's).
            let own = on
                .and_then(|on| on.get(filament))
                .map(|text| text.trim().to_owned());
            let own_length = lengths
                .and_then(|lengths| lengths.get(filament))
                .and_then(|text| text.trim().parse::<f64>().ok());
            match own.as_deref() {
                Some("0") | None => retract = 0,
                // `LongRectrationLevel::EnableFilament`.
                Some("1") if level == 2 => {
                    retract = own_length.map_or(printer_length as i32, |length| length as i32);
                }
                _ => {}
            }
            let extra = (f64::from(nozzle_volume)
                - std::f64::consts::PI * 1.75 * 1.75 / 4.0 * f64::from(retract))
                as i32;
            extra.max(0)
        })
        .collect()
}

/// The flushing volumes made the size of `count` filaments as the
/// reference makes them when filaments are added (PresetBundle.cpp
/// `update_multi_material_filament_presets`): two numbers of the vector for
/// each filament, the new ones copied from the first's (140 when there are
/// none); the matrix keeps what it had and is `vector[2i] + vector[2j + 1]`
/// from i to j where it had nothing, 0 on the diagonal.
pub fn resized(matrix: &[f64], vector: &[f64], count: usize) -> (Vec<f64>, Vec<f64>) {
    let old = (matrix.len() as f64).sqrt().round() as usize;
    if old * old != matrix.len() {
        return resized(&[], vector, count);
    }
    // Nothing is done while the matrix is the right size.
    if old == count {
        return (matrix.to_vec(), vector.to_vec());
    }
    let mut vector = vector.to_vec();
    while vector.len() < 2 * count {
        let first = if vector.len() > 1 { vector[0] } else { 140.0 };
        vector.push(first);
        let second = if vector.len() > 1 { vector[1] } else { 140.0 };
        vector.push(second);
    }
    vector.truncate(2 * count);
    let mut out = Vec::with_capacity(count * count);
    for i in 0..count {
        for j in 0..count {
            out.push(if i < old && j < old {
                matrix[i * old + j]
            } else if i == j {
                0.0
            } else {
                vector[2 * i] + vector[2 * j + 1]
            });
        }
    }
    (out, vector)
}

/// The flushing volumes with filament `gone` (from 0) taken out: its row
/// and column of the matrix and its two numbers of the vector.
///
/// The reference keeps the first n - 1 rows and columns instead whichever
/// filament goes (`update_multi_material_filament_presets` is handed the
/// filament deleted and does not use it for the matrix), so deleting
/// filament 2 of 3 leaves filament 3 with filament 2's volumes. That is
/// not followed.
pub fn without(matrix: &[f64], vector: &[f64], gone: usize) -> (Vec<f64>, Vec<f64>) {
    let old = (matrix.len() as f64).sqrt().round() as usize;
    let matrix = if old * old == matrix.len() && gone < old {
        (0..old)
            .filter(|i| *i != gone)
            .flat_map(|i| {
                (0..old)
                    .filter(|j| *j != gone)
                    .map(move |j| matrix[i * old + j])
            })
            .collect()
    } else {
        matrix.to_vec()
    };
    let vector = vector
        .chunks(2)
        .enumerate()
        .filter(|(at, _)| *at != gone)
        .flat_map(|(_, pair)| pair.iter().copied())
        .collect();
    (matrix, vector)
}

/// Plater.cpp `auto_calc_flushing_volumes(modify_id)`: the volumes from
/// every other filament to filament `changed` and from it to every other,
/// worked out from their colours; going to a support filament is a fixed
/// volume, and from one at least a fixed volume. `minimums` and `support`
/// are each filament's ([`min_flush_volumes`], `filament_is_support`).
pub fn auto_calc(
    matrix: &mut [f64],
    colours: &[String],
    minimums: &[i32],
    support: &[bool],
    changed: usize,
) {
    let count = colours.len();
    let n = (matrix.len() as f64).sqrt().round() as usize;
    if changed >= count {
        return;
    }
    let least = |filament: usize| minimums.get(filament).copied().unwrap_or(0);
    let is_support = |filament: usize| support.get(filament).copied().unwrap_or(false);
    for other in (0..count).filter(|other| *other != changed) {
        // From `other` to the one changed.
        let volume = if is_support(changed) {
            TO_SUPPORT
        } else {
            let volume =
                calc_flush_vol(&colours[other], &colours[changed], least(other), MAX_FLUSH);
            if is_support(other) {
                volume.max(FROM_SUPPORT)
            } else {
                volume
            }
        };
        if let Some(at) = matrix.get_mut(n * other + changed) {
            *at = f64::from(volume);
        }
        // From the one changed to `other`.
        let volume = if is_support(other) {
            TO_SUPPORT
        } else {
            let volume = calc_flush_vol(
                &colours[changed],
                &colours[other],
                least(changed),
                MAX_FLUSH,
            );
            if is_support(changed) {
                volume.max(FROM_SUPPORT)
            } else {
                volume
            }
        };
        if let Some(at) = matrix.get_mut(n * changed + other) {
            *at = f64::from(volume);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_colours_flush_the_least_and_dark_to_light_the_most() {
        assert_eq!(calc_flush_vol("#FFFFFF", "#FFFFFF", 0, MAX_FLUSH), 60);
        assert_eq!(calc_flush_vol("#000000", "#000000", 143, MAX_FLUSH), 203);
        let dark_to_light = calc_flush_vol("#000000", "#FFFFFF", 0, MAX_FLUSH);
        let light_to_dark = calc_flush_vol("#FFFFFF", "#000000", 0, MAX_FLUSH);
        assert!(
            dark_to_light > light_to_dark,
            "{dark_to_light} {light_to_dark}"
        );
        // Black to white: 560 for the luminance alone.
        assert_eq!(dark_to_light, 560);
        assert_eq!(calc_flush_vol("#000000", "#FFFFFF", 0, 300), 300);
    }

    #[test]
    fn a_filament_added_gets_the_vectors_sums_and_one_taken_away_its_row() {
        let (matrix, vector) = resized(&[0.0], &[140.0, 140.0], 2);
        assert_eq!(matrix, [0.0, 280.0, 280.0, 0.0]);
        assert_eq!(vector, [140.0; 4]);
        let three = [0.0, 1.0, 2.0, 3.0, 0.0, 5.0, 6.0, 7.0, 0.0];
        let (matrix, vector) = without(&three, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 1);
        assert_eq!(matrix, [0.0, 2.0, 6.0, 0.0]);
        assert_eq!(vector, [1.0, 2.0, 5.0, 6.0]);
        // The reference's default, four filaments, made one.
        let (matrix, vector) = resized(
            &[
                0.0, 280.0, 280.0, 280.0, 280.0, 0.0, 280.0, 280.0, 280.0, 280.0, 0.0, 280.0,
                280.0, 280.0, 280.0, 0.0,
            ],
            &[140.0; 8],
            1,
        );
        assert_eq!((matrix, vector), (vec![0.0], vec![140.0, 140.0]));
    }

    #[test]
    fn a_new_filaments_row_and_column_come_from_the_colours() {
        let mut matrix = vec![0.0, 280.0, 280.0, 0.0];
        let colours = ["#000000".to_owned(), "#FFFFFF".to_owned()];
        auto_calc(&mut matrix, &colours, &[0, 0], &[false, false], 1);
        assert_eq!(
            matrix,
            [
                0.0,
                560.0,
                f64::from(calc_flush_vol("#FFFFFF", "#000000", 0, MAX_FLUSH)),
                0.0
            ]
        );
        let mut matrix = vec![0.0, 280.0, 280.0, 0.0];
        auto_calc(&mut matrix, &colours, &[0, 0], &[false, true], 1);
        assert_eq!(matrix[1], 230.0);
        assert!(matrix[2] >= 420.0);
    }
}
