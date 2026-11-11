//! How much a contoured move lays and how fast (ContourZ.hpp,
//! ContourZ.cpp `contour_z_segment_feedrate`, and the writer's use of
//! them in GCode.cpp `_extrude`).

/// The slowest a move is slowed to for its height, in mm/s
/// (`ZAA_MIN_SPEED_MM_S`).
pub const MIN_SPEED: f64 = 10.0;

/// The share of the height a move's own may differ from the one that
/// set the speed in force and still keep it (`ZAA_SPEED_HYSTERESIS`).
pub const HYSTERESIS: f64 = 0.05;

/// What a move lays, as a share of what it would lay at the layer's
/// height: as tall as the layer and the mean of the heights its two
/// ends are moved by, the exact volume for a straight slope
/// (`contour_z_extrusion_ratio`, with GCode.cpp's trapezoid mean).
/// Ironing keeps its own flow.
pub fn flow_ratio(ironing: bool, height: f64, mean: f64) -> f64 {
    if ironing || height <= 1e-9 {
        1.0
    } else {
        (height + mean) / height
    }
}

/// Whether a move whose own height is `height` keeps the speed set for
/// one of height `set`, both in mm; nothing set yet when `set` is not
/// above 0 (`contour_z_speed_within_hysteresis`).
pub fn keeps(set: f64, height: f64) -> bool {
    set > 0.0 && height > 0.0 && (height - set).abs() <= HYSTERESIS * set
}

/// The speed of a contoured move, in mm/s: the line's own, `speed`,
/// by the move's height over the layer's, never faster than `speed`,
/// never slower than [`MIN_SPEED`] (nor than `speed` where that is
/// slower), and last held to what the hot end melts, `max_volumetric`
/// mm³/s (none at 0) over the move's own `mm3_per_mm`
/// (`contour_z_segment_feedrate`, there in mm/min).
pub fn speed(speed: f64, nominal: f64, height: f64, max_volumetric: f64, mm3_per_mm: f64) -> f64 {
    if speed <= 0.0 || nominal <= 1e-9 || height <= 1e-9 {
        return speed;
    }
    let mut scaled = (speed * height / nominal).min(speed);
    scaled = scaled.max(MIN_SPEED.min(speed));
    if max_volumetric > 0.0 && mm3_per_mm > 1e-12 {
        scaled = scaled.min(max_volumetric / mm3_per_mm);
    }
    scaled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thin_move_is_slowed_in_step_and_a_thick_one_not_sped_up() {
        assert!((speed(200.0, 0.2, 0.1, 0.0, 0.0) - 100.0).abs() < 1e-9);
        assert_eq!(speed(200.0, 0.2, 0.25, 0.0, 0.0), 200.0);
        // Never under 10 mm/s, unless the line itself is slower.
        assert_eq!(speed(200.0, 0.2, 0.005, 0.0, 0.0), 10.0);
        assert_eq!(speed(8.0, 0.2, 0.05, 0.0, 0.0), 8.0);
        // And the hot end's limit has the last word.
        assert!((speed(200.0, 0.2, 0.25, 12.0, 0.12) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_speed_holds_within_five_percent_of_its_height() {
        assert!(!keeps(0.0, 0.2));
        assert!(keeps(0.2, 0.209));
        assert!(!keeps(0.2, 0.211));
        assert!((flow_ratio(false, 0.2, -0.05) - 0.75).abs() < 1e-12);
        assert_eq!(flow_ratio(true, 0.2, -0.05), 1.0);
    }
}
