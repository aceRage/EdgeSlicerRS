//! Which way lines are best laid across a gap, and how much of the gap
//! lines laid that way would span from one support to another: a port of
//! the reference's `BridgeDetector` (BridgeDetector.cpp), as far as
//! `detect_angle` and `coverage`. The reference walls a hole that
//! narrows by it (PerimeterGenerator.cpp `process_no_bridge`); the fill
//! turns its bridges another way (`edge-infill`).
//!
//! The arithmetic is the reference's, rounding and all, so that the same
//! outline gives the same direction. Two things are settled here that
//! the reference leaves open. Directions that span equally much are
//! taken in the order of their angles, where the reference sorts them
//! with `std::sort`, which may leave equals in any order. And the
//! clipper is not the reference's, so an outline comes with its points a
//! nanometre off: on a shape that is the same in many directions, a
//! round hole in a round one, the direction is as much a matter of
//! chance there as here, and need not be the same.

use std::f64::consts::PI;

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

use crate::SAFETY_OFFSET;

/// Directions are tried every five degrees (`resolution`).
const STEP: f64 = PI / 36.0;
/// Directions nearer one another than a degree are one direction.
const SAME_DIRECTION: f64 = PI / 180.0;
/// `EPSILON` (libslic3r.h), which angles are compared by.
const EPSILON: f64 = 1e-4;

pub(crate) struct BridgeDetector<'a> {
    /// The gap, not grown.
    area: &'a ExPolygon,
    /// How far apart the lines tried are, and how far beyond the gap
    /// what holds a line up is looked for.
    spacing: Coord,
    /// The parts of the grown gap's outline that lie within the outlines
    /// of what is beneath, holes not counted (`_edges`).
    edges: Vec<Vec<Point2>>,
    /// What the gap, grown, has beneath it (`_anchor_regions`).
    anchors: Vec<ExPolygon>,
}

impl<'a> BridgeDetector<'a> {
    /// `lower` is what there is to rest on; `spacing` the distance
    /// between the lines tried (`BridgeDetector::initialize`).
    pub(crate) fn new(
        area: &'a ExPolygon,
        lower: &[ExPolygon],
        spacing: Coord,
    ) -> Result<Self, ClipError> {
        let own = std::slice::from_ref(area);
        let grown = edge_clip::offset(own, spacing, Join::MITER)?;
        // "Detect what edges lie on lower slices by turning bridge
        // contour and holes into polylines and then clipping them with
        // each lower slice's contour."
        let contours: Vec<ExPolygon> = lower
            .iter()
            .map(|piece| ExPolygon {
                contour: piece.contour.clone(),
                holes: Vec::new(),
            })
            .collect();
        let outlines: Vec<Vec<Point2>> = grown
            .iter()
            .flat_map(ExPolygon::rings)
            .filter(|ring| !ring.points.is_empty())
            .map(|ring| {
                let mut closed = ring.points.clone();
                closed.push(ring.points[0]);
                closed
            })
            .collect();
        let edges = edge_clip::clip_lines(&outlines, &contours)?;
        let anchors = edge_clip::intersection(
            &grown,
            &edge_clip::offset(lower, SAFETY_OFFSET, Join::MITER)?,
        )?;
        Ok(Self {
            area,
            spacing,
            edges,
            anchors,
        })
    }

    /// The direction, in radians counter-clockwise from +x and under a
    /// half turn, along which the most line is held at both ends; of
    /// those that hold within one spacing of the most, the one whose
    /// longest line is shortest. With `fixed` given only that direction
    /// is tried. None where nothing holds a line at both ends
    /// (`BridgeDetector::detect_angle`).
    pub(crate) fn detect_angle(&self, fixed: Option<f64>) -> Result<Option<f64>, ClipError> {
        if self.edges.is_empty() || self.anchors.is_empty() || self.spacing <= 0 {
            return Ok(None);
        }
        let angles = match fixed {
            Some(angle) => vec![angle],
            None => self.candidates(),
        };
        // "Outset the bridge expolygon by half the amount we used for
        // detecting anchors; we'll use this one to clip our test lines
        // and be sure that their endpoints are inside the anchors and
        // not on their contours leading to false negatives."
        let clip_area = edge_clip::offset(
            std::slice::from_ref(self.area),
            self.spacing / 2,
            Join::MITER,
        )?;
        // (angle, length held, longest line held)
        let mut tried: Vec<(f64, f64, f64)> = Vec::with_capacity(angles.len());
        let mut any = false;
        for angle in angles {
            let (s, c) = angle.sin_cos();
            let Some((low, high)) = extents_rotated(&self.anchors, -angle) else {
                continue;
            };
            let at = |x: Coord, y: Coord| Point2 {
                x: (c * x as f64 - s * y as f64).round() as Coord,
                y: (c * y as f64 + s * x as f64).round() as Coord,
            };
            let mut lines = Vec::new();
            let mut y = low.y;
            while y <= high.y {
                lines.push(vec![at(low.x, y), at(high.x, y)]);
                y += self.spacing;
            }
            let (mut total, mut longest) = (0.0f64, 0.0f64);
            for piece in edge_clip::clip_lines(&lines, &clip_area)? {
                let (Some(&a), Some(&b)) = (piece.first(), piece.last()) else {
                    continue;
                };
                if contains(&self.anchors, a) && contains(&self.anchors, b) {
                    let length = ((b.x - a.x) as f64).hypot((b.y - a.y) as f64);
                    total += length;
                    longest = longest.max(length);
                }
            }
            any |= total != 0.0;
            tried.push((angle, total, longest));
        }
        if !any {
            return Ok(None);
        }
        // Most held first; equals stay in the order of their angles.
        tried.sort_by(|a, b| b.1.total_cmp(&a.1));
        // "if any other direction is within extrusion width of coverage,
        // prefer it if shorter"
        let mut best = 0;
        let mut other = 1;
        while other < tried.len() && tried[best].1 - tried[other].1 < self.spacing as f64 {
            if tried[other].2 < tried[best].2 {
                best = other;
            }
            other += 1;
        }
        let mut angle = tried[best].0;
        if angle >= PI {
            angle -= PI;
        }
        Ok(Some(angle))
    }

    /// The directions worth trying: every five degrees, along each edge
    /// of the gap, and from end to end of each supporting edge that is
    /// not a closed one (`bridge_direction_candidates`).
    fn candidates(&self) -> Vec<f64> {
        let mut angles = Vec::new();
        let mut step = 0;
        while step as f64 <= PI / STEP {
            angles.push(step as f64 * STEP);
            step += 1;
        }
        for ring in self.area.rings() {
            let points = &ring.points;
            for (index, a) in points.iter().enumerate() {
                angles.push(direction(*a, points[(index + 1) % points.len()]));
            }
        }
        for edge in &self.edges {
            if let (Some(&first), Some(&last)) = (edge.first(), edge.last())
                && first != last
            {
                angles.push(direction(first, last));
            }
        }
        angles.sort_by(f64::total_cmp);
        let mut index = 1;
        while index < angles.len() {
            if parallel(angles[index], angles[index - 1]) {
                angles.remove(index);
            } else {
                index += 1;
            }
        }
        // "compare first value with last one and remove the greatest one
        // (PI) in case they are parallel (PI, 0)"
        if let (Some(&first), Some(&last)) = (angles.first(), angles.last())
            && parallel(first, last)
        {
            angles.pop();
        }
        angles
    }

    /// The part of the gap, a little grown, that lines along `angle`
    /// would span from one support to another: the gap is cut into
    /// strips along the angle, one spacing wide, and a strip counts when
    /// it rests on support in two places (`BridgeDetector::coverage`
    /// with `precise` set, the way the reference calls it here).
    pub(crate) fn coverage(&self, angle: f64) -> Result<Vec<ExPolygon>, ClipError> {
        // Turned so that the lines run straight up.
        let turn = PI / 2.0 - angle;
        let anchors: Vec<ExPolygon> = self
            .anchors
            .iter()
            .map(|piece| rotated_region(piece, turn))
            .collect();
        let quarter = self.spacing / 4;
        let mut covered: Vec<Polygon> = Vec::new();
        let turned = rotated_region(self.area, turn);
        for piece in edge_clip::offset(&[turned], self.spacing / 2, Join::MITER)? {
            for mut strip in strips(&piece, self.spacing)? {
                let held = edge_clip::intersection(
                    &[ExPolygon {
                        contour: strip.clone(),
                        holes: Vec::new(),
                    }],
                    &anchors,
                )?;
                let rings: Vec<&Polygon> = held.iter().flat_map(ExPolygon::rings).collect();
                if rings.len() < 2 {
                    continue;
                }
                // "trim it to not allow to go outside of the
                // intersections": from the middle of the lowest place it
                // rests on to the middle of the highest.
                let middles = rings.iter().filter_map(|ring| {
                    let low = ring.points.iter().map(|p| p.y).min()?;
                    let high = ring.points.iter().map(|p| p.y).max()?;
                    Some((low + high) / 2)
                });
                let (Some(min_y), Some(max_y)) = (middles.clone().min(), middles.max()) else {
                    continue;
                };
                let xs = strip.points.iter().map(|p| p.x);
                let (Some(mut min_x), Some(mut max_x)) = (xs.clone().min(), xs.max()) else {
                    continue;
                };
                // "add what get_trapezoids3 has removed (+EPSILON)"
                min_x -= quarter + 1;
                max_x += quarter + 1;
                let mid_x = (min_x + max_x) / 2;
                for point in &mut strip.points {
                    point.y = point.y.clamp(min_y, max_y);
                    if point.x > min_x && point.x < mid_x {
                        point.x = min_x;
                    }
                    if point.x < max_x && point.x > mid_x {
                        point.x = max_x;
                    }
                }
                covered.push(strip);
            }
        }
        // "Unite the trapezoids before rotation, as the rotation creates
        // tiny gaps and intersections between the trapezoids instead of
        // exact overlaps."
        let back: Vec<Polygon> = edge_clip::union(&covered)?
            .iter()
            .flat_map(ExPolygon::rings)
            .map(|ring| rotated(ring, -turn))
            .collect();
        edge_clip::union(&back)
    }
}

/// A region cut into upright strips `spacing` wide, each a quarter of a
/// spacing short of its neighbours on either side
/// (`get_trapezoids3_half`).
fn strips(region: &ExPolygon, spacing: Coord) -> Result<Vec<Polygon>, ClipError> {
    let points = || region.rings().flat_map(|ring| ring.points.iter());
    let (Some(min_x), Some(max_x), Some(min_y), Some(max_y)) = (
        points().map(|p| p.x).min(),
        points().map(|p| p.x).max(),
        points().map(|p| p.y).min(),
        points().map(|p| p.y).max(),
    ) else {
        return Ok(Vec::new());
    };
    if spacing <= 0 {
        return Ok(Vec::new());
    }
    let mut xs = Vec::new();
    let mut x = min_x;
    while x < max_x - spacing / 2 {
        xs.push(x);
        x += spacing;
    }
    xs.push(max_x);
    let quarter = spacing / 4;
    let mut out = Vec::new();
    for pair in xs.windows(2) {
        if pair[0] == pair[1] {
            continue;
        }
        let (left, right) = (pair[0] + quarter, pair[1] - quarter);
        let strip = ExPolygon {
            contour: Polygon {
                points: vec![
                    Point2 { x: left, y: min_y },
                    Point2 { x: right, y: min_y },
                    Point2 { x: right, y: max_y },
                    Point2 { x: left, y: max_y },
                ],
            },
            holes: Vec::new(),
        };
        out.extend(
            edge_clip::intersection(&[strip], std::slice::from_ref(region))?
                .into_iter()
                .flat_map(|piece| std::iter::once(piece.contour).chain(piece.holes)),
        );
    }
    Ok(out)
}

/// A point turned about the origin, rounded as `Point::rotate` rounds.
fn turned(point: Point2, sin: f64, cos: f64) -> Point2 {
    let (x, y) = (point.x as f64, point.y as f64);
    Point2 {
        x: (cos * x - sin * y).round() as Coord,
        y: (cos * y + sin * x).round() as Coord,
    }
}

fn rotated(ring: &Polygon, angle: f64) -> Polygon {
    let (sin, cos) = angle.sin_cos();
    Polygon {
        points: ring.points.iter().map(|p| turned(*p, sin, cos)).collect(),
    }
}

fn rotated_region(region: &ExPolygon, angle: f64) -> ExPolygon {
    ExPolygon {
        contour: rotated(&region.contour, angle),
        holes: region.holes.iter().map(|h| rotated(h, angle)).collect(),
    }
}

/// The least and greatest corners of the regions' outlines once turned
/// by `angle` (`get_extents_rotated`, which goes by contours alone).
fn extents_rotated(regions: &[ExPolygon], angle: f64) -> Option<(Point2, Point2)> {
    let (sin, cos) = angle.sin_cos();
    let mut bounds: Option<(Point2, Point2)> = None;
    for point in regions.iter().flat_map(|region| &region.contour.points) {
        let p = turned(*point, sin, cos);
        let (low, high) = bounds.unwrap_or((p, p));
        bounds = Some((
            Point2 {
                x: low.x.min(p.x),
                y: low.y.min(p.y),
            },
            Point2 {
                x: high.x.max(p.x),
                y: high.y.max(p.y),
            },
        ));
    }
    bounds
}

/// The direction of a line from `a` to `b`, from 0 to under a half turn
/// (`Line::direction`).
fn direction(a: Point2, b: Point2) -> f64 {
    let angle = ((b.y - a.y) as f64).atan2((b.x - a.x) as f64);
    if (angle - PI).abs() < EPSILON {
        0.0
    } else if angle < 0.0 {
        angle + PI
    } else {
        angle
    }
}

/// `Geometry::directions_parallel` with a degree between them allowed.
fn parallel(one: f64, two: f64) -> bool {
    let difference = (one - two).abs();
    let most = SAME_DIRECTION + EPSILON;
    difference < most || (difference - PI).abs() < most
}

/// Whether a point is in any of the regions, their edges counted as
/// inside (`expolygons_contain`).
pub(crate) fn contains(regions: &[ExPolygon], point: Point2) -> bool {
    regions.iter().any(|region| {
        // Inside the outline or on it, and not strictly inside a hole.
        !matches!(side(&region.contour, point), Side::Outside)
            && region
                .holes
                .iter()
                .all(|hole| !matches!(side(hole, point), Side::Inside))
    })
}

enum Side {
    Inside,
    Edge,
    Outside,
}

/// Where a point lies by a ring, whichever way the ring is wound.
fn side(ring: &Polygon, point: Point2) -> Side {
    let points = &ring.points;
    let mut inside = false;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        let cross = i128::from(b.x - a.x) * i128::from(point.y - a.y)
            - i128::from(b.y - a.y) * i128::from(point.x - a.x);
        if cross == 0
            && point.x >= a.x.min(b.x)
            && point.x <= a.x.max(b.x)
            && point.y >= a.y.min(b.y)
            && point.y <= a.y.max(b.y)
        {
            return Side::Edge;
        }
        // A ray toward +x crosses the edge when the edge spans the
        // point's height and passes to its right.
        if (a.y > point.y) != (b.y > point.y) {
            let right = if b.y > a.y { cross > 0 } else { cross < 0 };
            if right {
                inside = !inside;
            }
        }
    }
    if inside { Side::Inside } else { Side::Outside }
}

#[cfg(test)]
mod tests {
    use edge_geom::scaled;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn solid(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: rect(x0, y0, x1, y1),
            holes: Vec::new(),
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn a_gap_between_two_banks_is_crossed_from_bank_to_bank() {
        // A gap 10 mm wide in x and 30 mm long, with a bank down each
        // long side and nothing at its ends.
        let gap = solid(0.0, 0.0, 10.0, 30.0);
        let banks = [solid(-5.0, 0.0, 0.0, 30.0), solid(10.0, 0.0, 15.0, 30.0)];
        let detector = BridgeDetector::new(&gap, &banks, scaled(0.4)).unwrap();
        let angle = detector.detect_angle(None).unwrap().unwrap();
        assert!(angle.abs() < 1e-9, "{angle}");
        // All of it is spanned that way, and a little of each bank.
        let covered = detector.coverage(angle).unwrap();
        assert!(
            edge_clip::difference(std::slice::from_ref(&gap), &covered)
                .unwrap()
                .is_empty()
        );
        assert!(area(&covered) < 10.5 * 30.5, "{}", area(&covered));
        // Turned a quarter, with the banks at the ends of the lines.
        let gap = solid(0.0, 0.0, 30.0, 10.0);
        let banks = [solid(0.0, -5.0, 30.0, 0.0), solid(0.0, 10.0, 30.0, 15.0)];
        let detector = BridgeDetector::new(&gap, &banks, scaled(0.4)).unwrap();
        let angle = detector.detect_angle(None).unwrap().unwrap();
        assert!((angle - PI / 2.0).abs() < 1e-9, "{angle}");
    }

    #[test]
    fn nothing_is_spanned_from_one_bank_alone() {
        let gap = solid(0.0, 0.0, 10.0, 30.0);
        let bank = [solid(-5.0, 0.0, 0.0, 30.0)];
        let detector = BridgeDetector::new(&gap, &bank, scaled(0.4)).unwrap();
        // The lines tried along the bank's edge lie on the bank from end
        // to end, so a direction is found, as in the reference; but no
        // strip along it rests on the bank in two places.
        let angle = detector.detect_angle(None).unwrap().unwrap();
        assert!((angle - PI / 2.0).abs() < 1e-9, "{angle}");
        assert!(detector.coverage(angle).unwrap().is_empty());
        // Nor does one across.
        assert!(detector.coverage(0.0).unwrap().is_empty());
        // And with nothing beneath at all there is nothing to try.
        let detector = BridgeDetector::new(&gap, &[], scaled(0.4)).unwrap();
        assert_eq!(detector.detect_angle(None).unwrap(), None);
    }

    #[test]
    fn strips_that_meet_a_hole_in_the_gap_are_not_spanned() {
        // A square gap with a bank all round it and a square hole in its
        // middle, crossed along y: the strips either side of the hole
        // reach from bank to bank, those through it do not.
        let mut hole = rect(4.0, 4.0, 8.0, 8.0);
        hole.points.reverse();
        let gap = ExPolygon {
            contour: rect(0.0, 0.0, 12.0, 12.0),
            holes: vec![hole],
        };
        let mut pit = rect(0.0, 0.0, 12.0, 12.0);
        pit.points.reverse();
        let banks = [ExPolygon {
            contour: rect(-5.0, -5.0, 17.0, 17.0),
            holes: vec![pit],
        }];
        let detector = BridgeDetector::new(&gap, &banks, scaled(0.1)).unwrap();
        let covered = detector.coverage(PI / 2.0).unwrap();
        assert_eq!(covered.len(), 2, "{covered:?}");
        // Each about 4 mm of the 12, the whole length.
        let each = area(&covered) / 2.0;
        assert!((each - 4.0 * 12.0).abs() < 2.0, "{each}");
        let through = solid(5.0, 0.0, 7.0, 12.0);
        assert!(
            edge_clip::intersection(&covered, &[through])
                .unwrap()
                .is_empty()
        );
        // A fixed direction is the only one tried.
        let fixed = detector.detect_angle(Some(PI / 2.0)).unwrap().unwrap();
        assert!((fixed - PI / 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_point_on_an_edge_counts_as_inside() {
        let mut hole = rect(4.0, 4.0, 8.0, 8.0);
        hole.points.reverse();
        let region = [ExPolygon {
            contour: rect(0.0, 0.0, 12.0, 12.0),
            holes: vec![hole],
        }];
        assert!(contains(&region, Point2::from_mm(1.0, 1.0)));
        assert!(contains(&region, Point2::from_mm(0.0, 6.0)));
        assert!(contains(&region, Point2::from_mm(12.0, 12.0)));
        assert!(contains(&region, Point2::from_mm(4.0, 6.0)));
        assert!(!contains(&region, Point2::from_mm(6.0, 6.0)));
        assert!(!contains(&region, Point2::from_mm(13.0, 6.0)));
        assert!(!contains(&region, Point2::from_mm(-1.0, 0.0)));
    }
}
