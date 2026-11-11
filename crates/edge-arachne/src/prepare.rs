//! An outline made fit for a Voronoi diagram: no detail finer than can
//! be printed, nothing touching or nearly touching itself, no point in
//! the middle of a straight run and no speck too small for a wall.
//!
//! A port of the free functions in WallToolPaths.cpp that
//! `WallToolPaths::generate` runs an outline through: `simplify`,
//! `fixSelfIntersections`, `removeDegenerateVerts`, `removeColinearEdges`
//! and `removeSmallAreas`. The reference finds the edges near a point
//! with a grid whose order is the hash table's; here they are taken in
//! the order of the outline.

use std::collections::HashMap;
use std::f64::consts::PI;

use edge_clip::ClipError;
use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};

use crate::graph::{length, nearest_on_segment};

/// The reference's `SCALED_EPSILON` (`colinear_vertex_tolerance`).
const COLINEAR_TOLERANCE: i64 = 100;

fn sq_dist(a: Point2, b: Point2) -> i64 {
    (a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y)
}

/// How far `point` is from the line through `a` and `b`, squared.
fn sq_dist_to_line(point: Point2, a: Point2, b: Point2) -> f64 {
    let v = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let va = ((point.x - a.x) as f64, (point.y - a.y) as f64);
    let l2 = v.0 * v.0 + v.1 * v.1;
    if l2 == 0.0 {
        return va.0 * va.0 + va.1 * va.1;
    }
    let t = (va.0 * v.0 + va.1 * v.1) / l2;
    (t * v.0 - va.0).powi(2) + (t * v.1 - va.1).powi(2)
}

/// Where the lines through `a` and through `b` cross
/// (`Line::intersection_infinite`).
fn crossing(a: (Point2, Point2), b: (Point2, Point2)) -> Option<Point2> {
    let v12 = ((b.0.x - a.0.x) as f64, (b.0.y - a.0.y) as f64);
    let v1 = ((a.1.x - a.0.x) as f64, (a.1.y - a.0.y) as f64);
    let v2 = ((b.1.x - b.0.x) as f64, (b.1.y - b.0.y) as f64);
    let denom = v1.0 * v2.1 - v1.1 * v2.0;
    if denom.abs() < 1e-4 {
        return None;
    }
    let t1 = (v12.0 * v2.1 - v12.1 * v2.0) / denom;
    let (x, y) = (a.0.x as f64 + t1 * v1.0, a.0.y as f64 + t1 * v1.1);
    let fits = |c: f64| c <= i64::MAX as f64 && c >= i64::MIN as f64;
    (fits(x) && fits(y)).then_some(Point2 {
        x: x as Coord,
        y: y as Coord,
    })
}

/// Drops points joined by segments shorter than the square root of
/// `smallest_line_segment_squared` where that moves the outline by no
/// more than the root of `allowed_error_distance_squared`
/// (WallToolPaths.cpp `simplify`).
fn simplify_polygon(
    poly: &mut Polygon,
    smallest_line_segment_squared: i64,
    allowed_error_distance_squared: i64,
) {
    let points = &poly.points;
    if points.len() < 3 {
        poly.points.clear();
        return;
    }
    if points.len() == 3 {
        return;
    }
    let cross = |a: Point2, b: Point2| a.x * b.y - a.y * b.x;
    let mut new_path: Vec<Point2> = Vec::new();
    let mut previous = points[points.len() - 1];
    let mut previous_previous = points[points.len() - 2];
    // Twice the area cut off so far, as a fan from the origin.
    let mut accumulated_area_removed = cross(previous, points[0]);

    for point_idx in 0..points.len() {
        let mut current = points[point_idx];
        let next = if point_idx + 1 < points.len() {
            points[point_idx + 1]
        } else if new_path.len() > 1 {
            // Round to the new outline's start.
            new_path[0]
        } else {
            points[(point_idx + 1) % points.len()]
        };
        let removed_area_next = cross(current, next);
        let negative_area_closing = cross(next, previous);
        accumulated_area_removed += removed_area_next;

        let length2 = sq_dist(current, previous);
        // Under 5 microns always goes.
        if length2 < scaled(0.005) * scaled(0.005) {
            continue;
        }
        let area_removed_so_far = accumulated_area_removed + negative_area_closing;
        let base_length_2 = sq_dist(next, previous);
        if base_length_2 == 0 {
            // There and back again, with no area.
            continue;
        }
        let height_2 =
            (area_removed_so_far as f64 * area_removed_so_far as f64 / base_length_2 as f64) as i64;
        if height_2 <= COLINEAR_TOLERANCE * COLINEAR_TOLERANCE
            && sq_dist_to_line(current, previous, next).sqrt() <= COLINEAR_TOLERANCE as f64
        {
            continue;
        }
        if length2 < smallest_line_segment_squared && height_2 <= allowed_error_distance_squared {
            let next_length2 = sq_dist(current, next);
            if next_length2 > 4 * smallest_line_segment_squared {
                // The next segment is long: rather than drop this point,
                // move it to where the two long sides would meet, and
                // drop the one before.
                let found =
                    crossing((previous_previous, previous), (current, next)).filter(|&at| {
                        sq_dist_to_line(at, previous, current)
                            <= allowed_error_distance_squared as f64
                            && sq_dist(at, previous) <= smallest_line_segment_squared
                            && sq_dist(at, next) <= smallest_line_segment_squared
                    });
                if let Some(at) = found {
                    current = at;
                    if !new_path.is_empty() {
                        new_path.pop();
                        previous = previous_previous;
                    }
                }
            } else {
                continue;
            }
        }
        accumulated_area_removed = removed_area_next;
        previous_previous = previous;
        previous = current;
        new_path.push(current);
    }
    poly.points = new_path;
}

fn simplify(polys: &mut Vec<Polygon>, smallest_line_segment: i64, allowed_error_distance: i64) {
    for poly in polys.iter_mut() {
        simplify_polygon(
            poly,
            smallest_line_segment * smallest_line_segment,
            allowed_error_distance * allowed_error_distance,
        );
    }
    polys.retain(|poly| poly.points.len() >= 3);
}

/// The regions covered an odd number of times, as rings: outlines
/// counter-clockwise, holes clockwise (`ClipperLib::SimplifyPolygons`
/// with the even-odd rule).
fn even_odd(polys: &[Polygon]) -> Result<Vec<Polygon>, ClipError> {
    Ok(rings_of(edge_clip::union_even_odd(polys)?))
}

pub(crate) fn rings_of(regions: Vec<ExPolygon>) -> Vec<Polygon> {
    let mut rings = Vec::new();
    for region in regions {
        let mut contour = region.contour;
        if !contour.is_ccw() {
            contour.points.reverse();
        }
        rings.push(contour);
        for mut hole in region.holes {
            if hole.is_ccw() {
                hole.points.reverse();
            }
            rings.push(hole);
        }
    }
    rings
}

/// Moves a point lying within half of `epsilon` of an edge it does not
/// belong to a little away from it, then unites the lot
/// (`fixSelfIntersections`).
fn fix_self_intersections(epsilon: Coord, polys: &mut Vec<Polygon>) -> Result<(), ClipError> {
    if epsilon < 1 {
        *polys = even_odd(polys)?;
        return Ok(());
    }
    let half_epsilon = (epsilon + 1) / 2;
    let grid_size: Coord = scaled(2.0);
    let cell = |c: Coord| c.div_euclid(grid_size);
    // Every edge, by the squares its bounds lie over.
    let mut grid: HashMap<(i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (poly_idx, poly) in polys.iter().enumerate() {
        let count = poly.points.len();
        for point_idx in 0..count {
            let (a, b) = (poly.points[point_idx], poly.points[(point_idx + 1) % count]);
            for x in cell(a.x.min(b.x) - epsilon)..=cell(a.x.max(b.x) + epsilon) {
                for y in cell(a.y.min(b.y) - epsilon)..=cell(a.y.max(b.y) + epsilon) {
                    grid.entry((x, y)).or_default().push((poly_idx, point_idx));
                }
            }
        }
    }
    let move_dist = (half_epsilon - 2).max(2);
    let half_epsilon_sqrd = half_epsilon * half_epsilon;
    for poly_idx in 0..polys.len() {
        let pathlen = polys[poly_idx].points.len();
        for point_idx in 0..pathlen {
            let at = polys[poly_idx].points[point_idx];
            let Some(lines) = grid.get(&(cell(at.x), cell(at.y))) else {
                continue;
            };
            for &(line_poly, line_point) in lines {
                let line_next = (line_point + 1) % polys[line_poly].points.len();
                if poly_idx == line_poly && (point_idx == line_point || point_idx == line_next) {
                    continue;
                }
                let pt = polys[poly_idx].points[point_idx];
                let (a, b) = (
                    polys[line_poly].points[line_point],
                    polys[line_poly].points[line_next],
                );
                let closest = nearest_on_segment(a, b, pt);
                if half_epsilon_sqrd < sq_dist(pt, closest) {
                    continue;
                }
                let other = polys[poly_idx].points[(point_idx + 1) % pathlen];
                let left = (b.x - a.x) * (other.y - a.y) - (b.y - a.y) * (other.x - a.x) > 0;
                let vec = if left {
                    (b.x - a.x, b.y - a.y)
                } else {
                    (a.x - b.x, a.y - b.y)
                };
                let len = length(Point2 { x: vec.0, y: vec.1 }, Point2 { x: 0, y: 0 });
                if len == 0 {
                    continue;
                }
                let pt = &mut polys[poly_idx].points[point_idx];
                pt.x += (-vec.1 * move_dist) / len;
                pt.y += (vec.0 * move_dist) / len;
            }
        }
    }
    *polys = even_odd(polys)?;
    Ok(())
}

/// Takes out a point where the outline turns straight back on itself
/// (`removeDegenerateVerts`).
fn remove_degenerate_verts(polys: &mut Vec<Polygon>) {
    let origin = Point2 { x: 0, y: 0 };
    let is_degenerate = |last: Point2, now: Point2, next: Point2| {
        let last_line = Point2 {
            x: now.x - last.x,
            y: now.y - last.y,
        };
        let next_line = Point2 {
            x: next.x - now.x,
            y: next.y - now.y,
        };
        last_line.x * next_line.x + last_line.y * next_line.y
            == -length(last_line, origin) * length(next_line, origin)
    };
    let mut poly_idx = 0;
    while poly_idx < polys.len() {
        let poly = &polys[poly_idx].points;
        let mut result: Vec<Point2> = Vec::new();
        let mut is_changed = false;
        for idx in 0..poly.len() {
            let last = result.last().copied().unwrap_or(poly[poly.len() - 1]);
            if idx + 1 == poly.len() && result.is_empty() {
                break;
            }
            let next = if idx + 1 == poly.len() {
                result[0]
            } else {
                poly[idx + 1]
            };
            if is_degenerate(last, poly[idx], next) {
                is_changed = true;
                while result.len() > 1
                    && is_degenerate(result[result.len() - 2], result[result.len() - 1], next)
                {
                    result.pop();
                }
            } else {
                result.push(poly[idx]);
            }
        }
        if is_changed {
            if result.len() > 2 {
                polys[poly_idx].points = result;
            } else {
                polys.remove(poly_idx);
                continue;
            }
        }
        poly_idx += 1;
    }
}

/// The angle on the left at `b` going from `a` to `c`, from 0 to 2π
/// (linearAlg2D.hpp `getAngleLeft`).
fn angle_left(a: Point2, b: Point2, c: Point2) -> f32 {
    let ba = (a.x - b.x, a.y - b.y);
    let bc = (c.x - b.x, c.y - b.y);
    let dott = ba.0 * bc.0 + ba.1 * bc.1;
    let det = ba.0 * bc.1 - ba.1 * bc.0;
    if det == 0 {
        return if (ba.0 != 0 && (ba.0 > 0) == (bc.0 > 0)) || (ba.0 == 0 && (ba.1 > 0) == (bc.1 > 0))
        {
            0.0
        } else {
            PI as f32
        };
    }
    let angle = (-(det as f64).atan2(dott as f64)) as f32;
    if angle >= 0.0 {
        angle
    } else {
        (PI * 2.0 + f64::from(angle)) as f32
    }
}

/// Takes out points where the outline turns by less than
/// `max_deviation_angle` (`removeColinearEdges`).
fn remove_colinear_edges_of(poly: &mut Polygon, max_deviation_angle: f64) {
    loop {
        let mut num_removed_in_iteration = 0;
        let mut process_indices = vec![true; poly.points.len()];
        let mut go = true;
        while go {
            go = false;
            let rpath = poly.points.clone();
            let pathlen = rpath.len();
            if pathlen <= 3 {
                return;
            }
            let mut skip_indices = vec![false; pathlen];
            let mut new_path: Vec<Point2> = Vec::new();
            let mut point_idx = 0;
            while point_idx < pathlen {
                if !process_indices[point_idx] {
                    new_path.push(rpath[point_idx]);
                    point_idx += 1;
                    continue;
                }
                // With the old first point gone, the last is left for
                // the next round.
                if point_idx == pathlen - 1 && skip_indices[0] {
                    skip_indices[new_path.len()] = true;
                    go = true;
                    new_path.push(rpath[point_idx]);
                    break;
                }
                let prev = rpath[(point_idx + pathlen - 1) % pathlen];
                let pt = rpath[point_idx];
                let next = rpath[(point_idx + 1) % pathlen];
                let mut angle = angle_left(prev, pt, next);
                if angle >= PI as f32 {
                    angle -= PI as f32;
                }
                let angle = f64::from(angle);
                if angle > max_deviation_angle && angle < PI - max_deviation_angle {
                    new_path.push(pt);
                } else if point_idx != pathlen - 1 {
                    // This one goes, and the next is not looked at this round.
                    skip_indices[new_path.len()] = true;
                    go = true;
                    new_path.push(next);
                    point_idx += 1;
                }
                point_idx += 1;
            }
            num_removed_in_iteration += pathlen - new_path.len();
            poly.points = new_path;
            process_indices = skip_indices;
        }
        if num_removed_in_iteration == 0 {
            return;
        }
    }
}

fn remove_colinear_edges(polys: &mut Vec<Polygon>, max_deviation_angle: f64) {
    for poly in polys.iter_mut() {
        remove_colinear_edges_of(poly, max_deviation_angle);
    }
    polys.retain(|poly| poly.points.len() >= 3);
}

/// The area inside `poly` in square units, positive wound
/// counter-clockwise (`ClipperLib::Area`).
fn area(poly: &Polygon) -> f64 {
    let points = &poly.points;
    if points.len() < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    let mut j = points.len() - 1;
    for i in 0..points.len() {
        a += (points[j].x as f64 + points[i].x as f64) * (points[j].y as f64 - points[i].y as f64);
        j = i;
    }
    -a * 0.5
}

/// Whether `point` is inside `poly`, by crossings.
fn contains(poly: &Polygon, point: Point2) -> bool {
    let points = &poly.points;
    let mut inside = false;
    let mut j = points.len().wrapping_sub(1);
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a.y > point.y) != (b.y > point.y) {
            let at = (b.x - a.x) as f64 * (point.y - a.y) as f64 / (b.y - a.y) as f64 + a.x as f64;
            if (point.x as f64) < at {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Drops outlines smaller than `min_area_size` square units, and with
/// each small hole that began inside one of them, as the reference
/// does, the last ring still kept (`removeSmallAreas` with holes left
/// in: it means to drop the hole and drops that ring instead).
fn remove_small_areas(polys: &mut Vec<Polygon>, min_area_size: f64) {
    let mut new_end = polys.len();
    let mut small_holes: Vec<Polygon> = Vec::new();
    let mut it = 0;
    while it < new_end {
        let size = area(&polys[it]);
        if size.abs() < min_area_size {
            if size >= 0.0 {
                new_end -= 1;
                if it < new_end {
                    polys.swap(new_end, it);
                    continue;
                }
                break;
            }
            small_holes.push(polys[it].clone());
        }
        it += 1;
    }
    let removed_outlines_start = new_end;
    for hole in small_holes.iter().rev() {
        let Some(&first) = hole.points.first() else {
            continue;
        };
        if polys[removed_outlines_start..]
            .iter()
            .any(|outline| contains(outline, first))
            && new_end > 0
        {
            new_end -= 1;
        }
    }
    polys.truncate(new_end);
}

/// `outline` as rings fit for the skeleton: see the top of this file.
/// `small_area_length` is the side of the smallest square worth a wall.
pub(crate) fn prepare(
    outline: &[ExPolygon],
    smallest_segment: Coord,
    allowed_distance: Coord,
    small_area_length: f64,
) -> Result<Vec<Polygon>, ClipError> {
    let epsilon_offset = allowed_distance / 2 - 1;
    let join = edge_clip::Join::MITER;
    let closed = edge_clip::offset(
        &edge_clip::offset(
            &edge_clip::offset(outline, -epsilon_offset, join)?,
            epsilon_offset * 2,
            join,
        )?,
        -epsilon_offset,
        join,
    )?;
    let mut prepared = rings_of(closed);
    simplify(&mut prepared, smallest_segment, allowed_distance);
    fix_self_intersections(epsilon_offset, &mut prepared)?;
    remove_degenerate_verts(&mut prepared);
    remove_colinear_edges(&mut prepared, 0.005);
    // Which may have made it cross itself again.
    fix_self_intersections(epsilon_offset, &mut prepared)?;
    remove_degenerate_verts(&mut prepared);
    remove_small_areas(&mut prepared, small_area_length * small_area_length);
    // What is left could still overlap itself, which the skeleton
    // cannot have; uniting it settles that.
    Ok(rings_of(edge_clip::union(&prepared)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(points: &[(f64, f64)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
        }
    }

    #[test]
    fn points_on_a_straight_run_are_dropped() {
        let mut square = vec![poly(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ])];
        remove_colinear_edges(&mut square, 0.005);
        assert_eq!(square[0].points.len(), 4);
        // Simplifying does the same, and leaves a square alone.
        let mut again = vec![poly(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ])];
        simplify(&mut again, scaled(0.5), scaled(0.025));
        assert_eq!(again[0].points.len(), 4);
    }

    #[test]
    fn a_spike_of_no_width_is_taken_out() {
        let mut spiked = vec![poly(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (15.0, 5.0),
            (10.0, 5.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ])];
        remove_degenerate_verts(&mut spiked);
        assert!(!spiked[0].points.contains(&Point2::from_mm(15.0, 5.0)));
    }

    #[test]
    fn a_short_jog_is_smoothed_and_a_speck_dropped() {
        // A bump 0.01 mm high and 0.2 mm long half way along one side.
        let mut bumped = vec![poly(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (5.1, 0.01),
            (5.2, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ])];
        simplify(&mut bumped, scaled(0.5), scaled(0.025));
        // The bump goes; the points either side of it stay until points
        // on a straight run are taken out.
        assert!(
            bumped[0]
                .points
                .iter()
                .all(|p| p.y == 0 || p.y == 10_000_000)
        );
        remove_colinear_edges(&mut bumped, 0.005);
        assert_eq!(bumped[0].points.len(), 4, "{:?}", bumped[0].points);
        // A step between two long sides stays: neither side may be moved.
        let mut stepped = vec![poly(&[
            (0.0, 0.0),
            (5.0, 0.0),
            (5.0, 0.01),
            (10.0, 0.01),
            (10.0, 10.0),
            (0.0, 10.0),
        ])];
        simplify(&mut stepped, scaled(0.5), scaled(0.025));
        assert_eq!(stepped[0].points.len(), 6);

        let mut two = vec![
            poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
            poly(&[(20.0, 0.0), (20.1, 0.0), (20.1, 0.1), (20.0, 0.1)]),
        ];
        remove_small_areas(&mut two, (0.21e6f64).powi(2));
        assert_eq!(two.len(), 1);
        assert!((area(&two[0]) - 1e14).abs() < 1.0);
    }

    #[test]
    fn a_prepared_square_is_a_square() {
        let square = ExPolygon {
            contour: poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
            holes: vec![poly(&[(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)])],
        };
        let rings = prepare(&[square], scaled(0.5), scaled(0.025), 0.21e6).unwrap();
        assert_eq!(rings.len(), 2);
        assert!(rings[0].is_ccw() && !rings[1].is_ccw());
        assert_eq!((rings[0].points.len(), rings[1].points.len()), (4, 4));
    }
}
