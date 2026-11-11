//! Walls over air: where the fill area of a layer hangs over nothing and
//! is too wide, or too poorly held, to be bridged, it is covered with
//! lines that follow its outline inward from whatever holds it up, each
//! one laid against the one before (`extra_perimeters_on_overhangs`).
//!
//! Port of generate_extra_perimeters_over_overhangs, PerimeterGenerator.cpp,
//! with its helpers `paths_touch`, `reconnect_polylines` and
//! `sort_extra_perimeters`, and the unsupported distance of
//! `detect_bridging_direction` (BridgeDetector.hpp).
//!
//! The clipper here is not the reference's, so regions and the pieces of
//! a cut line come out in another order and lines may start at another
//! corner; what is covered is the same.

use std::collections::{BTreeMap, BTreeSet};

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Coord, ExPolygon, Point2, scaled};

use crate::Error;

/// How far, in mm, fill over air reaches onto what holds it up
/// (libslic3r.h `EXTERNAL_INFILL_MARGIN`).
const EXTERNAL_INFILL_MARGIN: f64 = 3.0;
/// The reference's `SCALED_EPSILON`: lengths below it count as nothing.
const EPSILON: Coord = 100;

/// The points of an open line.
type Line = Vec<Point2>;

/// `infill_area`: where fill would go on this layer (inside the walls).
/// `lower`: the layer below's outlines already grown by half the nozzle
/// diameter (the reference's `lower_slices_polygons()`).
/// `wall_loops`: the configured wall count (`perimeter_count`).
/// `flow`: the overhang (bridging) flow the lines are laid with.
///
/// Returns, per overhang region, its lines in the order they are to be
/// laid (role `Role::OverhangPerimeter`, `in_turn: true`, `part: 0`), and
/// the area those lines fill (to be taken out of the fill). As in the
/// reference, a region that is left to be bridged has an entry with no
/// lines in it.
///
/// A gap whose centre line cannot be found is left without its line and
/// not reported; [`over_overhangs_counting`] counts them.
#[cfg(test)]
pub(crate) fn over_overhangs(
    infill_area: &[ExPolygon],
    lower: &[ExPolygon],
    wall_loops: usize,
    flow: &Flow,
) -> Result<(Vec<Vec<ExtrusionPath>>, Vec<ExPolygon>), Error> {
    over_overhangs_counting(infill_area, lower, wall_loops, flow, &mut 0)
}

/// [`over_overhangs`], adding to `gaps_skipped` every gap of one line's
/// width whose centre line could not be found, as `Perimeters::gaps_skipped`
/// counts those between walls.
pub(crate) fn over_overhangs_counting(
    infill_area: &[ExPolygon],
    lower: &[ExPolygon],
    wall_loops: usize,
    flow: &Flow,
    gaps_skipped: &mut usize,
) -> Result<(Vec<Vec<ExtrusionPath>>, Vec<ExPolygon>), Error> {
    // The reference truncates these two to whole units from an `f32`;
    // this crate's flows round them from an `f64`, here as everywhere.
    let spacing = flow.scaled_spacing();
    let width = flow.scaled_width();
    let anchors_size = scaled(EXTERNAL_INFILL_MARGIN).min(spacing * (wall_loops as Coord + 1));
    let spacing = spacing as f64;

    // The reference first cuts the layer below down to the fill area's
    // bounds (`clip_clipper_polygons_with_subject_bbox`). That only saves
    // it time, and is left out.
    let overhangs = edge_clip::difference(infill_area, lower)?;
    if overhangs.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let below = Outline::new(lower);
    let anchors = edge_clip::intersection(infill_area, lower)?;
    // The one offset with squared corners (`EXTRA_PERIMETER_OFFSET_PARAMETERS`);
    // every other one below has the reference's default, mitred.
    let inset_anchors = edge_clip::difference(
        &anchors,
        &edge_clip::offset(
            &overhangs,
            whole(anchors_size as f64 + 0.1 * width as f64),
            Join::Square,
        )?,
    )?;
    let inset_overhang_area = edge_clip::difference(infill_area, &inset_anchors)?;

    let mut left_unfilled: Vec<ExPolygon> = Vec::new();
    let mut extra: Vec<Vec<ExtrusionPath>> = Vec::new();
    for overhang in edge_clip::union_ex(&inset_overhang_area)? {
        let to_cover = [overhang];
        let expanded = edge_clip::offset(&to_cover, whole(1.1 * spacing), Join::MITER)?;
        let shrunk = edge_clip::offset(&to_cover, -whole(0.1 * spacing), Join::MITER)?;

        let real_overhang = edge_clip::intersection(&to_cover, &overhangs)?;
        if real_overhang.is_empty() {
            left_unfilled.extend(to_cover);
            continue;
        }
        let mut region: Vec<ExtrusionPath> = Vec::new();

        let anchoring = edge_clip::intersection(&expanded, &inset_anchors)?;
        let mut perimeter_polygon = edge_clip::offset(
            &united(
                &edge_clip::offset(&to_cover, whole(0.1 * spacing), Join::MITER)?,
                &anchoring,
            )?,
            -whole(spacing * 0.6),
            Join::MITER,
        )?;

        let hull = edge_geom::convex_hull(
            anchoring
                .iter()
                .flat_map(ExPolygon::rings)
                .flat_map(|ring| ring.points.iter().copied()),
        );
        let hull = if hull.points.len() < 3 {
            Vec::new()
        } else {
            vec![ExPolygon {
                contour: hull,
                holes: Vec::new(),
            }]
        };
        let unbridgeable_area = area(&edge_clip::difference(&real_overhang, &hull)?);
        let unsupported = unsupported_distance(&real_overhang, &anchors)?;

        if unbridgeable_area < 0.2 * area(&real_overhang)
            && unsupported < outline_length(&real_overhang) * 0.2
        {
            // Held well enough on enough sides: left to be bridged.
            left_unfilled.extend(to_cover);
        } else {
            // Fill the overhang with lines.
            let mut continuation_loops = 2;
            while continuation_loops >= 0 {
                let prev = perimeter_polygon.clone();
                // The next lines, not added yet: if nothing is left once
                // the outline is stepped inward, the area may have been
                // too small for a whole loop, and a line down its middle
                // is tried first.
                let perimeter = edge_clip::clip_lines(&outlines(&perimeter_polygon), &shrunk)?;

                perimeter_polygon = united(&perimeter_polygon, &anchoring)?;
                perimeter_polygon = edge_clip::intersection(
                    &edge_clip::offset(&perimeter_polygon, -whole(spacing), Join::MITER)?,
                    &expanded,
                )?;

                if perimeter_polygon.is_empty() {
                    // Fill what gaps of one line's width there may be.
                    let shrinked = edge_clip::intersection(
                        &edge_clip::offset(&prev, -whole(0.3 * spacing), Join::MITER)?,
                        &expanded,
                    )?;
                    if !shrinked.is_empty() {
                        lay(&mut region, reconnect(perimeter, spacing), flow);
                    }
                    let gap = if shrinked.is_empty() {
                        edge_clip::offset(&prev, whole(spacing * 0.5), Join::MITER)?
                    } else {
                        shrinked
                    };
                    let mut fills: Vec<Line> = Vec::new();
                    for part in &gap {
                        match edge_medial::medial_axis(part, 0.75 * width as f64, 3.0 * spacing) {
                            Ok(found) => fills.extend(found.into_iter().map(|line| line.points)),
                            Err(_) => *gaps_skipped += 1,
                        }
                    }
                    if !fills.is_empty() {
                        let fills = edge_clip::clip_lines(&fills, &shrunk)?;
                        lay(&mut region, reconnect(fills, spacing), flow);
                    }
                    break;
                }
                lay(&mut region, reconnect(perimeter, spacing), flow);

                if edge_clip::intersection(&perimeter_polygon, &real_overhang)?.is_empty() {
                    continuation_loops -= 1;
                }
                if prev == perimeter_polygon {
                    break;
                }
            }

            perimeter_polygon =
                edge_clip::offset(&perimeter_polygon, whole(0.5 * spacing), Join::MITER)?;
            perimeter_polygon = united(&perimeter_polygon, &anchoring)?;
            left_unfilled.extend(perimeter_polygon);

            region.retain(|path| !path.points.is_empty());
            if !region.is_empty() {
                // When the first line eats all the room over the anchor it
                // is a closed loop, and looking at its two ends, as is
                // done for the others, says nothing about it.
                let front = &region[0].points;
                let first_is_closed_and_anchored = front.first() == front.last()
                    && !edge_clip::clip_lines(std::slice::from_ref(front), lower)?.is_empty();
                if !first_is_closed_and_anchored {
                    region.reverse();
                } else {
                    // The loop is begun at its point furthest onto the
                    // layer below, and stays a loop.
                    //
                    // Not as the reference, on purpose: it means this
                    // and writes `if (double d = distance < min_dist)`,
                    // which gives `d` the outcome of the comparison and
                    // not the distance, so it begins at the last point
                    // that is over the layer below at all; and it turns
                    // the points about without taking out the one that
                    // closed the loop, which leaves that point twice in
                    // the middle and the loop open where it starts.
                    let front = &mut region[0].points;
                    front.pop();
                    let mut start = 0;
                    let mut min_dist = f64::MAX;
                    for (index, &point) in front.iter().enumerate() {
                        let distance = below.signed_distance(point);
                        if distance < min_dist {
                            min_dist = distance;
                            start = index;
                        }
                    }
                    front.rotate_left(start);
                    front.extend(front.first().copied());
                }
                // The lines with an end over the layer below first, each
                // lot in the order it has (`std::stable_partition`).
                let (mut ordered, unanchored): (Vec<_>, Vec<_>) =
                    region.into_iter().partition(|path| {
                        ends(&path.points).is_some_and(|(first, last)| {
                            below.signed_distance(first) <= 0.0
                                || below.signed_distance(last) <= 0.0
                        })
                    });
                let first_unanchored = ordered.len();
                ordered.extend(unanchored);
                region = sort(&ordered, first_unanchored, spacing);
            }
        }
        extra.push(region);
    }

    let left_unfilled = edge_clip::union_ex(&left_unfilled)?;
    let filled = edge_clip::difference(&inset_overhang_area, &left_unfilled)?;
    Ok((extra, filled))
}

/// An offset distance as a whole number of units. The reference hands its
/// clipper these distances as `float`s, fractions and all; this one takes
/// whole units, so each goes to the nearest.
fn whole(distance: f64) -> Coord {
    distance.round() as Coord
}

/// Everything either set of regions covers (`union_` of two sets).
fn united(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Vec<ExPolygon>, ClipError> {
    edge_clip::union_ex(&[a, b].concat())
}

/// Area in mm², holes taken off (`area` of polygons).
fn area(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

/// The length of every ring, in scaled units (`total_length`).
fn outline_length(regions: &[ExPolygon]) -> f64 {
    regions
        .iter()
        .flat_map(ExPolygon::rings)
        .map(|ring| {
            let points = &ring.points;
            match (points.first(), points.last()) {
                (Some(&first), Some(&last)) if points.len() > 1 => {
                    distance_sq(last, first).sqrt()
                        + points
                            .windows(2)
                            .map(|pair| distance_sq(pair[0], pair[1]).sqrt())
                            .sum::<f64>()
                }
                _ => 0.0,
            }
        })
        .sum()
}

/// Every ring as a line that ends where it began (`to_polylines`).
fn outlines(regions: &[ExPolygon]) -> Vec<Line> {
    regions
        .iter()
        .flat_map(ExPolygon::rings)
        .filter_map(|ring| {
            let first = *ring.points.first()?;
            let mut line = ring.points.clone();
            line.push(first);
            Some(line)
        })
        .collect()
}

/// Adds the lines with a length to them to a region's paths
/// (`extrusion_paths_append`).
fn lay(region: &mut Vec<ExtrusionPath>, lines: Vec<Line>, flow: &Flow) {
    region.extend(
        lines
            .into_iter()
            .filter(|line| line.len() >= 2)
            .map(|points| ExtrusionPath {
                points,
                role: Role::OverhangPerimeter,
                flow: *flow,
                in_turn: true,
                part: 0,
            }),
    );
}

fn ends(line: &[Point2]) -> Option<(Point2, Point2)> {
    Some((*line.first()?, *line.last()?))
}

fn distance_sq(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
    dx * dx + dy * dy
}

/// The square of the distance from a point to a segment
/// (`line_alg::distance_to_squared`).
fn distance_to_segment_sq(p: Point2, a: Point2, b: Point2) -> f64 {
    let (vx, vy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let (px, py) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
    let length_sq = vx * vx + vy * vy;
    if length_sq == 0.0 {
        return px * px + py * py;
    }
    let along = (px * vx + py * vy) / length_sq;
    if along <= 0.0 {
        px * px + py * py
    } else if along >= 1.0 {
        distance_sq(p, b)
    } else {
        let (ox, oy) = (px - along * vx, py - along * vy);
        ox * ox + oy * oy
    }
}

/// The distance from a point to the nearest of `segments`; infinite when
/// there are none, as the reference's `LinesDistancer` has it.
fn distance_to_segments(p: Point2, segments: impl Iterator<Item = (Point2, Point2)>) -> f64 {
    segments
        .map(|(a, b)| distance_to_segment_sq(p, a, b))
        .fold(f64::INFINITY, f64::min)
        .sqrt()
}

/// The edges of the layer below, to measure against: the reference's
/// `AABBTreeLines::LinesDistancer` over `to_lines` of its outlines. It
/// keeps them in a tree to find the nearest one fast; they are gone
/// through one by one here, which gives the same answers.
struct Outline {
    edges: Vec<(Point2, Point2)>,
}

impl Outline {
    fn new(regions: &[ExPolygon]) -> Self {
        let edges = regions
            .iter()
            .flat_map(ExPolygon::rings)
            .filter(|ring| ring.points.len() >= 2)
            .flat_map(|ring| {
                let points = &ring.points;
                (0..points.len())
                    .map(move |index| (points[index], points[(index + 1) % points.len()]))
            })
            .collect();
        Self { edges }
    }

    /// The distance from a point to the nearest edge, negative for a
    /// point inside the outlines and zero for one that cannot be said to
    /// be in or out (`distance_from_lines<true>`).
    fn signed_distance(&self, point: Point2) -> f64 {
        let distance = distance_to_segments(point, self.edges.iter().copied());
        if distance.is_infinite() {
            return distance;
        }
        distance * f64::from(self.outside(point))
    }

    /// 1 for a point outside the outlines, -1 for one inside, 0 for one
    /// on an edge or that cannot be told (`point_outside_closed_contours`):
    /// by how many edges lie to either side of it, first along x and,
    /// if the two sides disagree, along y.
    fn outside(&self, point: Point2) -> i8 {
        if self.edges.is_empty() {
            return 1;
        }
        for along_y in [false, true] {
            let Some((above, below)) = self.ray_hits(point, along_y) else {
                return 0;
            };
            match (above % 2, below % 2) {
                (1, 1) => return -1,
                (0, 0) => return 1,
                _ => {}
            }
        }
        0
    }

    /// How many edges a line through `point` along one axis crosses on
    /// either side of it, or nothing when the point is on an edge
    /// (`coordinate_aligned_ray_hit_count`).
    fn ray_hits(&self, point: Point2, along_y: bool) -> Option<(u32, u32)> {
        // The coordinate along the line first, then the one across it.
        let split = |p: Point2| if along_y { (p.y, p.x) } else { (p.x, p.y) };
        let (at, across) = split(point);
        let (mut above, mut below) = (0, 0);
        for &(a, b) in &self.edges {
            let ((a_at, a_across), (b_at, b_across)) = (split(a), split(b));
            // Not below the upper end, so that where two edges meet on
            // the line only one of them is counted.
            if across < a_across.min(b_across) || across >= a_across.max(b_across) {
                continue;
            }
            if at > a_at.max(b_at) {
                above += 1;
            } else if at < a_at.min(b_at) {
                below += 1;
            } else {
                let t = (across - a_across) as f64 / (b_across - a_across) as f64;
                let crossing = a_at as f64 + t * (b_at - a_at) as f64;
                if at as f64 > crossing {
                    above += 1;
                } else if (at as f64) < crossing {
                    below += 1;
                } else {
                    return None;
                }
            }
        }
        Some((above, below))
    }
}

/// How much line would end in the air if the overhang were bridged the
/// best way round, in scaled units: the second value of the reference's
/// `detect_bridging_direction(to_cover, anchors_area)` (BridgeDetector.hpp).
/// `edge-infill` holds the other copy of that function (`bridge_direction`
/// in surfaces.rs), which wants the direction and not this distance; the
/// two crates do not depend on each other.
fn unsupported_distance(to_cover: &[ExPolygon], anchors: &[ExPolygon]) -> Result<f64, ClipError> {
    let overhang_area = edge_clip::difference(to_cover, anchors)?;
    let floating = edge_clip::lines_outside(
        &outlines(&overhang_area),
        &edge_clip::offset(anchors, EPSILON, Join::MITER)?,
    )?;
    let edges: Vec<(f64, f64)> = floating
        .iter()
        .flat_map(|line| line.windows(2))
        .map(|pair| {
            (
                (pair[1].x - pair[0].x) as f64,
                (pair[1].y - pair[0].y) as f64,
            )
        })
        .collect();
    if edges.is_empty() {
        // Held all the way round.
        return Ok(0.0);
    }
    // Each edge's normal is a way the bridge might be crossed; one is
    // kept for every thousandth of a radian, the first met. The reference
    // keeps them in a hash map, whose order decides nothing here: only
    // the least cost is wanted, not which direction has it.
    let mut directions: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for &(dx, dy) in &edges {
        let length = dx.hypot(dy);
        // An edge without length keeps a normal without length, as
        // Eigen's `normalized` leaves it.
        let normal = if length > 0.0 {
            (dy / length, -dx / length)
        } else {
            (dy, -dx)
        };
        let quantized = (normal.1.atan2(normal.0) * 1000.0).ceil() as i64;
        directions.entry(quantized).or_insert(normal);
    }
    // What it costs to bridge at right angles to each: the length of
    // floating edge that the lines' ends would then hang from.
    Ok(directions
        .values()
        .map(|normal| {
            edges
                .iter()
                .map(|(dx, dy)| (dx * normal.0 + dy * normal.1).abs())
                .sum::<f64>()
        })
        .fold(f64::MAX, f64::min))
}

/// Joins lines whose ends are nearer than `limit_distance`, as the pieces
/// of an outline cut where it began are (`reconnect_polylines`).
///
/// The reference keeps the lines in a hash map by their index and returns
/// them in whatever order that map hands them out, which differs between
/// standard libraries. Here they come back by index, lowest first.
fn reconnect(lines: Vec<Line>, limit_distance: f64) -> Vec<Line> {
    let limit_sq = limit_distance * limit_distance;
    let mut connected: Vec<Option<Line>> = lines
        .into_iter()
        .map(|line| (!line.is_empty()).then_some(line))
        .collect();
    for a in 0..connected.len() {
        let (upto, after) = connected.split_at_mut(a + 1);
        let Some(mut base) = upto[a].take() else {
            continue;
        };
        for later in after {
            let Some(mut next) = later.take() else {
                continue;
            };
            // Neither is empty: empty lines were left out above.
            let (Some((base_first, base_last)), Some((next_first, next_last))) =
                (ends(&base), ends(&next))
            else {
                *later = Some(next);
                continue;
            };
            if distance_sq(base_last, next_first) < limit_sq {
                append(&mut base, &next);
            } else if distance_sq(base_last, next_last) < limit_sq {
                base.extend(next.iter().rev());
            } else if distance_sq(base_first, next_last) < limit_sq {
                append(&mut next, &base);
                base = next;
                base.reverse();
            } else if distance_sq(base_first, next_first) < limit_sq {
                base.reverse();
                append(&mut base, &next);
                base.reverse();
            } else {
                *later = Some(next);
            }
        }
        upto[a] = Some(base);
    }
    connected.into_iter().flatten().collect()
}

/// Runs `more` on from the end of `line`, its first point left out where
/// the two meet in it. A single point is not a line and adds nothing
/// (`Polyline::append`).
fn append(line: &mut Line, more: &[Point2]) {
    if more.len() < 2 {
        return;
    }
    if line.last() != more.first() {
        line.push(more[0]);
    }
    line.extend_from_slice(&more[1..]);
}

/// Whether two paths touch: a point of either lies within
/// `limit_distance` of the other (`paths_touch`).
fn touch(one: &ExtrusionPath, two: &ExtrusionPath, limit_distance: f64) -> bool {
    // Not in the reference: paths whose bounds are that far apart cannot
    // touch, and need not be measured point by point.
    let bounds = |path: &ExtrusionPath| {
        path.points.iter().fold(
            (Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN),
            |(x0, y0, x1, y1), p| (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        )
    };
    let (a, b) = (bounds(one), bounds(two));
    let gap_x = (a.0.max(b.0) as f64) - (a.2.min(b.2) as f64);
    let gap_y = (a.1.max(b.1) as f64) - (a.3.min(b.3) as f64);
    if gap_x >= limit_distance || gap_y >= limit_distance {
        return false;
    }
    let near = |from: &ExtrusionPath, to: &ExtrusionPath| {
        from.points.iter().any(|&point| {
            let segments = to.points.windows(2).map(|pair| (pair[0], pair[1]));
            distance_to_segments(point, segments) < limit_distance
        })
    };
    near(one, two) || near(two, one)
}

/// Puts a region's lines in the order they are laid, joins those that
/// run on from one another and drops what is left too short to print
/// (`sort_extra_perimeters`). The first `first_unanchored` paths have an
/// end over the layer below. A path waits for the paths before it that
/// it touches; among the unanchored ones that waiting is first turned
/// round to run outward from the anchored ones, so that each is laid
/// against one already down.
///
/// The reference keeps what each path waits for in hash sets. They are
/// ordered sets here, though nothing reads them in order: they are asked
/// only whether they are empty or hold a path that is done, and are
/// changed one member at a time by rules that take each member alone.
fn sort(
    paths: &[ExtrusionPath],
    first_unanchored: usize,
    extrusion_spacing: f64,
) -> Vec<ExtrusionPath> {
    let Some(mut current_point) = paths.first().and_then(|path| path.points.first().copied())
    else {
        return Vec::new();
    };
    let count = paths.len();

    // To begin with a path waits for every earlier one it touches.
    let mut dependencies: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    for path in 0..count {
        for prev in 0..path {
            if touch(&paths[path], &paths[prev], extrusion_spacing * 1.5) {
                dependencies[path].insert(prev);
            }
        }
    }

    // Working outward from the anchored paths: a path that touches one
    // already reached is reached itself, and the paths it was waiting for
    // that are not reached yet are made to wait for it instead.
    let mut processed = vec![false; count];
    for done in processed.iter_mut().take(first_unanchored) {
        *done = true;
    }
    for _ in first_unanchored..count {
        let mut change = false;
        for path in first_unanchored..count {
            if processed[path] {
                continue;
            }
            if dependencies[path].iter().any(|&dep| processed[dep]) {
                let unreached: Vec<usize> = dependencies[path]
                    .iter()
                    .copied()
                    .filter(|&dep| !processed[dep])
                    .collect();
                for dep in unreached {
                    dependencies[dep].insert(path);
                    dependencies[path].remove(&dep);
                }
                processed[path] = true;
                change = true;
            }
        }
        if !change {
            break;
        }
    }

    // The path waiting for nothing whose end is nearest, which end that
    // is, and the square of how far. The first found wins among equals.
    // A path that has been laid is out of it: the reference gives it a
    // dependency that is never met.
    let nearest = |from: Point2, dependencies: &[BTreeSet<usize>], laid: &[bool]| {
        let mut dist = f64::MAX;
        let mut found = None;
        for (index, path) in paths.iter().enumerate() {
            if laid[index] || !dependencies[index].is_empty() {
                continue;
            }
            let Some((first, last)) = ends(&path.points) else {
                continue;
            };
            let dist_a = distance_sq(first, from);
            if dist_a < dist {
                dist = dist_a;
                found = Some((index, false));
            }
            let dist_b = distance_sq(last, from);
            if dist_b < dist {
                dist = dist_b;
                found = Some((index, true));
            }
        }
        (found, dist)
    };

    let mut sorted: Vec<ExtrusionPath> = Vec::new();
    let mut laid = vec![false; count];
    let mut next: Option<(usize, bool)> = None;
    loop {
        match next {
            None => {
                next = nearest(current_point, &dependencies, &laid).0;
                if next.is_none() {
                    break;
                }
            }
            Some((index, reverse)) => {
                let mut path = paths[index].clone();
                if reverse {
                    path.points.reverse();
                }
                if let Some(&last) = path.points.last() {
                    current_point = last;
                }
                sorted.push(path);
                laid[index] = true;
                for waiting in &mut dependencies {
                    waiting.remove(&index);
                }
                // The reference carries straight on to a path that starts
                // where this one ended and otherwise looks again, for the
                // same thing with no limit. The limit is a length, 5 mm,
                // which it compares with the square of the distance.
                let (found, dist) = nearest(current_point, &dependencies, &laid);
                next = if dist > scaled(5.0) as f64 {
                    None
                } else {
                    found
                };
            }
        }
    }

    // A path that starts within two spacings of the end of the one before
    // runs on from it.
    let mut reconnected: Vec<ExtrusionPath> = Vec::with_capacity(sorted.len());
    for path in sorted {
        match (reconnected.last_mut(), path.points.first()) {
            (Some(before), Some(&first))
                if before.points.last().is_some_and(|&last| {
                    distance_sq(last, first) < extrusion_spacing * extrusion_spacing * 4.0
                }) =>
            {
                before.points.extend(path.points);
            }
            _ => reconnected.push(path),
        }
    }

    reconnected.retain(|path| length(&path.points) > 3.0 * extrusion_spacing);
    reconnected
}

/// Length in scaled units.
fn length(line: &[Point2]) -> f64 {
    line.windows(2)
        .map(|pair| distance_sq(pair[0], pair[1]).sqrt())
        .sum()
}

#[cfg(test)]
mod tests {
    use edge_geom::{Polygon, unscaled};

    use super::*;

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    /// The round thread a 0.4 mm nozzle bridges with.
    fn flow() -> Flow {
        Flow::new(0.45, 0.2, 0.4).unwrap().bridging(1.0, true)
    }

    fn line(points: &[(f64, f64)]) -> Line {
        points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect()
    }

    fn path(points: &[(f64, f64)]) -> ExtrusionPath {
        ExtrusionPath {
            points: line(points),
            role: Role::OverhangPerimeter,
            flow: flow(),
            in_turn: true,
            part: 0,
        }
    }

    /// A 20 mm square of fill held up on the left as far as `held` mm.
    fn cantilever(held: f64) -> (Vec<Vec<ExtrusionPath>>, Vec<ExPolygon>) {
        over_overhangs(
            &[block(0.0, 0.0, 20.0, 20.0)],
            &[block(-5.0, -5.0, held, 25.0)],
            2,
            &flow(),
        )
        .unwrap()
    }

    fn total_mm(regions: &[Vec<ExtrusionPath>]) -> f64 {
        regions.iter().flatten().map(ExtrusionPath::length_mm).sum()
    }

    #[test]
    fn fill_with_the_layer_below_under_all_of_it_gets_nothing() {
        let (lines, filled) = over_overhangs(
            &[block(0.0, 0.0, 20.0, 20.0)],
            &[block(-5.0, -5.0, 25.0, 25.0)],
            2,
            &flow(),
        )
        .unwrap();
        assert!(lines.is_empty());
        assert!(filled.is_empty());
    }

    #[test]
    fn a_cantilever_is_covered_with_lines_from_its_support_outward() {
        let infill = [block(0.0, 0.0, 20.0, 20.0)];
        let (lines, filled) = cantilever(10.0);
        assert_eq!(lines.len(), 1);
        let region = &lines[0];
        assert!(!region.is_empty());
        for path in region {
            assert_eq!(
                (path.role, path.in_turn, path.part),
                (Role::OverhangPerimeter, true, 0)
            );
            assert_eq!(path.flow, flow());
            assert!(path.points.len() >= 2);
            // Inside the fill area.
            for point in &path.points {
                let (x, y) = (unscaled(point.x), unscaled(point.y));
                assert!((0.0..=20.0).contains(&x) && (0.0..=20.0).contains(&y));
            }
        }
        // Mostly over the air, reaching back onto the support only to
        // hold on to it.
        let over_air: f64 = region
            .iter()
            .flat_map(|path| path.points.windows(2))
            .filter(|pair| unscaled(pair[0].x + pair[1].x) / 2.0 >= 10.0)
            .map(|pair| distance_sq(pair[0], pair[1]).sqrt() / 1e6)
            .sum();
        let all = total_mm(&lines);
        assert!(over_air > 0.8 * all, "{over_air} of {all} mm over air");
        // Lines a spacing apart over 10 x 20 mm come to its area over
        // the spacing, more or less.
        let expected = 200.0 / flow().spacing;
        assert!(all > 0.8 * expected && all < 1.3 * expected, "{all} mm");

        // The first line laid holds on to the support.
        let first = &region[0].points;
        assert!(
            unscaled(first[0].x) < 10.0 || unscaled(first[first.len() - 1].x) < 10.0,
            "the first line starts and ends over air"
        );

        // What is filled lies in the fill area and takes in the overhang.
        assert!(area(&filled) > 200.0 && area(&filled) < 240.0);
        assert!(edge_clip::difference(&filled, &infill).unwrap().is_empty());
        let missed = edge_clip::difference(&[block(10.0, 0.0, 20.0, 20.0)], &filled).unwrap();
        assert!(area(&missed) < 0.01);
    }

    #[test]
    fn a_loop_that_crosses_its_support_is_laid_first_and_starts_over_it() {
        // Held on a strip too narrow to leave an anchor area: the first
        // line goes right round the fill, over the strip on its way.
        let (lines, filled) = cantilever(1.0);
        assert_eq!(lines.len(), 1);
        let first = &lines[0][0].points;
        assert!(unscaled(first[0].x) < 1.0);
        // Still a loop, with no point in it twice: the reference's slip
        // here is not copied.
        assert_eq!(first.len(), 5);
        assert!(first.windows(2).all(|pair| pair[0] != pair[1]));
        assert_eq!(first.first(), first.last());
        // The rest follow it inward.
        assert!(lines[0].len() > 1);
        assert!((area(&filled) - 400.0).abs() < 1e-6);
    }

    #[test]
    fn a_larger_overhang_takes_more_line() {
        // More length and more corners, not more paths: the lines of a
        // region run on from one another, here into a single path.
        let (small, small_filled) = cantilever(16.0);
        let (large, large_filled) = cantilever(10.0);
        assert!(total_mm(&small) > 0.0);
        assert!(total_mm(&large) > 1.5 * total_mm(&small));
        assert!(area(&large_filled) > area(&small_filled));
        let points = |regions: &[Vec<ExtrusionPath>]| -> usize {
            regions.iter().flatten().map(|path| path.points.len()).sum()
        };
        assert!(points(&large) > points(&small));
    }

    #[test]
    fn a_short_gap_held_on_both_sides_is_left_to_be_bridged() {
        // A 3 mm slot right across the fill.
        let (lines, filled) = over_overhangs(
            &[block(0.0, 0.0, 20.0, 20.0)],
            &[block(-5.0, -5.0, 8.5, 25.0), block(11.5, -5.0, 25.0, 25.0)],
            2,
            &flow(),
        )
        .unwrap();
        // The region is there, with nothing in it.
        assert_eq!(lines, [Vec::new()]);
        assert!(area(&filled) < 1e-6);
    }

    #[test]
    fn the_same_input_gives_the_same_lines() {
        let l_shape = [ExPolygon {
            contour: Polygon {
                points: line(&[
                    (0.0, 0.0),
                    (20.0, 0.0),
                    (20.0, 8.0),
                    (9.0, 8.0),
                    (9.0, 20.0),
                    (0.0, 20.0),
                ]),
            },
            holes: Vec::new(),
        }];
        let lower = [block(-5.0, -5.0, 7.0, 6.0)];
        let once = over_overhangs(&l_shape, &lower, 2, &flow()).unwrap();
        let again = over_overhangs(&l_shape, &lower, 2, &flow()).unwrap();
        assert!(once.0.iter().any(|region| !region.is_empty()));
        assert_eq!(once, again);
        assert_eq!(cantilever(10.0), cantilever(10.0));
    }

    #[test]
    fn the_unsupported_distance_is_what_hangs_free_the_best_way_round() {
        // Held on the left only: bridging either way leaves 20 mm of
        // line ends in the air.
        let overhang = [block(10.0, 0.0, 20.0, 20.0)];
        let one_side = unsupported_distance(&overhang, &[block(0.0, 0.0, 10.0, 20.0)]).unwrap();
        assert!((one_side / 1e6 - 20.0).abs() < 0.01, "{one_side}");
        // Held left and right: bridged across, nothing does.
        let both = [block(0.0, 0.0, 10.0, 20.0), block(20.0, 0.0, 30.0, 20.0)];
        assert!(unsupported_distance(&overhang, &both).unwrap() < 1.0);
        // Held all round.
        let around = [ExPolygon {
            contour: block(0.0, -10.0, 30.0, 30.0).contour,
            holes: vec![Polygon {
                points: overhang[0].contour.points.iter().rev().copied().collect(),
            }],
        }];
        assert_eq!(unsupported_distance(&overhang, &around).unwrap(), 0.0);
    }

    #[test]
    fn the_layer_below_tells_inside_from_outside() {
        let ring = [ExPolygon {
            contour: block(0.0, 0.0, 10.0, 10.0).contour,
            holes: vec![Polygon {
                points: line(&[(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)]),
            }],
        }];
        let below = Outline::new(&ring);
        let at = |x: f64, y: f64| below.signed_distance(Point2::from_mm(x, y)) / 1e6;
        assert_eq!(at(2.0, 5.0), -2.0);
        assert_eq!(at(5.0, 5.0), 1.0);
        assert_eq!(at(13.0, 14.0), 5.0);
        // On an edge, and level with a corner.
        assert_eq!(at(10.0, 5.0), 0.0);
        assert_eq!(at(-1.0, 10.0), 1.0);
        assert_eq!(at(1.0, 4.0), -1.0);
        assert_eq!(
            Outline::new(&[]).signed_distance(Point2::from_mm(1.0, 1.0)),
            f64::INFINITY
        );
    }

    #[test]
    fn reconnect_joins_lines_end_to_end_whichever_way_they_run() {
        let limit = scaled(0.45) as f64;
        // The two pieces of a square cut where it began.
        let joined = reconnect(
            vec![
                line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]),
                line(&[(10.0, 10.0), (0.0, 10.0), (0.0, 0.2)]),
            ],
            limit,
        );
        assert_eq!(
            joined,
            [line(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.2)
            ])]
        );
        // End to end.
        assert_eq!(
            reconnect(
                vec![
                    line(&[(0.0, 0.0), (5.0, 0.0)]),
                    line(&[(9.0, 0.0), (5.1, 0.0)])
                ],
                limit
            ),
            [line(&[(0.0, 0.0), (5.0, 0.0), (5.1, 0.0), (9.0, 0.0)])]
        );
        // The second ends where the first starts.
        assert_eq!(
            reconnect(
                vec![
                    line(&[(5.0, 0.0), (9.0, 0.0)]),
                    line(&[(0.0, 0.0), (4.9, 0.0)])
                ],
                limit
            ),
            [line(&[(9.0, 0.0), (5.0, 0.0), (4.9, 0.0), (0.0, 0.0)])]
        );
        // Start to start.
        assert_eq!(
            reconnect(
                vec![
                    line(&[(5.0, 0.0), (9.0, 0.0)]),
                    line(&[(4.9, 0.0), (0.0, 0.0)])
                ],
                limit
            ),
            [line(&[(0.0, 0.0), (4.9, 0.0), (5.0, 0.0), (9.0, 0.0)])]
        );
    }

    #[test]
    fn reconnect_keeps_apart_what_is_apart_in_the_order_given() {
        let limit = scaled(0.45) as f64;
        let lines = vec![
            line(&[(0.0, 5.0), (5.0, 5.0)]),
            Vec::new(),
            line(&[(0.0, 0.0), (5.0, 0.0)]),
            line(&[(5.45, 5.0), (9.0, 5.0)]),
            line(&[(5.2, 0.0), (9.0, 0.0)]),
        ];
        assert_eq!(
            reconnect(lines, limit),
            [
                // 0.45 mm is not nearer than the limit.
                line(&[(0.0, 5.0), (5.0, 5.0)]),
                line(&[(0.0, 0.0), (5.0, 0.0), (5.2, 0.0), (9.0, 0.0)]),
                line(&[(5.45, 5.0), (9.0, 5.0)]),
            ]
        );
        assert!(reconnect(Vec::new(), limit).is_empty());
    }

    #[test]
    fn paths_touch_within_the_limit_only() {
        let limit = scaled(0.675) as f64;
        let long = path(&[(0.0, 0.0), (10.0, 0.0)]);
        // Beside the middle of it, where it has no point of its own.
        assert!(touch(&long, &path(&[(4.0, 0.5), (6.0, 3.0)]), limit));
        assert!(touch(&path(&[(4.0, 0.5), (6.0, 3.0)]), &long, limit));
        assert!(!touch(&long, &path(&[(4.0, 0.7), (6.0, 3.0)]), limit));
        assert!(!touch(&long, &path(&[(10.5, 0.5), (12.0, 0.5)]), limit));
    }

    #[test]
    fn sort_lays_anchored_paths_first_and_each_other_against_one_laid() {
        let spacing = scaled(0.45) as f64;
        // Three lines side by side, staggered so that none runs on from
        // another. Only the first is anchored; the last given touches
        // it, and the middle one given touches only the last.
        let anchored = path(&[(0.0, 0.0), (10.0, 0.0)]);
        let far = path(&[(4.0, 0.9), (7.0, 0.9)]);
        let near = path(&[(3.0, 0.45), (6.0, 0.45)]);
        let sorted = sort(&[anchored.clone(), far.clone(), near.clone()], 1, spacing);
        // `near` is laid from the end nearer to where `anchored` stopped.
        let mut near_turned = near.clone();
        near_turned.points.reverse();
        assert_eq!(sorted, [anchored.clone(), near_turned, far.clone()]);

        // Paths away from every anchored one wait for nothing but each
        // other, and are laid all the same.
        let mut apart = path(&[(0.0, 5.0), (10.0, 5.0)]);
        let beside_it = path(&[(3.0, 5.45), (6.0, 5.45)]);
        let sorted = sort(
            &[anchored.clone(), apart.clone(), beside_it.clone()],
            1,
            spacing,
        );
        apart.points.reverse();
        assert_eq!(sorted, [anchored, apart, beside_it]);
        assert!(sort(&[], 0, spacing).is_empty());
    }

    #[test]
    fn sort_runs_paths_on_from_one_another_and_drops_short_ones() {
        let spacing = scaled(0.45) as f64;
        // Side by side with their ends level: one line there and back.
        let sorted = sort(
            &[
                path(&[(0.0, 0.0), (10.0, 0.0)]),
                path(&[(0.0, 0.9), (10.0, 0.9)]),
                path(&[(0.0, 0.45), (10.0, 0.45)]),
            ],
            1,
            spacing,
        );
        assert_eq!(
            sorted,
            [path(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 0.45),
                (0.0, 0.45),
                (0.0, 0.9),
                (10.0, 0.9),
            ])]
        );

        // Three spacings is 1.35 mm: a line must be longer than that.
        let sorted = sort(
            &[
                path(&[(0.0, 0.0), (1.35, 0.0)]),
                path(&[(0.0, 5.0), (1.36, 5.0)]),
                path(&[(0.0, 9.0), (1.0, 9.0)]),
            ],
            3,
            spacing,
        );
        // Laid from the end nearer to where the one before it stopped.
        assert_eq!(sorted, [path(&[(1.36, 5.0), (0.0, 5.0)])]);
    }
}
