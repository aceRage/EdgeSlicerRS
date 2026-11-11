//! What travel is routed by: a layer's outline drawn in to where the
//! nozzle may go along the inside of the walls, and the holes a move
//! between parts goes round.
//!
//! Ported from the reference's AvoidCrossingPerimeters.cpp:
//! `resample_polygon`, `contour_distance`, `inner_offset`, `get_boundary`,
//! `get_boundary_external` and the outline `init_layer` keeps.

use std::f64::consts::PI;

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};

use crate::grid::Grid;

/// The reference's `SCALED_EPSILON`: a tenth of a micrometre.
pub(crate) const EPSILON: Coord = 100;

type V = (f64, f64);

fn between(from: Point2, to: Point2) -> V {
    ((to.x - from.x) as f64, (to.y - from.y) as f64)
}

fn norm(v: V) -> f64 {
    v.0.hypot(v.1)
}

/// How far round a ring each of its points is from the first, and last
/// the length of the whole ring (`precompute_polygon_distances`).
pub(crate) fn lengths_along(ring: &[Point2]) -> Vec<f64> {
    let mut along = Vec::with_capacity(ring.len() + 1);
    let mut travelled = 0.0;
    along.push(travelled);
    for index in 1..=ring.len() {
        travelled += norm(between(ring[index - 1], ring[index % ring.len()]));
        along.push(travelled);
    }
    along
}

/// Adds points beside every corner of a ring, `dist_from_vertex` along
/// each edge that is long enough, and along what is left of the edge
/// between them so that no two are further apart than `max_allowed`:
/// drawing the ring in by a different distance at each point then
/// changes it only near where the distance changes (`resample_polygon`).
/// A new point is cut to whole units, as the reference's cast cuts it.
fn resample(ring: &mut Polygon, dist_from_vertex: f64, max_allowed: f64) {
    let points = &ring.points;
    let mut out = Vec::with_capacity(3 * points.len());
    for (index, &p1) in points.iter().enumerate() {
        out.push(p1);
        let p2 = points[(index + 1) % points.len()];
        let line = between(p1, p2);
        let length = norm(line);
        if length <= 2.0 * dist_from_vertex {
            continue;
        }
        let step = (
            (line.0 / length * dist_from_vertex) as Coord,
            (line.1 / length * dist_from_vertex) as Coord,
        );
        if step == (0, 0) {
            continue;
        }
        let first = Point2 {
            x: p1.x + step.0,
            y: p1.y + step.1,
        };
        let last = Point2 {
            x: p2.x - step.0,
            y: p2.y - step.1,
        };
        out.push(first);
        let rest = between(first, last);
        let rest_length = norm(rest);
        if rest_length > max_allowed {
            let parts = (rest_length / max_allowed).ceil() as usize;
            for part in 1..parts {
                let share = part as f64 / parts as f64;
                out.push(Point2 {
                    x: (first.x as f64 + rest.0 * share) as Coord,
                    y: (first.y as f64 + rest.1 * share) as Coord,
                });
            }
        }
        out.push(last);
    }
    ring.points = out;
}

/// The width of the part at every point of ring `own` of `rings`: the
/// distance to the nearest stretch of outline on the inner side of the
/// point, up to `search_radius`. Stretches of the point's own ring count
/// only when they lie a fair way round it: further along than a quarter
/// circle of the offset, and, the outline between being judged by its
/// length against the gap, not just the far side of a bulge
/// (`contour_distance`).
fn contour_distance(
    grid: &Grid,
    rings: &[Polygon],
    own: usize,
    along: &[f64],
    compensation: f64,
    search_radius: f64,
) -> Vec<f32> {
    let contour = &rings[own].points;
    let count = contour.len();
    let dist_same_contour_accept = 0.5 * compensation * PI;
    let dist_same_contour_reject = search_radius;
    let param_end = along[count];
    let cross2 = |a: V, b: V| a.0 * b.1 - a.1 * b.0;
    // Whether `point` lies on the inner side of the corner at the start
    // of edge `i` of the point's own ring, and of edge `i` itself.
    let inside_corner = |i: usize, point: Point2| {
        let pt_prev = contour[(i + count - 1) % count];
        let pt_this = contour[i];
        let pt_next = contour[(i + 1) % count];
        let v1 = between(pt_prev, pt_this);
        let v2 = between(pt_this, pt_next);
        let left_of_v1 = cross2(v1, between(pt_prev, point)) > 0.0;
        let left_of_v2 = cross2(v2, between(pt_this, point)) > 0.0;
        if cross2(v1, v2) > 0.0 {
            left_of_v1 && left_of_v2
        } else {
            left_of_v1 || left_of_v2
        }
    };
    let left_of_segment = |i: usize, point: Point2| {
        let pt_this = contour[i];
        let pt_next = contour[(i + 1) % count];
        cross2(between(pt_this, pt_next), between(pt_this, point)) > 0.0
    };

    let mut out = Vec::with_capacity(count);
    for (idx_point, &point) in contour.iter().enumerate() {
        // Direction into the part at the point, not normalized.
        let dir_inside = {
            let v1 = between(contour[(idx_point + count - 1) % count], point);
            let v2 = between(point, contour[(idx_point + 1) % count]);
            (-v1.1 - v2.1, v1.0 + v2.0)
        };
        let mut found = false;
        let mut distance = f64::MAX;
        // An edge shown twice changes nothing: the nearest is kept.
        grid.each_near(point, search_radius as Coord, |ring, ipt| {
            let points = &rings[ring].points;
            let first = points[ipt];
            let v = between(first, points[(ipt + 1) % points.len()]);
            let va = between(first, point);
            let l2 = v.0 * v.0 + v.1 * v.1;
            let t = if l2 == 0.0 {
                0.0
            } else {
                ((va.0 * v.0 + va.1 * v.1) / l2).clamp(0.0, 1.0)
            };
            // Closest point of the edge to the point, and the way to it.
            let bisector = (
                first.x as f64 + t * v.0 - point.x as f64,
                first.y as f64 + t * v.1 - point.y as f64,
            );
            let dist = norm(bisector);
            if !((!found || dist < distance)
                && dir_inside.0 * bisector.0 + dir_inside.1 * bisector.1 > 0.0)
            {
                return;
            }
            let mut accept = true;
            if ring == own {
                // The closest edge is of the point's own ring. Reject it
                // unless the way round the ring to it is long compared
                // with the way straight across.
                //
                // Not as the reference, on purpose: it leaves the last
                // edge's own start out of how far round the foot is (a
                // condition kept from ElephantFootCompensation.cpp, where
                // lengths are counted from the ring's last point and not
                // its first), so a point across from the edge that
                // closes a ring is judged as if that edge were where
                // the ring begins.
                let mut param_lo = along[idx_point];
                let mut param_hi = along[ipt] + t * l2.sqrt();
                if param_lo > param_hi {
                    std::mem::swap(&mut param_lo, &mut param_hi);
                }
                let dist_along_contour = (param_hi - param_lo).min(param_lo + param_end - param_hi);
                if dist_along_contour < dist_same_contour_accept {
                    accept = false;
                } else if dist < dist_same_contour_reject + EPSILON as f64 {
                    // The point is close to the foot. It is accepted only
                    // if the outline between them does not just bulge
                    // away from the straight line: the bulge is taken as
                    // 0.6 of a circle drawn on that line.
                    let inside = if t == 0.0 {
                        inside_corner(ipt, point)
                    } else if t == 1.0 {
                        inside_corner((ipt + 1) % count, point)
                    } else {
                        left_of_segment(ipt, point)
                    };
                    accept = inside && dist_along_contour > 0.6 * PI * dist;
                }
            }
            if accept {
                distance = dist;
                found = true;
            }
        });
        out.push(if found {
            distance.min(search_radius)
        } else {
            search_radius
        } as f32);
    }
    out
}

/// A ring wound the other way.
fn reversed(ring: &Polygon) -> Polygon {
    Polygon {
        points: ring.points.iter().rev().copied().collect(),
    }
}

/// A hole as a piece of its own.
fn solid(hole: &Polygon) -> ExPolygon {
    ExPolygon {
        contour: reversed(hole),
        holes: Vec::new(),
    }
}

/// Regions drawn in by `offset_dis`, less where a region is too narrow
/// for that: there it is drawn in by what its width allows, down to
/// nothing, so that no region is lost or parted and none loses a hole.
/// Each is done by itself, so what comes back may overlap
/// (`inner_offset`).
fn inner_offset(regions: &[ExPolygon], offset_dis: f64) -> Result<Vec<ExPolygon>, ClipError> {
    // The widths below which a region is left alone, tried in turn.
    let min_contour_widths = [
        offset_dis / 2.0,
        offset_dis,
        2.0 * offset_dis + EPSILON as f64,
    ];
    let mut out = regions.to_vec();
    for region in &mut out {
        // Holes too small to matter are left out: those that shrinking
        // by 0.1 mm leaves nothing of.
        let mut kept = Vec::with_capacity(region.holes.len());
        for hole in region.holes.drain(..) {
            if !edge_clip::offset(&[solid(&hole)], -scaled(0.1), Join::MITER)?.is_empty() {
                kept.push(hole);
            }
        }
        region.holes = kept;
        resample(&mut region.contour, offset_dis / 2.0, scaled(0.5) as f64);
        for hole in &mut region.holes {
            resample(hole, offset_dis / 2.0, scaled(0.5) as f64);
        }
    }
    for region in &mut out {
        // A ring of two points has no inside to measure.
        if region.rings().any(|ring| ring.points.len() < 3) {
            continue;
        }
        let points = || region.contour.points.iter();
        let span = |axis: fn(&Point2) -> Coord| {
            let low = points().map(axis).min().unwrap_or(0);
            let high = points().map(axis).max().unwrap_or(0);
            (high - low + 2 * EPSILON) as f64
        };
        // Specks are left as they are.
        if span(|p| p.x) * span(|p| p.y) < (scaled(0.1) as f64).powi(2) {
            continue;
        }
        let rings: Vec<Polygon> = region.rings().cloned().collect();
        let along: Vec<Vec<f64>> = rings
            .iter()
            .map(|ring| lengths_along(&ring.points))
            .collect();
        for (attempt, min_contour_width) in min_contour_widths.into_iter().enumerate() {
            let search_radius = 2.0 * (offset_dis + min_contour_width);
            let grid = Grid::new(&rings, (0.7 * search_radius) as Coord);
            let offsets: Vec<Vec<f32>> = (0..rings.len())
                .map(|ring| {
                    let mut distances = contour_distance(
                        &grid,
                        &rings,
                        ring,
                        &along[ring],
                        offset_dis,
                        search_radius,
                    );
                    for distance in &mut distances {
                        let width = f64::from(*distance);
                        *distance = if width < min_contour_width {
                            0.0
                        } else if width > min_contour_width + 2.0 * offset_dis {
                            -(offset_dis as f32)
                        } else {
                            -(*distance - min_contour_width as f32) / 2.0
                        };
                    }
                    distances
                })
                .collect();
            let mut drawn_in = edge_clip::variable_offset_inner(region, &offsets, 2.0)?;
            let whole = drawn_in.len() == 1 && drawn_in[0].holes.len() == region.holes.len();
            if !whole && attempt + 1 < min_contour_widths.len() {
                // Try again, leaving more of the region alone.
                continue;
            }
            // Of several pieces the largest is the region; the others
            // are slivers the offset left. With nothing left at all the
            // region stays as it was.
            let largest = (0..drawn_in.len()).reduce(|best, index| {
                if drawn_in[index].area_mm2() > drawn_in[best].area_mm2() {
                    index
                } else {
                    best
                }
            });
            if let Some(largest) = largest {
                *region = drawn_in.swap_remove(largest);
            }
            break;
        }
    }
    Ok(out)
}

/// A layer's outline drawn in by half as much again as its walls are
/// spaced, which is where a move inside the part runs: inside the outer
/// wall and along the one behind it (`get_boundary`, before top surfaces
/// are taken out). `spacing` is in scaled units.
pub(crate) fn inside(outline: &[ExPolygon], spacing: f64) -> Result<Vec<ExPolygon>, ClipError> {
    edge_clip::union_ex(&inner_offset(outline, 1.5 * spacing)?)
}

/// `inside` less the layer's top surfaces, which a move goes round and
/// not over, so that it leaves no mark on them (`get_boundary`).
pub(crate) fn clear_of_tops(
    inside: &[ExPolygon],
    tops: &[ExPolygon],
    spacing: f64,
) -> Result<Vec<ExPolygon>, ClipError> {
    let tops = edge_clip::union_ex(tops)?;
    let tops = edge_clip::offset(&tops, (-1.2 * (spacing / 2.0)) as Coord, Join::MITER)?;
    edge_clip::difference(inside, &tops)
}

/// What a move between parts goes round: every hole of the layer, made
/// larger by half the spacing of the walls so that the way round it
/// lies in the hole's own wall, and wound so that the part is on the
/// left of it (`get_boundary_external`).
pub(crate) fn holes(outline: &[ExPolygon], spacing: f64) -> Result<Vec<Polygon>, ClipError> {
    let holes: Vec<ExPolygon> = outline
        .iter()
        .flat_map(|region| &region.holes)
        .map(solid)
        .collect();
    if holes.is_empty() {
        return Ok(Vec::new());
    }
    let grown = edge_clip::offset(&holes, (spacing / 2.0) as Coord, Join::MITER)?;
    Ok(grown
        .iter()
        .flat_map(ExPolygon::rings)
        .map(reversed)
        .collect())
}

/// The outline drawn in to a little behind the middle of the outer
/// wall: a move that stays within one piece of it crosses no wall and
/// goes straight. Where that leaves nothing of a thin layer it is drawn
/// in by less (`AvoidCrossingPerimeters::init_layer`). `outer_width` is
/// in scaled units.
pub(crate) fn within_walls(
    outline: &[ExPolygon],
    outer_width: f64,
) -> Result<Vec<ExPolygon>, ClipError> {
    for share in [0.6, 0.5, 0.45] {
        let drawn_in = edge_clip::offset(outline, (-outer_width * share) as Coord, Join::MITER)?;
        if !drawn_in.is_empty() {
            return Ok(drawn_in);
        }
    }
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn a_ring_is_resampled_beside_its_corners_and_along_its_edges() {
        let mut ring = rect(0.0, 0.0, 2.0, 1.0);
        resample(&mut ring, scaled(0.3) as f64, scaled(0.5) as f64);
        // The long edges: the corner, 0.3 mm on, three more no further
        // apart than 0.5 mm, and 0.3 mm short of the next corner. The
        // short ones have room for the two beside the corners only.
        let xs: Vec<f64> = ring.points[..6]
            .iter()
            .map(|p| edge_geom::unscaled(p.x))
            .collect();
        assert_eq!(xs.len(), 6);
        for (x, wanted) in xs
            .iter()
            .zip([0.0, 0.3, 0.3 + 1.4 / 3.0, 0.3 + 2.8 / 3.0, 1.7, 2.0])
        {
            assert!((x - wanted).abs() < 1e-5, "{xs:?}");
        }
        assert_eq!(ring.points.len(), 2 * 5 + 2 * 3);
        assert!(
            (Polygon {
                points: ring.points
            }
            .signed_area_mm2()
                - 2.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn a_wide_region_is_drawn_in_all_round_and_keeps_its_hole() {
        let mut hole = rect(8.0, 8.0, 12.0, 12.0);
        hole.points.reverse();
        let frame = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![hole],
        };
        let spacing = scaled(0.4) as f64;
        let drawn_in = inside(&[frame], spacing).unwrap();
        assert_eq!(drawn_in.len(), 1);
        assert_eq!(drawn_in[0].holes.len(), 1);
        // In by 0.6 mm outside, and the hole larger by as much.
        assert!((area(&drawn_in) - (18.8 * 18.8 - 5.2 * 5.2)).abs() < 0.05);
    }

    #[test]
    fn a_narrow_neck_is_kept_and_a_small_hole_dropped() {
        // Two 10 mm squares joined by a neck 0.8 mm wide, too narrow to
        // be drawn in by 0.6 mm a side: the region stays one piece.
        let dumbbell = ExPolygon {
            contour: Polygon {
                points: [
                    (0.0, 0.0),
                    (10.0, 0.0),
                    (10.0, 4.6),
                    (15.0, 4.6),
                    (15.0, 0.0),
                    (25.0, 0.0),
                    (25.0, 10.0),
                    (15.0, 10.0),
                    (15.0, 5.4),
                    (10.0, 5.4),
                    (10.0, 10.0),
                    (0.0, 10.0),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        };
        let spacing = scaled(0.4) as f64;
        let drawn_in = inside(std::slice::from_ref(&dumbbell), spacing).unwrap();
        assert_eq!(drawn_in.len(), 1, "the neck parted the region");
        assert!(area(&drawn_in) < dumbbell.area_mm2());
        // A plain offset parts it.
        let plain = edge_clip::offset(&[dumbbell], -(1.5 * spacing) as Coord, Join::MITER).unwrap();
        assert_eq!(plain.len(), 2);

        // A hole 0.15 mm across is not gone round.
        let mut pin = rect(5.0, 5.0, 5.15, 5.15);
        pin.points.reverse();
        let pricked = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: vec![pin],
        };
        let drawn_in = inside(&[pricked], spacing).unwrap();
        assert_eq!(drawn_in.len(), 1);
        assert!(drawn_in[0].holes.is_empty());
    }

    #[test]
    fn top_surfaces_are_taken_out_and_holes_grown_for_moves_between_parts() {
        let mut hole = rect(8.0, 8.0, 12.0, 12.0);
        hole.points.reverse();
        let frame = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![hole],
        };
        let spacing = scaled(0.4) as f64;
        let drawn_in = inside(std::slice::from_ref(&frame), spacing).unwrap();
        let top = ExPolygon {
            contour: rect(2.0, 2.0, 6.0, 6.0),
            holes: Vec::new(),
        };
        let cut = clear_of_tops(&drawn_in, &[top], spacing).unwrap();
        // The top surface less 0.24 mm all round is gone from it.
        assert!((area(&drawn_in) - area(&cut) - 3.52 * 3.52).abs() < 0.01);
        assert_eq!(cut[0].holes.len(), 2);

        let round = holes(&[frame], spacing).unwrap();
        assert_eq!(round.len(), 1);
        // 0.2 mm larger all round, and clockwise: the part is on its left.
        assert!((round[0].signed_area_mm2() + 4.4 * 4.4).abs() < 0.01);

        // A layer too thin to be drawn in by 0.6 of its outer wall is
        // drawn in by less.
        let sliver = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 0.45),
            holes: Vec::new(),
        };
        let width = scaled(0.42) as f64;
        assert!(!within_walls(&[sliver], width).unwrap().is_empty());
    }
}
