//! Polygon booleans and offsetting on the engine's fixed-point geometry.
//!
//! This is the only crate that knows which clipping library does the work
//! (clipper2-rs, a port of Clipper2), so replacing it touches nothing else.
//! The reference slicer uses the older Clipper 6; results agree in area and
//! shape but not vertex for vertex.

use clipper2_rs::{
    ClipType, Clipper64, ClipperOffset, EndType, FillRule, JoinType, Path64, Paths64, Point64,
    PolyTree64, Rect64, rect_clip,
};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

mod nfp;
pub use nfp::{inner_fit_rectangle, minkowski_sum_convex, nfp_convex, reference_vertex};

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    /// Coordinates beyond about ±2.3e18, far outside any printer.
    #[error("polygon coordinates are outside the range clipping supports")]
    OutOfRange,
    #[error("offset distance is not a finite number")]
    InvalidDelta,
    /// The clipper could not order its intersections; the input is degenerate
    /// in a way it does not handle.
    #[error("clipping failed on this input")]
    Failed,
}

/// How an offset shapes the corners it pushes outward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    /// Sharp corners, cut square once they would reach further than `limit`
    /// times the offset distance.
    Miter { limit: f64 },
    /// Arcs, drawn so that no point strays more than `tolerance` from a true one.
    Round { tolerance: Coord },
    /// Corners cut square at exactly the offset distance.
    Square,
}

impl Join {
    /// The reference slicer's default corner treatment.
    pub const MITER: Join = Join::Miter { limit: 3.0 };
}

/// Unites closed contours into regions. A point is inside the result when
/// its winding number is non-zero, so counter-clockwise contours add
/// material, clockwise ones nested inside them cut holes, and overlapping
/// shells merge.
pub fn union<'a>(
    polygons: impl IntoIterator<Item = &'a Polygon>,
) -> Result<Vec<ExPolygon>, ClipError> {
    boolean(ClipType::Union, polygons.into_iter(), std::iter::empty())
}

/// The regions covered an odd number of times by `polygons`, whichever way
/// each is wound: what is inside one contour and not inside another
/// within it.
pub fn union_even_odd<'a>(
    polygons: impl IntoIterator<Item = &'a Polygon>,
) -> Result<Vec<ExPolygon>, ClipError> {
    let mut clipper = Clipper64::new();
    clipper.set_preserve_collinear(false);
    clipper.add_subject(&paths(polygons.into_iter()));
    let (tree, _open) = clipper.execute_tree(ClipType::Union, FillRule::EvenOdd)?;
    Ok(regions_of(&tree))
}

/// The regions with a positive winding number: what counter-clockwise
/// contours cover more often than clockwise ones. Slicing a closed mesh
/// whose surface folds through itself, as a tube bent sharply does, gives
/// contours whose folded parts wind the wrong way; this keeps only what
/// is inside (the reference's `MeshSlicingParams::SlicingMode::Positive`).
pub fn union_positive<'a>(
    polygons: impl IntoIterator<Item = &'a Polygon>,
) -> Result<Vec<ExPolygon>, ClipError> {
    let mut clipper = Clipper64::new();
    clipper.set_preserve_collinear(false);
    clipper.add_subject(&paths(polygons.into_iter()));
    let (tree, _open) = clipper.execute_tree(ClipType::Union, FillRule::Positive)?;
    Ok(regions_of(&tree))
}

/// Unites regions that may overlap or touch.
pub fn union_ex(regions: &[ExPolygon]) -> Result<Vec<ExPolygon>, ClipError> {
    boolean(ClipType::Union, rings(regions), std::iter::empty())
}

/// What is left of `subject` after removing everything `clip` covers.
pub fn difference(subject: &[ExPolygon], clip: &[ExPolygon]) -> Result<Vec<ExPolygon>, ClipError> {
    boolean(ClipType::Difference, rings(subject), rings(clip))
}

/// The area covered by both `a` and `b`.
pub fn intersection(a: &[ExPolygon], b: &[ExPolygon]) -> Result<Vec<ExPolygon>, ClipError> {
    boolean(ClipType::Intersection, rings(a), rings(b))
}

/// [`difference`] with `clip` first cut down to the box round `subject`
/// grown by `margin`, which gives the same regions inside that box and is
/// much quicker when `clip` is large and `subject` small (the reference's
/// `diff_clipped`).
pub fn difference_near(
    subject: &[ExPolygon],
    clip: &[ExPolygon],
    margin: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    let Some(window) = window(subject, margin) else {
        return Ok(Vec::new());
    };
    let near = near(clip, &window);
    let mut clipper = Clipper64::new();
    clipper.set_preserve_collinear(false);
    clipper.add_subject(&paths(rings(subject)));
    clipper.add_clip(&near);
    let (tree, _open) = clipper.execute_tree(ClipType::Difference, FillRule::NonZero)?;
    Ok(regions_of(&tree))
}

/// [`intersection`] with `b` cut down to the box round `a` grown by
/// `margin` (the reference's `intersection_clipped`).
pub fn intersection_near(
    a: &[ExPolygon],
    b: &[ExPolygon],
    margin: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    let Some(window) = window(a, margin) else {
        return Ok(Vec::new());
    };
    let near = near(b, &window);
    if near.is_empty() {
        return Ok(Vec::new());
    }
    let mut clipper = Clipper64::new();
    clipper.set_preserve_collinear(false);
    clipper.add_subject(&paths(rings(a)));
    clipper.add_clip(&near);
    let (tree, _open) = clipper.execute_tree(ClipType::Intersection, FillRule::NonZero)?;
    Ok(regions_of(&tree))
}

/// The box round the outer rings of `regions`, grown by `margin`.
fn window(regions: &[ExPolygon], margin: Coord) -> Option<Rect64> {
    let mut points = regions.iter().flat_map(|region| &region.contour.points);
    let first = points.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in points {
        (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
    }
    Some(Rect64::new(
        x0 - margin,
        y0 - margin,
        x1 + margin,
        y1 + margin,
    ))
}

/// The rings of the regions that reach into `window`, cut to it; a region
/// wholly inside is taken as it is.
fn near(regions: &[ExPolygon], window: &Rect64) -> Paths64 {
    let mut out = Paths64::new();
    for region in regions {
        let points = &region.contour.points;
        let Some(first) = points.first() else {
            continue;
        };
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
        for p in points {
            (x0, y0, x1, y1) = (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y));
        }
        if x0 > window.right || x1 < window.left || y0 > window.bottom || y1 < window.top {
            continue;
        }
        let inside =
            x0 >= window.left && x1 <= window.right && y0 >= window.top && y1 <= window.bottom;
        let rings = paths(region.rings());
        if inside {
            out.extend(rings);
        } else {
            out.extend(rect_clip(window, &rings));
        }
    }
    out
}

/// Grows regions by `delta` (or shrinks them when it is negative), merging
/// whatever grows together and dropping whatever shrinks to nothing.
pub fn offset(
    regions: &[ExPolygon],
    delta: Coord,
    join: Join,
) -> Result<Vec<ExPolygon>, ClipError> {
    if delta == 0 {
        return union_ex(regions);
    }
    let (join_type, miter_limit, arc_tolerance) = match join {
        Join::Miter { limit } => (JoinType::Miter, limit, 0.0),
        Join::Round { tolerance } => (JoinType::Round, 2.0, tolerance as f64),
        Join::Square => (JoinType::Square, 2.0, 0.0),
    };
    let mut offsetter = ClipperOffset::new(miter_limit, arc_tolerance, false, false);
    offsetter.add_paths(&paths(rings(regions)), join_type, EndType::Polygon);
    offsetter
        .execute_tree(delta as f64)
        .map(|tree| regions_of(&tree))
        .map_err(ClipError::from)
}

/// Two offsets in a row. Shrinking then growing by less removes whatever
/// is too narrow to survive the first step; growing then shrinking fills
/// what is too narrow to stay open.
pub fn offset2(
    regions: &[ExPolygon],
    first: Coord,
    second: Coord,
    join: Join,
) -> Result<Vec<ExPolygon>, ClipError> {
    offset(&offset(regions, first, join)?, second, join)
}

/// Removes slivers and spurs narrower than twice `radius` by shrinking the
/// regions and growing them back.
pub fn opening(
    regions: &[ExPolygon],
    radius: Coord,
    join: Join,
) -> Result<Vec<ExPolygon>, ClipError> {
    if radius <= 0 {
        return union_ex(regions);
    }
    offset2(regions, -radius, radius, join)
}

/// Fills cracks and notches narrower than twice `radius` by growing the
/// regions and shrinking them back. This is how the reference merges parts
/// modelled to sit flush whose surfaces miss each other by a hair
/// (`offset2_ex` in TriangleMeshSlicer.cpp, driven by `slice_closing_radius`).
pub fn closing(
    regions: &[ExPolygon],
    radius: Coord,
    join: Join,
) -> Result<Vec<ExPolygon>, ClipError> {
    if radius <= 0 {
        return union_ex(regions);
    }
    offset2(regions, radius, -radius, join)
}

/// The parts of open lines that lie within `regions`, as open lines (the
/// reference's `intersection_pl`). A piece keeps the points of the line
/// it was cut from, but may come out the other way round.
pub fn clip_lines(
    lines: &[Vec<Point2>],
    regions: &[ExPolygon],
) -> Result<Vec<Vec<Point2>>, ClipError> {
    let open: Paths64 = lines
        .iter()
        .filter(|line| line.len() >= 2)
        .map(|line| line.iter().map(|p| Point64::new(p.x, p.y)).collect())
        .collect();
    if open.is_empty() || regions.is_empty() {
        return Ok(Vec::new());
    }
    let mut clipper = Clipper64::new();
    clipper.add_open_subject(&open);
    clipper.add_clip(&paths(rings(regions)));
    let (_closed, open) = clipper.execute_tree(ClipType::Intersection, FillRule::NonZero)?;
    Ok(open
        .into_iter()
        .map(|line| {
            line.iter()
                .map(|p| Point2 { x: p.x, y: p.y })
                .collect::<Vec<_>>()
        })
        .filter(|line| line.len() >= 2)
        .collect())
}

/// The parts of open lines that lie outside `regions`, as open lines (the
/// reference's `diff_pl`).
pub fn lines_outside(
    lines: &[Vec<Point2>],
    regions: &[ExPolygon],
) -> Result<Vec<Vec<Point2>>, ClipError> {
    let kept = || {
        lines
            .iter()
            .filter(|line| line.len() >= 2)
            .cloned()
            .collect::<Vec<_>>()
    };
    if regions.is_empty() {
        return Ok(kept());
    }
    let open: Paths64 = kept()
        .iter()
        .map(|line| line.iter().map(|p| Point64::new(p.x, p.y)).collect())
        .collect();
    if open.is_empty() {
        return Ok(Vec::new());
    }
    let mut clipper = Clipper64::new();
    clipper.add_open_subject(&open);
    clipper.add_clip(&paths(rings(regions)));
    let (_closed, open) = clipper.execute_tree(ClipType::Difference, FillRule::NonZero)?;
    Ok(open
        .into_iter()
        .map(|line| line.iter().map(|p| Point2 { x: p.x, y: p.y }).collect())
        .collect())
}

/// Shrinks one region by a distance that differs from vertex to vertex:
/// `deltas[0]` holds one value per point of the contour and `deltas[1 + i]`
/// one per point of hole `i`, each zero or negative, in the units of the
/// coordinates. The contour moves in and the holes grow. Corners pushed
/// outward are mitred no further than `miter_limit` times the distance
/// (2 at the least) and rounded off with one cut beyond that.
///
/// A port of the reference's `variable_offset_inner_ex` (ClipperUtils.cpp):
/// each ring is moved by `mittered_offset_path_scaled`, the loops the move
/// turned inside out are dropped by a union under the positive fill rule
/// (the reference gets the same area from Clipper 6 by uniting the ring
/// with a frame under the negative rule and discarding the frame), and
/// the grown holes are cut out of what is left. More than one region comes
/// back where the shrinking pinched the shape apart.
pub fn variable_offset_inner(
    region: &ExPolygon,
    deltas: &[Vec<f32>],
    miter_limit: f64,
) -> Result<Vec<ExPolygon>, ClipError> {
    let matched = deltas.len() == region.holes.len() + 1
        && region
            .rings()
            .zip(deltas)
            .all(|(ring, deltas)| ring.points.len() == deltas.len());
    let inward = deltas
        .iter()
        .flatten()
        .all(|delta| delta.is_finite() && *delta <= 0.0);
    if !matched || !inward || !miter_limit.is_finite() {
        return Err(ClipError::InvalidDelta);
    }
    // What a ring that may now cross itself encloses, counting only the
    // turns made in the given direction.
    let enclosed = |path: Path64, rule: FillRule| -> Result<Vec<ExPolygon>, ClipError> {
        if path.is_empty() {
            return Ok(Vec::new());
        }
        let mut clipper = Clipper64::new();
        clipper.set_preserve_collinear(false);
        clipper.add_subject(&vec![path]);
        let (tree, _open) = clipper.execute_tree(ClipType::Union, rule)?;
        Ok(regions_of(&tree))
    };
    let contours = enclosed(
        mitred_offset_path(&region.contour.points, &deltas[0], miter_limit),
        FillRule::Positive,
    )?;
    // Holes run clockwise, so what one encloses is wound negatively.
    let mut holes = Vec::new();
    for (hole, deltas) in region.holes.iter().zip(&deltas[1..]) {
        holes.extend(enclosed(
            mitred_offset_path(&hole.points, deltas, miter_limit),
            FillRule::Negative,
        )?);
    }
    if holes.is_empty() {
        return Ok(contours);
    }
    difference(&contours, &holes)
}

/// Moves every vertex of a closed ring to the right of its direction of
/// travel by its own delta (so a negative delta moves a counter-clockwise
/// contour inward and a clockwise hole outward), joining the moved edges
/// as Clipper 6 joins mitred ones. The result may cross itself.
///
/// A port of the reference's `mittered_offset_path_scaled`
/// (ClipperUtils.cpp), statement for statement, including the edges it
/// skips for being shorter than 0.005 of the largest delta and the
/// rounding of each new point to the nearest unit, halves away from zero.
fn mitred_offset_path(contour: &[Point2], deltas: &[f32], miter_limit: f64) -> Path64 {
    /// `ClipperOffsetShortestEdgeFactor`.
    const SHORTEST_EDGE_FACTOR: f64 = 0.005;
    /// The reference's `sin_min_parallel`, "equal to Clipper": compared
    /// with the sine of the turn times the delta, in coordinate units.
    const SIN_MIN_PARALLEL: f64 = 1.0;
    type V = (f64, f64);
    let at = |index: usize| -> V { (contour[index].x as f64, contour[index].y as f64) };
    let sub = |a: V, b: V| -> V { (a.0 - b.0, a.1 - b.1) };
    let squared_norm = |v: V| v.0 * v.0 + v.1 * v.1;
    let perp = |v: V| -> V { (v.1, -v.0) };
    let normalized = |v: V| -> V {
        let squared = squared_norm(v);
        if squared > 0.0 {
            let norm = squared.sqrt();
            (v.0 / norm, v.1 / norm)
        } else {
            v
        }
    };

    let mut out = Path64::new();
    if deltas.len() <= 2 || contour.len() != deltas.len() {
        return out;
    }
    out.reserve(contour.len() * 2);
    let mut add = |point: V| {
        let x = point.0 + if point.0 < 0.0 { -0.5 } else { 0.5 };
        let y = point.1 + if point.1 < 0.0 { -0.5 } else { 0.5 };
        out.push(Point64::new(x as i64, y as i64));
    };
    // The reference clamps the limit to 2 and keeps it as the least
    // `1 + cos` of a turn that is still mitred.
    let miter_limit = if miter_limit > 2.0 {
        2.0 / (miter_limit * miter_limit)
    } else {
        0.5
    };
    // As the reference: the largest delta, which among inward (negative)
    // deltas is the one nearest zero, where Clipper goes by the size of
    // the delta. Whether that was meant is not clear, so it is kept; all
    // it decides is which edges under a micrometre are skipped.
    let largest = deltas.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let shortest = f64::from(largest) * SHORTEST_EDGE_FACTOR;
    let shortest_sq = shortest * shortest;

    // The last point further from the first than the shortest edge.
    let mut pt = at(0);
    let mut last = contour.len() - 1;
    while last > 0 && squared_norm(sub(at(last), pt)) <= shortest_sq {
        last -= 1;
    }
    if last == 0 {
        return out;
    }
    let mut normal_prev = normalized(perp(sub(pt, at(last))));
    let mut index = 0;
    loop {
        // The next point further from this one than the shortest edge.
        let mut next = index + 1;
        while next <= last && squared_norm(sub(at(next), pt)) <= shortest_sq {
            next += 1;
        }
        let pt_next = if next > last {
            // The last edge is too short: it is merged with the one before.
            index = last;
            at(0)
        } else {
            at(next)
        };
        let normal_next = normalized(perp(sub(pt_next, pt)));
        let delta = f64::from(deltas[index]);
        let sin_a =
            (normal_prev.0 * normal_next.1 - normal_prev.1 * normal_next.0).clamp(-1.0, 1.0);
        let convex = sin_a * delta;
        if convex <= -SIN_MIN_PARALLEL {
            // The moved edges cross: both ends and the corner itself, for
            // the union to cut the loop off at.
            add((pt.0 + normal_prev.0 * delta, pt.1 + normal_prev.1 * delta));
            add(pt);
            add((pt.0 + normal_next.0 * delta, pt.1 + normal_next.1 * delta));
        } else {
            let dot = normal_prev.0 * normal_next.0 + normal_prev.1 * normal_next.1;
            if convex < SIN_MIN_PARALLEL && dot > 0.0 {
                // Nearly straight on.
                add((pt.0 + normal_prev.0 * delta, pt.1 + normal_prev.1 * delta));
            } else {
                let r = 1.0 + dot;
                if r >= miter_limit {
                    let scale = delta / r;
                    add((
                        pt.0 + (normal_prev.0 + normal_next.0) * scale,
                        pt.1 + (normal_prev.1 + normal_next.1) * scale,
                    ));
                } else {
                    // Too sharp to mitre: one cut across the corner, its
                    // middle at the delta from the vertex.
                    let dx = (sin_a.atan2(dot) / 4.0).tan();
                    let (along_prev, along_next) = (perp(normal_prev), perp(normal_next));
                    add((
                        pt.0 + (normal_prev.0 - along_prev.0 * dx) * delta,
                        pt.1 + (normal_prev.1 - along_prev.1 * dx) * delta,
                    ));
                    add((
                        pt.0 + (normal_next.0 + along_next.0 * dx) * delta,
                        pt.1 + (normal_next.1 + along_next.1 * dx) * delta,
                    ));
                }
            }
        }
        if index == last {
            break;
        }
        normal_prev = normal_next;
        pt = pt_next;
        index = next;
    }
    out
}

/// The ground open lines cover when each is drawn `half_width` to either
/// side, with corners squared and ends cut off flush: the reference's
/// `offset` of a polyline, which is how it measures what an extrusion
/// covers.
pub fn thicken<'a>(
    lines: impl IntoIterator<Item = &'a [Point2]>,
    half_width: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    let lines: Paths64 = lines
        .into_iter()
        .filter(|line| line.len() >= 2)
        .map(|line| line.iter().map(|p| Point64::new(p.x, p.y)).collect())
        .collect();
    if lines.is_empty() || half_width <= 0 {
        return Ok(Vec::new());
    }
    let mut offsetter = ClipperOffset::new(2.0, 0.0, false, false);
    offsetter.add_paths(&lines, JoinType::Square, EndType::Butt);
    offsetter
        .execute_tree(half_width as f64)
        .map(|tree| regions_of(&tree))
        .map_err(ClipError::from)
}

/// The ground within `radius` of open lines: each drawn `radius` to
/// either side with corners and ends rounded, no point of an arc more
/// than `tolerance` off a true one. A line that ends where it began is
/// rounded there too, which covers what a closed line would.
pub fn thicken_round<'a>(
    lines: impl IntoIterator<Item = &'a [Point2]>,
    radius: Coord,
    tolerance: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    let lines: Paths64 = lines
        .into_iter()
        .filter(|line| line.len() >= 2)
        .map(|line| line.iter().map(|p| Point64::new(p.x, p.y)).collect())
        .collect();
    if lines.is_empty() || radius <= 0 {
        return Ok(Vec::new());
    }
    let mut offsetter = ClipperOffset::new(2.0, tolerance as f64, false, false);
    offsetter.add_paths(&lines, JoinType::Round, EndType::Round);
    offsetter
        .execute_tree(radius as f64)
        .map(|tree| regions_of(&tree))
        .map_err(ClipError::from)
}

/// Drops vertices that deviate from the outline by no more than `tolerance`
/// (Douglas-Peucker on each ring, anchored at its first point), then
/// re-unites the rings in case simplifying made them cross. Matches the
/// reference's `ExPolygon::simplify_p` followed by `union_ex`.
pub fn simplify(regions: &[ExPolygon], tolerance: Coord) -> Result<Vec<ExPolygon>, ClipError> {
    let simplified: Vec<Polygon> = rings(regions)
        .map(|ring| {
            // Closing the ring makes its first point both ends of the path.
            let mut closed = ring.points.clone();
            closed.extend(ring.points.first().copied());
            let mut points = douglas_peucker(&closed, tolerance as f64);
            points.pop();
            Polygon { points }
        })
        .collect();
    union(&simplified)
}

/// Douglas-Peucker on every ring as in [`simplify`], without uniting the
/// rings again afterwards (the reference's `polygons_simplify` with
/// `strictly_simple` off): a ring may then cross itself or another by up
/// to `tolerance`, which the next boolean or offset sorts out. A contour
/// left with under three points drops its region, a hole only itself.
pub fn simplify_rings(regions: &[ExPolygon], tolerance: Coord) -> Vec<ExPolygon> {
    let ring = |ring: &Polygon| {
        let mut closed = ring.points.clone();
        closed.extend(ring.points.first().copied());
        let mut points = douglas_peucker(&closed, tolerance as f64);
        points.pop();
        Polygon { points }
    };
    regions
        .iter()
        .filter_map(|region| {
            let contour = ring(&region.contour);
            (contour.points.len() >= 3).then(|| ExPolygon {
                contour,
                holes: region
                    .holes
                    .iter()
                    .map(ring)
                    .filter(|hole| hole.points.len() >= 3)
                    .collect(),
            })
        })
        .collect()
}

/// Simplifies an open path, keeping both ends. A port of the reference's
/// `MultiPoint::_douglas_peucker`, including measuring to the segment rather
/// than to the infinite line through it.
fn douglas_peucker(points: &[Point2], tolerance: f64) -> Vec<Point2> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut kept = vec![first];
    if points.len() == 1 {
        return kept;
    }
    let tolerance_sq = tolerance * tolerance;
    let mut anchor = 0;
    // Ends of the stretches still to be examined, nearest last.
    let mut floaters = vec![points.len() - 1];
    while let Some(&floater) = floaters.last() {
        let (mut furthest, mut furthest_sq) = (anchor, 0.0);
        for index in anchor + 1..floater {
            let distance_sq =
                distance_to_segment_sq(points[index], points[anchor], points[floater]);
            if distance_sq > furthest_sq {
                (furthest, furthest_sq) = (index, distance_sq);
            }
        }
        if furthest_sq <= tolerance_sq {
            kept.push(points[floater]);
            anchor = floater;
            floaters.pop();
        } else {
            floaters.push(furthest);
        }
    }
    kept
}

fn distance_to_segment_sq(p: Point2, a: Point2, b: Point2) -> f64 {
    let (px, py) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let length_sq = dx * dx + dy * dy;
    let along = if length_sq > 0.0 {
        ((px * dx + py * dy) / length_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ox, oy) = (px - along * dx, py - along * dy);
    ox * ox + oy * oy
}

fn rings(regions: &[ExPolygon]) -> impl Iterator<Item = &Polygon> {
    regions.iter().flat_map(ExPolygon::rings)
}

fn paths<'a>(polygons: impl Iterator<Item = &'a Polygon>) -> Paths64 {
    polygons
        .map(|polygon| {
            polygon
                .points
                .iter()
                .map(|p| Point64::new(p.x, p.y))
                .collect()
        })
        .collect()
}

fn boolean<'a>(
    operation: ClipType,
    subject: impl Iterator<Item = &'a Polygon>,
    clip: impl Iterator<Item = &'a Polygon>,
) -> Result<Vec<ExPolygon>, ClipError> {
    let mut clipper = Clipper64::new();
    // Vertices in the middle of a straight run carry no shape and only cost
    // every later stage time; slicing produces many of them.
    clipper.set_preserve_collinear(false);
    clipper.add_subject(&paths(subject));
    clipper.add_clip(&paths(clip));
    let (tree, _open) = clipper.execute_tree(operation, FillRule::NonZero)?;
    Ok(regions_of(&tree))
}

/// Flattens the clipper's containment tree: every polygon at an even depth
/// is a region and its children are that region's holes. Islands inside
/// holes are regions of their own.
fn regions_of(tree: &PolyTree64) -> Vec<ExPolygon> {
    let polygon = |id: usize| Polygon {
        points: tree
            .node(id)
            .polygon
            .iter()
            .map(|p| Point2 { x: p.x, y: p.y })
            .collect(),
    };
    let mut regions = Vec::new();
    let mut outers: Vec<usize> = tree.roots().iter().rev().copied().collect();
    while let Some(outer) = outers.pop() {
        let contour = polygon(outer);
        if is_dust(&contour) {
            continue;
        }
        let mut holes = Vec::new();
        for &id in tree.node(outer).children.iter().rev() {
            let hole = polygon(id);
            if !is_dust(&hole) {
                holes.push(hole);
                outers.extend(tree.node(id).children.iter().rev());
            }
        }
        holes.reverse();
        regions.push(ExPolygon { contour, holes });
    }
    regions
}

/// Whether a ring is thinner than the coordinate grid: its area is no more
/// than its perimeter times one unit. Snapping intersections to integers
/// leaves a few such slivers behind, a nanometre wide and of arbitrary
/// orientation; they are rounding residue, not shape, and whatever lies
/// inside one is smaller still.
fn is_dust(ring: &Polygon) -> bool {
    let points = &ring.points;
    let (mut doubled_area, mut perimeter) = (0i128, 0.0f64);
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        doubled_area += i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y);
        perimeter += ((b.x - a.x) as f64).hypot((b.y - a.y) as f64);
    }
    doubled_area.unsigned_abs() as f64 <= 2.0 * perimeter
}

impl From<clipper2_rs::ClipperError> for ClipError {
    fn from(error: clipper2_rs::ClipperError) -> Self {
        match error {
            clipper2_rs::ClipperError::CoordinateOutOfRange => ClipError::OutOfRange,
            clipper2_rs::ClipperError::InvalidDelta => ClipError::InvalidDelta,
            _ => ClipError::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use edge_geom::scaled;

    use super::*;

    /// A counter-clockwise axis-aligned rectangle in millimetres.
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn hole(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        let mut polygon = rect(x0, y0, x1, y1);
        polygon.points.reverse();
        polygon
    }

    fn solid(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<ExPolygon> {
        vec![ExPolygon {
            contour: rect(x0, y0, x1, y1),
            holes: Vec::new(),
        }]
    }

    #[test]
    fn a_clipped_difference_is_the_difference() {
        // A large frame with a hole, and small squares over its edge, in
        // its hole, and far from it.
        let frame = vec![ExPolygon {
            contour: rect(0.0, 0.0, 100.0, 100.0),
            holes: vec![hole(20.0, 20.0, 80.0, 80.0)],
        }];
        for (x, y) in [(15.0, 50.0), (50.0, 50.0), (150.0, 50.0), (78.0, 78.0)] {
            let small = solid(x, y, x + 5.0, y + 5.0);
            let near = difference_near(&small, &frame, 10).unwrap();
            let full = difference(&small, &frame).unwrap();
            assert!((area(&near) - area(&full)).abs() < 1e-9, "{x} {y}");
            let near = intersection_near(&small, &frame, 10).unwrap();
            let full = intersection(&small, &frame).unwrap();
            assert!((area(&near) - area(&full)).abs() < 1e-9, "{x} {y}");
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    fn assert_oriented(regions: &[ExPolygon]) {
        for region in regions {
            assert!(region.contour.is_ccw(), "contour is clockwise");
            assert!(
                region.holes.iter().all(|hole| !hole.is_ccw()),
                "hole is counter-clockwise"
            );
        }
    }

    #[test]
    fn a_line_thickened_round_covers_what_is_within_reach_of_it() {
        // A 10 mm line a millimetre to either side, with a half disc at
        // each end: a little under 20 + pi mm², the arcs being chords.
        let line = [Point2::from_mm(0.0, 0.0), Point2::from_mm(10.0, 0.0)];
        let found = thicken_round([line.as_slice()], scaled(1.0), scaled(0.001)).unwrap();
        assert_eq!(found.len(), 1);
        assert!(
            (area(&found) - (20.0 + PI)).abs() < 0.02,
            "{}",
            area(&found)
        );
        // It reaches past the line's ends, where `thicken` stops flush.
        let reach = found[0].contour.points.iter().map(|p| p.x).max().unwrap();
        assert!((reach - scaled(11.0)).abs() <= scaled(0.001), "{reach}");
        // A line with one point, or no width, covers nothing.
        assert!(
            thicken_round([&line[..1]], scaled(1.0), 10)
                .unwrap()
                .is_empty()
        );
        assert!(thicken_round([line.as_slice()], 0, 10).unwrap().is_empty());
    }

    #[test]
    fn union_merges_overlaps_and_drops_collinear_points() {
        let mut edge_point = rect(0.0, 0.0, 2.0, 2.0);
        edge_point.points.insert(1, Point2::from_mm(1.0, 0.0));
        let regions = union(&[edge_point, rect(1.0, 1.0, 3.0, 3.0)]).unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(area(&regions), 7.0);
        assert_eq!(regions[0].contour.points.len(), 8);
        assert_oriented(&regions);
    }

    #[test]
    fn union_nests_holes_and_islands() {
        let regions = union(&[
            rect(0.0, 0.0, 10.0, 10.0),
            hole(2.0, 2.0, 8.0, 8.0),
            rect(4.0, 4.0, 6.0, 6.0),
            rect(20.0, 0.0, 21.0, 1.0),
        ])
        .unwrap();
        let shapes: Vec<_> = regions
            .iter()
            .map(|r| (r.area_mm2(), r.holes.len()))
            .collect();
        assert_eq!(shapes, [(64.0, 1), (4.0, 0), (1.0, 0)]);
        assert_oriented(&regions);
    }

    #[test]
    fn union_keeps_material_where_a_hole_is_covered() {
        let ring = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: vec![hole(2.0, 2.0, 8.0, 8.0)],
        };
        let mut both = vec![ring];
        both.extend(solid(1.0, 1.0, 9.0, 9.0));
        let regions = union_ex(&both).unwrap();
        assert_eq!(
            (regions.len(), area(&regions), regions[0].holes.len()),
            (1, 100.0, 0)
        );
    }

    #[test]
    fn difference_and_intersection() {
        let cut = difference(&solid(0.0, 0.0, 10.0, 10.0), &solid(4.0, 4.0, 6.0, 6.0)).unwrap();
        assert_eq!((cut.len(), cut[0].holes.len(), area(&cut)), (1, 1, 96.0));
        assert_oriented(&cut);

        let split = difference(&solid(0.0, 0.0, 10.0, 10.0), &solid(4.0, -1.0, 6.0, 11.0)).unwrap();
        assert_eq!((split.len(), area(&split)), (2, 80.0));

        let common =
            intersection(&solid(0.0, 0.0, 10.0, 10.0), &solid(8.0, 8.0, 12.0, 12.0)).unwrap();
        assert_eq!((common.len(), area(&common)), (1, 4.0));
        assert!(
            intersection(&solid(0.0, 0.0, 1.0, 1.0), &solid(5.0, 5.0, 6.0, 6.0))
                .unwrap()
                .is_empty()
        );
        assert!(
            difference(&solid(0.0, 0.0, 1.0, 1.0), &solid(-1.0, -1.0, 2.0, 2.0))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn offset_grows_by_the_join_shape() {
        let square = solid(0.0, 0.0, 10.0, 10.0);
        let d = scaled(1.0);

        let miter = offset(&square, d, Join::MITER).unwrap();
        assert_eq!((area(&miter), miter[0].contour.points.len()), (144.0, 4));

        let round = offset(
            &square,
            d,
            Join::Round {
                tolerance: scaled(0.001),
            },
        )
        .unwrap();
        let exact = 100.0 + 40.0 + PI;
        assert!(
            (area(&round) - exact).abs() < 0.01,
            "round area {} vs {exact}",
            area(&round)
        );
        assert!(area(&round) <= exact);

        // A square join cuts each corner at the offset distance.
        let squared = offset(&square, d, Join::Square).unwrap();
        assert!(area(&squared) > exact - 1.0 && area(&squared) < 144.0);
        assert_oriented(&miter);
    }

    #[test]
    fn offset_shrinks_grows_holes_and_drops_what_vanishes() {
        let ring = vec![ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: vec![hole(4.0, 4.0, 6.0, 6.0)],
        }];
        let shrunk = offset(&ring, scaled(-1.0), Join::MITER).unwrap();
        assert_eq!(
            (shrunk.len(), shrunk[0].holes.len(), area(&shrunk)),
            (1, 1, 64.0 - 16.0)
        );
        assert_oriented(&shrunk);

        // Growing by 1 closes the 2 mm hole completely.
        let grown = offset(&ring, scaled(1.0), Join::MITER).unwrap();
        assert_eq!((grown[0].holes.len(), area(&grown)), (0, 144.0));

        assert!(offset(&ring, scaled(-3.0), Join::MITER).unwrap().is_empty());
        assert_eq!(area(&offset(&ring, 0, Join::MITER).unwrap()), 96.0);
    }

    #[test]
    fn offset_merges_neighbours_that_grow_together() {
        let mut pair = solid(0.0, 0.0, 10.0, 10.0);
        pair.extend(solid(11.0, 0.0, 21.0, 10.0));
        assert_eq!(offset(&pair, scaled(0.4), Join::MITER).unwrap().len(), 2);
        let merged = offset(&pair, scaled(1.0), Join::MITER).unwrap();
        assert_eq!((merged.len(), area(&merged)), (1, 23.0 * 12.0));
    }

    #[test]
    fn closing_fills_narrow_cracks_and_keeps_wide_ones() {
        // Two blocks 0.05 mm apart, and a third 1 mm further on.
        let mut blocks = solid(0.0, 0.0, 10.0, 10.0);
        blocks.extend(solid(10.05, 0.0, 20.0, 10.0));
        blocks.extend(solid(21.0, 0.0, 30.0, 10.0));
        let closed = closing(&blocks, scaled(0.049), Join::MITER).unwrap();
        // The clipper's output order is repeatable but carries no meaning.
        let mut shapes: Vec<_> = closed
            .iter()
            .map(|region| (region.area_mm2(), region.contour.points.len()))
            .collect();
        shapes.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(shapes, [(90.0, 4), (200.0, 4)]);
        assert_eq!(area(&closing(&blocks, 0, Join::MITER).unwrap()), 289.5);
    }

    #[test]
    fn simplify_drops_small_deviations_only() {
        // A square with a 0.005 mm bump on one side and a 1 mm notch on another.
        let points = [
            (0.0, 0.0),
            (5.0, -0.005),
            (10.0, 0.0),
            (10.0, 4.0),
            (9.0, 5.0),
            (10.0, 6.0),
            (10.0, 10.0),
            (0.0, 10.0),
        ];
        let bumpy = vec![ExPolygon {
            contour: Polygon {
                points: points.map(|(x, y)| Point2::from_mm(x, y)).to_vec(),
            },
            holes: vec![hole(4.0, 4.0, 6.0, 6.0)],
        }];
        let simple = simplify(&bumpy, scaled(0.01)).unwrap();
        assert_eq!(simple.len(), 1);
        assert_eq!(simple[0].contour.points.len(), 7);
        assert_eq!(simple[0].holes.len(), 1);
        assert_eq!(area(&simple), 100.0 - 1.0 - 4.0);
        assert_eq!(
            area(&simplify(&bumpy, 0).unwrap()),
            100.0 + 0.025 - 1.0 - 4.0
        );
    }

    #[test]
    fn douglas_peucker_keeps_ends_and_real_corners() {
        let path: Vec<Point2> = [(0.0, 0.0), (1.0, 0.004), (2.0, 0.0), (3.0, 1.0), (4.0, 0.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        let kept = douglas_peucker(&path, scaled(0.01) as f64);
        assert_eq!(kept, [path[0], path[2], path[3], path[4]]);
        assert_eq!(
            douglas_peucker(&path, scaled(5.0) as f64),
            [path[0], path[4]]
        );
        assert_eq!(douglas_peucker(&path[..1], 1.0), [path[0]]);
        assert!(douglas_peucker(&[], 1.0).is_empty());
    }

    #[test]
    fn opening_drops_slivers_and_keeps_the_rest() {
        let mut shape = solid(0.0, 0.0, 10.0, 10.0);
        shape.extend(solid(20.0, 0.0, 20.05, 10.0));
        let opened = opening(&shape, scaled(0.045), Join::MITER).unwrap();
        assert_eq!((opened.len(), area(&opened)), (1, 100.0));
        assert_eq!(area(&opening(&shape, 0, Join::MITER).unwrap()), 100.5);
    }

    #[test]
    fn offset2_removes_what_is_too_narrow() {
        // A 10 mm block with a 0.3 mm wide fin sticking out of it.
        let mut shape = solid(0.0, 0.0, 10.0, 10.0);
        shape.extend(solid(10.0, 4.0, 15.0, 4.3));
        let opened = offset2(&shape, scaled(-0.2), scaled(0.2), Join::MITER).unwrap();
        assert_eq!((opened.len(), area(&opened)), (1, 100.0));
    }

    #[test]
    fn slivers_thinner_than_the_grid_are_dust() {
        // A sliver the clipper left in a real layer: 350 nm long, 0.05 nm wide.
        let sliver = Polygon {
            points: [
                (166669416, 149467333),
                (166669338, 149467420),
                (166669570, 149467161),
            ]
            .map(|(x, y)| Point2 { x, y })
            .to_vec(),
        };
        assert!(is_dust(&sliver));
        assert!(is_dust(&Polygon::default()));
        // One micrometre square, far below anything printable, is still shape.
        assert!(!is_dust(&rect(0.0, 0.0, 0.001, 0.001)));
        assert!(!is_dust(&hole(0.0, 0.0, 0.001, 0.001)));
    }

    /// One delta per point of every ring of `region`, all the same.
    fn even(region: &ExPolygon, delta: f32) -> Vec<Vec<f32>> {
        region
            .rings()
            .map(|ring| vec![delta; ring.points.len()])
            .collect()
    }

    /// Lowest and highest x and y of a region's contour, in mm.
    fn bounds(region: &ExPolygon) -> (f64, f64, f64, f64) {
        let points = &region.contour.points;
        let low = |axis: fn(&Point2) -> Coord| points.iter().map(axis).min().unwrap() as f64 / 1e6;
        let high = |axis: fn(&Point2) -> Coord| points.iter().map(axis).max().unwrap() as f64 / 1e6;
        (low(|p| p.x), low(|p| p.y), high(|p| p.x), high(|p| p.y))
    }

    #[test]
    fn variable_offset_with_even_deltas_is_a_mitred_offset() {
        let ring = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: vec![hole(4.0, 4.0, 6.0, 6.0)],
        };
        let shrunk = variable_offset_inner(&ring, &even(&ring, -1e6), 2.0).unwrap();
        let plain = offset(
            std::slice::from_ref(&ring),
            scaled(-1.0),
            Join::Miter { limit: 2.0 },
        )
        .unwrap();
        assert_eq!(shrunk.len(), 1);
        assert_eq!(area(&shrunk), area(&plain));
        assert_eq!(area(&shrunk), 64.0 - 16.0);
        assert_eq!(shrunk[0].contour.points.len(), 4);
        assert_eq!(shrunk[0].holes[0].points.len(), 4);
        assert_oriented(&shrunk);
        // Nothing to move leaves the region as it was.
        let same = variable_offset_inner(&ring, &even(&ring, 0.0), 2.0).unwrap();
        assert_eq!(area(&same), 96.0);
        assert_eq!(same[0].contour.points.len(), 4);
    }

    #[test]
    fn variable_offset_moves_each_vertex_by_its_own_delta() {
        // A 20 x 4 bar with a point every 5 mm along both long sides; the
        // left end gives up 0.5 mm, the right end nothing, and the points
        // between fall off evenly.
        let xs = [0.0, 5.0, 10.0, 15.0, 20.0];
        let bottom = xs.iter().map(|&x| (x, 0.0));
        let top = xs.iter().rev().map(|&x| (x, 4.0));
        let points: Vec<(f64, f64)> = bottom.chain(top).collect();
        let deltas: Vec<f32> = points
            .iter()
            .map(|&(x, _)| (-0.5e6 * (1.0 - x / 20.0)) as f32)
            .collect();
        let bar = ExPolygon {
            contour: Polygon {
                points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
            },
            holes: Vec::new(),
        };
        let shaped = variable_offset_inner(&bar, &[deltas], 2.0).unwrap();
        assert_eq!(shaped.len(), 1);
        assert_eq!(bounds(&shaped[0]), (0.5, 0.0, 20.0, 4.0));
        // A straight taper: 3 + x / 20 wide from the left end at x = 0.5
        // to the untouched right end.
        let taper = 3.0 * 19.5 + (400.0 - 0.25) / 40.0;
        assert!(
            (area(&shaped) - taper).abs() < 1e-6,
            "area {} vs {taper}",
            area(&shaped)
        );
        // The width at each station is what its delta leaves.
        for (x, width) in [(5.0, 3.25), (10.0, 3.5), (15.0, 3.75)] {
            let cut = intersection(&shaped, &solid(x - 0.001, -1.0, x + 0.001, 5.0)).unwrap();
            let (_, low, _, high) = bounds(&cut[0]);
            assert!(
                (high - low - width).abs() < 0.01,
                "width {} at x = {x}",
                high - low
            );
        }
        assert_oriented(&shaped);
    }

    #[test]
    fn variable_offset_grows_holes_and_reports_a_pinched_shape() {
        // Only the hole is moved: it grows from 2 mm to 3 mm across.
        let ring = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: vec![hole(4.0, 4.0, 6.0, 6.0)],
        };
        let deltas = vec![vec![0.0; 4], vec![-0.5e6; 4]];
        let grown = variable_offset_inner(&ring, &deltas, 2.0).unwrap();
        assert_eq!((grown.len(), area(&grown)), (1, 100.0 - 9.0));
        assert_oriented(&grown);

        // Two 10 mm blocks joined by a neck 1 mm wide: shrinking by 0.6 mm
        // all round parts them, and the caller is told so.
        let dumbbell = union(&[
            rect(0.0, 0.0, 10.0, 10.0),
            rect(10.0, 4.5, 15.0, 5.5),
            rect(15.0, 0.0, 25.0, 10.0),
        ])
        .unwrap();
        let apart = variable_offset_inner(&dumbbell[0], &even(&dumbbell[0], -0.6e6), 2.0).unwrap();
        assert_eq!(apart.len(), 2);
        assert!((area(&apart) - 2.0 * 8.8 * 8.8).abs() < 1e-9);
        // Shrinking by more than there is leaves nothing.
        let gone = variable_offset_inner(&ring, &even(&ring, -6e6), 2.0).unwrap();
        assert!(gone.is_empty());
    }

    #[test]
    fn variable_offset_cuts_sharp_corners_square() {
        // A block with a notch 0.4 mm wide at the mouth running 4 mm in to
        // a point. Shrinking by 0.2 mm pushes the notch's point outward;
        // mitred it would run on several millimetres, cut square it stops
        // 0.2 mm beyond the point.
        let notched = ExPolygon {
            contour: Polygon {
                points: [
                    (0.0, 0.0),
                    (4.8, 0.0),
                    (5.0, 4.0),
                    (5.2, 0.0),
                    (10.0, 0.0),
                    (10.0, 10.0),
                    (0.0, 10.0),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        };
        let shrunk = variable_offset_inner(&notched, &even(&notched, -0.2e6), 2.0).unwrap();
        assert_eq!(shrunk.len(), 1);
        let reach = shrunk[0]
            .contour
            .points
            .iter()
            .filter(|p| (p.x - scaled(5.0)).abs() < scaled(1.0))
            .map(|p| p.y)
            .max()
            .unwrap() as f64
            / 1e6;
        assert!((reach - 4.2).abs() < 0.001, "the notch reaches {reach}");
    }

    #[test]
    fn variable_offset_wants_a_delta_for_every_point() {
        let square = &solid(0.0, 0.0, 10.0, 10.0)[0];
        for deltas in [
            vec![],
            vec![vec![-1.0; 3]],
            vec![vec![-1.0; 4], vec![-1.0; 4]],
            vec![vec![1.0; 4]],
            vec![vec![f32::NAN; 4]],
        ] {
            assert!(matches!(
                variable_offset_inner(square, &deltas, 2.0),
                Err(ClipError::InvalidDelta)
            ));
        }
    }

    #[test]
    fn out_of_range_coordinates_are_an_error() {
        let far = Polygon {
            points: vec![
                Point2 { x: 0, y: 0 },
                Point2 { x: i64::MAX, y: 0 },
                Point2 { x: 0, y: i64::MAX },
            ],
        };
        assert!(matches!(union(&[far]), Err(ClipError::OutOfRange)));
    }
    #[test]
    fn open_lines_are_cut_to_what_lies_inside() {
        let square = vec![ExPolygon {
            contour: Polygon {
                points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: vec![Polygon {
                points: [(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            }],
        }];
        // One line right across through the hole, one that misses it,
        // and one wholly outside.
        let lines = vec![
            vec![Point2::from_mm(-5.0, 5.0), Point2::from_mm(15.0, 5.0)],
            vec![Point2::from_mm(-5.0, 2.0), Point2::from_mm(15.0, 2.0)],
            vec![Point2::from_mm(-5.0, 20.0), Point2::from_mm(15.0, 20.0)],
        ];
        let cut = clip_lines(&lines, &square).unwrap();
        let mut lengths: Vec<f64> = cut
            .iter()
            .map(|line| {
                line.windows(2)
                    .map(|pair| {
                        ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64)
                    })
                    .sum::<f64>()
                    / 1e6
            })
            .collect();
        lengths.sort_by(f64::total_cmp);
        assert_eq!(lengths, [4.0, 4.0, 10.0]);
        assert!(clip_lines(&lines, &[]).unwrap().is_empty());
        assert!(clip_lines(&[], &square).unwrap().is_empty());
    }
}
