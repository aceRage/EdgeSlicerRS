//! The path of an inward wipe (`wipe_inward`): the wipe that follows an
//! outer wall, moved in off the wall and over the inner walls already
//! printed behind it, so the hot nozzle is not dragged back along the
//! face of the part.
//!
//! This is GCode/WipePathHelpers.cpp, function for function: the path
//! the nozzle would be wiped along is offset toward the nearest inner
//! wall, or failing that moved bodily toward it, or failing that
//! replaced by one short move in, and each of those is taken only if
//! every part of it lies over a wall that is already down, on the side
//! of the outer wall the inner wall is on. A path stored here keeps the
//! reference's form: its first point is a stand-in for wherever the
//! nozzle is when the wipe is made, and the wipe runs from there through
//! the points after it.
//!
//! The reference looks its lines up in a tree; here they are few (the
//! caller hands over only those near the seam) and are gone through one
//! by one.

use edge_geom::Point2;

/// libslic3r.h `EPSILON`, and `SCALED_EPSILON` in coordinate units.
const EPSILON: f64 = 1e-4;
pub(crate) const SCALED_EPSILON: f64 = 100.0;
/// "Matches DefaultMiterLimit from ClipperUtils.hpp. When the miter join
/// extends more than miter_limit * offset_dist from the original vertex,
/// the miter is replaced by a bevel join."
const MITER_LIMIT: f64 = 3.0;
/// "Normalized dot product below this means the segments point in nearly
/// opposite directions (angle > ~172°). Offsetting such a path is unsafe."
const REVERSAL_DOT_THRESHOLD: f64 = -0.99;
/// "candidates pointing more than 60 degrees away from the selected inner
/// wall are too tangent to distinguish the material side reliably at a
/// cusp."
const MIN_SUPPORT_ALIGNMENT: f64 = 0.5;

/// A vector in coordinate units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct V {
    pub(crate) x: f64,
    pub(crate) y: f64,
}

impl V {
    pub(crate) fn of(point: Point2) -> Self {
        Self {
            x: point.x as f64,
            y: point.y as f64,
        }
    }

    /// From `from` to `to`.
    pub(crate) fn between(from: Point2, to: Point2) -> Self {
        Self {
            x: (to.x - from.x) as f64,
            y: (to.y - from.y) as f64,
        }
    }

    pub(crate) fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    fn squared(self) -> f64 {
        self.dot(self)
    }

    pub(crate) fn norm(self) -> f64 {
        self.squared().sqrt()
    }

    /// One unit long; a vector of no length stays as it is.
    pub(crate) fn normalized(self) -> Self {
        let norm = self.norm();
        if norm > 0.0 {
            self.times(1.0 / norm)
        } else {
            self
        }
    }

    /// Turned a quarter turn to the left.
    fn left(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }

    pub(crate) fn times(self, factor: f64) -> Self {
        Self {
            x: self.x * factor,
            y: self.y * factor,
        }
    }

    pub(crate) fn plus(self, other: Self) -> Self {
        Self {
            x: self.x + other.x,
            y: self.y + other.y,
        }
    }

    fn minus(self, other: Self) -> Self {
        Self {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    /// The point it reaches from the origin, cut to whole units as the
    /// reference's `cast<coord_t>()` cuts.
    pub(crate) fn point(self) -> Point2 {
        Point2 {
            x: self.x as i64,
            y: self.y as i64,
        }
    }
}

/// A straight stretch of a printed wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) a: Point2,
    pub(crate) b: Point2,
}

impl Line {
    /// Line.hpp `distance_to_squared`: how far `point` is from the
    /// stretch, squared, and the point of the stretch nearest it.
    pub(crate) fn nearest(&self, point: Point2) -> (f64, Point2) {
        let v = V::between(self.a, self.b);
        let va = V::between(self.a, point);
        let l2 = v.squared();
        if l2 == 0.0 {
            return (va.squared(), self.a);
        }
        let t = va.dot(v) / l2;
        if t <= 0.0 {
            (va.squared(), self.a)
        } else if t >= 1.0 {
            (V::between(self.b, point).squared(), self.b)
        } else {
            (
                v.times(t).minus(va).squared(),
                V::of(self.a).plus(v.times(t)).point(),
            )
        }
    }

    fn distance_to(&self, point: Point2) -> f64 {
        self.nearest(point).0.sqrt()
    }

    /// Line.hpp `line_alg::intersection`: where the two stretches cross,
    /// if they do; never for two that run alongside.
    fn crossing(&self, other: &Line) -> Option<Point2> {
        let v1 = V::between(self.a, self.b);
        let v2 = V::between(other.a, other.b);
        let denom = v1.cross(v2);
        if denom.abs() < EPSILON {
            return None;
        }
        let v12 = V::between(other.a, self.a);
        let t1 = v2.cross(v12) / denom;
        let t2 = v1.cross(v12) / denom;
        ((0.0..=1.0).contains(&t1) && (0.0..=1.0).contains(&t2))
            .then(|| V::of(self.a).plus(v1.times(t1)).point())
    }
}

/// The lines of a path from point to point.
pub(crate) fn lines_of(points: &[Point2]) -> Vec<Line> {
    points
        .windows(2)
        .map(|pair| Line {
            a: pair[0],
            b: pair[1],
        })
        .collect()
}

/// AABBTreeLines.hpp `distance_from_lines_extra`: how far `point` is
/// from the nearest of `lines`, which line that is, and its point
/// nearest `point`. None of no lines.
fn nearest(lines: &[Line], point: Point2) -> Option<(f64, usize, Point2)> {
    let mut best: Option<(f64, usize, Point2)> = None;
    for (index, line) in lines.iter().enumerate() {
        let (squared, at) = line.nearest(point);
        if best.is_none_or(|(least, _, _)| squared < least) {
            best = Some((squared, index, at));
        }
    }
    best.map(|(squared, index, at)| (squared.sqrt(), index, at))
}

/// How far `point` is from the nearest of `lines`; no end of far from none.
pub(crate) fn distance_from(lines: &[Line], point: Point2) -> f64 {
    nearest(lines, point).map_or(f64::INFINITY, |(distance, _, _)| distance)
}

fn length(points: &[Point2]) -> f64 {
    points
        .windows(2)
        .map(|pair| V::between(pair[0], pair[1]).norm())
        .sum()
}

/// Polyline.cpp `Polyline::clip_end`: shortens a path by `distance`
/// from its end. One no longer than that is left empty.
fn clip_end(points: &mut Vec<Point2>, mut distance: f64) {
    while distance > 0.0 {
        let Some(last) = points.pop() else {
            return;
        };
        let Some(&before) = points.last() else {
            return;
        };
        let v = V::between(last, before);
        let squared = v.squared();
        if squared > distance * distance {
            points.push(V::of(last).plus(v.times(distance / squared.sqrt())).point());
            return;
        }
        distance -= squared.sqrt();
    }
}

/// "Keep a scaled-coordinate rounding floor while allowing the tolerance
/// to follow the relevant offset or path length."
fn wipe_tolerance(distance: f64, relative: f64) -> f64 {
    (4.0 * SCALED_EPSILON).max(relative * distance)
}

/// `prepare_source`: "consecutive duplicates carry no path length and
/// can be removed safely. A reversal, however, is real travelled
/// distance: removing its vertex would replace a long backtracking wipe
/// with a short, unrelated shortcut." False for a path that doubles back.
fn prepare_source(points: &mut Vec<Point2>) -> bool {
    points.dedup();
    if points.len() < 2 {
        return false;
    }
    points.windows(3).all(|three| {
        let before = V::between(three[0], three[1]);
        let after = V::between(three[1], three[2]);
        before.dot(after) / (before.norm() * after.norm()) >= REVERSAL_DOT_THRESHOLD
    })
}

/// `build_offset_polyline`: the path moved `offset` to its left (with
/// `dir` at 1) or its right (at -1), its corners mitred, or cut off
/// where a mitre would reach too far. Also says which of the points
/// made is the corner between the path's first two stretches.
fn build_offset_polyline(
    original: &[Point2],
    dir: f64,
    offset: f64,
) -> Option<(Vec<Point2>, usize)> {
    if original.len() < 2 {
        return None;
    }
    let mut source = original.to_vec();
    if !prepare_source(&mut source) {
        return None;
    }
    let n = source.len();
    // "compute the perpendicular offset for segment i->i+1"
    let offset_segment = |a: Point2, b: Point2| -> Line {
        let v = V::between(a, b);
        let len = v.norm();
        let perp = if len > SCALED_EPSILON {
            v.left().times(dir * offset / len)
        } else {
            V { x: 0.0, y: 0.0 }
        };
        Line {
            a: V::of(a).plus(perp).point(),
            b: V::of(b).plus(perp).point(),
        }
    };
    let mut result = Vec::with_capacity(n);
    let mut first_join = 0;
    let mut previous_line = offset_segment(source[0], source[1]);
    result.push(previous_line.a);
    for i in 1..n - 1 {
        let next_line = offset_segment(source[i], source[i + 1]);
        let previous = V::between(source[i - 1], source[i]).normalized();
        let next = V::between(source[i], source[i + 1]).normalized();
        let denominator = 1.0 + previous.dot(next);
        let miter = (denominator > EPSILON)
            .then(|| {
                previous
                    .left()
                    .plus(next.left())
                    .times(dir * offset / denominator)
            })
            .filter(|miter| miter.norm() <= MITER_LIMIT * offset);
        match miter {
            Some(miter) => result.push(V::of(source[i]).plus(miter).point()),
            None => {
                result.push(previous_line.b);
                if result.last() != Some(&next_line.a) {
                    result.push(next_line.a);
                }
            }
        }
        if i == 1 {
            first_join = result.len() - 1;
        }
        previous_line = next_line;
    }
    result.push(previous_line.b);
    Some((result, first_join))
}

/// `wipe_offset_direction`: "the side of the printed path on which the
/// material lies. dir +1 is left and -1 is right".
pub(crate) fn offset_direction(is_ccw: bool, is_hole: bool) -> f64 {
    if is_ccw != is_hole { 1.0 } else { -1.0 }
}

/// `starts_by_backtracking`: whether the move from `actual_start` onto
/// the stored path heads back against the way the path then goes.
fn starts_by_backtracking(path: &[Point2], actual_start: Point2) -> bool {
    if path.len() < 3 {
        return false;
    }
    let connector = V::between(actual_start, path[1]);
    let outgoing = V::between(path[1], path[2]);
    connector.dot(outgoing) < -4.0 * SCALED_EPSILON * outgoing.norm()
}

/// `sample_polyline_at_distance`: the point `target` along a path, or
/// its end where the path is shorter.
fn sample_polyline_at_distance(points: &[Point2], mut target: f64) -> Point2 {
    let mut result = points[0];
    for &point in &points[1..] {
        if target <= 0.0 {
            break;
        }
        let segment = V::between(result, point);
        let length = segment.norm();
        if length <= SCALED_EPSILON {
            continue;
        }
        if target <= length {
            return V::of(result).plus(segment.times(target / length)).point();
        }
        target -= length;
        result = point;
    }
    result
}

/// `store_wipe_path`: a path the nozzle is to follow, from where it is,
/// as a wipe keeps it: no longer than the wipe, and with `seam_start`
/// standing in for its first point.
fn store_wipe_path(
    seam_start: Point2,
    mut actual: Vec<Point2>,
    max_wipe_length: f64,
) -> Option<Vec<Point2>> {
    if actual.len() < 2 || max_wipe_length <= SCALED_EPSILON {
        return None;
    }
    let actual_length = length(&actual);
    if actual_length <= SCALED_EPSILON {
        return None;
    }
    if actual_length - max_wipe_length > SCALED_EPSILON {
        clip_end(&mut actual, actual_length - max_wipe_length);
    }
    if actual.len() < 2 || actual.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    actual[0] = seam_start;
    Some(actual)
}

/// `offset_wipe_path`: the wipe path moved `offset_dist` to one side.
/// "The seam-gap or closing edge determines the join with the first
/// outgoing perimeter edge, but its offset is not part of the executable
/// wipe. Only the prefix needed by Wipe::wipe() is offset." `polyline`
/// is the wall as it was laid, from `seam_start` round to `seam_end`;
/// `wipe_start` is where the nozzle is.
pub(crate) fn offset_wipe_path(
    polyline: &[Point2],
    seam_start: Point2,
    seam_end: Point2,
    wipe_start: Point2,
    dir: f64,
    offset_dist: f64,
    max_wipe_length: f64,
) -> Option<Vec<Point2>> {
    if polyline.first() != Some(&seam_start) || max_wipe_length <= SCALED_EPSILON {
        return None;
    }
    let original_length = length(polyline);
    if original_length <= SCALED_EPSILON {
        return None;
    }
    let mut source_length = original_length.min(max_wipe_length);
    loop {
        let mut source = polyline.to_vec();
        let clip_distance = original_length - source_length;
        if clip_distance > SCALED_EPSILON {
            clip_end(&mut source, clip_distance);
        }
        let mut wrapped = Vec::with_capacity(source.len() + 1);
        if seam_start == seam_end {
            // "the stored loop is open at seam_start even when the seam
            // gap is zero. Prepend the closing edge so
            // build_offset_polyline() creates the proper join"
            let closing = polyline.iter().rposition(|&point| point != seam_start)?;
            wrapped.push(polyline[closing]);
        } else {
            // "use the unextruded seam-gap edge to determine the
            // incoming direction at the seam. Its offset is
            // construction geometry only"
            wrapped.push(seam_end);
        }
        wrapped.extend_from_slice(&source);

        let (mut offset_points, first_join) = build_offset_polyline(&wrapped, dir, offset_dist)?;
        if first_join == 0 || first_join >= offset_points.len() {
            return None;
        }
        // "discard the offset of the prepended edge and, for a bevel,
        // its incoming endpoint."
        offset_points.drain(..first_join);
        let mut actual = Vec::with_capacity(offset_points.len() + 1);
        actual.push(wipe_start);
        actual.extend(offset_points);

        // "A loop pre-move may advance past an otherwise valid offset
        // join. Enter at the nozzle's projection instead of returning to
        // the join."
        if seam_start != seam_end
            && wipe_start != seam_start
            && wipe_start != seam_end
            && starts_by_backtracking(&actual, wipe_start)
            && !starts_by_backtracking(&actual, seam_end)
        {
            let mut entry = 1;
            while entry + 1 < actual.len() {
                let edge = V::between(actual[entry], actual[entry + 1]);
                let projection = V::between(actual[entry], wipe_start).dot(edge);
                if projection <= 0.0 {
                    break;
                }
                if projection < edge.squared() {
                    actual[entry] = V::of(actual[entry])
                        .plus(edge.times(projection / edge.squared()))
                        .point();
                    break;
                }
                entry += 1;
            }
            actual.drain(1..entry);
        }
        // "a wide seam gap or a sharp cusp may put the first miter
        // behind its outgoing edge. Reject this offset candidate"
        if seam_start != seam_end
            && wipe_start == seam_end
            && starts_by_backtracking(&actual, wipe_start)
        {
            return None;
        }

        let actual_length = length(&actual);
        let source_exhausted = original_length - source_length <= SCALED_EPSILON;
        if actual_length + SCALED_EPSILON < max_wipe_length && !source_exhausted {
            // "offset joins may shorten the path at every corner. Grow
            // the source until the executable offset path ... reaches
            // the configured wipe distance."
            let deficit = max_wipe_length - actual_length;
            let next_length =
                original_length.min(source_length + deficit.max(2.0 * SCALED_EPSILON));
            if next_length - source_length <= SCALED_EPSILON {
                return None;
            }
            source_length = next_length;
            continue;
        }
        return store_wipe_path(seam_start, actual, max_wipe_length);
    }
}

/// `translated_wipe_path`: the wipe path moved bodily by `translation`.
fn translated_wipe_path(
    polyline: &[Point2],
    seam_start: Point2,
    seam_end: Point2,
    wipe_start: Point2,
    translation: V,
    max_wipe_length: f64,
) -> Option<Vec<Point2>> {
    if translation.norm() <= SCALED_EPSILON || max_wipe_length <= SCALED_EPSILON {
        return None;
    }
    let mut actual = Vec::with_capacity(polyline.len() + 2);
    actual.push(wipe_start);
    let append = |actual: &mut Vec<Point2>, point: Point2| {
        let moved = V::of(point).plus(translation).point();
        if actual.last() != Some(&moved) {
            actual.push(moved);
        }
    };
    // "translate the seam join directly. Translating seam_end and then
    // following the unextruded gap back to seam_start makes the wipe
    // double back whenever a gap ends near a sharp corner."
    append(&mut actual, seam_start);
    for &point in polyline {
        append(&mut actual, point);
    }
    if seam_start != seam_end
        && wipe_start == seam_end
        && starts_by_backtracking(&actual, wipe_start)
    {
        // "Prefer a shorter local inward move at the actual extrusion
        // end over a longer lightning-shaped wipe."
        actual.truncate(1);
        append(&mut actual, seam_end);
    }
    store_wipe_path(seam_start, actual, max_wipe_length)
}

/// `segment_is_supported`: "A segment whose endpoints lie within one
/// line's distance capsule is fully supported, since that capsule is
/// convex. Subdivide only when support changes between lines".
fn segment_is_supported(start: Point2, end: Point2, lines: &[Line], max_distance: f64) -> bool {
    let midpoint = V::of(start).plus(V::of(end)).times(0.5).point();
    let Some((distance, index, _)) = nearest(lines, midpoint) else {
        return false;
    };
    if distance > max_distance {
        return false;
    }
    let line = &lines[index];
    if line.distance_to(start) <= max_distance && line.distance_to(end) <= max_distance {
        return true;
    }
    if distance_from(lines, start) > max_distance || distance_from(lines, end) > max_distance {
        return false;
    }
    // "Conservatively reject an unresolved transition at coordinate
    // precision."
    if V::between(start, end).norm() <= SCALED_EPSILON {
        return false;
    }
    segment_is_supported(start, midpoint, lines, max_distance)
        && segment_is_supported(midpoint, end, lines, max_distance)
}

/// `wipe_path_support_score`: how far the stored path's first move ends
/// from the inner walls in `target`, or none where no such wall is near
/// `wipe_start` or any of the path has nothing in `all_support` under
/// it. "decide direction at the seam. Scoring the complete path may
/// select the wrong initial side when two contours converge".
pub(crate) fn wipe_path_support_score(
    polyline: &[Point2],
    wipe_start: Point2,
    target: &[Line],
    all_support: &[Line],
    max_distance: f64,
) -> Option<f64> {
    if polyline.len() < 2 || target.is_empty() || max_distance <= 0.0 {
        return None;
    }
    // "require a local neighbour, not merely an earlier perimeter
    // elsewhere in the region."
    if distance_from(target, wipe_start) > MITER_LIMIT * max_distance + 4.0 * SCALED_EPSILON {
        return None;
    }
    let mut previous = wipe_start;
    for &point in &polyline[1..] {
        if !segment_is_supported(
            previous,
            point,
            all_support,
            max_distance + 4.0 * SCALED_EPSILON,
        ) {
            return None;
        }
        previous = point;
    }
    Some(distance_from(target, polyline[1]))
}

/// `initial_connector_is_clear`: whether the first move of a stored path
/// leaves the wall it starts on without meeting it again.
fn initial_connector_is_clear(
    polyline: &[Point2],
    wipe_start: Point2,
    seam_start: Point2,
    current: &[Line],
    contact_tolerance: f64,
) -> bool {
    if polyline.len() < 2 || polyline[1] == wipe_start {
        return false;
    }
    // "without a seam gap, the connector necessarily starts at the wall
    // and a self-touching cusp may share that same endpoint"
    if seam_start == wipe_start {
        return true;
    }
    let connector = Line {
        a: wipe_start,
        b: polyline[1],
    };
    let crossed = current
        .iter()
        .filter_map(|line| connector.crossing(line))
        .any(|at| V::between(wipe_start, at).norm() > contact_tolerance);
    if crossed {
        return false;
    }
    // "Treat a close pass through that point as retracing the external
    // wall, but keep the unavoidable contact at the actual start."
    let (squared, closest) = connector.nearest(seam_start);
    !(squared <= contact_tolerance * contact_tolerance
        && V::between(wipe_start, closest).norm() > contact_tolerance)
}

/// `support_offset_at_start`: the way from the start of `source` to the
/// nearest of the `support` lines. With `disambiguate_branch`, "a nonzero
/// gap may put the seam beside the wrong branch of a cusp. Sample farther
/// along the path to identify its actual neighbouring wall", and then
/// "find the local point on the same material-side branch" from
/// `local_origin`.
fn support_offset_at_start(
    source: &[Point2],
    local_origin: Point2,
    disambiguate_branch: bool,
    support: &[Line],
    max_support_distance: f64,
) -> Option<V> {
    if source.len() < 2 {
        return None;
    }
    let query = if disambiguate_branch {
        sample_polyline_at_distance(source, 2.0 * max_support_distance)
    } else {
        source[0]
    };
    let (_, index, at) = nearest(support, query)?;
    let mut sampled = V::between(query, at);
    if disambiguate_branch {
        // "an endpoint projection also contains distance along the
        // support segment. Remove that tangent component"
        let edge = V::between(support[index].a, support[index].b);
        if edge.norm() > SCALED_EPSILON {
            let tangent = edge.normalized();
            sampled = sampled.minus(tangent.times(sampled.dot(tangent)));
        }
    }
    if sampled.norm() <= SCALED_EPSILON {
        return None;
    }
    if !disambiguate_branch {
        return Some(sampled);
    }
    let direction = sampled.normalized();
    let mut local = sampled;
    let mut best = f64::INFINITY;
    let radius = 2.0 * max_support_distance + 4.0 * SCALED_EPSILON;
    for line in support {
        let (squared, at) = line.nearest(local_origin);
        // Within the radius, the edge of it left out (AABBTreeLines.hpp
        // `all_lines_in_radius`).
        if squared >= radius * radius {
            continue;
        }
        let candidate = V::between(local_origin, at);
        let distance = squared.sqrt();
        if distance <= SCALED_EPSILON {
            continue;
        }
        let alignment = candidate.normalized().dot(direction);
        if alignment < MIN_SUPPORT_ALIGNMENT {
            continue;
        }
        let score = distance / alignment;
        if score < best {
            best = score;
            local = candidate;
        }
    }
    Some(local)
}

/// `executable_path_length`: a stored path's length from where the
/// nozzle is.
fn executable_path_length(stored: &[Point2], wipe_start: Point2) -> f64 {
    if stored.len() < 2 {
        return 0.0;
    }
    V::between(wipe_start, stored[1]).norm() + length(&stored[1..])
}

/// `material_side_support_lines`: for a loop closed at its seam, the
/// `support` lines on the side of the seam the part is on. "the bisector
/// of the incoming and outgoing material-side normals is a local side
/// test that remains valid for globally self-touching Arachne contours."
fn material_side_support_lines(
    path: &[Point2],
    seam: Point2,
    preferred_dir: f64,
    support: &[Line],
) -> Vec<Line> {
    if path.len() < 4 || path.first() != path.last() {
        return Vec::new();
    }
    let outgoing = path[1..].iter().find(|&&point| point != seam);
    let incoming = path[..path.len() - 1].iter().rfind(|&&point| point != seam);
    let (Some(&outgoing), Some(&incoming)) = (outgoing, incoming) else {
        return Vec::new();
    };
    let outgoing = V::between(seam, outgoing).normalized();
    let incoming = V::between(incoming, seam).normalized();
    let material = outgoing.left().plus(incoming.left()).times(preferred_dir);
    if material.norm() <= EPSILON {
        return Vec::new();
    }
    support
        .iter()
        .filter(|line| V::between(seam, line.nearest(seam).1).dot(material) > SCALED_EPSILON)
        .copied()
        .collect()
}

/// `wipe_path_stays_on_material_side`: whether a stored path sets off
/// toward the inner wall and stays on that side of the wall just laid
/// (`current`) all along, and, with `require_clearance`, clear of it by
/// most of `effective_offset`. "An open or self-touching wall has no
/// reliable polygon-wide sign. Orient its local normal toward the
/// neighbouring printed inner wall, then test the candidate on that side
/// at every sample."
pub(crate) fn wipe_path_stays_on_material_side(
    path: &[Point2],
    path_start: Point2,
    support_direction: V,
    target: &[Line],
    current: &[Line],
    effective_offset: f64,
    require_clearance: bool,
) -> bool {
    if path.len() < 2
        || support_direction.norm() <= EPSILON
        || target.is_empty()
        || current.is_empty()
        || effective_offset <= SCALED_EPSILON
    {
        return false;
    }
    let initial = V::between(path_start, path[1]);
    if initial.norm() <= SCALED_EPSILON
        || initial.normalized().dot(support_direction.normalized()) < MIN_SUPPORT_ALIGNMENT
    {
        return false;
    }
    // "an inward offset must retain most of its requested clearance from
    // the current external wall."
    let minimum_clearance = effective_offset - wipe_tolerance(effective_offset, 0.25);
    let left_normal = |line: &Line| -> V {
        let edge = V::between(line.a, line.b);
        if edge.norm() <= SCALED_EPSILON {
            V { x: 0.0, y: 0.0 }
        } else {
            edge.left().normalized()
        }
    };
    let on_material_side = |point: Point2| -> bool {
        let Some((distance, index, at)) = nearest(current, point) else {
            return false;
        };
        let line = &current[index];
        let mut normal = left_normal(line);
        // "At a shared vertex use both incident edges"
        let previous = &current[(index + current.len() - 1) % current.len()];
        let next = &current[(index + 1) % current.len()];
        if V::between(line.a, at).norm() <= SCALED_EPSILON && previous.b == line.a {
            normal = normal.plus(left_normal(previous));
        }
        if V::between(line.b, at).norm() <= SCALED_EPSILON && next.a == line.b {
            normal = normal.plus(left_normal(next));
        }
        if normal.norm() <= EPSILON {
            return false;
        }
        let normal = normal.normalized();
        let Some((_, _, support_point)) = nearest(target, at) else {
            return false;
        };
        let support_side = V::between(at, support_point).dot(normal);
        if support_side.abs() <= 4.0 * SCALED_EPSILON {
            return false;
        }
        let side = V::between(at, point).dot(normal) * support_side.signum();
        side >= -4.0 * SCALED_EPSILON
            && (!require_clearance || distance + 4.0 * SCALED_EPSILON >= minimum_clearance)
    };

    let mut previous = path[1];
    if !on_material_side(previous) {
        return false;
    }
    for &point in &path[2..] {
        let segment = V::between(previous, point);
        let samples = ((segment.norm() / effective_offset).ceil() as usize).max(1);
        for sample in 1..=samples {
            let at = V::of(previous)
                .plus(segment.times(sample as f64 / samples as f64))
                .point();
            if !on_material_side(at) {
                return false;
            }
        }
        previous = point;
    }
    true
}

/// A path the wipe might take, and what it is chosen by.
struct Candidate {
    path: Vec<Point2>,
    /// How far its first move ends from the inner wall.
    support_score: f64,
    path_length: f64,
}

/// `offset_wipe_path_toward_support`: the path of the inward wipe after
/// a wall, or none where no safe one is found and the nozzle is wiped
/// along the wall as usual. `polyline` is the wall as it was laid, from
/// `seam_start` to `seam_end`; `wipe_start` is where the nozzle is;
/// `preferred_dir` the side of the wall the part is on;
/// `target` the inner walls already printed, `printed` every wall
/// already printed, `current` the wall just laid.
#[allow(clippy::too_many_arguments)]
pub(crate) fn offset_wipe_path_toward_support(
    polyline: &[Point2],
    seam_start: Point2,
    seam_end: Point2,
    wipe_start: Point2,
    preferred_dir: f64,
    offset_dist: f64,
    max_wipe_length: f64,
    target: &[Line],
    printed: &[Line],
    current: &[Line],
    max_support_distance: f64,
) -> Option<Vec<Point2>> {
    if polyline.len() < 2
        || target.is_empty()
        || current.is_empty()
        || offset_dist <= SCALED_EPSILON
        || max_wipe_length <= SCALED_EPSILON
        || max_support_distance <= SCALED_EPSILON
    {
        return None;
    }
    let gap = seam_start != seam_end;
    // "another contour may have a geometrically closer inner wall on
    // this loop's air side. Restrict zero-gap support using the local
    // seam normals before choosing the nearest wall."
    let material_side;
    let support: &[Line] = if gap {
        target
    } else {
        material_side = material_side_support_lines(polyline, seam_start, preferred_dir, target);
        if material_side.is_empty() {
            return None;
        }
        &material_side
    };
    let toward_support =
        support_offset_at_start(polyline, seam_end, gap, support, max_support_distance)?;
    let local_support_distance = toward_support.norm();
    let effective_offset = offset_dist.min(local_support_distance);
    if effective_offset <= SCALED_EPSILON {
        return None;
    }
    let support_direction = toward_support.times(1.0 / local_support_distance);

    let all_support: Vec<Line> = printed.iter().chain(current).copied().collect();
    // "allow only the contact needed to leave the extrusion endpoint."
    let contact_tolerance = wipe_tolerance(effective_offset, 0.1);

    // "Direction and wall contact have different origins after a loop
    // pre-move."
    let validate = |path: Vec<Point2>,
                    path_start: Point2,
                    direction_start: Point2,
                    path_contact_tolerance: f64,
                    candidate_direction: V,
                    candidate_offset: f64,
                    require_clearance: bool|
     -> Option<Candidate> {
        // "backtracking indicates a wrong join only across a nonzero gap."
        let backtracks_across_gap = gap && starts_by_backtracking(&path, wipe_start);
        let material_side = wipe_path_stays_on_material_side(
            &path,
            direction_start,
            candidate_direction,
            support,
            current,
            candidate_offset,
            require_clearance && !gap,
        );
        let connector_clear = initial_connector_is_clear(
            &path,
            wipe_start,
            path_start,
            current,
            path_contact_tolerance,
        );
        if backtracks_across_gap || !material_side || !connector_clear {
            return None;
        }
        let support_score = wipe_path_support_score(
            &path,
            wipe_start,
            support,
            &all_support,
            max_support_distance,
        )?;
        let path_length = executable_path_length(&path, wipe_start);
        Some(Candidate {
            path,
            support_score,
            path_length,
        })
    };

    let offset_candidate = |dir: f64| -> Option<Candidate> {
        let path = offset_wipe_path(
            polyline,
            seam_start,
            seam_end,
            wipe_start,
            dir,
            effective_offset,
            max_wipe_length,
        )?;
        validate(
            path,
            seam_start,
            seam_start,
            contact_tolerance,
            support_direction,
            effective_offset,
            true,
        )
    };
    let preferred = offset_candidate(preferred_dir);
    let alternate = offset_candidate(-preferred_dir);

    // "forward and reverse fallbacks share the same clamping,
    // translation, connector tolerance, and complete-path validation."
    let translated_candidate = |source: &[Point2],
                                source_start: Point2,
                                source_end: Point2,
                                support_offset: V|
     -> Option<Candidate> {
        let support_distance = support_offset.norm();
        let candidate_offset = offset_dist.min(support_distance);
        if candidate_offset <= SCALED_EPSILON {
            return None;
        }
        let translation = support_offset.times(candidate_offset / support_distance);
        let path = translated_wipe_path(
            source,
            source_start,
            source_end,
            wipe_start,
            translation,
            max_wipe_length,
        )?;
        validate(
            path,
            source_start,
            source_start,
            wipe_tolerance(candidate_offset, 0.1),
            support_offset.times(1.0 / support_distance),
            candidate_offset,
            true,
        )
    };
    let translated = translated_candidate(polyline, seam_start, seam_end, toward_support);

    // "if every full-length construction folds back onto the external
    // wall, retain a short direct inward move instead of accepting an
    // outward candidate or falling back to the standard wipe along the
    // outer wall."
    let direct = (|| {
        let destination = V::of(seam_end)
            .plus(support_direction.times(effective_offset))
            .point();
        if destination == wipe_start {
            return None;
        }
        let path = store_wipe_path(seam_start, vec![wipe_start, destination], max_wipe_length)?;
        validate(
            path,
            seam_end,
            wipe_start,
            wipe_tolerance(effective_offset, 0.1),
            support_direction,
            effective_offset,
            false,
        )
    })();

    let length_margin = wipe_tolerance(max_wipe_length, 0.1);
    let mut reversed: Option<Candidate> = None;
    if gap && polyline.last() == Some(&seam_end) {
        // "when a large gap straddles a sharp corner, connecting the
        // extrusion end to the forward offset may either reverse or
        // leave only a short local move. The already printed incoming
        // wall is equally safe: follow it backwards"
        let reversed_source: Vec<Point2> = polyline.iter().rev().copied().collect();
        if let Some(reversed_offset) = support_offset_at_start(
            &reversed_source,
            seam_end,
            true,
            support,
            max_support_distance,
        ) {
            reversed = translated_candidate(&reversed_source, seam_end, seam_end, reversed_offset);
            // "Offset the incoming wall itself when translation cannot
            // supply a complete wipe"
            if reversed
                .as_ref()
                .is_none_or(|found| found.path_length + length_margin < max_wipe_length)
            {
                let reverse_offset = offset_dist.min(reversed_offset.norm());
                let offset_path = (reverse_offset > SCALED_EPSILON)
                    .then(|| {
                        offset_wipe_path(
                            &reversed_source,
                            seam_end,
                            seam_start,
                            wipe_start,
                            -preferred_dir,
                            reverse_offset,
                            max_wipe_length,
                        )
                    })
                    .flatten();
                if let Some(mut path) = offset_path {
                    path[0] = seam_start;
                    let candidate = validate(
                        path,
                        seam_end,
                        seam_end,
                        wipe_tolerance(reverse_offset, 0.1),
                        reversed_offset.normalized(),
                        reverse_offset,
                        true,
                    );
                    if let Some(candidate) = candidate
                        && reversed.as_ref().is_none_or(|found| {
                            candidate.path_length > found.path_length + length_margin
                                && candidate.support_score
                                    <= found.support_score + wipe_tolerance(reverse_offset, 0.1)
                        })
                    {
                        reversed = Some(candidate);
                    }
                }
            }
        }
    }

    // "among the remaining paths, prefer the one whose first point is
    // materially closer to that wall."
    let direction_change_margin = wipe_tolerance(effective_offset, 0.1);
    let mut selected = preferred;
    if let Some(translated) = translated
        && selected.as_ref().is_none_or(|found| {
            translated.support_score + direction_change_margin < found.support_score
        })
    {
        selected = Some(translated);
    }
    // "Prefer a direct inward move when the normal offset cannot be
    // used. An alternate offset is eligible only after the same
    // material-side checks."
    let mut selected = selected.or(direct).or(alternate);
    // "prefer a complete reverse wipe over a forward fallback that had
    // to stop at the corner."
    if let Some(reversed) = reversed
        && selected.as_ref().is_none_or(|found| {
            reversed.path_length > found.path_length + length_margin
                && reversed.support_score <= found.support_score + direction_change_margin
        })
    {
        selected = Some(reversed);
    }
    selected.map(|found| found.path)
}

#[cfg(test)]
#[path = "inward_tests.rs"]
mod tests;
