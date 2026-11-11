//! Seams lined up from layer to layer: the seams already chosen are
//! gathered into strings that run up the part, each a seam on one layer
//! and the seam, or a point no worse by much, near it on the next; and a
//! smooth curve is fitted through each string, to which its seams are
//! moved, the less the sharper the corner they are at.
//!
//! A port of the reference's `SeamPlacer::align_seam_points` and
//! `find_seam_string` (SeamPlacer.cpp), with the curve of
//! Geometry/Curves.hpp (`fit_cubic_bspline`) and the kernel of
//! Geometry/Bicubic.hpp.
//!
//! Not as the reference:
//! - A string is looked for upward from its first seam and then
//!   downward. Where the way up ends short of the top layer, the
//!   reference turns round and steps down once more before it looks, so
//!   the layer just beneath the first seam is passed over and its seam
//!   left out of the line (`find_seam_string`: `reverse_lookup_direction`
//!   sets the layer beneath, and the loop's own step then moves on from
//!   it). Here that layer is looked at like any other.
//! - The curve is fitted in double precision by the normal equations,
//!   where the reference solves the same least squares in single
//!   precision by a QR decomposition; the two agree to what single
//!   precision holds.

use crate::seam_plan::{
    Candidate, Comparator, LayerSeams, Paint, angle_penalty, find_next_seam_in_layer,
};

/// How far a seam is looked for from the one on the next layer, in line
/// widths (`seam_align_tolerable_dist_factor`).
const REACH: f32 = 4.0;
/// A string of fewer seams than this is not worth lining up
/// (`seam_align_minimum_string_seams`).
const SHORTEST: usize = 6;
/// The length of string each stretch of the curve covers, in mm
/// (`seam_align_mm_per_segment`).
const MM_PER_SEGMENT: f32 = 4.0;
/// A corner this sharp keeps its seam where it is
/// (`sharp_angle_snapping_threshold`).
const SHARP: f32 = 55.0 * std::f32::consts::PI / 180.0;

/// A seam, or a point taken for one: its layer and which of the layer's
/// candidates it is.
type At = (usize, usize);

/// The seams near enough from layer to layer to make one line, both ways
/// from `start` (`find_seam_string`).
fn find_seam_string(layers: &[LayerSeams], start: At, comparator: &Comparator) -> Vec<At> {
    let reach = REACH
        * layers[start.0].perimeters[layers[start.0].points[start.1].perimeter as usize].flow_width;
    let mut string = vec![start];
    for upward in [true, false] {
        let mut previous = start;
        loop {
            let next = if upward {
                previous.0 + 1
            } else if previous.0 == 0 {
                break;
            } else {
                previous.0 - 1
            };
            let Some(layer) = layers.get(next) else {
                break;
            };
            let from = layers[previous.0].points[previous.1].position;
            // A layer the object has nothing on ends the line.
            let Some(z) = layer.points.first().map(|point| point.position[2]) else {
                break;
            };
            let projected = [from[0], from[1], z];
            let Some(found) = find_next_seam_in_layer(layer, projected, reach, comparator) else {
                break;
            };
            previous = (next, found);
            string.push(previous);
        }
    }
    string
}

/// The cubic B-spline's kernel (Bicubic.hpp `CubicBSplineKernel`).
fn kernel(x: f64) -> f64 {
    let x = x.abs();
    if x >= 2.0 {
        return 0.0;
    }
    if x <= 1.0 {
        return 4.0 / 6.0 - x * x + 0.5 * x * x * x;
    }
    let x = x - 1.0;
    1.0 / 6.0 - 0.5 * x + 0.5 * x * x - x * x * x / 6.0
}

/// A smooth curve through points seen at rising heights
/// (Curves.hpp `PiecewiseFittedCurve`).
#[derive(Clone, Debug)]
pub(crate) struct Curve {
    /// A place for each knot.
    coefficients: Vec<[f64; 2]>,
    start: f64,
    segment: f64,
}

impl Curve {
    /// Which knots bear on height `at`, and how much each.
    fn bearing(&self, at: f64) -> [(usize, f64); 4] {
        let middle = ((at - self.start) / self.segment).floor() as i64;
        std::array::from_fn(|step| {
            let segment = middle - 1 + step as i64;
            let from = self.start + segment as f64 * self.segment;
            let knot = segment.clamp(0, self.coefficients.len() as i64 - 1) as usize;
            (knot, kernel((from - at) / self.segment))
        })
    }

    /// The curve of `segments` stretches that best fits `observations`
    /// made at `heights`, which rise, each counting for its weight
    /// (`fit_cubic_bspline`).
    // A matrix reads best by row and column.
    #[allow(clippy::needless_range_loop)]
    pub(crate) fn fit(
        observations: &[[f32; 2]],
        heights: &[f32],
        weights: &[f32],
        segments: usize,
    ) -> Self {
        let (first, last) = (f64::from(heights[0]), f64::from(heights[heights.len() - 1]));
        let count = segments + 1;
        let mut curve = Self {
            coefficients: vec![[0.0; 2]; count],
            start: first,
            segment: (last - first) / segments as f64,
        };
        if curve.segment <= 0.0 {
            // All at one height: no curve, the place of the first.
            curve.segment = 1.0;
            curve.coefficients = vec![observations[0].map(f64::from); count];
            return curve;
        }
        // Weighted least squares, by the normal equations.
        let mut matrix = vec![vec![0.0f64; count]; count];
        let mut right = vec![[0.0f64; 2]; count];
        for ((observation, height), weight) in observations.iter().zip(heights).zip(weights) {
            // The row of this observation: a knot at either end bears
            // for those beyond it too.
            let mut row: Vec<(usize, f64)> = Vec::with_capacity(4);
            for (knot, share) in curve.bearing(f64::from(*height)) {
                match row.iter_mut().find(|entry| entry.0 == knot) {
                    Some(entry) => entry.1 += share,
                    None => row.push((knot, share)),
                }
            }
            let weight = f64::from(*weight);
            for &(a, share_a) in &row {
                for &(b, share_b) in &row {
                    matrix[a][b] += weight * share_a * share_b;
                }
                for axis in 0..2 {
                    right[a][axis] += weight * share_a * f64::from(observation[axis]);
                }
            }
        }
        // Gaussian elimination with the largest pivot of each column.
        for column in 0..count {
            let pivot = (column..count)
                .max_by(|&a, &b| matrix[a][column].abs().total_cmp(&matrix[b][column].abs()))
                .unwrap_or(column);
            matrix.swap(column, pivot);
            right.swap(column, pivot);
            let by = matrix[column][column];
            // A knot nothing bears on is left where it is.
            if by.abs() < 1e-12 {
                continue;
            }
            for row in column + 1..count {
                let factor = matrix[row][column] / by;
                if factor == 0.0 {
                    continue;
                }
                for across in column..count {
                    matrix[row][across] -= factor * matrix[column][across];
                }
                for axis in 0..2 {
                    right[row][axis] -= factor * right[column][axis];
                }
            }
        }
        for column in (0..count).rev() {
            let by = matrix[column][column];
            for axis in 0..2 {
                let mut value = right[column][axis];
                for across in column + 1..count {
                    value -= matrix[column][across] * curve.coefficients[across][axis];
                }
                curve.coefficients[column][axis] = if by.abs() < 1e-12 { 0.0 } else { value / by };
            }
        }
        curve
    }

    /// Where the curve is at a height (`get_fitted_value`).
    pub(crate) fn at(&self, height: f32) -> [f32; 2] {
        let mut place = [0.0f64; 2];
        for (knot, share) in self.bearing(f64::from(height)) {
            place[0] += share * self.coefficients[knot][0];
            place[1] += share * self.coefficients[knot][1];
        }
        place.map(|c| c as f32)
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// The angle between two directions.
fn angle_between(a: [f32; 3], b: [f32; 3]) -> f32 {
    let unit = |v: [f32; 3]| {
        let long = length(v);
        if long > 0.0 { v.map(|c| c / long) } else { v }
    };
    let (a, b) = (unit(a), unit(b));
    (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).acos().abs()
}

/// Lines up the seams of an object, layer by layer from the bottom
/// (`align_seam_points`). Each seam lined up has its place settled
/// (`Perimeter::finalized`), which need not be a point of its wall.
pub(crate) fn align(layers: &mut [LayerSeams], comparator: &Comparator) {
    let point = |layers: &[LayerSeams], at: At| -> Candidate { layers[at.0].points[at.1] };
    let settled = |layers: &[LayerSeams], at: At| {
        layers[at.0].perimeters[layers[at.0].points[at.1].perimeter as usize]
            .finalized
            .is_some()
    };
    // Every seam, the best first: "Alignment is sensitive to
    // initialization, this gives it better chance to choose something
    // nice".
    let mut seams: Vec<At> = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        seams.extend(
            layer
                .perimeters
                .iter()
                .map(|perimeter| (index, perimeter.seam)),
        );
    }
    seams.sort_by(|&a, &b| {
        let (a, b) = (point(layers, a), point(layers, b));
        if comparator.is_first_better(&a, &b, [0.0; 2]) {
            std::cmp::Ordering::Less
        } else if comparator.is_first_better(&b, &a, [0.0; 2]) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });

    let mut index = 0;
    while index < seams.len() {
        let start = seams[index];
        index += 1;
        if settled(layers, start) {
            continue;
        }
        let mut string = find_seam_string(layers, start, comparator);
        // A longer line may be had by setting out from another of its
        // seams.
        let step = 1 + string.len() / 20;
        let mut alternative = 0;
        while alternative < string.len() {
            let from = string[alternative];
            let own =
                layers[from.0].perimeters[layers[from.0].points[from.1].perimeter as usize].seam;
            let other = find_seam_string(layers, (from.0, own), comparator);
            if other.len() > string.len() {
                string = other;
            }
            alternative += step;
        }
        if string.len() < SHORTEST {
            continue;
        }
        string.sort_by_key(|at| at.0);
        // The seam set out from comes round again, "since it could be
        // skipped due to alternative path being aligned".
        index -= 1;

        // Where the seams are and how much each counts: a seam at a
        // sharp corner for more.
        let places: Vec<Candidate> = string.iter().map(|&at| point(layers, at)).collect();
        let observations: Vec<[f32; 2]> = (places.iter())
            .map(|place| [place.position[0], place.position[1]])
            .collect();
        let heights: Vec<f32> = places.iter().map(|place| place.position[2]).collect();
        // A painted point and the middle of a joint count for more again.
        let weights: Vec<f32> = (places.iter())
            .map(|place| {
                1.0 / (0.1 + angle_penalty(place.angle))
                    + if place.paint == Paint::Enforced {
                        3.0
                    } else {
                        0.0
                    }
                    + if place.central_part_joint { 3.0 } else { 0.0 }
            })
            .collect();
        // How long the line is, which says how many stretches the curve
        // has: a seam where the line bends more than its wall does
        // counts against the length.
        let mut total_length = 0.0f32;
        let mut last = places[0].position;
        for (at, place) in places.iter().enumerate() {
            let mut bend = 0.0;
            if at > 0 && at + 1 < places.len() {
                bend = angle_between(
                    sub(place.position, places[at - 1].position),
                    sub(places[at + 1].position, place.position),
                );
            }
            let curling = if f64::from(bend) > 2.0 * f64::from(place.angle.abs())
                && place.paint != Paint::Enforced
                && !place.central_part_joint
            {
                -0.8
            } else {
                1.0
            };
            total_length += curling * length(sub(last, place.position));
            last = place.position;
        }
        if comparator.directional() {
            total_length *= 0.3;
        }
        let segments = ((total_length.max(0.0) / MM_PER_SEGMENT) as usize).max(1);
        let curve = Curve::fit(&observations, &heights, &weights, segments);

        // Each seam goes to the curve, the less the sharper its corner.
        for (&at, place) in string.iter().zip(&places) {
            let mut keep = (place.angle.abs() / SHARP).powf(3.0).min(1.0);
            // A painted point keeps to its place more.
            if place.paint == Paint::Enforced {
                keep = keep.max(0.4);
            }
            let fitted = curve.at(place.position[2]);
            let mut settled = [
                keep * place.position[0] + (1.0 - keep) * fitted[0],
                keep * place.position[1] + (1.0 - keep) * fitted[1],
                place.position[2],
            ];
            let layer = &mut layers[at.0];
            let perimeter = &mut layer.perimeters[layer.points[at.1].perimeter as usize];
            if place.central_part_joint {
                // Along the joint, but never off it: no further than half
                // a line's width from the joint's point.
                let shift = [
                    settled[0] - place.position[0],
                    settled[1] - place.position[1],
                    settled[2] - place.position[2],
                ];
                let (moved, most) = (length(shift), 0.5 * perimeter.flow_width);
                if moved > most {
                    settled = std::array::from_fn(|axis| {
                        place.position[axis] + shift[axis] * (most / moved)
                    });
                }
            }
            perimeter.seam = at.1;
            perimeter.finalized = Some(settled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_is_fitted_through_the_points_it_is_given() {
        let heights: Vec<f32> = (0..50).map(|at| 0.1 + 0.2 * at as f32).collect();
        let weights = vec![1.0; 50];
        // Points all in one place: the curve is that place, however
        // many stretches it has.
        let still = vec![[3.0f32, -2.0]; 50];
        for segments in [1, 2, 5] {
            let curve = Curve::fit(&still, &heights, &weights, segments);
            for z in &heights {
                let found = curve.at(*z);
                assert!((found[0] - 3.0).abs() < 1e-5 && (found[1] + 2.0).abs() < 1e-5);
            }
        }
        // Points on a slanting line: with stretches enough the curve
        // follows it, but for its ends, which a curve of this kind
        // flattens.
        let line: Vec<[f32; 2]> = (heights.iter())
            .map(|z| [3.0 + 0.5 * z, -2.0 - 0.25 * z])
            .collect();
        let curve = Curve::fit(&line, &heights, &weights, 5);
        for (z, place) in heights.iter().zip(&line).skip(8).take(34) {
            let found = curve.at(*z);
            assert!(
                (found[0] - place[0]).abs() < 0.02 && (found[1] - place[1]).abs() < 0.02,
                "at {z}: {found:?} {place:?}"
            );
        }
        // Points scattered about a line are drawn in to it, and a point
        // that counts for much more than the rest draws the curve to it.
        let scattered: Vec<[f32; 2]> = (line.iter().enumerate())
            .map(|(at, place)| [place[0] + if at % 2 == 0 { 0.3 } else { -0.3 }, place[1]])
            .collect();
        let curve = Curve::fit(&scattered, &heights, &weights, 5);
        for (z, place) in heights.iter().zip(&line).skip(8).take(34) {
            assert!((curve.at(*z)[0] - place[0]).abs() < 0.05);
        }
        let mut heavy = weights.clone();
        heavy[25] = 1000.0;
        let pulled = Curve::fit(&scattered, &heights, &heavy, 5);
        let off = |curve: &Curve| (curve.at(heights[25])[0] - scattered[25][0]).abs();
        assert!(
            off(&pulled) < 0.2 * off(&curve),
            "{} {}",
            off(&pulled),
            off(&curve)
        );
        // The kernel is a bump, and the bumps of the knots round any
        // place come to one.
        for step in 0..10 {
            let x = step as f64 * 0.1;
            let sum = kernel(x - 1.0) + kernel(x) + kernel(x + 1.0) + kernel(x - 2.0);
            assert!((sum - 1.0).abs() < 1e-12);
        }
        assert_eq!((kernel(2.0), kernel(-3.0)), (0.0, 0.0));
    }
}
