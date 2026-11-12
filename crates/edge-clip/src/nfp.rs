//! Minkowski sums and no-fit polygons of convex outlines, for arranging
//! (libnest2d `geometry_traits_nfp.hpp`: `nfpConvexOnly`,
//! `correctNfpPosition`, `nfpInnerRectBed`).
//!
//! These need no clipping, only exact integer arithmetic; they sit here
//! beside the clipping wrapper because the region the arranger searches is
//! what the inner-fit rectangle leaves once the no-fit polygons are taken
//! out of it ([`crate::difference`]).

use edge_geom::{Coord, Point2, Polygon, convex_hull};

/// Which way `a -> b -> c` turns: positive to the left. Products of two
/// coordinates overflow `i64`.
fn turn(a: Point2, b: Point2, c: Point2) -> i128 {
    i128::from(b.x - a.x) * i128::from(c.y - a.y) - i128::from(b.y - a.y) * i128::from(c.x - a.x)
}

/// The corners of a convex outline counter-clockwise from its lowest
/// (then leftmost) corner, with repeated corners and corners in the middle
/// of a straight side left out. Outlines given clockwise are turned round.
fn normalized(polygon: &Polygon) -> Vec<Point2> {
    let hull = convex_hull(polygon.points.iter().copied());
    let mut points = hull.points;
    if points.len() < 3 {
        return points;
    }
    let start = (0..points.len())
        .min_by_key(|&i| (points[i].y, points[i].x))
        .unwrap_or(0);
    points.rotate_left(start);
    points
}

/// The Minkowski sum of two convex outlines: every point of one plus every
/// point of the other. Counter-clockwise, from its lowest corner, with no
/// corner in the middle of a side. Outlines that are not convex are taken
/// as their convex hulls; a point or a line counts as the outline it is.
pub fn minkowski_sum_convex(a: &Polygon, b: &Polygon) -> Polygon {
    let (p, q) = (normalized(a), normalized(b));
    if p.is_empty() || q.is_empty() {
        return Polygon::default();
    }
    if p.len() < 3 || q.len() < 3 {
        // A point or a segment: the hull of the sums of the corners.
        return convex_hull(p.iter().flat_map(|u| {
            q.iter().map(move |v| Point2 {
                x: u.x + v.x,
                y: u.y + v.y,
            })
        }));
    }
    // Merge the sides of the two by the angle they make, both being
    // counter-clockwise from their lowest corners.
    let (n, m) = (p.len(), q.len());
    let mut sum = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        let at = Point2 {
            x: p[i % n].x + q[j % m].x,
            y: p[i % n].y + q[j % m].y,
        };
        if sum.last() != Some(&at) {
            sum.push(at);
        }
        let side = |points: &[Point2], k: usize| {
            let (from, to) = (points[k % points.len()], points[(k + 1) % points.len()]);
            Point2 {
                x: to.x - from.x,
                y: to.y - from.y,
            }
        };
        let cross = if i < n && j < m {
            let (u, v) = (side(&p, i), side(&q, j));
            i128::from(u.x) * i128::from(v.y) - i128::from(u.y) * i128::from(v.x)
        } else if i < n {
            1
        } else {
            -1
        };
        if cross >= 0 && i < n {
            i += 1;
        }
        if cross <= 0 && j < m {
            j += 1;
        }
    }
    // The walk ends where it began.
    if sum.len() > 1 && sum.first() == sum.last() {
        sum.pop();
    }
    // Parallel sides of the two give corners in a straight line.
    let mut kept: Vec<Point2> = Vec::with_capacity(sum.len());
    for k in 0..sum.len() {
        let (before, here, after) = (
            sum[(k + sum.len() - 1) % sum.len()],
            sum[k],
            sum[(k + 1) % sum.len()],
        );
        if turn(before, here, after) != 0 {
            kept.push(here);
        }
    }
    Polygon { points: kept }
}

/// An outline's reference vertex in libnest2d: its top corner, the
/// rightmost of them (`rightmostUpVertex`, ordered by y and then x).
pub fn reference_vertex(polygon: &Polygon) -> Option<Point2> {
    polygon.points.iter().copied().max_by_key(|p| (p.y, p.x))
}

/// The no-fit polygon of two convex outlines: the places for `orbiter`'s
/// reference vertex ([`reference_vertex`]) where it touches `stationary`
/// (on the outline) or overlaps it (inside). This is libnest2d's
/// `nfpConvexOnly` moved where `correctNfpPosition` moves it, built
/// directly as `stationary` plus `orbiter` turned about its reference
/// vertex.
pub fn nfp_convex(stationary: &Polygon, orbiter: &Polygon) -> Polygon {
    let Some(reference) = reference_vertex(orbiter) else {
        return Polygon::default();
    };
    let reflected = Polygon {
        points: orbiter
            .points
            .iter()
            .map(|p| Point2 {
                x: reference.x - p.x,
                y: reference.y - p.y,
            })
            .collect(),
    };
    minkowski_sum_convex(stationary, &reflected)
}

/// The places for `orbiter`'s reference vertex that keep it wholly inside
/// the box `bed` (`[min_x, min_y, max_x, max_y]`): libnest2d's
/// `nfpInnerRectBed`. None when it is wider or deeper than the bed.
pub fn inner_fit_rectangle(bed: [Coord; 4], orbiter: &Polygon) -> Option<Polygon> {
    let reference = reference_vertex(orbiter)?;
    let (mut x0, mut y0, mut x1, mut y1) = (Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN);
    for p in &orbiter.points {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    if y1 - y0 > bed[3] - bed[1] || x1 - x0 > bed[2] - bed[0] {
        return None;
    }
    // The reference vertex is the top one, so it is (y1 - y0) over the
    // outline's foot.
    let (left, right) = (reference.x - x0, reference.x - x1);
    let (low, high) = (bed[1] + (y1 - y0), bed[3]);
    Some(Polygon {
        points: vec![
            Point2 {
                x: bed[0] + left,
                y: low,
            },
            Point2 {
                x: bed[2] + right,
                y: low,
            },
            Point2 {
                x: bed[2] + right,
                y: high,
            },
            Point2 {
                x: bed[0] + left,
                y: high,
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[(Coord, Coord)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| Point2 { x, y }).collect(),
        }
    }

    fn area2(polygon: &Polygon) -> i128 {
        let pts = &polygon.points;
        (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
            })
            .sum()
    }

    /// Whether a point is inside (or on) a counter-clockwise convex outline.
    fn inside(polygon: &Polygon, p: Point2) -> bool {
        let pts = &polygon.points;
        (0..pts.len()).all(|i| turn(pts[i], pts[(i + 1) % pts.len()], p) >= 0)
    }

    #[test]
    fn two_squares_sum_to_a_square_of_both_sides() {
        let a = polygon(&[(0, 0), (10, 0), (10, 10), (0, 10)]);
        let b = polygon(&[(0, 0), (4, 0), (4, 4), (0, 4)]);
        let sum = minkowski_sum_convex(&a, &b);
        assert_eq!(sum, polygon(&[(0, 0), (14, 0), (14, 14), (0, 14)]));
    }

    #[test]
    fn a_sum_is_the_hull_of_every_pair_of_corners() {
        // A triangle and a pentagon, one of them clockwise.
        let a = polygon(&[(0, 0), (7, 1), (2, 9)]);
        let b = polygon(&[(0, 0), (-3, 4), (-1, 8), (4, 7), (5, 2)]);
        let sum = minkowski_sum_convex(&a, &b);
        let hull = convex_hull(a.points.iter().flat_map(|u| {
            b.points.iter().map(move |v| Point2 {
                x: u.x + v.x,
                y: u.y + v.y,
            })
        }));
        assert_eq!(area2(&sum), area2(&hull));
        assert_eq!(sum.points.len(), hull.points.len());
        for p in &hull.points {
            assert!(sum.points.contains(p), "{p:?} is a corner of the hull");
        }
    }

    #[test]
    fn the_no_fit_polygon_is_where_the_two_touch() {
        // A 10 by 10 square and a 4 by 2 orbiter: its reference vertex is
        // its top right corner, which touches the square anywhere on the
        // square grown by the orbiter.
        let square = polygon(&[(0, 0), (10, 0), (10, 10), (0, 10)]);
        let orbiter = polygon(&[(100, 100), (104, 100), (104, 102), (100, 102)]);
        assert_eq!(reference_vertex(&orbiter), Some(Point2 { x: 104, y: 102 }));
        let nfp = nfp_convex(&square, &orbiter);
        assert_eq!(nfp, polygon(&[(0, 0), (14, 0), (14, 12), (0, 12)]));
        // With the reference vertex at (7, 6) the orbiter covers 3..7 by
        // 4..6, inside the square; at (20, 6) it is clear of it; at
        // (14, 6) its left side lies on the square's right side.
        assert!(inside(&nfp, Point2 { x: 7, y: 6 }));
        assert!(!inside(&nfp, Point2 { x: 20, y: 6 }));
        assert!(
            inside(&nfp, Point2 { x: 14, y: 6 }),
            "touching is on the outline"
        );
    }

    #[test]
    fn the_inner_fit_rectangle_keeps_the_outline_on_the_bed() {
        let orbiter = polygon(&[(0, 0), (30, 0), (20, 10)]);
        let fit = inner_fit_rectangle([0, 0, 100, 50], &orbiter).unwrap();
        // The reference vertex (20, 10) runs from 20 to 90 across and
        // from 10 to 50 up.
        assert_eq!(fit, polygon(&[(20, 10), (90, 10), (90, 50), (20, 50)]));
        assert!(inner_fit_rectangle([0, 0, 25, 50], &orbiter).is_none());
    }
}
