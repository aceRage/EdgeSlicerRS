//! The height of one point of a line, and the smoothing of a line's
//! heights: ContourZ.cpp `contour_z_sample_delta` and
//! `contour_z_smooth_profile`, as pure functions of numbers.

/// What a line is, as far as how high it may go is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A wall: never raised, only lowered to the surface (and below it
    /// on a slope, by `zaa_minimize_perimeter_height`).
    Wall,
    /// Top surface fill: lowered to the surface, or raised by up to
    /// `zaa_min_z`.
    Top,
    /// Ironing over a top surface: anywhere from a layer and 0.1 mm
    /// below the layer's top to a layer above it.
    Ironing,
}

/// The upward ray from the slicing plane, where it left the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// How far above the slicing plane, in mm.
    pub distance: f64,
    /// The face's normal there (any length).
    pub normal: [f64; 3],
}

/// Everything the height of one point turns on (ContourZ.hpp
/// `ContourZSampleInput`). The reference's `z_offset_mm`, the shift of
/// `offset_layers`' raised walls, is not here: those walls are not made
/// by this engine, so it is always 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub kind: Kind,
    pub hit: Option<Hit>,
    /// The layer's top and where it was sliced, in mm.
    pub print_z: f64,
    pub slice_z: f64,
    /// The layer's height, in mm.
    pub height: f64,
    /// `zaa_min_z`, in mm.
    pub min_z: f64,
    /// `zaa_minimize_perimeter_height`, in degrees; 0 for none.
    pub minimize_perimeter_height: f64,
    /// Half the line's width, in mm.
    pub half_width: f64,
}

/// Upstream's tolerance above the band within which a point still counts
/// as on the top surface (ContourZ.hpp `ZAA_TOP_TOLERANCE_MM`).
pub const TOP_TOLERANCE: f64 = 0.03;
/// The band of slopes over which a wall's drop down the slope fades in
/// (`ZAA_SLOPE_RAMP_DEGREES`).
pub const SLOPE_RAMP_DEGREES: f64 = 20.0;
/// Half the window, in points, of the moving average over a line's
/// heights (`ZAA_SMOOTH_RADIUS_SAMPLES`).
pub const SMOOTH_RADIUS: usize = 3;

/// A smoothstep over the band above `zaa_minimize_perimeter_height`
/// (ContourZ.cpp `slope_ramp`).
fn slope_ramp(degrees_above_threshold: f64) -> f64 {
    let t = (degrees_above_threshold / SLOPE_RAMP_DEGREES).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far from the vertical a face's normal leans, in radians: the
/// slope of the face (`slope_from_normal`).
fn slope_of(normal: [f64; 3]) -> f64 {
    let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    if length == 0.0 {
        return 0.0;
    }
    (normal[2] / length).abs().min(1.0).acos()
}

/// How far a point of a line is moved up (positive) or down from the
/// layer's top, in mm (ContourZ.cpp `contour_z_sample_delta`).
pub fn delta(sample: &Sample) -> f64 {
    let Some(hit) = sample.hit else {
        // Too far from the surface to be a top: left as it is.
        return 0.0;
    };
    // From the layer's top to the surface the ray left the mesh by.
    let mut d = hit.distance - (sample.print_z - sample.slice_z);
    let (max_up, min_down) = match sample.kind {
        Kind::Ironing => (sample.height, -(sample.height + 0.1)),
        Kind::Wall | Kind::Top => (sample.min_z, -(sample.height - sample.min_z)),
    };
    if sample.kind == Kind::Wall {
        let slope = slope_of(hit.normal);
        let degrees = slope.to_degrees();
        let threshold = sample.minimize_perimeter_height;
        if d > min_down && threshold > 0.0 && threshold < degrees {
            // Down the slope by half the line's width, so the wall's top
            // edge meets the surface rather than its middle. The
            // reference applies it in full at the threshold, which upstream
            // reports as walls torn up where the slope crosses it
            // (OrcaSlicer#13552); it fades it in over a band above.
            let adjustment = (-sample.half_width * slope.sin()).min(0.0);
            d += adjustment * slope_ramp(degrees - threshold);
            d = d.max(min_down);
        }
    }
    if d < -sample.height {
        // Not near the top surface (ContourZ.cpp: "probably because this
        // is not a top surface").
        return 0.0;
    }
    if d > max_up {
        // Faded to nothing over the tolerance above the band, where
        // upstream pins it to the band's top and then drops it to 0.
        let over = d - max_up;
        if over >= TOP_TOLERANCE {
            return 0.0;
        }
        d = max_up * (1.0 - over / TOP_TOLERANCE);
    } else if d < min_down {
        d = min_down;
    }
    // "Do not increase the height of perimeters as this may create the
    // appearance of a seam."
    if sample.kind == Kind::Wall && d > 0.0 {
        d = 0.0;
    }
    d
}

/// A moving average over a line's heights, `radius` points either side,
/// the window shrinking at the line's two ends
/// (`contour_z_smooth_profile`).
pub fn smooth(heights: &mut Vec<f64>, radius: usize) {
    if radius < 1 || heights.len() < 3 {
        return;
    }
    let count = heights.len();
    let smoothed: Vec<f64> = (0..count)
        .map(|at| {
            let (low, high) = (at.saturating_sub(radius), (at + radius).min(count - 1));
            heights[low..=high].iter().sum::<f64>() / (high - low + 1) as f64
        })
        .collect();
    *heights = smoothed;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: Kind, distance: Option<f64>, normal: [f64; 3]) -> Sample {
        // A layer 0.2 mm tall, its top at 1.0 and sliced 0.05 above its
        // bottom: the ray starts 0.15 below the top.
        Sample {
            kind,
            hit: distance.map(|distance| Hit { distance, normal }),
            print_z: 1.0,
            slice_z: 0.85,
            height: 0.2,
            min_z: 0.05,
            minimize_perimeter_height: 35.0,
            half_width: 0.2,
        }
    }

    const UP: [f64; 3] = [0.0, 0.0, 1.0];

    #[test]
    fn a_top_line_follows_the_surface_within_its_band() {
        let at = |distance| delta(&sample(Kind::Top, Some(distance), UP));
        // The surface 0.05 below the top, at it, and at the band's top.
        assert!((at(0.10) + 0.05).abs() < 1e-12);
        assert!(at(0.15).abs() < 1e-12);
        assert!((at(0.20) - 0.05).abs() < 1e-12);
        // Fading to nothing over the 0.03 above the band.
        assert!((at(0.215) - 0.025).abs() < 1e-12);
        assert_eq!(at(0.231), 0.0);
        assert_eq!(at(1.0), 0.0);
        // Never lower than the slicing plane.
        assert!((at(0.0) + 0.15).abs() < 1e-12);
        // No surface above at all: left alone.
        assert_eq!(delta(&sample(Kind::Top, None, UP)), 0.0);
    }

    #[test]
    fn a_wall_is_never_raised_and_drops_down_a_slope() {
        let flat = |distance| delta(&sample(Kind::Wall, Some(distance), UP));
        assert_eq!(flat(0.2), 0.0);
        assert!((flat(0.1) + 0.05).abs() < 1e-12);
        // On a slope of 60 degrees, 25 above the threshold of 35, the
        // drop of half a width down the slope is in full.
        let normal = [60f64.to_radians().sin(), 0.0, 60f64.to_radians().cos()];
        let steep = delta(&sample(Kind::Wall, Some(0.2), normal));
        assert!((steep - (0.05 - 0.2 * 60f64.to_radians().sin())).abs() < 1e-9);
        // At 45 degrees, half way into the band, the smoothstep's half.
        let normal = [45f64.to_radians().sin(), 0.0, 45f64.to_radians().cos()];
        let mid = delta(&sample(Kind::Wall, Some(0.2), normal));
        assert!((mid - (0.05 - 0.5 * 0.2 * 45f64.to_radians().sin())).abs() < 1e-9);
        // Below the threshold, nothing.
        let normal = [20f64.to_radians().sin(), 0.0, 20f64.to_radians().cos()];
        assert!((delta(&sample(Kind::Wall, Some(0.12), normal)) + 0.03).abs() < 1e-12);
        // And never below the slicing plane.
        let normal = [80f64.to_radians().sin(), 0.0, 80f64.to_radians().cos()];
        assert!((delta(&sample(Kind::Wall, Some(0.01), normal)) + 0.15).abs() < 1e-12);
    }

    #[test]
    fn ironing_has_a_band_a_layer_tall() {
        let at = |distance| delta(&sample(Kind::Ironing, Some(distance), UP));
        assert!((at(0.30) - 0.15).abs() < 1e-12);
        assert!((at(0.35) - 0.2).abs() < 1e-12);
        assert_eq!(at(0.40), 0.0);
    }

    #[test]
    fn smoothing_keeps_a_ramp_and_takes_out_a_spike() {
        let mut ramp: Vec<f64> = (0..20).map(|i| -0.01 * i as f64).collect();
        let before = ramp.clone();
        smooth(&mut ramp, SMOOTH_RADIUS);
        for (a, b) in before[3..17].iter().zip(&ramp[3..17]) {
            assert!((a - b).abs() < 1e-12);
        }
        let mut spike = vec![0.0; 9];
        spike[4] = -0.07;
        smooth(&mut spike, SMOOTH_RADIUS);
        assert!((spike[4] + 0.01).abs() < 1e-12);
        assert!(spike.iter().all(|d| *d <= 0.0));
    }
}
