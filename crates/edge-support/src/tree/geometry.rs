//! Small geometric pieces the organic trees use throughout, after the
//! reference's helpers of the same names.

use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};

use crate::SupportError;

pub(crate) type Polys = Vec<ExPolygon>;

/// The reference's `jtMiter` with a limit of 1.2, which most of the tree
/// code offsets with.
pub(crate) const MITER: Join = Join::Miter { limit: 1.2 };
/// The reference's `ApplySafetyOffset::Yes`: the clip grown by 10 units.
pub(crate) const SAFETY: Coord = 10;

pub(crate) fn round(tolerance: Coord) -> Join {
    Join::Round {
        tolerance: tolerance.max(1),
    }
}

/// Area of regions in scaled units squared, holes taken off.
pub(crate) fn area(regions: &[ExPolygon]) -> f64 {
    regions
        .iter()
        .map(|region| {
            region
                .rings()
                .map(|ring| doubled(&ring.points))
                .sum::<i128>()
        })
        .sum::<i128>() as f64
        / 2.0
}

fn doubled(points: &[Point2]) -> i128 {
    let mut sum = 0i128;
    for (index, a) in points.iter().enumerate() {
        let b = points[(index + 1) % points.len()];
        sum += i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y);
    }
    sum
}

/// `tiny_area_threshold`: a micrometre squared.
pub(crate) fn tiny_area() -> f64 {
    (scaled(0.001) as f64).powi(2)
}

/// Extent of regions: min x, min y, max x, max y; `None` when empty.
pub(crate) fn extent(regions: &[ExPolygon]) -> Option<[Coord; 4]> {
    let mut points = regions.iter().flat_map(|region| &region.contour.points);
    let first = points.next()?;
    Some(
        points.fold([first.x, first.y, first.x, first.y], |[a, b, c, d], p| {
            [a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y)]
        }),
    )
}

pub(crate) fn union(regions: &[ExPolygon]) -> Result<Polys, SupportError> {
    Ok(edge_clip::union_ex(regions)?)
}

pub(crate) fn union2(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if b.is_empty() {
        return union(a);
    }
    if a.is_empty() {
        return union(b);
    }
    let mut all = a.to_vec();
    all.extend_from_slice(b);
    union(&all)
}

pub(crate) fn diff(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() {
        return Ok(Vec::new());
    }
    if b.is_empty() {
        return union(a);
    }
    Ok(edge_clip::difference(a, b)?)
}

/// `diff` with the clip cut down to the box round `a` first
/// (`diff_clipped`).
pub(crate) fn diff_near(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() {
        return Ok(Vec::new());
    }
    if b.is_empty() {
        return union(a);
    }
    Ok(edge_clip::difference_near(
        a,
        b,
        crate::tree::config::SCALED_EPSILON,
    )?)
}

/// `intersection` with `b` cut down to the box round `a` first
/// (`intersection_clipped`).
pub(crate) fn intersection_near(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    Ok(edge_clip::intersection_near(
        a,
        b,
        crate::tree::config::SCALED_EPSILON,
    )?)
}

/// `diff` with the clip grown by the safety offset.
pub(crate) fn diff_safe(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() || b.is_empty() {
        return diff(a, b);
    }
    diff(a, &edge_clip::offset(b, SAFETY, Join::MITER)?)
}

pub(crate) fn intersection(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    Ok(edge_clip::intersection(a, b)?)
}

pub(crate) fn offset(
    regions: &[ExPolygon],
    delta: Coord,
    join: Join,
) -> Result<Polys, SupportError> {
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    Ok(edge_clip::offset(regions, delta, join)?)
}

/// `polygons_simplify`: Douglas-Peucker to `tolerance` on every ring, the
/// rings not united again (as the reference, `strictly_simple` off).
pub(crate) fn simplify(regions: &[ExPolygon], tolerance: Coord) -> Result<Polys, SupportError> {
    if regions.is_empty() || tolerance <= 0 {
        return Ok(regions.to_vec());
    }
    Ok(edge_clip::simplify_rings(regions, tolerance))
}

/// Drops islands smaller than `threshold` (`remove_small`).
pub(crate) fn remove_small(regions: &mut Polys, threshold: f64) {
    regions.retain(|region| area(std::slice::from_ref(region)) >= threshold);
}

/// Unites regions, and should that lose everything, keeps their outlines
/// a hair wide instead (TreeSupport3D.cpp `safe_union`): influence areas
/// stand for where a branch's centre may be and can be very thin.
pub(crate) fn safe_union(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Polys, SupportError> {
    if a.is_empty() && b.is_empty() {
        return Ok(Vec::new());
    }
    // What is passed here alone is always the result of a boolean or an
    // offset, already united; the reference unites it again.
    if b.is_empty() {
        return Ok(a.to_vec());
    }
    let result = union2(a, b)?;
    if !result.is_empty() {
        return Ok(result);
    }
    let lines: Vec<Vec<Point2>> = a
        .iter()
        .chain(b)
        .flat_map(ExPolygon::rings)
        .map(|ring| {
            let mut points = ring.points.clone();
            points.extend(ring.points.first().copied());
            points
        })
        .collect();
    Ok(edge_clip::thicken(
        lines.iter().map(Vec::as_slice),
        scaled(0.002),
    )?)
}

/// Grows regions by `distance` in steps no longer than `safe_step`, taking
/// `collision` off after each, so that nothing slips through an obstacle
/// thinner than the whole distance (TreeSupport3D.cpp `safe_offset_inc`).
pub(crate) fn safe_offset_inc(
    me: &[ExPolygon],
    distance: Coord,
    collision: &[ExPolygon],
    safe_step: Coord,
    last_step_offset_without_check: Coord,
    min_amount_offset: usize,
) -> Result<Polys, SupportError> {
    let mut do_final_difference = last_step_offset_without_check == 0;
    let mut ret = safe_union(me, &[])?;
    // Each difference takes only the collision near what it cuts, as the
    // reference trims the collision to the area's box once.
    if distance == 0 || safe_step < 0 || last_step_offset_without_check < 0 {
        return if do_final_difference {
            diff_near(&ret, collision)
        } else {
            union(&ret)
        };
    }
    let mut step_size = safe_step.max(1);
    let mut steps = if distance > last_step_offset_without_check {
        (distance - last_step_offset_without_check) / step_size
    } else {
        0
    };
    if distance - steps * step_size > last_step_offset_without_check {
        if (steps + 1) * step_size <= distance {
            steps += 1;
        } else {
            do_final_difference = true;
        }
    }
    let partial =
        Coord::from(distance < last_step_offset_without_check || distance % step_size != 0);
    if steps + partial < min_amount_offset as Coord && min_amount_offset > 1 {
        step_size = distance / min_amount_offset as Coord;
        if step_size >= safe_step {
            step_size = safe_step;
            steps = min_amount_offset as Coord;
        } else {
            steps = distance / step_size.max(1);
        }
    }
    let arc = round(scaled(0.01));
    for step in 0..steps {
        ret = diff_near(&offset(&ret, step_size, arc)?, collision)?;
        // Round offsets add vertices; thin them now and then.
        if step % 10 == 7 {
            ret = simplify(&ret, scaled(0.015))?;
        }
    }
    let last = distance - steps * step_size;
    if last > crate::tree::config::SCALED_EPSILON {
        ret = offset(&ret, last, arc)?;
    }
    ret = simplify(&ret, scaled(0.015))?;
    if do_final_difference {
        ret = diff_near(&ret, collision)?;
    }
    union(&ret)
}

#[cfg(test)]
pub(crate) fn rectangle(x0: Coord, y0: Coord, x1: Coord, y1: Coord) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: vec![
                Point2 { x: x0, y: y0 },
                Point2 { x: x1, y: y0 },
                Point2 { x: x1, y: y1 },
                Point2 { x: x0, y: y1 },
            ],
        },
        holes: Vec::new(),
    }
}

/// Whether a point lies inside regions.
pub(crate) fn contains(regions: &[ExPolygon], point: Point2) -> bool {
    regions.iter().any(|region| {
        let mut within = false;
        for ring in region.rings() {
            let points = &ring.points;
            for index in 0..points.len() {
                let (a, b) = (points[index], points[(index + 1) % points.len()]);
                if (a.y > point.y) != (b.y > point.y) {
                    let side = i128::from(b.x - a.x) * i128::from(point.y - a.y)
                        - i128::from(point.x - a.x) * i128::from(b.y - a.y);
                    if (side > 0) == (b.y > a.y) {
                        within = !within;
                    }
                }
            }
        }
        within
    })
}

/// The nearest point on the outlines of `regions` to `from`, if any
/// (TreeSupport3D.cpp `move_inside` with no distance).
pub(crate) fn nearest_on_outline(regions: &[ExPolygon], from: Point2) -> Option<Point2> {
    let mut best: Option<(i128, Point2)> = None;
    for ring in regions.iter().flat_map(ExPolygon::rings) {
        let points = &ring.points;
        if points.len() < 2 {
            continue;
        }
        for index in 0..points.len() {
            let a = points[index];
            let b = points[(index + 1) % points.len()];
            let (abx, aby) = (i128::from(b.x - a.x), i128::from(b.y - a.y));
            let (apx, apy) = (i128::from(from.x - a.x), i128::from(from.y - a.y));
            let length2 = abx * abx + aby * aby;
            let foot = if length2 == 0 {
                a
            } else {
                let dot = abx * apx + aby * apy;
                if dot <= 0 {
                    a
                } else if dot >= length2 {
                    b
                } else {
                    let t = dot as f64 / length2 as f64;
                    Point2 {
                        x: a.x + (abx as f64 * t) as Coord,
                        y: a.y + (aby as f64 * t) as Coord,
                    }
                }
            };
            let (dx, dy) = (i128::from(foot.x - from.x), i128::from(foot.y - from.y));
            let distance2 = dx * dx + dy * dy;
            if best.is_none_or(|(least, _)| distance2 < least) {
                best = Some((distance2, foot));
            }
        }
    }
    best.map(|(_, point)| point)
}

/// `from` if it lies inside `regions`, or else the nearest point of their
/// outlines (`move_inside_if_outside`).
pub(crate) fn move_inside_if_outside(regions: &[ExPolygon], from: Point2) -> Point2 {
    if contains(regions, from) {
        from
    } else {
        nearest_on_outline(regions, from).unwrap_or(from)
    }
}

/// A ring as an open line that ends where it begins.
pub(crate) fn closed_line(ring: &Polygon) -> Vec<Point2> {
    let mut points = ring.points.clone();
    points.extend(ring.points.first().copied());
    points
}

pub(crate) fn distance(a: Point2, b: Point2) -> f64 {
    ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
}

pub(crate) fn polyline_length(points: &[Point2]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        rectangle(scaled(x0), scaled(y0), scaled(x1), scaled(y1))
    }

    #[test]
    fn an_area_grows_round_an_obstacle_but_not_through_it() {
        // A wall 0.2 mm thick between a seed and the far side.
        let seed = [square(0.0, 0.0, 1.0, 10.0)];
        let wall = [square(1.5, -20.0, 1.7, 20.0)];
        let grown = safe_offset_inc(&seed, scaled(3.0), &wall, scaled(0.15), 0, 1).unwrap();
        // It reaches up to the wall on the near side ...
        assert!(contains(&grown, Point2::from_mm(1.4, 5.0)));
        // ... but not beyond it, as one offset of 3 mm would.
        assert!(!contains(&grown, Point2::from_mm(2.0, 5.0)));
        assert!(!contains(&grown, Point2::from_mm(1.6, 5.0)));
        // Elsewhere it grows the whole way.
        assert!(contains(&grown, Point2::from_mm(-2.9, 5.0)));
    }

    #[test]
    fn a_point_outside_moves_to_the_nearest_edge() {
        let block = [square(0.0, 0.0, 10.0, 10.0)];
        assert_eq!(
            move_inside_if_outside(&block, Point2::from_mm(5.0, 5.0)),
            Point2::from_mm(5.0, 5.0)
        );
        assert_eq!(
            move_inside_if_outside(&block, Point2::from_mm(12.0, 5.0)),
            Point2::from_mm(10.0, 5.0)
        );
        assert!((area(&block) - 1e14).abs() < 1.0);
    }
}
