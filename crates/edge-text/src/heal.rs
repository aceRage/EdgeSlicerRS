//! Shapes made clean enough to raise into a solid (Emboss.cpp
//! `heal_polygons`, `heal_expolygons`, `heal_dupl_inter`; Clipper 6's
//! `CleanPolygon`).

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

use crate::TextError;

/// "little grater than sqrt(2)": points closer than this, in shape units,
/// are one (`heal_polygons`' `clean_distance`).
const CLEAN_DISTANCE: f64 = 1.415;

fn square(at: Point2, corners: [(Coord, Coord); 4]) -> Polygon {
    Polygon {
        points: corners
            .iter()
            .map(|&(x, y)| Point2 {
                x: at.x + x,
                y: at.y + y,
            })
            .collect(),
    }
}

/// `pts_2x2`, anticlockwise from the point.
fn square_2x2(at: Point2) -> Polygon {
    square(at, [(0, 0), (1, 0), (1, 1), (0, 1)])
}

/// `pts_3x3`, anticlockwise round the point.
fn square_3x3(at: Point2) -> Polygon {
    square(at, [(-1, -1), (1, -1), (1, 1), (-1, 1)])
}

/// A glyph's contours united by the non-zero rule, cleaned of points
/// that nearly meet, and healed (`heal_polygons`).
pub(crate) fn heal_polygons(
    shape: &[Polygon],
    max_iteration: u32,
) -> Result<Vec<ExPolygon>, TextError> {
    // `SimplifyPolygons`, then `CleanPolygons`.
    let united = edge_clip::union(shape)?;
    let mut polygons: Vec<Polygon> = united
        .iter()
        .flat_map(ExPolygon::rings)
        .map(|ring| clean_polygon(ring, CLEAN_DISTANCE))
        .filter(|ring| ring.points.len() >= 3)
        .collect();
    if polygons.is_empty() {
        return Ok(Vec::new());
    }
    // "Overlap all duplicit points by rectangle 3x3".
    let points: Vec<Point2> = polygons.iter().flat_map(|p| p.points.clone()).collect();
    for p in duplicates(points) {
        polygons.push(square_3x3(p));
    }
    let mut result = edge_clip::union(&polygons)?;
    heal(&mut result, max_iteration)?;
    Ok(result)
}

/// The points that are in `points` more than once, each once, in order
/// (Point.cpp `collect_duplicates`).
fn duplicates(mut points: Vec<Point2>) -> Vec<Point2> {
    points.sort_unstable_by_key(|p| (p.x, p.y));
    let mut found: Vec<Point2> = Vec::new();
    for pair in points.windows(2) {
        if pair[0] == pair[1] && found.last() != Some(&pair[1]) {
            found.push(pair[1]);
        }
    }
    found
}

/// Clipper 6's `CleanPolygon`: points within `distance` of the one before,
/// and those within it of the line through their neighbours, left out.
pub(crate) fn clean_polygon(ring: &Polygon, distance: f64) -> Polygon {
    let pts = &ring.points;
    let mut size = pts.len();
    if size == 0 {
        return Polygon::default();
    }
    let mut next: Vec<usize> = (0..size).map(|i| (i + 1) % size).collect();
    let mut prev: Vec<usize> = (0..size).map(|i| (i + size - 1) % size).collect();
    let mut done = vec![false; size];
    let dist_sqrd = distance * distance;
    let close = |a: Point2, b: Point2| {
        let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
        dx * dx + dy * dy <= dist_sqrd
    };
    // `ExcludeOp`: answers the point before, marked to be looked at again.
    let exclude = |op: usize, next: &mut Vec<usize>, prev: &mut Vec<usize>, done: &mut [bool]| {
        let result = prev[op];
        next[result] = next[op];
        prev[next[op]] = result;
        done[result] = false;
        result
    };
    let mut op = 0;
    while !done[op] && next[op] != prev[op] {
        if close(pts[op], pts[prev[op]]) {
            op = exclude(op, &mut next, &mut prev, &mut done);
            size -= 1;
        } else if close(pts[prev[op]], pts[next[op]]) {
            exclude(next[op], &mut next, &mut prev, &mut done);
            op = exclude(op, &mut next, &mut prev, &mut done);
            size = size.saturating_sub(2);
        } else if slopes_near_collinear(pts[prev[op]], pts[op], pts[next[op]], dist_sqrd) {
            op = exclude(op, &mut next, &mut prev, &mut done);
            size -= 1;
        } else {
            done[op] = true;
            op = next[op];
        }
    }
    if size < 3 {
        size = 0;
    }
    let mut out = Vec::with_capacity(size);
    for _ in 0..size {
        out.push(pts[op]);
        op = next[op];
    }
    Polygon { points: out }
}

fn distance_from_line_sqrd(pt: Point2, ln1: Point2, ln2: Point2) -> f64 {
    let a = (ln1.y - ln2.y) as f64;
    let b = (ln2.x - ln1.x) as f64;
    let mut c = a * ln1.x as f64 + b * ln1.y as f64;
    c = a * pt.x as f64 + b * pt.y as f64 - c;
    (c * c) / (a * a + b * b)
}

fn slopes_near_collinear(pt1: Point2, pt2: Point2, pt3: Point2, dist_sqrd: f64) -> bool {
    // "more accurate when the point that's geometrically between the other
    // 2 points is the one that's tested for distance"
    if (pt1.x - pt2.x).abs() > (pt1.y - pt2.y).abs() {
        if (pt1.x > pt2.x) == (pt1.x < pt3.x) {
            distance_from_line_sqrd(pt1, pt2, pt3) < dist_sqrd
        } else if (pt2.x > pt1.x) == (pt2.x < pt3.x) {
            distance_from_line_sqrd(pt2, pt1, pt3) < dist_sqrd
        } else {
            distance_from_line_sqrd(pt3, pt1, pt2) < dist_sqrd
        }
    } else if (pt1.y > pt2.y) == (pt1.y < pt3.y) {
        distance_from_line_sqrd(pt1, pt2, pt3) < dist_sqrd
    } else if (pt2.y > pt1.y) == (pt2.y < pt3.y) {
        distance_from_line_sqrd(pt2, pt1, pt3) < dist_sqrd
    } else {
        distance_from_line_sqrd(pt3, pt1, pt2) < dist_sqrd
    }
}

/// Points next to the same point dropped, and rings left with fewer than
/// three (`remove_same_neighbor`).
fn remove_same_neighbor(shape: &mut Vec<ExPolygon>) {
    let tidy = |ring: &mut Polygon| {
        ring.points.dedup();
        while ring.points.len() > 1 && ring.points.first() == ring.points.last() {
            ring.points.pop();
        }
    };
    for region in shape.iter_mut() {
        tidy(&mut region.contour);
        for hole in &mut region.holes {
            tidy(hole);
        }
        region.holes.retain(|hole| hole.points.len() > 2);
    }
    shape.retain(|region| region.contour.points.len() > 2);
}

/// Every point of the shape that some other point of it is also at.
fn duplicate_points(shape: &[ExPolygon]) -> Vec<Point2> {
    duplicates(
        shape
            .iter()
            .flat_map(ExPolygon::rings)
            .flat_map(|ring| ring.points.iter().copied())
            .collect(),
    )
}

/// Where edges of the shape meet that share no end (IntersectionPoints.cpp
/// `compute_intersections`), each once, taken down to whole units.
fn crossings(shape: &[ExPolygon]) -> Vec<Point2> {
    let mut lines: Vec<(Point2, Point2)> = Vec::new();
    for ring in shape.iter().flat_map(ExPolygon::rings) {
        let n = ring.points.len();
        for k in 0..n {
            lines.push((ring.points[k], ring.points[(k + 1) % n]));
        }
    }
    let mut found = Vec::new();
    let boxes: Vec<[Coord; 4]> = lines
        .iter()
        .map(|(a, b)| [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)])
        .collect();
    for i in 0..lines.len() {
        for j in i + 2..lines.len() {
            let (bi, bj) = (boxes[i], boxes[j]);
            if bi[0] > bj[2] || bj[0] > bi[2] || bi[1] > bj[3] || bj[1] > bi[3] {
                continue;
            }
            let ((a, b), (c, d)) = (lines[i], lines[j]);
            if a == c || a == d || b == c || b == d {
                // "it is duplicit point not intersection"
                continue;
            }
            if let Some(p) = meet(a, b, c, d) {
                found.push(p);
            }
        }
    }
    found.sort_unstable_by_key(|p| (p.x, p.y));
    found.dedup();
    found
}

/// Where segments `a b` and `c d` meet, touching included.
fn meet(a: Point2, b: Point2, c: Point2, d: Point2) -> Option<Point2> {
    let cross = |o: Point2, p: Point2, q: Point2| {
        i128::from(p.x - o.x) * i128::from(q.y - o.y)
            - i128::from(p.y - o.y) * i128::from(q.x - o.x)
    };
    let (d1, d2) = (cross(a, b, c), cross(a, b, d));
    let (d3, d4) = (cross(c, d, a), cross(c, d, b));
    if (d1 > 0 && d2 > 0) || (d1 < 0 && d2 < 0) || (d3 > 0 && d4 > 0) || (d3 < 0 && d4 < 0) {
        return None;
    }
    let denominator = d1 - d2;
    if denominator == 0 {
        if d1 != 0 {
            return None;
        }
        // In one line: an end of one on the other, if they overlap.
        let on = |p: Point2, s: Point2, e: Point2| {
            p.x >= s.x.min(e.x) && p.x <= s.x.max(e.x) && p.y >= s.y.min(e.y) && p.y <= s.y.max(e.y)
        };
        return [(c, a, b), (d, a, b), (a, c, d), (b, c, d)]
            .into_iter()
            .find(|(p, s, e)| on(*p, *s, *e))
            .map(|(p, _, _)| p);
    }
    let t = d1 as f64 / denominator as f64;
    Some(Point2 {
        x: (c.x as f64 + t * (d.x - c.x) as f64).floor() as Coord,
        y: (c.y as f64 + t * (d.y - c.y) as f64).floor() as Coord,
    })
}

/// Shapes that touch or cross themselves mended by cutting tiny squares
/// out where they do (Emboss.cpp `heal_expolygons`, `heal_dupl_inter`):
/// 2 × 2 units at a crossing, 3 × 3 round a point met twice, and a square
/// cut before that only made things worse filled in again. Answers whether
/// it is healed. Not as the reference: sharp spikes at such points are not
/// bevelled (`remove_spikes_in_duplicates`), and a shape still not healed
/// is kept, where the reference puts a frame of its box in its place.
pub fn heal(shape: &mut Vec<ExPolygon>, max_iteration: u32) -> Result<bool, TextError> {
    if shape.is_empty() {
        return Ok(true);
    }
    remove_same_neighbor(shape);
    let mut holes: Vec<Polygon> = Vec::new();
    let mut left = max_iteration;
    loop {
        left = left.saturating_sub(1);
        if left == 0 {
            break;
        }
        let duplicated = duplicate_points(shape);
        let crossed = crossings(shape);
        if duplicated.is_empty() && crossed.is_empty() {
            return Ok(true);
        }
        // `fill_trouble_holes`.
        if !holes.is_empty() {
            let trouble: Vec<Polygon> = holes
                .iter()
                .filter(|hole| {
                    hole.points
                        .iter()
                        .any(|p| duplicated.contains(p) || crossed.contains(p))
                })
                .cloned()
                .collect();
            if !trouble.is_empty() {
                let mut all: Vec<Polygon> =
                    shape.iter().flat_map(ExPolygon::rings).cloned().collect();
                all.extend(trouble);
                *shape = edge_clip::union(&all)?;
                holes.clear();
                continue;
            }
        }
        holes = crossed.iter().map(|p| square_2x2(*p)).collect();
        holes.extend(duplicated.iter().map(|p| square_3x3(*p)));
        let cut: Vec<ExPolygon> = holes
            .iter()
            .map(|hole| ExPolygon {
                contour: hole.clone(),
                holes: Vec::new(),
            })
            .collect();
        *shape = edge_clip::difference(shape, &cut)?;
    }
    Ok(duplicate_points(shape).is_empty() && crossings(shape).is_empty())
}
