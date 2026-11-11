//! What one wall line can say about itself (its length, its narrowest
//! point, which way round it runs, the area it encloses) and the pass
//! that thins out its points: a point goes when the line would move by
//! less than the allowed error without it and the wall would not carry
//! noticeably more or less plastic.
//!
//! A port of the reference's
//! `src/libslic3r/Arachne/utils/ExtrusionLine.hpp` and `ExtrusionLine.cpp`
//! (from CuraEngine), with the few pieces of `Line.hpp`, `Line.cpp`,
//! `Point.hpp` and Clipper's `Area` that they call. The arithmetic is kept
//! as it is there, oddities included; each is noted where it happens.
//!
//! The reference keeps a closed line with its first junction repeated at
//! the end (`WallToolPaths::stitchToolPaths` adds it). `simplify` relies
//! on that, `get_length` and the two area functions do not mind either
//! way.

use edge_geom::{Coord, Point2, Polygon};

use crate::{ExtrusionLine, Junction};

/// The reference's `SCALING_FACTOR`: millimetres per unit.
const SCALING_FACTOR: f64 = 0.000001;

/// The reference's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// `scaled<coord_t>(0.025)`: 25 000. The reference compares a squared
/// length with it, so a segment goes unseen below about 158 units, not
/// below the 5 micron its comment speaks of.
const NEGLIGIBLE_LENGTH_SQUARED: i64 = (0.025 / SCALING_FACTOR) as i64;

/// `scaled<coord_t>(0.001)`: 1 000, again compared with a squared length
/// (the height of the triangle a point makes with its neighbours).
const COLINEAR_HEIGHT_SQUARED: i64 = (0.001 / SCALING_FACTOR) as i64;

/// `scaled<double>(0.001)`: a micron, as a distance this time.
const COLINEAR_DISTANCE: f64 = 0.001 / SCALING_FACTOR;

/// How far off a straight line a point may be and still count as on it in
/// the wall generator's own tidying pass (`colinear_vertex_tolerance()` in
/// ExtrusionLine.hpp, which is `SCALED_EPSILON`): 100 units.
pub const fn colinear_vertex_tolerance() -> Coord {
    (EPSILON / SCALING_FACTOR) as Coord
}

/// The squared distance between two points (`(a - b).squaredNorm()` on
/// 64-bit integers).
pub(crate) fn v_size2(a: Point2, b: Point2) -> i64 {
    let (dx, dy) = (a.x - b.x, a.y - b.y);
    dx * dx + dy * dy
}

/// The distance between two points, cut down to a whole number: Eigen's
/// `norm()` on a vector of 64-bit integers takes the root as a double and
/// converts it back.
pub(crate) fn v_size(a: Point2, b: Point2) -> i64 {
    (v_size2(a, b) as f64).sqrt() as i64
}

/// Twice the signed area of the triangle from the origin to `a` and `b`:
/// one term of the shoelace formula. It wraps instead of overflowing, as
/// the compiled reference does, because the terms are of positions on the
/// bed, not of differences: on a bed two metres across single terms pass
/// what 64 bits hold while the sums `simplify` makes of them, the area of
/// a small triangle, come out right all the same.
fn shoelace(a: Point2, b: Point2) -> i64 {
    a.x.wrapping_mul(b.y).wrapping_sub(a.y.wrapping_mul(b.x))
}

/// The squared distance from `point` to the endless line through `a` and
/// `b` (`Line::distance_to_infinite_squared`).
fn distance_to_infinite_squared(point: Point2, a: Point2, b: Point2) -> f64 {
    let v = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let va = ((point.x - a.x) as f64, (point.y - a.y) as f64);
    let l2 = v.0 * v.0 + v.1 * v.1;
    if l2 == 0.0 {
        return va.0 * va.0 + va.1 * va.1;
    }
    let t = (va.0 * v.0 + va.1 * v.1) / l2;
    let d = (t * v.0 - va.0, t * v.1 - va.1);
    d.0 * d.0 + d.1 * d.1
}

/// Where the endless lines through `a1`,`b1` and through `a2`,`b2` cross,
/// cut down to whole units; `None` when they are parallel or cross out of
/// range (`Line::intersection_infinite`).
fn intersection_infinite(a1: Point2, b1: Point2, a2: Point2, b2: Point2) -> Option<Point2> {
    let start = (a1.x as f64, a1.y as f64);
    let v12 = ((a2.x - a1.x) as f64, (a2.y - a1.y) as f64);
    let v1 = ((b1.x - a1.x) as f64, (b1.y - a1.y) as f64);
    let v2 = ((b2.x - a2.x) as f64, (b2.y - a2.y) as f64);
    let denom = v1.0 * v2.1 - v1.1 * v2.0;
    if denom.abs() < EPSILON {
        return None;
    }
    let t1 = (v12.0 * v2.1 - v12.1 * v2.0) / denom;
    let result = (start.0 + t1 * v1.0, start.1 + t1 * v1.1);
    let (max, min) = (i64::MAX as f64, i64::MIN as f64);
    if result.0 > max || result.0 < min || result.1 > max || result.1 < min {
        return None;
    }
    Some(Point2 {
        x: result.0 as i64,
        y: result.1 as i64,
    })
}

/// The `dist_greater` lambda of `simplify`: whether two points are further
/// apart than a squared distance. The reference works in unsigned 64-bit
/// here, where the sum of squares wraps, and first compares each
/// coordinate with the squared threshold itself; both are kept.
fn dist_greater(p1: Point2, p2: Point2, threshold: i64) -> bool {
    let threshold = threshold as u64;
    let x = (p1.x - p2.x).unsigned_abs();
    let y = (p1.y - p2.y).unsigned_abs();
    if x > threshold || y > threshold {
        return true;
    }
    x.wrapping_mul(x).wrapping_add(y.wrapping_mul(y)) > threshold
}

impl ExtrusionLine {
    /// An open line with no junctions yet.
    pub fn new(inset_idx: usize, is_odd: bool) -> Self {
        Self {
            inset_idx,
            is_odd,
            is_closed: false,
            junctions: Vec::new(),
        }
    }

    /// The length of the line: each segment's length cut down to a whole
    /// number, summed. A closed line also counts the way from its last
    /// junction back to its first.
    pub fn get_length(&self) -> i64 {
        let (Some(first), Some(last)) = (self.junctions.first(), self.junctions.last()) else {
            return 0;
        };
        let mut len = 0;
        let mut prev = first.p;
        for next in &self.junctions {
            len += v_size(next.p, prev);
            prev = next.p;
        }
        if self.is_closed {
            len += v_size(first.p, last.p);
        }
        len
    }

    /// The width of the line where it is narrowest; `Coord::MAX` for a
    /// line with no junctions. The reference has no function for this:
    /// it is the loop at the top of `WallToolPaths::removeSmallLines`.
    pub fn get_minimal_width(&self) -> Coord {
        self.junctions
            .iter()
            .fold(Coord::MAX, |min_width, j| min_width.min(j.w))
    }

    /// Turns the line round: the last junction becomes the first.
    pub fn reverse(&mut self) {
        self.junctions.reverse();
    }

    /// Every junction's place, as a polygon (`ExtrusionLine::toPolygon`).
    /// For a line that is not closed, read it as a polyline.
    pub fn to_polygon(&self) -> Polygon {
        Polygon {
            points: to_points(self),
        }
    }

    /// Takes out junctions the line does not need.
    ///
    /// A junction goes when it is on the straight line between its
    /// neighbours (to a micron) and taking it out changes the plastic laid
    /// by no more than `maximum_extrusion_area_deviation`, or when the
    /// segment before it is shorter than the root of
    /// `smallest_line_segment_squared` and the line moves by no more than
    /// the root of `allowed_error_distance_squared`. Where such a short
    /// segment is followed by a long one, the junction is moved to where
    /// the segments on either side of the short one would meet and the
    /// junction before it goes instead, so the long segment keeps its
    /// direction. The first and last junctions of an open line stay, and
    /// a closed line (first junction repeated at the end) keeps three.
    pub fn simplify(
        &mut self,
        smallest_line_segment_squared: i64,
        allowed_error_distance_squared: i64,
        maximum_extrusion_area_deviation: i64,
    ) {
        let min_path_size = if self.is_closed { 3 } else { 2 };
        if self.junctions.len() <= min_path_size {
            return;
        }
        let is_closed = self.is_closed;
        let junctions = &self.junctions;
        let size = junctions.len();

        // The first junction is always in the result.
        let mut new_junctions = vec![junctions[0]];

        let mut previous = junctions[0];
        // An open line has nothing before its first junction. In a closed
        // one the last junction is the first again, so take the one before.
        let mut previous_previous = if is_closed {
            junctions[size - 2]
        } else {
            junctions[0]
        };

        // The area cut off by leaving junctions out is kept as a fan of
        // triangles from the origin (twice the shoelace sum); closing the
        // fan with the short cut gives twice the area cut off, and that
        // over the short cut's length the height of a triangle as large.
        let initial = junctions[1];
        let mut accumulated_area_removed = shoelace(previous.p, initial.p);

        // A closed line's last junction, the same as its first, is gone
        // through as well.
        let end = size - if is_closed { 0 } else { 1 };
        for point_idx in 1..end {
            // The first junction may have been moved since.
            let is_last = point_idx + 1 == size;
            let current = if is_last {
                new_junctions[0]
            } else {
                junctions[point_idx]
            };

            // A closed line keeps three junctions.
            if is_closed && new_junctions.len() + (size - point_idx) <= 3 {
                new_junctions.push(current);
                continue;
            }

            // Past the end, go on from the start of the new line, unless
            // that would be `previous` itself.
            let spill_over =
                is_closed && point_idx + 2 >= size && point_idx + 2 - size < new_junctions.len();
            let next = if spill_over {
                new_junctions[point_idx + 2 - size]
            } else {
                junctions[point_idx + 1]
            };

            let removed_area_next = shoelace(current.p, next.p);
            // Between the origin and the short cut.
            let negative_area_closing = shoelace(next.p, previous.p);
            accumulated_area_removed = accumulated_area_removed.wrapping_add(removed_area_next);

            let length2 = v_size2(current.p, previous.p);
            if length2 < NEGLIGIBLE_LENGTH_SQUARED {
                // Too short to matter, whatever its width.
                continue;
            }

            let area_removed_so_far = accumulated_area_removed.wrapping_add(negative_area_closing);
            let base_length_2 = v_size2(next.p, previous.p);
            if base_length_2 == 0 {
                // Out and straight back: no area. The junction goes.
                continue;
            }
            // height = 2 * area / base, squared: (2 * area)^2 / base^2.
            let height_2 = (area_removed_so_far as f64 * area_removed_so_far as f64
                / base_length_2 as f64) as i64;
            let extrusion_area_error =
                Self::calculate_extrusion_area_deviation_error(previous, current, next);
            // On the straight line, and not only because areas to either
            // side cancelled, and the width may go with it.
            if height_2 <= COLINEAR_HEIGHT_SQUARED
                && distance_to_infinite_squared(current.p, previous.p, next.p).sqrt()
                    <= COLINEAR_DISTANCE
                && extrusion_area_error <= maximum_extrusion_area_deviation
            {
                continue;
            }

            if length2 < smallest_line_segment_squared && height_2 <= allowed_error_distance_squared
            {
                let next_length2 = v_size2(current.p, next.p);
                if next_length2 <= 4 * smallest_line_segment_squared {
                    // The junction goes.
                    continue;
                }
                // The next segment is long, and would visibly turn if this
                // junction went. Move the junction to where the segment
                // before the short one and the long one meet, and take out
                // the junction before it:
                //
                //                o < previous_previous
                //                |
                //                o < previous
                //                  \  < short segment
                // intersection > +   o-------------------o < next
                //                    ^ current
                let intersection =
                    intersection_infinite(previous_previous.p, previous.p, current.p, next.p)
                        .filter(|&point| {
                            distance_to_infinite_squared(point, previous.p, current.p)
                                <= allowed_error_distance_squared as f64
                                && !dist_greater(point, previous.p, smallest_line_segment_squared)
                                && !dist_greater(point, current.p, smallest_line_segment_squared)
                        });
                // With no good place for it the junction stays where it is.
                if let Some(intersection_point) = intersection {
                    let new_to_add = Junction {
                        p: intersection_point,
                        w: current.w,
                        perimeter_index: current.perimeter_index,
                    };
                    // Never empty: the first junction is in from the start
                    // and each one taken out here is replaced below.
                    if !new_junctions.is_empty() {
                        new_junctions.pop();
                        previous = previous_previous;
                    }
                    accumulated_area_removed = removed_area_next;
                    previous_previous = previous;
                    previous = new_to_add;
                    new_junctions.push(new_to_add);
                    continue;
                }
            }

            // The junction stays. From here the fan starts again, at the
            // triangle of the origin, this junction and the next.
            accumulated_area_removed = removed_area_next;
            previous_previous = previous;
            previous = current;
            new_junctions.push(current);
        }

        if is_closed {
            // The last junction was gone through above and may have moved;
            // the first has to be where it is.
            let last = new_junctions[new_junctions.len() - 1].p;
            new_junctions[0].p = last;
        } else {
            // The last junction is always in the result.
            new_junctions.push(junctions[size - 1]);
        }

        self.junctions = new_junctions;
    }

    /// How much the area of plastic changes, in square units, when `b`
    /// is taken out from between `a` and `c` and the one segment left is
    /// given the average of the two segments' widths, weighted by length.
    ///
    /// As in the reference the sum is in whole numbers: for three points
    /// in a row the result is no more than what the division dropped, and
    /// where the widths differ by a unit or less it is that difference
    /// times the shorter segment.
    pub fn calculate_extrusion_area_deviation_error(a: Junction, b: Junction, c: Junction) -> i64 {
        let ab_length = v_size(b.p, a.p);
        let bc_length = v_size(c.p, b.p);
        let width_diff = (b.w - a.w).abs().max((c.w - b.w).abs());
        if width_diff > 1 {
            // Only where the widths differ, or rounding gives a wrong
            // average.
            let ab_weight = (a.w + b.w) / 2;
            let bc_weight = (b.w + c.w) / 2;
            // The reference divides by zero when all three points are one
            // (`simplify` never asks that). An arm64 build gets 0 from
            // it, an x86-64 one stops; 0 it is here.
            let weighted_average_width = (ab_length * ab_weight + bc_length * bc_weight)
                .checked_div(ab_length + bc_length)
                .unwrap_or(0);
            let ac_length = v_size(c.p, a.p);
            ((ab_weight * ab_length + bc_weight * bc_length) - (weighted_average_width * ac_length))
                .abs()
        } else if ab_length > bc_length {
            width_diff * bc_length
        } else {
            width_diff * ab_length
        }
    }

    /// Whether this is the wall round the outside of a part rather than
    /// round a hole: a closed line that runs clockwise, which is how the
    /// wall generator leaves outer walls. A closed line of no area counts
    /// as counter-clockwise, as in Clipper's `Orientation`.
    pub fn is_contour(&self) -> bool {
        if !self.is_closed {
            return false;
        }
        // ClipperLib::Area, in doubles.
        let mut area = 0.0;
        if self.junctions.len() >= 3 {
            let mut a = 0.0;
            let mut prev = self.junctions[self.junctions.len() - 1].p;
            for junction in &self.junctions {
                let cur = junction.p;
                a += (prev.x as f64 + cur.x as f64) * (prev.y as f64 - cur.y as f64);
                prev = cur;
            }
            area = -a * 0.5;
        }
        // Orientation() is `area >= 0`, is_clockwise() its opposite.
        area < 0.0
    }

    /// The area a closed line goes round, in square units, positive when
    /// it runs counter-clockwise; 0 for fewer than three junctions.
    pub fn area(&self) -> f64 {
        debug_assert!(self.is_closed);
        let mut a = 0.0;
        if self.junctions.len() >= 3 {
            let last = self.junctions[self.junctions.len() - 1].p;
            let mut p1 = (last.x as f64, last.y as f64);
            for junction in &self.junctions {
                let p2 = (junction.p.x as f64, junction.p.y as f64);
                a += p1.0 * p2.1 - p1.1 * p2.0;
                p1 = p2;
            }
        }
        0.5 * a
    }
}

/// A closed line whose first junction is repeated at the end, as a
/// polygon without the repeat (the free `to_polygon` of ExtrusionLine.hpp).
pub fn to_polygon(line: &ExtrusionLine) -> Polygon {
    debug_assert!(line.junctions.len() >= 3);
    debug_assert!(
        line.junctions.first().map(|j| j.p) == line.junctions.last().map(|j| j.p),
        "the first junction has to be repeated at the end"
    );
    let keep = line.junctions.len().saturating_sub(1);
    Polygon {
        points: line.junctions[..keep].iter().map(|j| j.p).collect(),
    }
}

/// Every junction's place, in order (`to_points` of ExtrusionLine.hpp).
pub fn to_points(line: &ExtrusionLine) -> Vec<Point2> {
    line.junctions.iter().map(|j| j.p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A millimetre in units.
    const MM: i64 = 1_000_000;
    /// A usual line width.
    const W: i64 = 400_000;
    /// (0.5 mm)^2, the reference's usual "maximum wall resolution".
    const SMALLEST2: i64 = 500_000 * 500_000;
    /// (0.025 mm)^2, the reference's usual "maximum wall deviation".
    const ALLOWED2: i64 = 25_000 * 25_000;
    /// `meshfix_maximum_extrusion_area_deviation()`: `scaled(2.)`.
    const AREA_DEVIATION: i64 = 2_000_000;

    fn j(x: i64, y: i64, w: i64) -> Junction {
        Junction {
            p: Point2 { x, y },
            w,
            perimeter_index: 0,
        }
    }

    fn line(is_closed: bool, junctions: &[Junction]) -> ExtrusionLine {
        ExtrusionLine {
            inset_idx: 0,
            is_odd: false,
            is_closed,
            junctions: junctions.to_vec(),
        }
    }

    fn points(line: &ExtrusionLine) -> Vec<(i64, i64)> {
        line.junctions.iter().map(|j| (j.p.x, j.p.y)).collect()
    }

    #[test]
    fn constants_are_the_references_truncated_ones() {
        assert_eq!(NEGLIGIBLE_LENGTH_SQUARED, 25_000);
        assert_eq!(COLINEAR_HEIGHT_SQUARED, 1_000);
        assert_eq!(COLINEAR_DISTANCE as i64, 1_000);
        assert_eq!(colinear_vertex_tolerance(), 100);
    }

    #[test]
    fn length_of_an_open_line_sums_truncated_segments() {
        // 3-4-5 triangle legs: 5 mm, then 2 mm straight on.
        let l = line(
            false,
            &[j(0, 0, W), j(3 * MM, 4 * MM, W), j(3 * MM, 6 * MM, W)],
        );
        assert_eq!(l.get_length(), 7 * MM);
        // sqrt(2) = 1.414..., cut to 1; twice.
        let l = line(false, &[j(0, 0, W), j(1, 1, W), j(2, 2, W)]);
        assert_eq!(l.get_length(), 2);
        assert_eq!(line(false, &[]).get_length(), 0);
        assert_eq!(line(false, &[j(5, 5, W)]).get_length(), 0);
    }

    #[test]
    fn length_of_a_closed_line_adds_the_way_back() {
        let square = [
            j(0, 0, W),
            j(10 * MM, 0, W),
            j(10 * MM, 10 * MM, W),
            j(0, 10 * MM, W),
        ];
        assert_eq!(line(false, &square).get_length(), 30 * MM);
        assert_eq!(line(true, &square).get_length(), 40 * MM);
        // With the first junction repeated the way back is nothing.
        let mut repeated = square.to_vec();
        repeated.push(square[0]);
        assert_eq!(line(true, &repeated).get_length(), 40 * MM);
    }

    #[test]
    fn minimal_width_is_the_narrowest_junction() {
        let l = line(
            false,
            &[j(0, 0, 450_000), j(MM, 0, 300_000), j(2 * MM, 0, 420_000)],
        );
        assert_eq!(l.get_minimal_width(), 300_000);
        assert_eq!(line(false, &[]).get_minimal_width(), Coord::MAX);
    }

    #[test]
    fn reverse_turns_the_junctions_round() {
        let mut l = line(false, &[j(0, 0, 1), j(MM, 0, 2), j(2 * MM, 0, 3)]);
        l.reverse();
        assert_eq!(l.junctions, vec![j(2 * MM, 0, 3), j(MM, 0, 2), j(0, 0, 1)]);
    }

    #[test]
    fn counter_clockwise_square_is_a_hole_with_positive_area() {
        let ccw = [
            j(0, 0, W),
            j(10 * MM, 0, W),
            j(10 * MM, 10 * MM, W),
            j(0, 10 * MM, W),
        ];
        let l = line(true, &ccw);
        assert_eq!(l.area(), 100.0 * (MM as f64) * (MM as f64));
        assert!(!l.is_contour());

        let mut cw = l.clone();
        cw.reverse();
        assert_eq!(cw.area(), -100.0 * (MM as f64) * (MM as f64));
        assert!(cw.is_contour());

        // The repeated first junction adds nothing.
        let mut repeated = ccw.to_vec();
        repeated.push(ccw[0]);
        assert_eq!(line(true, &repeated).area(), l.area());

        // An open line is no contour whichever way it runs.
        let mut open = cw.clone();
        open.is_closed = false;
        assert!(!open.is_contour());
        // Fewer than three junctions: no area, and not clockwise.
        let two = line(true, &[j(0, 0, W), j(MM, 0, W)]);
        assert_eq!(two.area(), 0.0);
        assert!(!two.is_contour());
    }

    #[test]
    fn polygon_and_points_conversions() {
        let closed = line(true, &[j(0, 0, W), j(MM, 0, W), j(MM, MM, W), j(0, 0, W)]);
        let all = vec![
            Point2 { x: 0, y: 0 },
            Point2 { x: MM, y: 0 },
            Point2 { x: MM, y: MM },
            Point2 { x: 0, y: 0 },
        ];
        assert_eq!(to_points(&closed), all);
        assert_eq!(closed.to_polygon().points, all);
        assert_eq!(to_polygon(&closed).points, all[..3]);
    }

    #[test]
    fn area_deviation_error_by_hand() {
        // Equal widths: nothing changes.
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, W),
                j(10 * MM, 0, W),
                j(20 * MM, 0, W)
            ),
            0
        );
        // Widths one unit apart: that unit times the shorter segment.
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, W),
                j(10 * MM, 0, W + 1),
                j(13 * MM, 0, W)
            ),
            3 * MM
        );
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, W),
                j(3 * MM, 0, W + 1),
                j(13 * MM, 0, W)
            ),
            3 * MM
        );
        // 10 mm at (400000 + 400000) / 2 and 5 mm at
        // (400000 + 600001) / 2 = 500000: 4e12 + 2.5e12 = 6.5e12 over
        // 15 mm is 433333 (cut down), and 433333 * 15e6 = 6.499995e12.
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, 400_000),
                j(10 * MM, 0, 400_000),
                j(15 * MM, 0, 600_001)
            ),
            5_000_000
        );
        // Off a straight line the one segment left is shorter than the
        // two: 3 mm and 4 mm at 500000 and 500000 make 3.5e12, the
        // average width is 500000 and the 5 mm left make 2.5e12.
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, 400_000),
                j(3 * MM, 0, 600_000),
                j(3 * MM, 4 * MM, 400_000)
            ),
            1_000_000_000_000
        );
        // All three in one place: no area, and no division by zero.
        assert_eq!(
            ExtrusionLine::calculate_extrusion_area_deviation_error(
                j(0, 0, 400_000),
                j(0, 0, 600_000),
                j(0, 0, 400_000)
            ),
            0
        );
    }

    #[test]
    fn simplify_drops_a_point_on_a_straight_line() {
        let mut l = line(false, &[j(0, 0, W), j(10 * MM, 0, W), j(20 * MM, 0, W)]);
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(points(&l), vec![(0, 0), (20 * MM, 0)]);
    }

    #[test]
    fn simplify_leaves_two_points_and_short_closed_lines_alone() {
        let mut open = line(false, &[j(0, 0, W), j(10, 0, W)]);
        let before = open.clone();
        open.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(open, before);

        let mut closed = line(true, &[j(0, 0, W), j(10, 0, W), j(0, 0, W)]);
        let before = closed.clone();
        closed.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(closed, before);
    }

    #[test]
    fn simplify_keeps_a_width_change_over_the_area_limit() {
        // The error worked out in `area_deviation_error_by_hand`: 5e6.
        let junctions = [
            j(0, 0, 400_000),
            j(10 * MM, 0, 400_000),
            j(15 * MM, 0, 600_001),
        ];
        let mut kept = line(false, &junctions);
        kept.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(kept.junctions, junctions.to_vec());

        // At the limit it goes.
        let mut dropped = line(false, &junctions);
        dropped.simplify(SMALLEST2, ALLOWED2, 5_000_000);
        assert_eq!(dropped.junctions, vec![junctions[0], junctions[2]]);

        // One unit of width over 10 mm is 1e7 by the reference's sum,
        // over the limit too.
        let junctions = [j(0, 0, W), j(10 * MM, 0, W + 1), j(20 * MM, 0, W)];
        let mut kept = line(false, &junctions);
        kept.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(kept.junctions, junctions.to_vec());
    }

    #[test]
    fn simplify_merges_short_segments() {
        // A 0.01 mm bump in the middle of short segments. At (100000,
        // 10000): twice the area cut off is -2e9, over a 200000 base
        // that is a height of 10000, squared 1e8 <= ALLOWED2; the next
        // segment is short too, so the point goes. At (200000, 0): the
        // same area over the 300000 base from the start, 4e18 / 9e10 =
        // 44444444 <= ALLOWED2, and it goes as well.
        let mut l = line(
            false,
            &[
                j(0, 0, W),
                j(100_000, 10_000, W),
                j(200_000, 0, W),
                j(300_000, 0, W),
            ],
        );
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(points(&l), vec![(0, 0), (300_000, 0)]);

        // With no deviation allowed both stay.
        let mut l = line(
            false,
            &[
                j(0, 0, W),
                j(100_000, 10_000, W),
                j(200_000, 0, W),
                j(300_000, 0, W),
            ],
        );
        l.simplify(SMALLEST2, 0, AREA_DEVIATION);
        assert_eq!(l.junctions.len(), 4);
    }

    #[test]
    fn simplify_makes_a_corner_of_a_short_segment_between_long_ones() {
        // Down the y axis, a 0.02 mm cut across the corner, along the x
        // axis. At (20000, 0) the segment before is short (8e8 <
        // SMALLEST2), the height is about 19960 (squared 398401598 <=
        // ALLOWED2) and the next segment is long, so the junction moves
        // to where the two long segments meet, the origin (14142 from
        // the cut, within the allowed 25000), and (0, 20000) goes.
        let mut l = line(
            false,
            &[
                j(0, 10 * MM, W),
                j(0, 20_000, W),
                j(20_000, 0, 300_000),
                j(10 * MM, 0, W),
            ],
        );
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(
            l.junctions,
            // The moved junction keeps the width of the one it replaces.
            vec![j(0, 10 * MM, W), j(0, 0, 300_000), j(10 * MM, 0, W)]
        );

        // A 0.1 mm cut is too far from the corner: it stays as it is.
        let cut = [
            j(0, 10 * MM, W),
            j(0, 100_000, W),
            j(100_000, 0, W),
            j(10 * MM, 0, W),
        ];
        let mut l = line(false, &cut);
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(l.junctions, cut.to_vec());
    }

    #[test]
    fn simplify_closed_square_drops_the_point_on_a_side() {
        let mut l = line(
            true,
            &[
                j(0, 0, W),
                j(5 * MM, 0, W),
                j(10 * MM, 0, W),
                j(10 * MM, 10 * MM, W),
                j(0, 10 * MM, W),
                j(0, 0, W),
            ],
        );
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(
            points(&l),
            vec![
                (0, 0),
                (10 * MM, 0),
                (10 * MM, 10 * MM),
                (0, 10 * MM),
                (0, 0)
            ]
        );
    }

    #[test]
    fn simplify_keeps_three_junctions_of_a_closed_line() {
        // Every segment is under the 158 units that always go. The first
        // point after the start is taken out; then only three would be
        // left, so the rest stay.
        let mut l = line(
            true,
            &[j(0, 0, W), j(100, 0, W), j(100, 100, W), j(0, 0, W)],
        );
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(points(&l), vec![(0, 0), (100, 100), (0, 0)]);

        // The same as an open line loses both middle points.
        let mut l = line(
            false,
            &[j(0, 0, W), j(100, 0, W), j(100, 100, W), j(0, 0, W)],
        );
        l.simplify(SMALLEST2, ALLOWED2, AREA_DEVIATION);
        assert_eq!(points(&l), vec![(0, 0), (0, 0)]);
    }
}
