//! Cutting a wall where it crosses the edge of a region, so that the part
//! inside can be treated apart from the rest.
//!
//! This does the job of the reference's Algorithm/LineSplit.cpp
//! (`split_line`, `do_split_line`) and keeps its result: the same
//! junctions with the same meaning. How it gets there is different on
//! purpose. The reference intersects the path with the region in a Clipper
//! that carries each point's index through as a Z coordinate, then works
//! out from those indices where each clipped piece came from and stitches
//! the pieces back into the path. Here the path is walked once and each
//! of its segments is cut where it crosses the region's boundary, with
//! whole-number arithmetic; nothing has to be found again afterwards, and
//! the clipping library is not involved.

use edge_geom::{Coord, ExPolygon, Point2};

/// One point of a split path: the reference's `SplitLineJunction`
/// (Algorithm/LineSplit.hpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitLineJunction {
    pub p: Point2,
    /// Whether the piece from this point to the next one is inside the
    /// region (or on its edge).
    pub clipped: bool,
    /// Where the point comes from: the index of a point of the path that
    /// was split, or, for a new point where the path crosses the region's
    /// edge, `-(1 + i)` with `i` the index of the path point its segment
    /// starts from.
    pub src_idx: i64,
}

impl SplitLineJunction {
    /// Whether this is a point of the path that was split, not a new one.
    pub fn is_src(&self) -> bool {
        self.src_idx >= 0
    }

    /// The index of the path point this is, or of the one its segment
    /// starts from.
    pub fn get_src_index(&self) -> usize {
        if self.is_src() {
            self.src_idx as usize
        } else {
            (-self.src_idx - 1) as usize
        }
    }
}

/// The cross product (b - a) x (c - a): positive when c is to the left of
/// the line from a to b. Products of two coordinates overflow i64.
fn orient(a: Point2, b: Point2, c: Point2) -> i128 {
    i128::from(b.x - a.x) * i128::from(c.y - a.y) - i128::from(b.y - a.y) * i128::from(c.x - a.x)
}

/// Whether `p` is within the box two points span, edges included.
fn in_box(p: Point2, a: Point2, b: Point2) -> bool {
    a.x.min(b.x) <= p.x && p.x <= a.x.max(b.x) && a.y.min(b.y) <= p.y && p.y <= a.y.max(b.y)
}

/// `numerator / denominator`, rounded to the nearest whole number, halves
/// away from zero. The denominator is not zero.
fn div_round(numerator: i128, denominator: i128) -> i128 {
    let (numerator, denominator) = if denominator < 0 {
        (-numerator, -denominator)
    } else {
        (numerator, denominator)
    };
    let half = denominator / 2;
    if numerator >= 0 {
        (numerator + half) / denominator
    } else {
        -((-numerator + half) / denominator)
    }
}

/// The edges of a region, sorted into horizontal strips so that a segment
/// or a point is only held against the edges near it.
struct Boundary {
    /// Every edge of every ring, each from one point to the next.
    edges: Vec<(Point2, Point2)>,
    /// For each strip, the edges that reach into it.
    strips: Vec<Vec<u32>>,
    min_y: Coord,
    max_y: Coord,
    strip_height: Coord,
}

impl Boundary {
    fn new(region: &[ExPolygon]) -> Self {
        Self::with_strips(region, None)
    }

    /// `strips` sets how many strips there are; `None` picks a number
    /// that suits the region. The result of a split does not depend on
    /// it, which the tests check.
    fn with_strips(region: &[ExPolygon], strips: Option<usize>) -> Self {
        let mut edges = Vec::new();
        for ring in region.iter().flat_map(ExPolygon::rings) {
            let points = &ring.points;
            // Two points enclose nothing.
            if points.len() < 3 {
                continue;
            }
            for (i, &c) in points.iter().enumerate() {
                let d = points[(i + 1) % points.len()];
                if c != d {
                    edges.push((c, d));
                }
            }
        }
        let min_y = edges.iter().map(|&(c, d)| c.y.min(d.y)).min().unwrap_or(0);
        let max_y = edges.iter().map(|&(c, d)| c.y.max(d.y)).max().unwrap_or(0);
        // About as many strips as edges in a strip.
        let strip_count = strips
            .unwrap_or((edges.len() as f64).sqrt() as usize)
            .clamp(1, 4096);
        let strip_height = ((max_y - min_y) / strip_count as Coord + 1).max(1);
        let mut boundary = Self {
            edges,
            strips: vec![Vec::new(); strip_count],
            min_y,
            max_y,
            strip_height,
        };
        for index in 0..boundary.edges.len() {
            let (c, d) = boundary.edges[index];
            for strip in boundary.strip_of(c.y.min(d.y))..=boundary.strip_of(c.y.max(d.y)) {
                boundary.strips[strip].push(index as u32);
            }
        }
        boundary
    }

    /// The strip a height falls in; heights beyond the region go to the
    /// first or last.
    fn strip_of(&self, y: Coord) -> usize {
        let strip = (y.clamp(self.min_y, self.max_y) - self.min_y) / self.strip_height;
        (strip as usize).min(self.strips.len() - 1)
    }

    /// Calls `visit` once for each edge whose box overlaps that of the
    /// segment from `a` to `b`.
    fn edges_near(&self, a: Point2, b: Point2, mut visit: impl FnMut(Point2, Point2)) {
        let (low, high) = (a.y.min(b.y), a.y.max(b.y));
        if high < self.min_y || low > self.max_y {
            return;
        }
        let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
        let first = self.strip_of(low);
        for strip in first..=self.strip_of(high) {
            for &index in &self.strips[strip] {
                let (c, d) = self.edges[index as usize];
                // An edge spanning several strips is taken in the first
                // of them that the segment reaches.
                if self.strip_of(c.y.min(d.y)).max(first) != strip {
                    continue;
                }
                if c.x.max(d.x) < min_x
                    || c.x.min(d.x) > max_x
                    || c.y.max(d.y) < low
                    || c.y.min(d.y) > high
                {
                    continue;
                }
                visit(c, d);
            }
        }
    }

    /// Whether the middle of the piece from `a` to `b` is in the region:
    /// on one of its edges, or with a winding number other than zero
    /// around it (non-zero fill, so a counter-clockwise contour adds and
    /// the clockwise hole in it takes away). Exact: the test is made at
    /// twice the scale, where the middle `a + b` is a whole point.
    fn covers_middle(&self, a: Point2, b: Point2) -> bool {
        let (mx, my) = (a.x + b.x, a.y + b.y);
        let floor_y = my.div_euclid(2);
        if floor_y < self.min_y || floor_y > self.max_y {
            return false;
        }
        let mut winding = 0;
        // Every edge that passes the middle's height reaches into the
        // strip of that height, rounded down.
        for &index in &self.strips[self.strip_of(floor_y)] {
            let (c, d) = self.edges[index as usize];
            let (cx, cy, dx, dy) = (2 * c.x, 2 * c.y, 2 * d.x, 2 * d.y);
            let cross = i128::from(dx - cx) * i128::from(my - cy)
                - i128::from(dy - cy) * i128::from(mx - cx);
            if cross == 0
                && cx.min(dx) <= mx
                && mx <= cx.max(dx)
                && cy.min(dy) <= my
                && my <= cy.max(dy)
            {
                return true;
            }
            if cy <= my {
                if dy > my && cross > 0 {
                    winding += 1;
                }
            } else if dy <= my && cross < 0 {
                winding -= 1;
            }
        }
        winding != 0
    }
}

/// Where the segment from `a` to `b` may change sides of the boundary:
/// the points, strictly between its ends and in order from `a`, at which
/// it meets an edge. The flag says whether it meets one anywhere, its
/// ends included.
fn cuts_of_segment(boundary: &Boundary, a: Point2, b: Point2) -> (Vec<Point2>, bool) {
    let mut cuts: Vec<Point2> = Vec::new();
    let mut touches = false;
    boundary.edges_near(a, b, |c, d| {
        let (d1, d2) = (orient(a, b, c), orient(a, b, d));
        if d1 == 0 && d2 == 0 {
            // The edge lies along the segment's line (or the segment is a
            // single point). Where they overlap, the overlap's ends are
            // where the segment may come off the boundary.
            touches = true;
            for end in [c, d] {
                if in_box(end, a, b) {
                    cuts.push(end);
                }
            }
            return;
        }
        let (d3, d4) = (orient(c, d, a), orient(c, d, b));
        // The ends of each are on either side of the other, or on it.
        if d1.signum() * d2.signum() > 0 || d3.signum() * d4.signum() > 0 {
            return;
        }
        touches = true;
        if d1 == 0 {
            cuts.push(c);
        } else if d2 == 0 {
            cuts.push(d);
        } else if d3 != 0 && d4 != 0 {
            // A crossing inside both. `d3` and `d4` measure how far `a`
            // and `b` are from the edge's line, on opposite sides, so the
            // crossing is the share d3 / (d3 - d4) of the way along.
            // Rounded to whole units.
            let along = d3 - d4;
            cuts.push(Point2 {
                x: a.x + div_round(i128::from(b.x - a.x) * d3, along) as Coord,
                y: a.y + div_round(i128::from(b.y - a.y) * d3, along) as Coord,
            });
        }
        // Otherwise the edge passes through an end of the segment, which
        // is a point of the path already.
    });

    // In order along the segment. A cut rounded onto an end, or past
    // one, is dropped: the end stands for it.
    let position = |p: Point2| -> i128 {
        i128::from(p.x - a.x) * i128::from(b.x - a.x)
            + i128::from(p.y - a.y) * i128::from(b.y - a.y)
    };
    let length = position(b);
    cuts.retain(|&p| {
        let at = position(p);
        0 < at && at < length
    });
    cuts.sort_unstable_by_key(|&p| (position(p), p.x, p.y));
    cuts.dedup();
    (cuts, touches)
}

/// Splits a path along the edge of a region: the reference's `split_line`
/// (Algorithm/LineSplit.hpp).
///
/// `path` is a polygon when `closed` (it is walked round, the segment
/// from its last point back to its first included) and an open line
/// otherwise, walked from its first point to its last. A wall of changing
/// width is passed as its junctions' points and as open, whether or not
/// it ends where it began, as the reference passes it. `clip` is the
/// region, filled where the winding number of its contours and holes is
/// not zero.
///
/// The result is the path with a new point wherever it goes into or out
/// of the region. Each junction says whether the piece from it to the
/// next one is inside (`clipped`), and where it comes from (`src_idx`).
/// The junction a clipped run ends at is therefore not `clipped`, and nor
/// is the last junction of an open path, which has no piece after it. Of
/// a closed path the first point is not repeated at the end: the last
/// junction's piece runs back to the first. When no piece of the path is
/// inside, the result is empty.
///
/// Different from the reference on purpose (see the top of this file for
/// how the work is done):
/// - A piece is in or out by where its middle lies, and a piece that runs
///   along the region's edge counts as inside, as the reference's
///   `SplitLineJunction` says it should. What the reference's Clipper
///   makes of such a piece depends on which way the two run.
/// - A new point is the crossing rounded to whole units, each coordinate
///   to the nearest. Clipper rounds the same crossing its own way, so a
///   new point can differ from the reference's by a unit.
/// - A path that only touches the region, at a point, is not split there.
pub fn split_line(path: &[Point2], clip: &[ExPolygon], closed: bool) -> Vec<SplitLineJunction> {
    split_along(path, &Boundary::new(clip), closed)
}

/// [`split_line`] against a region's edges already sorted.
fn split_along(path: &[Point2], boundary: &Boundary, closed: bool) -> Vec<SplitLineJunction> {
    let n = path.len();
    if n < 2 || boundary.edges.is_empty() {
        return Vec::new();
    }

    let segments = if closed { n } else { n - 1 };
    let mut result: Vec<SplitLineJunction> = Vec::with_capacity(n + 8);
    // Whether the piece before the one in hand is inside, once known.
    let mut state: Option<bool> = None;
    for i in 0..segments {
        let (a, b) = (path[i], path[(i + 1) % n]);
        let (cuts, touches) = cuts_of_segment(boundary, a, b);
        let mut from = a;
        for (j, &to) in cuts.iter().chain(std::iter::once(&b)).enumerate() {
            let inside = match state {
                // A segment that meets no edge anywhere, its ends
                // included, is on the side the path was on before it.
                Some(before) if !touches => before,
                _ => boundary.covers_middle(from, to),
            };
            // A crossing that does not change sides (the path touches the
            // boundary and turns back, or passes from one part of the
            // region into another) is no split.
            if j == 0 {
                result.push(SplitLineJunction {
                    p: from,
                    clipped: inside,
                    src_idx: i as i64,
                });
            } else if state != Some(inside) {
                result.push(SplitLineJunction {
                    p: from,
                    clipped: inside,
                    src_idx: -(i as i64) - 1,
                });
            }
            state = Some(inside);
            from = to;
        }
    }
    if !closed {
        // The end of an open path: no piece follows it.
        result.push(SplitLineJunction {
            p: path[n - 1],
            clipped: false,
            src_idx: (n - 1) as i64,
        });
    }

    if result.iter().any(|junction| junction.clipped) {
        result
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::CallRng;
    use crate::testing::{mm, rect, rect_region, segment_distance};
    use edge_geom::Polygon;

    /// (x mm, y mm, clipped, src_idx) of each junction.
    fn listed(junctions: &[SplitLineJunction]) -> Vec<(f64, f64, bool, i64)> {
        junctions
            .iter()
            .map(|j| (j.p.x as f64 / 1e6, j.p.y as f64 / 1e6, j.clipped, j.src_idx))
            .collect()
    }

    fn square() -> Vec<Point2> {
        rect(0.0, 0.0, 20.0, 20.0).points
    }

    #[test]
    fn junction_helpers() {
        let src = SplitLineJunction {
            p: mm(1.0, 2.0),
            clipped: true,
            src_idx: 4,
        };
        assert!(src.is_src());
        assert_eq!(src.get_src_index(), 4);
        let first = SplitLineJunction { src_idx: 0, ..src };
        assert!(first.is_src());
        assert_eq!(first.get_src_index(), 0);
        // A new point on the segment that starts from point 4, and on the
        // one from point 0.
        let cut = SplitLineJunction { src_idx: -5, ..src };
        assert!(!cut.is_src());
        assert_eq!(cut.get_src_index(), 4);
        let cut = SplitLineJunction { src_idx: -1, ..src };
        assert!(!cut.is_src());
        assert_eq!(cut.get_src_index(), 0);
    }

    #[test]
    fn rounding_division() {
        assert_eq!(div_round(7, 2), 4);
        assert_eq!(div_round(-7, 2), -4);
        assert_eq!(div_round(7, -2), -4);
        assert_eq!(div_round(-7, -2), 4);
        assert_eq!(div_round(10, 3), 3);
        assert_eq!(div_round(11, 3), 4);
        assert_eq!(div_round(-10, 3), -3);
        assert_eq!(div_round(0, 5), 0);
        assert_eq!(div_round(1 << 100, 1 << 40), 1 << 60);
    }

    #[test]
    fn closed_square_against_its_right_half() {
        let right = [rect_region(10.0, -5.0, 30.0, 25.0)];
        let out = split_line(&square(), &right, true);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, false, 0),
                // In on the bottom side, the segment from point 0.
                (10.0, 0.0, true, -1),
                (20.0, 0.0, true, 1),
                (20.0, 20.0, true, 2),
                // Out on the top side, the segment from point 2: the end
                // of the clipped run, not clipped itself.
                (10.0, 20.0, false, -3),
                // The first point is not repeated after the last.
                (0.0, 20.0, false, 3),
            ]
        );
    }

    #[test]
    fn closed_square_with_the_seam_inside() {
        // The left half: the run inside goes from the top side through
        // the last point, round to the first and on to the bottom side.
        let left = [rect_region(-10.0, -5.0, 10.0, 25.0)];
        let out = split_line(&square(), &left, true);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, true, 0),
                (10.0, 0.0, false, -1),
                (20.0, 0.0, false, 1),
                (20.0, 20.0, false, 2),
                (10.0, 20.0, true, -3),
                // Its piece is the closing one, back to the first point.
                (0.0, 20.0, true, 3),
            ]
        );
        // A cut on the closing segment is a new point of the last one.
        let top = [rect_region(-5.0, 10.0, 25.0, 30.0)];
        let out = split_line(&square(), &top, true);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, false, 0),
                (20.0, 0.0, false, 1),
                (20.0, 10.0, true, -2),
                (20.0, 20.0, true, 2),
                (0.0, 20.0, true, 3),
                (0.0, 10.0, false, -4),
            ]
        );
    }

    #[test]
    fn open_path_against_a_half() {
        // The same four points as an open line: no segment back.
        let right = [rect_region(10.0, -5.0, 30.0, 25.0)];
        let out = split_line(&square(), &right, false);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, false, 0),
                (10.0, 0.0, true, -1),
                (20.0, 0.0, true, 1),
                (20.0, 20.0, true, 2),
                (10.0, 20.0, false, -3),
                (0.0, 20.0, false, 3),
            ]
        );
        // Ending inside: the last junction still is not clipped, since
        // nothing follows it.
        let left = [rect_region(-10.0, -5.0, 10.0, 25.0)];
        let out = split_line(&square(), &left, false);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, true, 0),
                (10.0, 0.0, false, -1),
                (20.0, 0.0, false, 1),
                (20.0, 20.0, false, 2),
                (10.0, 20.0, true, -3),
                (0.0, 20.0, false, 3),
            ]
        );
    }

    #[test]
    fn region_with_a_hole() {
        let mut hole = rect(8.0, -2.0, 12.0, 2.0);
        hole.points.reverse();
        let region = [ExPolygon {
            contour: rect(-5.0, -5.0, 25.0, 25.0),
            holes: vec![hole],
        }];
        // The bottom side passes over the hole.
        let line = [mm(0.0, 0.0), mm(20.0, 0.0)];
        assert_eq!(
            listed(&split_line(&line, &region, false)),
            [
                (0.0, 0.0, true, 0),
                (8.0, 0.0, false, -1),
                (12.0, 0.0, true, -1),
                (20.0, 0.0, false, 1),
            ]
        );
        // Round the square: two new points on its first segment, in the
        // order the path meets them.
        let out = split_line(&square(), &region, true);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, true, 0),
                (8.0, 0.0, false, -1),
                (12.0, 0.0, true, -1),
                (20.0, 0.0, true, 1),
                (20.0, 20.0, true, 2),
                (0.0, 20.0, true, 3),
            ]
        );
        // Walked the other way, the nearer cut still comes first.
        let back = [mm(20.0, 0.0), mm(0.0, 0.0)];
        assert_eq!(
            listed(&split_line(&back, &region, false)),
            [
                (20.0, 0.0, true, 0),
                (12.0, 0.0, false, -1),
                (8.0, 0.0, true, -1),
                (0.0, 0.0, false, 1),
            ]
        );
        // A path wholly within the hole has nothing inside.
        let in_hole = [mm(9.0, -1.0), mm(11.0, 1.0)];
        assert!(split_line(&in_hole, &region, false).is_empty());
    }

    #[test]
    fn wholly_inside_and_wholly_outside() {
        let around = [rect_region(-5.0, -5.0, 25.0, 25.0)];
        let out = split_line(&square(), &around, true);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, true, 0),
                (20.0, 0.0, true, 1),
                (20.0, 20.0, true, 2),
                (0.0, 20.0, true, 3),
            ]
        );
        // Open, all but the last.
        let out = split_line(&square(), &around, false);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, true, 0),
                (20.0, 0.0, true, 1),
                (20.0, 20.0, true, 2),
                (0.0, 20.0, false, 3),
            ]
        );

        let away = [rect_region(50.0, 50.0, 60.0, 60.0)];
        assert!(split_line(&square(), &away, true).is_empty());
        assert!(split_line(&square(), &away, false).is_empty());
        // A region the path goes round without entering.
        let within = [rect_region(5.0, 5.0, 15.0, 15.0)];
        assert!(split_line(&square(), &within, true).is_empty());
        // No region, no path.
        assert!(split_line(&square(), &[], true).is_empty());
        assert!(split_line(&[], &around, true).is_empty());
        assert!(split_line(&[mm(1.0, 1.0)], &around, false).is_empty());
        // A ring of two points encloses nothing.
        let sliver = [ExPolygon {
            contour: Polygon {
                points: vec![mm(-5.0, 0.0), mm(25.0, 0.0)],
            },
            holes: Vec::new(),
        }];
        assert!(split_line(&square(), &sliver, true).is_empty());
    }

    #[test]
    fn touching_a_vertex_of_the_region() {
        // A triangle below the square, its tip on the bottom side.
        let below = [ExPolygon {
            contour: Polygon {
                points: vec![mm(5.0, -5.0), mm(15.0, -5.0), mm(10.0, 0.0)],
            },
            holes: Vec::new(),
        }];
        // Touched, not entered: nothing is inside, so nothing comes back.
        assert!(split_line(&square(), &below, true).is_empty());
        assert!(split_line(&[mm(0.0, 0.0), mm(20.0, 0.0)], &below, false).is_empty());

        // The same touch on a path that is inside elsewhere: no new
        // point at the tip.
        let with_right = [below[0].clone(), rect_region(18.0, -5.0, 30.0, 25.0)];
        let out = split_line(&[mm(0.0, 0.0), mm(20.0, 0.0)], &with_right, false);
        assert_eq!(
            listed(&out),
            [
                (0.0, 0.0, false, 0),
                (18.0, 0.0, true, -1),
                (20.0, 0.0, false, 1)
            ]
        );

        // A path that goes in through a vertex of the region: the vertex
        // is the new point.
        let diamond = [ExPolygon {
            contour: Polygon {
                points: vec![mm(10.0, 0.0), mm(15.0, 5.0), mm(10.0, 10.0), mm(5.0, 5.0)],
            },
            holes: Vec::new(),
        }];
        let up = [mm(10.0, -5.0), mm(10.0, 15.0)];
        assert_eq!(
            listed(&split_line(&up, &diamond, false)),
            [
                (10.0, -5.0, false, 0),
                (10.0, 0.0, true, -1),
                (10.0, 10.0, false, -1),
                (10.0, 15.0, false, 1),
            ]
        );
    }

    #[test]
    fn path_points_on_the_boundary_need_no_new_point() {
        let region = [rect_region(10.0, 0.0, 30.0, 20.0)];
        // The middle point lies on the region's left side.
        let line = [mm(0.0, 10.0), mm(10.0, 10.0), mm(20.0, 10.0)];
        assert_eq!(
            listed(&split_line(&line, &region, false)),
            [
                (0.0, 10.0, false, 0),
                (10.0, 10.0, true, 1),
                (20.0, 10.0, false, 2)
            ]
        );
        // The path ends on the boundary, coming from outside.
        let line = [mm(0.0, 10.0), mm(10.0, 10.0)];
        assert!(split_line(&line, &region, false).is_empty());
        // And starts on it, going in.
        let line = [mm(10.0, 10.0), mm(20.0, 10.0), mm(40.0, 10.0)];
        assert_eq!(
            listed(&split_line(&line, &region, false)),
            [
                (10.0, 10.0, true, 0),
                (20.0, 10.0, true, 1),
                (30.0, 10.0, false, -2),
                (40.0, 10.0, false, 2),
            ]
        );
    }

    #[test]
    fn running_along_the_boundary_counts_as_inside() {
        // The square's bottom side lies on the region's bottom side from
        // x = 10 on.
        let region = [rect_region(10.0, 0.0, 30.0, 20.0)];
        let line = [mm(0.0, 0.0), mm(20.0, 0.0)];
        assert_eq!(
            listed(&split_line(&line, &region, false)),
            [
                (0.0, 0.0, false, 0),
                (10.0, 0.0, true, -1),
                (20.0, 0.0, false, 1)
            ]
        );
        // Along it and past its end.
        let line = [mm(0.0, 0.0), mm(40.0, 0.0)];
        assert_eq!(
            listed(&split_line(&line, &region, false)),
            [
                (0.0, 0.0, false, 0),
                (10.0, 0.0, true, -1),
                (30.0, 0.0, false, -1),
                (40.0, 0.0, false, 1),
            ]
        );
    }

    #[test]
    fn parts_of_a_region_that_meet_do_not_split() {
        // Two rectangles sharing a side, not united: the path crosses
        // from one into the other and is inside throughout.
        let halves = [
            rect_region(-5.0, -5.0, 10.0, 25.0),
            rect_region(10.0, -5.0, 25.0, 25.0),
        ];
        let out = split_line(&square(), &halves, true);
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|j| j.clipped && j.is_src()));
        // Overlapping parts: inside both is still inside (non-zero fill).
        let overlapping = [
            rect_region(-5.0, -5.0, 12.0, 25.0),
            rect_region(8.0, -5.0, 25.0, 25.0),
        ];
        let out = split_line(&square(), &overlapping, true);
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|j| j.clipped && j.is_src()));
        // A clockwise contour fills as well.
        let mut clockwise = rect(10.0, -5.0, 30.0, 25.0);
        clockwise.points.reverse();
        let region = [ExPolygon {
            contour: clockwise,
            holes: Vec::new(),
        }];
        assert_eq!(
            split_line(&square(), &region, true),
            split_line(&square(), &[rect_region(10.0, -5.0, 30.0, 25.0)], true)
        );
    }

    #[test]
    fn new_points_are_rounded_to_whole_units() {
        let region = [ExPolygon {
            contour: Polygon {
                points: vec![
                    Point2 { x: 1, y: -100 },
                    Point2 { x: 100, y: -100 },
                    Point2 { x: 100, y: 100 },
                    Point2 { x: 1, y: 100 },
                ],
            },
            holes: Vec::new(),
        }];
        // Crosses x = 1 at y = 10 / 3, and x = 100 nowhere.
        let line = [Point2 { x: 0, y: 0 }, Point2 { x: 3, y: 10 }];
        let out = split_line(&line, &region, false);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].p, Point2 { x: 1, y: 3 });
        assert_eq!(
            (out[0].clipped, out[1].clipped, out[1].src_idx),
            (false, true, -1)
        );
        // At y = 20 / 3: rounds up.
        let line = [Point2 { x: 0, y: 0 }, Point2 { x: 3, y: 20 }];
        assert_eq!(
            split_line(&line, &region, false)[1].p,
            Point2 { x: 1, y: 7 }
        );
        // Downwards and right to left the same.
        let line = [Point2 { x: 3, y: -20 }, Point2 { x: 0, y: 0 }];
        let out = split_line(&line, &region, false);
        assert_eq!(out[1].p, Point2 { x: 1, y: -7 });
        assert_eq!((out[0].clipped, out[1].clipped), (true, false));
    }

    #[test]
    fn far_coordinates_do_not_overflow() {
        // A part two metres across, well away from the origin.
        let far = 3_000_000_000_i64;
        let size = 2_000_000_000_i64;
        let at = |x: i64, y: i64| Point2 {
            x: far + x,
            y: far + y,
        };
        let region = [ExPolygon {
            contour: Polygon {
                points: vec![
                    at(size / 2, -size),
                    at(2 * size, -size),
                    at(2 * size, 2 * size),
                    at(size / 2, 2 * size),
                ],
            },
            holes: Vec::new(),
        }];
        let path = [at(0, 0), at(size, 1), at(size, size), at(0, size - 1)];
        let out = split_line(&path, &region, true);
        assert_eq!(out.len(), 6);
        assert_eq!(out[1].p, at(size / 2, 1));
        assert_eq!(out[4].p, at(size / 2, size - 1));
        let flags: Vec<bool> = out.iter().map(|j| j.clipped).collect();
        assert_eq!(flags, [false, true, true, true, false, false]);
    }

    /// A random simple polygon around a centre: a star with as many
    /// points as asked, each at its own distance.
    fn star(rng: &mut CallRng, cx: f64, cy: f64, radius: f64, points: usize) -> Polygon {
        Polygon {
            points: (0..points)
                .map(|i| {
                    let angle = i as f64 / points as f64 * std::f64::consts::TAU;
                    let r = radius * (0.35 + 0.65 * rng.value());
                    mm(cx + r * angle.cos(), cy + r * angle.sin())
                })
                .collect(),
        }
    }

    #[test]
    fn random_paths_split_consistently() {
        let mut rng = CallRng::for_call(1.0, mm(0.0, 0.0), 7);
        let mut total_cuts = 0;
        for round in 0..40 {
            // A region of a few stars, one with a hole; they may overlap.
            let mut region = Vec::new();
            for _ in 0..3 {
                let (cx, cy) = (rng.value() * 30.0, rng.value() * 30.0);
                let contour = star(&mut rng, cx, cy, 12.0, 40 + round);
                let mut holes = Vec::new();
                if rng.value() < 0.5 {
                    let mut hole = star(&mut rng, cx, cy, 3.5, 12);
                    hole.points.reverse();
                    holes.push(hole);
                }
                region.push(ExPolygon { contour, holes });
            }
            let path = star(&mut rng, 15.0, 15.0, 22.0, 60).points;

            let indexed = Boundary::new(&region);
            let unsorted = Boundary::with_strips(&region, Some(1));
            assert!(indexed.strips.len() > 5);
            assert_eq!(unsorted.strips.len(), 1);
            for closed in [true, false] {
                let out = split_along(&path, &indexed, closed);
                // The strips are only a way to look less far.
                assert_eq!(out, split_along(&path, &unsorted, closed));
                assert_eq!(out, split_line(&path, &region, closed));
                assert!(!out.is_empty());

                // Every point of the path is there once, in order, and
                // each new point lies on the segment it names, between
                // the path's points before and after.
                let sources: Vec<i64> = out
                    .iter()
                    .filter(|j| j.is_src())
                    .map(|j| j.src_idx)
                    .collect();
                assert_eq!(sources, (0..path.len() as i64).collect::<Vec<_>>());
                let mut current = 0;
                for (k, junction) in out.iter().enumerate() {
                    if junction.is_src() {
                        current = junction.get_src_index();
                        assert_eq!(junction.p, path[current]);
                        continue;
                    }
                    total_cuts += 1;
                    assert_eq!(junction.get_src_index(), current);
                    let (a, b) = (path[current], path[(current + 1) % path.len()]);
                    assert!(segment_distance(junction.p, a, b) <= 0.75);
                    // A new point is where the path changes sides.
                    assert_ne!(junction.clipped, out[k - 1].clipped);
                }
                if !closed {
                    assert!(!out[out.len() - 1].clipped);
                }

                // Each piece is in or out as its middle is. The middle is
                // tested here in floating point against every edge, with
                // pieces too close to the boundary to call left out.
                let pieces = if closed { out.len() } else { out.len() - 1 };
                for k in 0..pieces {
                    let (from, to) = (out[k].p, out[(k + 1) % out.len()].p);
                    let (mx, my) = ((from.x + to.x) as f64 / 2.0, (from.y + to.y) as f64 / 2.0);
                    let mut winding = 0;
                    let mut nearest = f64::INFINITY;
                    for &(c, d) in &unsorted.edges {
                        let (cx, cy, dx, dy) = (c.x as f64, c.y as f64, d.x as f64, d.y as f64);
                        let cross = (dx - cx) * (my - cy) - (dy - cy) * (mx - cx);
                        if cy <= my {
                            if dy > my && cross > 0.0 {
                                winding += 1;
                            }
                        } else if dy <= my && cross < 0.0 {
                            winding -= 1;
                        }
                        let middle = Point2 {
                            x: mx.round() as Coord,
                            y: my.round() as Coord,
                        };
                        nearest = nearest.min(segment_distance(middle, c, d));
                    }
                    if nearest > 4.0 {
                        assert_eq!(out[k].clipped, winding != 0, "round {round} piece {k}");
                    }
                }
            }
        }
        assert!(total_cuts > 400, "{total_cuts}");
    }
}
