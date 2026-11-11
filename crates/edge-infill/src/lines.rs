use std::collections::BTreeMap;

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

/// Where a fill row meets the area's outline.
struct Crossing {
    /// Which of the requested directions the row belongs to, and its number.
    direction: usize,
    row: i64,
    /// Position along the row.
    u: f64,
    ring: usize,
    /// The ring edge crossed (from vertex `edge` to the next).
    edge: usize,
    /// Distance along the ring from its first vertex, in scaled units.
    at: f64,
    point: Point2,
    /// The row stretch this crossing ends, when that stretch is kept.
    segment: Option<usize>,
    /// The crossing this one is joined to along the outline, and whether
    /// that one lies forwards along the ring from here.
    link: Option<(usize, bool)>,
    /// Where the stretch ends instead when this end is not joined to
    /// another: further out, on the outline the lines are cut to.
    loose: Option<Point2>,
    /// A short run along the outline that anchors an end joined to no
    /// other: whether it goes forwards along the ring, and how far.
    hook: Option<(bool, f64)>,
}

/// How line ends are joined along the outline they are cut to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Joining {
    /// Two ends are joined when the outline between them is shorter than
    /// this; zero joins nothing.
    pub max_link: Coord,
    /// An end joined to no other runs this far along the outline, to hold
    /// on to the wall (the reference's `infill_anchor`); zero leaves it bare.
    pub anchor: Coord,
    /// Half a line's width: how far short of the next line's end an
    /// anchor stops.
    pub half_width: Coord,
    /// Where a line passes this close to the outline without ending
    /// there, the outline is left alone: nothing is joined or anchored
    /// along it, since the line already lies beside the wall. Zero turns
    /// this off.
    pub beside: Coord,
    /// How much of each end of a line is not counted for `beside`, so
    /// that a line does not rule out the outline it ends on.
    pub beside_clip: Coord,
    /// Which ends are joined, and the order the paths come out in.
    pub order: Order,
}

/// The order line ends are joined in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Order {
    /// Nearest pairs along the outline first (FillBase.cpp
    /// `connect_infill`).
    #[default]
    Nearest,
    /// By following the lines from the first row on, each to its
    /// neighbour on the row before or after: the reference's way with
    /// solid lines, which keeps a zigzag whole where the outline curves
    /// (FillRectilinear.cpp `traverse_graph_generate_polylines`). No
    /// anchors are run.
    Sweep,
    /// Swept from the first row to the last, so that no line is laid
    /// between two already down: see [`monotonic`]. A line is joined only
    /// to the one laid next, and the paths come out in the order they are
    /// to be printed, each from its first point. No anchors are run.
    Monotonic,
}

/// How far the point `t` along a→b is from the segment c→d.
fn apart(a: (f64, f64), b: (f64, f64), t: f64, c: (f64, f64), d: (f64, f64)) -> f64 {
    let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    let (vx, vy) = (d.0 - c.0, d.1 - c.1);
    let length2 = vx * vx + vy * vy;
    let along = if length2 > 0.0 {
        (((p.0 - c.0) * vx + (p.1 - c.1) * vy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - (c.0 + vx * along)).hypot(p.1 - (c.1 + vy * along))
}

/// The stretch of a→b, as shares of it, that lies within `radius` of the
/// segment c→d; none when it stays further off. Distance to a segment
/// along a line has one low point, so the stretch is one piece.
fn near(
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    d: (f64, f64),
    radius: f64,
) -> Option<(f64, f64)> {
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let (one, two) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        if apart(a, b, one, c, d) < apart(a, b, two, c, d) {
            high = two;
        } else {
            low = one;
        }
    }
    let nearest = (low + high) / 2.0;
    if apart(a, b, nearest, c, d) >= radius {
        return None;
    }
    // Outwards from the nearest point to where the distance reaches `radius`.
    let edge = |mut inside: f64, mut outside: f64| {
        if apart(a, b, outside, c, d) < radius {
            return outside;
        }
        for _ in 0..50 {
            let middle = (inside + outside) / 2.0;
            if apart(a, b, middle, c, d) < radius {
                inside = middle;
            } else {
                outside = middle;
            }
        }
        inside
    };
    Some((edge(nearest, 0.0), edge(nearest, 1.0)))
}

/// Where each direction's rows lie: one `first` across from the origin
/// and the rest every spacing from it, kept only `within` these bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rows {
    pub first: f64,
    pub within: (f64, f64),
}

impl Rows {
    /// Rows on the grid through `first`, everywhere.
    pub fn everywhere(first: f64) -> Self {
        Self {
            first,
            within: (f64::NEG_INFINITY, f64::INFINITY),
        }
    }
}

/// The points an anchor passes from `point` on a ring's edge `edge`,
/// going `length` along the ring, forwards or backwards.
fn along(
    ring: &Polygon,
    edge: usize,
    point: Point2,
    forward: bool,
    mut length: f64,
) -> Vec<Point2> {
    let count = ring.points.len();
    let (mut from, mut index) = (point, edge);
    let mut passed = Vec::new();
    for _ in 0..=count {
        let corner = if forward {
            ring.points[(index + 1) % count]
        } else {
            ring.points[index % count]
        };
        let (dx, dy) = ((corner.x - from.x) as f64, (corner.y - from.y) as f64);
        let distance = dx.hypot(dy);
        if distance >= length {
            if distance > 0.0 {
                let share = length / distance;
                passed.push(Point2 {
                    x: from.x + (dx * share).round() as Coord,
                    y: from.y + (dy * share).round() as Coord,
                });
            }
            break;
        }
        passed.push(corner);
        length -= distance;
        from = corner;
        index = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
    }
    passed
}

/// Where the rows of each direction cross `rings`, and each ring's length.
fn crossings_of(
    rings: &[&Polygon],
    angles: &[f64],
    spacing: f64,
    rows: &[Rows],
) -> (Vec<Crossing>, Vec<f64>) {
    let mut ring_lengths = vec![0.0f64; rings.len()];
    let mut crossings: Vec<Crossing> = Vec::new();
    for (direction, angle) in angles.iter().enumerate() {
        let Rows { first, within } = rows[direction.min(rows.len() - 1)];
        let (sin, cos) = angle.sin_cos();
        // In the rotated frame `u` runs along the rows and `v` across them.
        let rotated = |p: Point2| {
            (
                p.x as f64 * cos + p.y as f64 * sin,
                p.y as f64 * cos - p.x as f64 * sin,
            )
        };
        for (ring, polygon) in rings.iter().enumerate() {
            let mut travelled = 0.0;
            for (edge, &a) in polygon.points.iter().enumerate() {
                let b = polygon.points[(edge + 1) % polygon.points.len()];
                let ((ua, va), (ub, vb)) = (rotated(a), rotated(b));
                let length = (ub - ua).hypot(vb - va);
                // A row crosses this edge when exactly one end is at or
                // below it, so a row through a vertex counts the two edges
                // meeting there once between them, not twice or never.
                let from = ((va.min(vb) - first) / spacing).ceil() as i64;
                let end = ((va.max(vb) - first) / spacing).ceil() as i64;
                for row in from..end {
                    let v = first + row as f64 * spacing;
                    if v < within.0 || v > within.1 {
                        continue;
                    }
                    let t = (v - va) / (vb - va);
                    let u = ua + t * (ub - ua);
                    crossings.push(Crossing {
                        direction,
                        row,
                        u,
                        ring,
                        edge,
                        at: travelled + t * length,
                        point: Point2 {
                            x: (u * cos - v * sin).round() as Coord,
                            y: (u * sin + v * cos).round() as Coord,
                        },
                        segment: None,
                        link: None,
                        loose: None,
                        hook: None,
                    });
                }
                travelled += length;
            }
            ring_lengths[ring] = travelled;
        }
    }
    (crossings, ring_lengths)
}

/// Crossings in the order they lie along their rows, row after row.
fn along_rows(crossings: &[Crossing]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..crossings.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&crossings[a], &crossings[b]);
        (a.direction, a.row)
            .cmp(&(b.direction, b.row))
            .then(a.u.total_cmp(&b.u))
    });
    order
}

/// Parallel lines across `area`, `spacing` apart, in each of `angles`
/// (radians counter-clockwise from +x), joined into longer paths where they
/// can be. Lines sit on one grid for the whole plane, so separate areas and
/// successive layers line up.
///
/// A row stretch shorter than `min_length` is dropped. Ends of stretches
/// that are neighbours along the outline, no more than `max_link` apart
/// along it, are joined by following the outline between them, nearest
/// pairs first and each end joined at most once, so the nozzle keeps
/// extruding instead of hopping. This is the scheme of the reference's
/// `connect_infill`, without the short anchors it adds to ends left
/// unjoined. A `max_link` of zero joins nothing.
pub fn paths(
    area: &ExPolygon,
    spacing: Coord,
    angles: &[f64],
    min_length: Coord,
    max_link: Coord,
) -> Vec<Vec<Point2>> {
    paths_from(area, spacing as f64, 0.0, angles, min_length, max_link)
}

/// How far across its lines an area reaches, in the frame turned to
/// `angle`: the least and the greatest of its points' distances across.
pub fn extent_across(area: &ExPolygon, angle: f64) -> Option<(f64, f64)> {
    let (sin, cos) = angle.sin_cos();
    area.contour
        .points
        .iter()
        .map(|p| p.y as f64 * cos - p.x as f64 * sin)
        .fold(None, |extent, v| match extent {
            None => Some((v, v)),
            Some((low, high)) => Some((f64::min(low, v), f64::max(high, v))),
        })
}

/// As [`paths`], with the lines on a grid of the caller's choosing: a
/// line lies `first` across from the origin, in the frame turned to each
/// angle, and the rest every `spacing` from it. Solid fill is laid this
/// way, fitted to the area it fills.
pub fn paths_from(
    area: &ExPolygon,
    spacing: f64,
    first: f64,
    angles: &[f64],
    min_length: Coord,
    max_link: Coord,
) -> Vec<Vec<Point2>> {
    let rings: Vec<&Polygon> = area.rings().collect();
    let joining = Joining {
        max_link,
        ..Joining::default()
    };
    lay(
        &rings,
        None,
        spacing,
        &[Rows::everywhere(first)],
        angles,
        min_length,
        joining,
    )
}

/// Sparse lines as the reference's `fill_surface_by_multilines` and
/// `connect_infill` lay them: each direction's rows where `rows` puts
/// them, ends joined along the outline where it is short enough, and an
/// end joined to nothing anchored by a short run along the outline.
pub fn paths_anchored(
    area: &ExPolygon,
    spacing: f64,
    rows: &[Rows],
    angles: &[f64],
    joining: Joining,
) -> Vec<Vec<Point2>> {
    let rings: Vec<&Polygon> = area.rings().collect();
    lay(&rings, None, spacing, rows, angles, 0, joining)
}

/// Solid lines as the reference's `FillRectilinear` lays them, across
/// everything in `outer` at `angle`: a line runs from edge to edge of
/// `outer`, and where it is joined to its neighbour it turns short of the
/// edge, on the outline of `inner`, which lies inside `outer`. A line that
/// does not reach into `inner` at all is left out, as there. A `max_link`
/// of zero joins nothing, and every line then runs edge to edge. With
/// `monotonic` the paths come in the order they are to be laid, each from
/// its first point ([`Order::Monotonic`]).
pub fn paths_turned(
    outer: &[ExPolygon],
    inner: &[ExPolygon],
    spacing: f64,
    first: f64,
    angle: f64,
    max_link: Coord,
    monotonic: bool,
) -> Vec<Vec<Point2>> {
    let rings: Vec<&Polygon> = inner.iter().flat_map(ExPolygon::rings).collect();
    let edge: Vec<&Polygon> = outer.iter().flat_map(ExPolygon::rings).collect();
    let joining = Joining {
        max_link,
        order: if monotonic {
            Order::Monotonic
        } else {
            Order::Sweep
        },
        ..Joining::default()
    };
    lay(
        &rings,
        Some(&edge),
        spacing,
        &[Rows::everywhere(first)],
        &[angle],
        0,
        joining,
    )
}

/// How near the outline an end of a line must lie to be joined along it,
/// in scaled units. Lines cut to an area end on its outline to within the
/// rounding of the cut.
const ON_OUTLINE: f64 = 2_000.0;

/// Lines of any shape that have been cut to `area`, joined end to end
/// along its outline and anchored where they are not, as the reference's
/// `connect_infill` does for the patterns that are not straight rows.
/// An end that does not lie on the outline is left as it is.
pub fn paths_joined(
    area: &ExPolygon,
    lines: Vec<Vec<Point2>>,
    joining: Joining,
) -> Vec<Vec<Point2>> {
    let rings: Vec<&Polygon> = area.rings().collect();
    let ring_lengths: Vec<f64> = rings
        .iter()
        .map(|ring| {
            let points = &ring.points;
            (0..points.len())
                .map(|at| {
                    let (a, b) = (points[at], points[(at + 1) % points.len()]);
                    ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
                })
                .sum()
        })
        .collect();
    // Where on the outline a point lies: (ring, edge, distance round the
    // ring), when it is near enough to be on it at all.
    let place = |point: Point2| -> Option<(usize, usize, f64)> {
        let (px, py) = (point.x as f64, point.y as f64);
        let mut best: Option<(f64, usize, usize, f64)> = None;
        for (ring, polygon) in rings.iter().enumerate() {
            let points = &polygon.points;
            let mut travelled = 0.0;
            for (edge, &a) in points.iter().enumerate() {
                let b = points[(edge + 1) % points.len()];
                let (ax, ay) = (a.x as f64, a.y as f64);
                let (dx, dy) = (b.x as f64 - ax, b.y as f64 - ay);
                let length = dx.hypot(dy);
                if length > 0.0 {
                    let along = (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, length);
                    let off = (px - ax - dx * along / length).hypot(py - ay - dy * along / length);
                    if off <= ON_OUTLINE && best.is_none_or(|best| off < best.0) {
                        best = Some((off, ring, edge, travelled + along));
                    }
                }
                travelled += length;
            }
        }
        best.map(|(_, ring, edge, at)| (ring, edge, at))
    };
    let mut crossings = Vec::with_capacity(2 * lines.len());
    let mut segments = Vec::with_capacity(lines.len());
    let mut bends = Vec::with_capacity(lines.len());
    for (number, line) in lines.into_iter().enumerate() {
        if line.len() < 2 {
            continue;
        }
        for (end, &point) in [line[0], line[line.len() - 1]].iter().enumerate() {
            // An end off the outline is on no ring, and so joined to nothing.
            let (ring, edge, at) = place(point).unwrap_or((usize::MAX, 0, 0.0));
            crossings.push(Crossing {
                direction: 0,
                row: number as i64,
                u: end as f64,
                ring,
                edge,
                at,
                point,
                segment: None,
                link: None,
                loose: None,
                hook: None,
            });
        }
        segments.push([crossings.len() - 2, crossings.len() - 1]);
        bends.push(line[1..line.len() - 1].to_vec());
    }
    let by_row: Vec<usize> = (0..crossings.len()).collect();
    join(
        &rings,
        &ring_lengths,
        crossings,
        &segments,
        &bends,
        &by_row,
        joining,
    )
}

/// Rows across `rings`, joined along them. With `edge` given, a row is a
/// stretch across `edge` instead, joined where it crosses `rings`.
fn lay(
    rings: &[&Polygon],
    edge: Option<&[&Polygon]>,
    spacing: f64,
    rows: &[Rows],
    angles: &[f64],
    min_length: Coord,
    joining: Joining,
) -> Vec<Vec<Point2>> {
    if spacing <= 0.0 || rows.is_empty() {
        return Vec::new();
    }
    let (mut crossings, ring_lengths) = crossings_of(rings, angles, spacing, rows);
    let by_row = along_rows(&crossings);
    let row_of = |crossing: &Crossing| (crossing.direction, crossing.row);
    let segments: Vec<[usize; 2]> = match edge {
        // Along each row the area is inside between the 1st and 2nd
        // crossing, the 3rd and 4th, and so on.
        None => by_row
            .chunk_by(|&a, &b| row_of(&crossings[a]) == row_of(&crossings[b]))
            .flat_map(|row| row.as_chunks::<2>().0)
            .filter(|pair| crossings[pair[1]].u - crossings[pair[0]].u >= min_length as f64)
            .copied()
            .collect(),
        // A stretch across the edge is kept when it crosses the rings, and
        // is joined at the first and last of those crossings: it runs
        // straight over any place between where the rings pinch in.
        Some(edge) => {
            let (ends, _) = crossings_of(edge, angles, spacing, rows);
            let rows: BTreeMap<(usize, i64), &[usize]> = by_row
                .chunk_by(|&a, &b| row_of(&crossings[a]) == row_of(&crossings[b]))
                .map(|row| (row_of(&crossings[row[0]]), row))
                .collect();
            let mut segments = Vec::new();
            let mut loose = Vec::new();
            for row in along_rows(&ends).chunk_by(|&a, &b| row_of(&ends[a]) == row_of(&ends[b])) {
                let Some(inside) = rows.get(&row_of(&ends[row[0]])) else {
                    continue;
                };
                for stretch in row.as_chunks::<2>().0 {
                    let (from, to) = (&ends[stretch[0]], &ends[stretch[1]]);
                    let mut within = inside
                        .iter()
                        .copied()
                        .filter(|&c| crossings[c].u >= from.u && crossings[c].u <= to.u);
                    if let (Some(a), Some(b)) = (within.next(), within.next_back()) {
                        segments.push([a, b]);
                        loose.extend([(a, from.point), (b, to.point)]);
                    }
                }
            }
            for (crossing, point) in loose {
                crossings[crossing].loose = Some(point);
            }
            segments
        }
    };
    join(
        rings,
        &ring_lengths,
        crossings,
        &segments,
        &[],
        &by_row,
        joining,
    )
}

/// Joins stretches end to end along the rings their ends lie on, and
/// gives the paths that makes. `crossings` are the ends, `segments` the
/// two ends of each stretch, and `bends` the points a stretch passes
/// between them, from its first end to its second: none for a straight
/// one, and none at all (an empty list) when every stretch is straight.
/// `by_row` is the order paths are begun in.
fn join(
    rings: &[&Polygon],
    ring_lengths: &[f64],
    mut crossings: Vec<Crossing>,
    segments: &[[usize; 2]],
    bends: &[Vec<Point2>],
    by_row: &[usize],
    joining: Joining,
) -> Vec<Vec<Point2>> {
    for (segment, ends) in segments.iter().enumerate() {
        for &end in ends {
            crossings[end].segment = Some(segment);
        }
    }

    // Ends that are neighbours along a ring, with the outline distance
    // between them: (distance, from, to), `to` lying forwards of `from`.
    // And for each end, its neighbours either way and how much of the
    // outline towards each is still free to run an anchor along.
    #[derive(Clone, Copy)]
    struct Free {
        prev: usize,
        next: usize,
        to_prev: f64,
        to_next: f64,
        free_prev: f64,
        free_next: f64,
        /// Joined or anchored already.
        taken: bool,
        /// The outline towards the neighbour is partly spoken for, so the
        /// two cannot be joined along it.
        cut_prev: bool,
        cut_next: bool,
    }
    let mut free: Vec<Option<Free>> = vec![None; crossings.len()];
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    // Each stretch less `beside_clip` at either end, for telling where it
    // runs beside the outline without ending there.
    let shortened: Vec<Vec<(f64, f64)>> = if joining.beside > 0 {
        let as_pair = |p: Point2| (p.x as f64, p.y as f64);
        segments
            .iter()
            .enumerate()
            .filter_map(|(segment, ends)| {
                let mut line = vec![as_pair(crossings[ends[0]].point)];
                line.extend(
                    bends
                        .get(segment)
                        .into_iter()
                        .flatten()
                        .copied()
                        .map(as_pair),
                );
                line.push(as_pair(crossings[ends[1]].point));
                shorten(&line, joining.beside_clip as f64)
            })
            .collect()
    } else {
        Vec::new()
    };
    for (ring, &ring_length) in ring_lengths.iter().enumerate() {
        let mut ends: Vec<usize> = (0..crossings.len())
            .filter(|&c| crossings[c].ring == ring && crossings[c].segment.is_some())
            .collect();
        ends.sort_by(|&a, &b| crossings[a].at.total_cmp(&crossings[b].at));
        for (index, &from) in ends.iter().enumerate() {
            let to = ends[(index + 1) % ends.len()];
            let before = ends[(index + ends.len() - 1) % ends.len()];
            // An end alone on its ring has the whole ring either way.
            let span = |a: usize, b: usize| {
                if a == b {
                    ring_length
                } else {
                    (crossings[b].at - crossings[a].at).rem_euclid(ring_length)
                }
            };
            let (to_prev, to_next) = (span(before, from), span(from, to));
            free[from] = Some(Free {
                prev: before,
                next: to,
                to_prev,
                to_next,
                free_prev: to_prev,
                free_next: to_next,
                taken: false,
                cut_prev: false,
                cut_next: false,
            });
            if from != to {
                candidates.push((to_next, from, to));
            }
        }
        // Where a line runs beside this ring without ending on it, the
        // ring is left alone (FillBase.cpp
        // `mark_boundary_segments_touching_infill`).
        if joining.beside > 0 && !ends.is_empty() {
            let radius = joining.beside as f64;
            let as_pair = |p: Point2| (p.x as f64, p.y as f64);
            let corners = &rings[ring].points;
            let mut travelled = 0.0;
            for (edge, &from) in corners.iter().enumerate() {
                let (a, b) = (as_pair(from), as_pair(corners[(edge + 1) % corners.len()]));
                let length = (b.0 - a.0).hypot(b.1 - a.1);
                let start = travelled;
                travelled += length;
                if length <= 0.0 {
                    continue;
                }
                for (c, d) in shortened
                    .iter()
                    .flat_map(|line| line.windows(2))
                    .map(|pair| (pair[0], pair[1]))
                {
                    let clear = |low: f64, high: f64, one: f64, two: f64| {
                        low.min(high) > one.max(two) + radius
                            || low.max(high) < one.min(two) - radius
                    };
                    if clear(a.0, b.0, c.0, d.0) || clear(a.1, b.1, c.1, d.1) {
                        continue;
                    }
                    let Some((low, high)) = near(a, b, c, d, radius) else {
                        continue;
                    };
                    let (low, high) = (start + low * length, start + high * length);
                    // The ends either side of the stretch, and any within it.
                    let before = ends
                        .iter()
                        .rposition(|&end| crossings[end].at <= low)
                        .unwrap_or(ends.len() - 1);
                    let after = ends
                        .iter()
                        .position(|&end| crossings[end].at >= high)
                        .unwrap_or(0);
                    for &end in &ends {
                        let at = crossings[end].at;
                        if at > low
                            && at < high
                            && let Some(own) = free[end].as_mut()
                        {
                            (own.taken, own.free_prev, own.free_next) = (true, 0.0, 0.0);
                            (own.cut_prev, own.cut_next) = (true, true);
                        }
                    }
                    let left = (low - crossings[ends[before]].at).rem_euclid(ring_length);
                    if let Some(own) = free[ends[before]].as_mut() {
                        own.free_next = own.free_next.min(left);
                        own.cut_next = true;
                    }
                    let left = (crossings[ends[after]].at - high).rem_euclid(ring_length);
                    if let Some(own) = free[ends[after]].as_mut() {
                        own.free_prev = own.free_prev.min(left);
                        own.cut_prev = true;
                    }
                }
            }
        }
    }
    // Runs an anchor from an end, forwards or backwards along its ring: as
    // long as asked, but no further than the outline is free, and stopping
    // half a line short of the next end (FillBase.cpp `take_limited`).
    let hook =
        |crossings: &mut [Crossing], free: &mut [Option<Free>], end: usize, forward: bool| {
            let Some(own) = free[end] else { return };
            let (room, arc, neighbour) = if forward {
                (own.free_next, own.to_next, own.next)
            } else {
                (own.free_prev, own.to_prev, own.prev)
            };
            if own.taken || room <= 1.0 {
                return;
            }
            let length = (joining.anchor as f64)
                .min(room)
                .min(arc - joining.half_width as f64)
                .max(0.0);
            if let Some(own) = free[end].as_mut() {
                own.taken = true;
                if forward || neighbour == end {
                    own.free_next = 0.0;
                }
                if !forward || neighbour == end {
                    own.free_prev = 0.0;
                }
            }
            if length > 1.0 {
                crossings[end].hook = Some((forward, length));
                if neighbour != end
                    && let Some(other) = free[neighbour].as_mut()
                {
                    // What is left of the outline between the two.
                    if forward {
                        other.free_prev = other.free_prev.min(arc - length);
                        other.cut_prev = true;
                    } else {
                        other.free_next = other.free_next.min(arc - length);
                        other.cut_next = true;
                    }
                }
            }
        };
    let taken = |free: &[Option<Free>], end: usize| free[end].is_none_or(|end| end.taken);
    // Nearest pairs first. Each end is joined once at most, and never to a
    // path it already belongs to, which would close a loop. Distances are
    // compared to the micrometre and equal ones keep their order along the
    // ring: evenly spaced rows then pair up in sequence and form one zigzag,
    // where rounding noise would pair them haphazardly and leave gaps.
    // Two ends too far apart to join each run an anchor towards the other
    // (FillBase.cpp `connect_infill`).
    candidates.sort_by(|a, b| (a.0 / 1000.0).round().total_cmp(&(b.0 / 1000.0).round()));
    let mut group: Vec<usize> = (0..segments.len()).collect();
    fn root(group: &mut [usize], mut segment: usize) -> usize {
        while group[segment] != segment {
            group[segment] = group[group[segment]];
            segment = group[segment];
        }
        segment
    }
    // Where the paths start, in the order they come out.
    let mut starts: Option<Vec<usize>> = None;
    if joining.order == Order::Monotonic {
        candidates.clear();
        let turns = monotonic(&crossings, segments);
        // A line is joined to the one laid next when that is on the next
        // row and its end is the next along the outline.
        for pair in turns.windows(2) {
            let ((segment, entry), (_, to)) = (pair[0], pair[1]);
            let [a, b] = segments[segment];
            let far = if a == entry { b } else { a };
            let Some(own) = free[far] else { continue };
            let (here, there) = (&crossings[far], &crossings[to]);
            if there.direction != here.direction || (there.row - here.row).abs() != 1 {
                continue;
            }
            let link = [
                (own.to_next, own.next, true),
                (own.to_prev, own.prev, false),
            ]
            .into_iter()
            .filter(|&(distance, end, _)| end == to && distance <= joining.max_link as f64)
            .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, _, forward)) = link {
                crossings[far].link = Some((to, forward));
                crossings[to].link = Some((far, !forward));
            }
        }
        starts = Some(turns.into_iter().map(|(_, entry)| entry).collect());
    }
    if joining.order == Order::Sweep {
        candidates.clear();
        // From the first line not yet passed, along it, and on to the
        // line beside it whose end is next along the outline; the nearer
        // of the two where both sides offer one. A path ends where
        // neither does.
        let mut passed = vec![false; segments.len()];
        for &start in by_row {
            let mut at = start;
            while let Some(segment) = crossings[at].segment.filter(|&segment| !passed[segment]) {
                passed[segment] = true;
                let [a, b] = segments[segment];
                let far = if a == at { b } else { a };
                let Some(own) = free[far] else { break };
                let here = (crossings[far].direction, crossings[far].row);
                let next = [
                    (own.to_next, own.next, true),
                    (own.to_prev, own.prev, false),
                ]
                .into_iter()
                .filter(|&(distance, to, _)| {
                    let there = &crossings[to];
                    distance <= joining.max_link as f64
                        && there.direction == here.0
                        && (there.row - here.1).abs() == 1
                        && there.link.is_none()
                        && there.segment.is_some_and(|segment| !passed[segment])
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, to, forward)) = next else { break };
                crossings[far].link = Some((to, forward));
                crossings[to].link = Some((far, !forward));
                at = to;
            }
        }
    }
    for (distance, from, to) in candidates {
        if taken(&free, from) || taken(&free, to) {
            continue;
        }
        let (a, b) = (
            crossings[from].segment.expect("kept"),
            crossings[to].segment.expect("kept"),
        );
        let (a, b) = (root(&mut group, a), root(&mut group, b));
        if a == b {
            continue;
        }
        let whole = !free[from].is_some_and(|end| end.cut_next)
            && !free[to].is_some_and(|end| end.cut_prev);
        if !whole {
            // Part of the outline between them is spoken for: each is
            // anchored by itself further on.
            continue;
        }
        if distance <= joining.max_link as f64 {
            group[a] = b;
            crossings[from].link = Some((to, true));
            crossings[to].link = Some((from, false));
            if let Some(end) = free[from].as_mut() {
                (end.taken, end.free_next) = (true, 0.0);
            }
            if let Some(end) = free[to].as_mut() {
                (end.taken, end.free_prev) = (true, 0.0);
            }
        } else if joining.anchor > 0 {
            hook(&mut crossings, &mut free, from, true);
            hook(&mut crossings, &mut free, to, false);
        }
    }
    // Whatever end is still bare takes an anchor towards whichever side
    // has more of the outline free.
    if joining.anchor > 0 {
        for ends in segments {
            for &end in ends {
                if let Some(own) = free[end].filter(|own| !own.taken) {
                    hook(
                        &mut crossings,
                        &mut free,
                        end,
                        own.free_next >= own.free_prev,
                    );
                }
            }
        }
    }

    // Walk each chain of stretches and links from one loose end to the other.
    let mut walked = vec![false; segments.len()];
    let mut paths = Vec::new();
    for &start in starts.as_deref().unwrap_or(by_row) {
        let Some(first) = crossings[start].segment else {
            continue;
        };
        if crossings[start].link.is_some() || walked[first] {
            continue;
        }
        let end = |crossing: &Crossing| crossing.loose.unwrap_or(crossing.point);
        // An anchored end: the run along the outline, from the line's end out.
        let anchor = |crossing: &Crossing| match crossing.hook {
            Some((forward, length)) => along(
                rings[crossing.ring],
                crossing.edge,
                crossing.point,
                forward,
                length,
            ),
            None => Vec::new(),
        };
        let mut points: Vec<Point2> = anchor(&crossings[start]).into_iter().rev().collect();
        points.push(end(&crossings[start]));
        let (mut at, mut segment) = (start, first);
        loop {
            walked[segment] = true;
            let [a, b] = segments[segment];
            let far = if a == at { b } else { a };
            // Along the stretch, by whatever it passes on the way.
            if let Some(bends) = bends.get(segment) {
                if a == at {
                    points.extend(bends.iter().copied());
                } else {
                    points.extend(bends.iter().rev().copied());
                }
            }
            let Some((next, is_forward)) = crossings[far].link else {
                points.push(end(&crossings[far]));
                points.extend(anchor(&crossings[far]));
                break;
            };
            points.push(crossings[far].point);
            // Corners of the outline passed between the two ends.
            let (here, there) = (&crossings[far], &crossings[next]);
            let corners = &rings[here.ring].points;
            let count = corners.len();
            let mut steps = if is_forward {
                there.edge + count - here.edge
            } else {
                here.edge + count - there.edge
            } % count;
            // On the same edge but behind where the link is heading: the
            // link goes all the way round the ring.
            let behind = if is_forward {
                there.at < here.at
            } else {
                there.at > here.at
            };
            if steps == 0 && behind {
                steps = count;
            }
            for step in 0..steps {
                let corner = if is_forward {
                    here.edge + 1 + step
                } else {
                    here.edge + count - step
                };
                points.push(corners[corner % count]);
            }
            points.push(there.point);
            (at, segment) = (
                next,
                there.segment.expect("links only reach kept stretches"),
            );
        }
        points.dedup();
        if points.len() >= 2 {
            paths.push(points);
        }
    }
    paths
}

/// A line with `clip` cut off each end; `None` when nothing is left.
fn shorten(line: &[(f64, f64)], clip: f64) -> Option<Vec<(f64, f64)>> {
    let length = |a: (f64, f64), b: (f64, f64)| (b.0 - a.0).hypot(b.1 - a.1);
    let total: f64 = line.windows(2).map(|pair| length(pair[0], pair[1])).sum();
    if total <= 2.0 * clip {
        return None;
    }
    // The point `along` the line from its start.
    let at = |along: f64| {
        let mut left = along;
        for pair in line.windows(2) {
            let step = length(pair[0], pair[1]);
            if left <= step && step > 0.0 {
                let t = left / step;
                return (
                    pair[0].0 + t * (pair[1].0 - pair[0].0),
                    pair[0].1 + t * (pair[1].1 - pair[0].1),
                );
            }
            left -= step;
        }
        line[line.len() - 1]
    };
    let mut out = vec![at(clip)];
    let mut travelled = 0.0;
    for pair in line.windows(2) {
        travelled += length(pair[0], pair[1]);
        if travelled > clip && travelled < total - clip {
            out.push(pair[1]);
        }
    }
    out.push(at(total - clip));
    Some(out)
}

/// The order to lay solid lines in so that the area is swept from its
/// first row to its last and no line goes down between two already
/// laid, which shows on a surface: each stretch with the end it is
/// entered at (FillRectilinear.cpp `generate_montonous_regions`,
/// `connect_monotonic_regions` and `chain_monotonic_regions`).
///
/// Stretches fall into regions: a run of rows on which each stretch has
/// just the one stretch beside it on the next row, and that one just it
/// on the row before. A region is laid whole, zigzagging from either end
/// of its first row, and only once every region beside its first row on
/// the side already swept is down. Of the orders that allows, the one
/// with the least travel between regions is taken. The reference looks
/// for it with an ant colony run on random numbers, from a first guess
/// that begins at the last region with nothing before it; here it is
/// worked out exactly for up to [`EXACT_REGIONS`] regions, and for more
/// the best of the nearest-next orders from every possible start is
/// kept. So the order can differ from the reference's, never the rule.
///
/// Two stretches on neighbouring rows are beside each other when they
/// overlap along the rows. The reference follows the outline between
/// them, which differs only where a gap narrower than a row slants
/// between two parts of the area.
/// The most regions whose every order is tried: with more, the table of
/// orders grows past a few megabytes.
const EXACT_REGIONS: usize = 12;

fn monotonic(crossings: &[Crossing], segments: &[[usize; 2]]) -> Vec<(usize, usize)> {
    let line_of = |segment: usize| {
        let end = &crossings[segments[segment][0]];
        (end.direction, end.row)
    };
    let span = |segment: usize| {
        let [a, b] = segments[segment];
        (crossings[a].u, crossings[b].u)
    };
    let mut lines: BTreeMap<(usize, i64), Vec<usize>> = BTreeMap::new();
    for segment in 0..segments.len() {
        lines.entry(line_of(segment)).or_default().push(segment);
    }
    for members in lines.values_mut() {
        members.sort_by(|&a, &b| span(a).0.total_cmp(&span(b).0));
    }
    let beside = |segment: usize, line: (usize, i64)| -> Vec<usize> {
        let (low, high) = span(segment);
        lines.get(&line).map_or_else(Vec::new, |members| {
            members
                .iter()
                .copied()
                .filter(|&other| {
                    let (from, to) = span(other);
                    from < high && to > low
                })
                .collect()
        })
    };

    const NONE: usize = usize::MAX;
    let mut region_of = vec![NONE; segments.len()];
    let mut regions: Vec<Vec<usize>> = Vec::new();
    for (&line, members) in &lines {
        for &seed in members {
            if region_of[seed] != NONE {
                continue;
            }
            region_of[seed] = regions.len();
            let (mut run, mut at, mut line) = (vec![seed], seed, line);
            loop {
                let next = (line.0, line.1 + 1);
                let [only] = beside(at, next)[..] else { break };
                if beside(only, line) != [at] || region_of[only] != NONE {
                    break;
                }
                region_of[only] = regions.len();
                run.push(only);
                (at, line) = (only, next);
            }
            regions.push(run);
        }
    }

    // What each region waits for, and what it leads into.
    let mut before: Vec<Vec<usize>> = vec![Vec::new(); regions.len()];
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); regions.len()];
    for (region, run) in regions.iter().enumerate() {
        let line = line_of(run[0]);
        for segment in beside(run[0], (line.0, line.1 - 1)) {
            let other = region_of[segment];
            if other != region && !before[region].contains(&other) {
                before[region].push(other);
                after[other].push(region);
            }
        }
    }

    // A region entered at the low end of its first row leaves by the
    // high end of its last when it has an odd number of rows.
    let entry = |region: usize, flipped: bool| segments[regions[region][0]][usize::from(flipped)];
    let exit = |region: usize, flipped: bool| {
        let run = &regions[region];
        let high = (run.len() % 2 == 1) != flipped;
        segments[run[run.len() - 1]][usize::from(high)]
    };
    let apart = |a: usize, b: usize| {
        let (a, b) = (crossings[a].point, crossings[b].point);
        ((a.x - b.x) as f64).hypot((a.y - b.y) as f64)
    };
    // How far the nozzle moves from the end of one region, entered at
    // one end or the other, to the start of the next.
    let count = regions.len();
    let state = |region: usize, flipped: bool| 2 * region + usize::from(flipped);
    let mut hops = vec![0.0f64; 4 * count * count];
    for from in 0..2 * count {
        for to in 0..2 * count {
            hops[from * 2 * count + to] =
                apart(exit(from / 2, from % 2 == 1), entry(to / 2, to % 2 == 1));
        }
    }
    let hop = |from: usize, to: usize| hops[from * 2 * count + to];

    let mut order: Vec<(usize, bool)> = Vec::with_capacity(count);
    if count <= EXACT_REGIONS {
        // Every order the regions may be laid in, by which are down and
        // which was last: the least travel to have got there, and from where.
        let needs: Vec<usize> = before
            .iter()
            .map(|before| before.iter().fold(0, |mask, &region| mask | 1 << region))
            .collect();
        let sets = 1usize << count;
        let at = |set: usize, last: usize| set * 2 * count + last;
        let mut least = vec![f64::INFINITY; sets * 2 * count];
        let mut came = vec![usize::MAX; sets * 2 * count];
        for region in (0..count).filter(|&region| needs[region] == 0) {
            for flipped in [false, true] {
                least[at(1 << region, state(region, flipped))] = 0.0;
            }
        }
        for set in 1..sets {
            for last in 0..2 * count {
                let so_far = least[at(set, last)];
                if so_far.is_infinite() {
                    continue;
                }
                for (next, need) in needs.iter().enumerate() {
                    if set & 1 << next != 0 || need & !set != 0 {
                        continue;
                    }
                    for flipped in [false, true] {
                        let to = state(next, flipped);
                        let total = so_far + hop(last, to);
                        let slot = at(set | 1 << next, to);
                        if total < least[slot] {
                            (least[slot], came[slot]) = (total, last);
                        }
                    }
                }
            }
        }
        let full = sets - 1;
        let mut last =
            (0..2 * count).min_by(|&a, &b| least[at(full, a)].total_cmp(&least[at(full, b)]));
        let mut set = full;
        while let Some(here) = last {
            order.push((here / 2, here % 2 == 1));
            let from = came[at(set, here)];
            set &= !(1 << (here / 2));
            last = (from != usize::MAX).then_some(from);
        }
        order.reverse();
    } else {
        // Too many to try every order: from each region that may go
        // first, entered at either end, always on to the nearest start
        // that is free to go, and the shortest of those is kept.
        let mut shortest = f64::INFINITY;
        for first in (0..2 * count).filter(|&first| before[first / 2].is_empty()) {
            let mut waiting: Vec<usize> = before.iter().map(Vec::len).collect();
            let mut laid = vec![false; count];
            let (mut tried, mut total, mut last) = (vec![first], 0.0, first);
            laid[first / 2] = true;
            loop {
                for &next in &after[last / 2] {
                    waiting[next] -= 1;
                }
                let next = (0..2 * count)
                    .filter(|&next| !laid[next / 2] && waiting[next / 2] == 0)
                    .min_by(|&a, &b| hop(last, a).total_cmp(&hop(last, b)));
                let Some(next) = next else { break };
                total += hop(last, next);
                laid[next / 2] = true;
                tried.push(next);
                last = next;
            }
            if total < shortest {
                shortest = total;
                order = tried.iter().map(|&at| (at / 2, at % 2 == 1)).collect();
            }
        }
    }
    // Nothing should be left; if anything is, it is still printed.
    let mut laid = vec![false; count];
    for &(region, _) in &order {
        laid[region] = true;
    }
    order.extend(
        (0..count)
            .filter(|&region| !laid[region])
            .map(|region| (region, false)),
    );

    let mut turns = Vec::with_capacity(segments.len());
    for (region, flipped) in order {
        let mut high = flipped;
        for &segment in &regions[region] {
            turns.push((segment, segments[segment][usize::from(high)]));
            high = !high;
        }
    }
    turns
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use edge_geom::scaled;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn length(paths: &[Vec<Point2>]) -> f64 {
        let segment = |pair: &[Point2]| {
            ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64)
        };
        paths
            .iter()
            .flat_map(|path| path.windows(2))
            .map(segment)
            .sum::<f64>()
            / 1e6
    }

    #[test]
    fn unjoined_rows_cross_a_square_on_the_global_grid() {
        let square = ExPolygon {
            contour: rect(0.25, 0.0, 10.25, 10.0),
            holes: Vec::new(),
        };
        let rows = paths(&square, scaled(1.0), &[0.0], 0, 0);
        // Rows at y = 0 .. 9; the top edge at y = 10 is not inside.
        assert_eq!(rows.len(), 10);
        assert_eq!(
            rows[0],
            [Point2::from_mm(0.25, 0.0), Point2::from_mm(10.25, 0.0)]
        );
        assert_eq!(length(&rows), 100.0);

        let columns = paths(&square, scaled(1.0), &[FRAC_PI_2], 0, 0);
        // Columns at x = 1 .. 10: the grid is the plane's, not the square's.
        assert_eq!(columns.len(), 10);
        assert!(
            columns
                .iter()
                .all(|path| path[0].x == path[1].x && path[0].x % scaled(1.0) == 0)
        );
    }

    #[test]
    fn rows_join_into_one_zigzag_along_the_outline() {
        let square = ExPolygon {
            contour: rect(0.0, 0.5, 10.0, 9.5),
            holes: Vec::new(),
        };
        let joined = paths(&square, scaled(1.0), &[0.0], 0, scaled(3.0));
        // Nine rows of 10 mm and eight 1 mm links up the sides.
        assert_eq!(joined.len(), 1);
        assert_eq!(length(&joined), 98.0);
        // It alternates sides: every point is on the left or right edge.
        assert!(joined[0].iter().all(|p| p.x == 0 || p.x == scaled(10.0)));
        assert_eq!(joined[0].len(), 18);
        // Links longer than allowed are not made.
        assert_eq!(paths(&square, scaled(1.0), &[0.0], 0, scaled(0.5)).len(), 9);
    }

    #[test]
    fn solid_lines_round_a_curve_stay_one_zigzag() {
        // A disc: along its outline the ends of neighbouring rows are
        // never the same distance apart, which is where joining the
        // nearest ends first used to leave the zigzag in pieces.
        let disc = |radius: f64| ExPolygon {
            contour: Polygon {
                points: (0..180)
                    .map(|step| {
                        let angle = f64::from(step) * std::f64::consts::TAU / 180.0;
                        Point2::from_mm(20.0 + radius * angle.cos(), 20.0 + radius * angle.sin())
                    })
                    .collect(),
            },
            holes: Vec::new(),
        };
        for angle in [0.0, FRAC_PI_2 / 2.0, 1.0] {
            let paths = paths_turned(
                &[disc(10.0)],
                &[disc(9.8)],
                scaled(0.45) as f64,
                0.0,
                angle,
                Coord::MAX / 4,
                false,
            );
            assert_eq!(paths.len(), 1, "at {angle} rad");
        }
        // With joins held to a length, the rows near the top and bottom,
        // whose ends lie far apart along the outline, are left apart.
        let limited = paths_turned(
            &[disc(10.0)],
            &[disc(9.8)],
            scaled(0.45) as f64,
            0.0,
            0.0,
            scaled(1.0),
            false,
        );
        assert!(limited.len() > 1);
    }

    /// An area and the same drawn in by `inset`, as `paths_turned` takes them.
    fn drawn_in(area: &ExPolygon, inset: f64) -> (Vec<ExPolygon>, Vec<ExPolygon>) {
        let inner = edge_clip::offset(
            std::slice::from_ref(area),
            -scaled(inset),
            edge_clip::Join::MITER,
        )
        .unwrap();
        (vec![area.clone()], inner)
    }

    /// Whether separate rows along x were laid so that none went down
    /// before a row just below it that it lies over: each as (y, from, to).
    fn swept(paths: &[Vec<Point2>]) -> bool {
        let rows: Vec<(Coord, Coord, Coord)> = paths
            .iter()
            .map(|path| {
                let (a, b) = (path[0], path[path.len() - 1]);
                assert!(path.len() == 2 && a.y == b.y, "one straight row each");
                (a.y, a.x.min(b.x), a.x.max(b.x))
            })
            .collect();
        rows.iter().enumerate().all(|(at, &(y, from, to))| {
            rows.iter().enumerate().all(|(other, &(below, low, high))| {
                let under = below < y && y - below <= scaled(1.0) && low < to && high > from;
                !under || other < at
            })
        })
    }

    #[test]
    fn monotonic_rows_never_go_down_between_two_already_laid() {
        // A square with a square hole: the rows below the hole, those to
        // either side of it and those above are four regions.
        let mut hole = rect(8.0, 8.0, 12.0, 12.0);
        hole.points.reverse();
        let holed = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![hole],
        };
        // Two prongs joined across the top, and the same upside down.
        let arch = ExPolygon {
            contour: Polygon {
                points: [
                    (0.0, 0.0),
                    (6.0, 0.0),
                    (6.0, 12.0),
                    (14.0, 12.0),
                    (14.0, 0.0),
                    (20.0, 0.0),
                    (20.0, 20.0),
                    (0.0, 20.0),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        };
        let mut cup = arch.clone();
        for point in &mut cup.contour.points {
            point.y = scaled(20.0) - point.y;
        }
        cup.contour.points.reverse();
        for area in [&holed, &arch, &cup] {
            let (outer, inner) = drawn_in(area, 0.2);
            let rows = paths_turned(&outer, &inner, 1e6, 0.5e6, 0.0, 0, true);
            assert!(swept(&rows));
            // The same rows as when they are laid in any order.
            let any = paths_turned(&outer, &inner, 1e6, 0.5e6, 0.0, 0, false);
            assert_eq!(rows.len(), any.len());
            assert!((length(&rows) - length(&any)).abs() < 1e-6);
            // Each row begins where the nozzle is nearer: a millimetre up
            // from the last one's end, but for the hops between regions.
            let hops = rows
                .windows(2)
                .filter(|pair| {
                    let (from, to) = (pair[0][1], pair[1][0]);
                    ((to.x - from.x) as f64).hypot((to.y - from.y) as f64) > 1.5e6
                })
                .count();
            assert!(hops <= 3, "{hops} hops");
        }

        // Joined, a region is one zigzag, and a plain square one path.
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: Vec::new(),
        };
        let (outer, inner) = drawn_in(&square, 0.5);
        let one = paths_turned(&outer, &inner, 1e6, 1e6, 0.0, Coord::MAX / 4, true);
        assert_eq!(one.len(), 1);
        let (outer, inner) = drawn_in(&holed, 0.2);
        let joined = paths_turned(&outer, &inner, 1e6, 0.5e6, 0.0, Coord::MAX / 4, true);
        assert!((1..=4).contains(&joined.len()), "{} paths", joined.len());
    }

    #[test]
    fn many_regions_are_still_swept_in_order() {
        // A comb of sixteen teeth on a spine: more regions than every
        // order is tried for.
        let mut points = vec![(0.0, 0.0), (64.0, 0.0), (64.0, 4.0)];
        for tooth in (0..16).rev() {
            let x = 4.0 * tooth as f64;
            points.extend([(x + 2.0, 4.0), (x + 2.0, 10.0), (x, 10.0), (x, 4.0)]);
        }
        points.dedup();
        let comb = ExPolygon {
            contour: Polygon {
                points: points
                    .into_iter()
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .collect(),
            },
            holes: Vec::new(),
        };
        let (outer, inner) = drawn_in(&comb, 0.2);
        let rows = paths_turned(&outer, &inner, 1e6, 0.5e6, 0.0, 0, true);
        // Four rows of spine and six on each tooth.
        assert_eq!(rows.len(), 4 + 16 * 6);
        assert!(swept(&rows));
        // One tooth is finished before the next is begun.
        let teeth: Vec<Coord> = rows[4..].iter().map(|row| row[0].x / scaled(4.0)).collect();
        assert_eq!(teeth.chunk_by(|a, b| a == b).count(), 16);
    }

    #[test]
    fn lines_of_any_shape_are_joined_where_their_ends_meet_the_outline() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: Vec::new(),
        };
        let bent = |y: f64| {
            vec![
                Point2::from_mm(0.0, y),
                Point2::from_mm(5.0, y + 0.5),
                Point2::from_mm(10.0, y),
            ]
        };
        let joining = Joining {
            max_link: scaled(3.0),
            ..Joining::default()
        };
        // Three bent lines right across: one path, bends and all, with
        // the two joins running up the sides.
        let joined = paths_joined(&square, vec![bent(2.0), bent(4.0), bent(6.0)], joining);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].len(), 9);
        assert!((length(&joined) - (3.0 * 2.0 * 5.0249378 + 4.0)).abs() < 1e-3);
        // Too far apart to join, they stay three.
        let apart = paths_joined(&square, vec![bent(1.0), bent(5.0), bent(9.0)], joining);
        assert_eq!(apart.len(), 3);
        // A line that stops short of the outline keeps that end to itself.
        let short = vec![Point2::from_mm(0.0, 3.0), Point2::from_mm(5.0, 3.0)];
        let mixed = paths_joined(&square, vec![bent(2.0), short.clone()], joining);
        assert_eq!(mixed.len(), 1);
        let ends = [mixed[0][0], mixed[0][mixed[0].len() - 1]];
        assert!(ends.contains(&Point2::from_mm(5.0, 3.0)));
        // With no joining allowed every line comes back as it went in.
        let alone = paths_joined(&square, vec![bent(2.0), short], Joining::default());
        assert_eq!(alone.len(), 2);
    }

    #[test]
    fn turned_lines_run_edge_to_edge_and_turn_inside() {
        let whole = |polygon| ExPolygon {
            contour: polygon,
            holes: Vec::new(),
        };
        let outer = [whole(rect(0.0, 0.0, 10.0, 10.0))];
        let inner = [whole(rect(0.5, 0.5, 9.5, 9.5))];
        // Rows at y = 1 .. 9.
        let joined = paths_turned(&outer, &inner, 1e6, 1e6, 0.0, scaled(3.0), false);
        assert_eq!(joined.len(), 1);
        let path = &joined[0];
        // Both loose ends reach the edge; every turn is half a millimetre in.
        let on_edge = |p: &Point2| p.x == 0 || p.x == scaled(10.0);
        assert!(on_edge(&path[0]) && on_edge(&path[path.len() - 1]));
        assert!(
            path[1..path.len() - 1]
                .iter()
                .all(|p| p.x == scaled(0.5) || p.x == scaled(9.5))
        );
        // Two rows of 9.5 mm, seven of 9 mm and eight 1 mm turns.
        assert_eq!(length(&joined), 2.0 * 9.5 + 7.0 * 9.0 + 8.0);

        // Unjoined, each of the nine rows runs the full 10 mm.
        let separate = paths_turned(&outer, &inner, 1e6, 1e6, 0.0, 0, false);
        assert_eq!(separate.len(), 9);
        assert_eq!(length(&separate), 90.0);

        // A row that does not reach the inner outline is not laid.
        let low = [whole(rect(0.5, 2.0, 9.5, 8.0))];
        let rows = paths_turned(&outer, &low, 1e6, 0.5e6, 0.0, 0, false);
        assert_eq!(rows.len(), 6);
        assert!(
            rows.iter()
                .all(|row| row[0].y >= scaled(2.0) && row[0].y < scaled(8.0))
        );
        assert!(paths_turned(&outer, &[], 1e6, 0.0, 0.0, 0, false).is_empty());
    }

    #[test]
    fn an_end_joined_to_nothing_is_anchored_along_the_outline() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: Vec::new(),
        };
        // Rows along x at y = 2.5 and 7.5: 5 mm apart up each side, with
        // joins allowed only up to 3 mm, so nothing is joined.
        let rows = [Rows::everywhere(2.5e6)];
        let joining = Joining {
            max_link: scaled(3.0),
            anchor: scaled(1.5),
            half_width: scaled(0.25),
            ..Joining::default()
        };
        let anchored = paths_anchored(&square, 5e6, &rows, &[0.0], joining);
        assert_eq!(anchored.len(), 2);
        // Each row runs the 10 mm across with 1.5 mm up or down a side at
        // each end.
        assert_eq!(length(&anchored), 2.0 * (10.0 + 2.0 * 1.5));
        for path in &anchored {
            assert_eq!(path.len(), 4);
            let (first, last) = (path[0], path[3]);
            assert!(first.x == 0 || first.x == scaled(10.0));
            assert!(last.x == 0 || last.x == scaled(10.0));
            assert_eq!((path[1].y, path[2].y), (path[1].y, path[1].y));
        }
        // Without an anchor length the rows are bare.
        let bare = paths_anchored(
            &square,
            5e6,
            &rows,
            &[0.0],
            Joining {
                anchor: 0,
                ..joining
            },
        );
        assert_eq!(length(&bare), 20.0);
        // Where they may be joined, they are, and only the two loose ends
        // are anchored.
        let joined = paths_anchored(
            &square,
            5e6,
            &rows,
            &[0.0],
            Joining {
                max_link: scaled(6.0),
                ..joining
            },
        );
        assert_eq!(joined.len(), 1);
        assert_eq!(length(&joined), 2.0 * 10.0 + 5.0 + 2.0 * 1.5);
    }

    #[test]
    fn an_anchor_stops_half_a_line_short_of_the_next_end() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: Vec::new(),
        };
        // Rows 1 mm apart that may not be joined: an anchor of 1.5 mm has
        // only the 1 mm to the next row, less half a line.
        let joining = Joining {
            max_link: scaled(0.5),
            anchor: scaled(1.5),
            half_width: scaled(0.25),
            ..Joining::default()
        };
        let rows = paths_anchored(&square, 1e6, &[Rows::everywhere(0.5e6)], &[0.0], joining);
        assert_eq!(rows.len(), 10);
        let longest = rows
            .iter()
            .flat_map(|path| path.windows(2))
            .filter(|pair| pair[0].x == pair[1].x)
            .map(|pair| (pair[1].y - pair[0].y).abs())
            .max()
            .unwrap();
        assert_eq!(longest, scaled(0.75));
        // No anchor runs into another: between two rows at most one does.
        let up_the_sides: f64 = length(&rows) - 100.0;
        assert!(up_the_sides <= 2.0 * 10.0 * 0.75 + 1e-6, "{up_the_sides}");
    }

    #[test]
    fn rows_lie_where_they_are_put_and_keep_within_their_bounds() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 10.0, 10.0),
            holes: Vec::new(),
        };
        // Through y = 5.2, 2 mm apart, but none within 1.5 mm of an edge.
        let rows = [Rows {
            first: 5.2e6,
            within: (1.5e6, 8.5e6),
        }];
        let lines = paths_anchored(&square, 2e6, &rows, &[0.0], Joining::default());
        let mut heights: Vec<i64> = lines.iter().map(|line| line[0].y).collect();
        heights.sort_unstable();
        assert_eq!(heights, [scaled(3.2), scaled(5.2), scaled(7.2)]);
    }

    #[test]
    fn the_outline_beside_a_line_is_left_alone() {
        // A long row 0.3 mm above the bottom edge of a strip, crossed by
        // three upright lines whose lower ends are on that edge.
        let strip = ExPolygon {
            contour: rect(0.0, 0.0, 30.0, 6.0),
            holes: Vec::new(),
        };
        let rows = [Rows::everywhere(0.3e6), Rows::everywhere(-5e6)];
        let angles = [0.0, FRAC_PI_2];
        let joining = Joining {
            max_link: scaled(12.0),
            anchor: scaled(2.0),
            half_width: scaled(0.2),
            ..Joining::default()
        };
        let on_the_bottom = |paths: &[Vec<Point2>]| {
            paths
                .iter()
                .flat_map(|path| path.windows(2))
                .filter(|pair| pair[0].y == 0 && pair[1].y == 0)
                .map(|pair| (pair[1].x - pair[0].x).abs())
                .sum::<i64>()
        };
        // Left to itself, the joining runs along the bottom edge, right
        // beside the row.
        let free = paths_anchored(&strip, 10e6, &rows, &angles, joining);
        assert!(on_the_bottom(&free) > 0);
        // With the rule on, the edge beside the row is spoken for:
        // nothing is joined or anchored along it.
        let kept_off = paths_anchored(
            &strip,
            10e6,
            &rows,
            &angles,
            Joining {
                beside: scaled(0.5),
                beside_clip: scaled(1.0),
                ..joining
            },
        );
        assert_eq!(on_the_bottom(&kept_off), 0);
        assert!(length(&kept_off) < length(&free));
    }

    #[test]
    fn how_much_of_a_line_lies_near_a_segment() {
        // A line along y = 1 past a segment on the x axis from 2 to 4,
        // within 2 of it between x = 2 - √3 and 4 + √3.
        let (low, high) = near((0.0, 1.0), (10.0, 1.0), (2.0, 0.0), (4.0, 0.0), 2.0).unwrap();
        assert!((low * 10.0 - (2.0 - 3f64.sqrt())).abs() < 1e-6, "{low}");
        assert!((high * 10.0 - (4.0 + 3f64.sqrt())).abs() < 1e-6, "{high}");
        // Too far off, there is none; wholly inside, it is all of it.
        assert_eq!(
            near((0.0, 3.0), (10.0, 3.0), (2.0, 0.0), (4.0, 0.0), 2.0),
            None
        );
        assert_eq!(
            near((2.5, 0.5), (3.5, 0.5), (2.0, 0.0), (4.0, 0.0), 2.0),
            Some((0.0, 1.0))
        );
    }

    #[test]
    fn links_follow_the_outline_along_slanted_sides() {
        let points = [(5.0, 0.2), (9.8, 5.0), (5.0, 9.8), (0.2, 5.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        let diamond = ExPolygon {
            contour: Polygon { points },
            holes: Vec::new(),
        };
        let joined = paths(&diamond, scaled(1.0), &[0.0], 0, scaled(3.0));
        assert_eq!(joined.len(), 1);
        // Nine rows, so eight links, each one row spacing up a 45° side.
        let unjoined = length(&paths(&diamond, scaled(1.0), &[0.0], 0, 0));
        assert!(
            (length(&joined) - unjoined - 8.0 * 2f64.sqrt()).abs() < 1e-3,
            "{}",
            length(&joined) - unjoined
        );
    }

    #[test]
    fn crossing_directions_join_to_their_nearest_neighbours() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 40.0, 40.0),
            holes: Vec::new(),
        };
        let both = [45f64.to_radians(), 135f64.to_radians()];
        let separate = paths(&square, scaled(4.0), &both, 0, 0);
        let joined = paths(&square, scaled(4.0), &both, 0, scaled(20.0));
        assert!(
            joined.len() < separate.len() / 2,
            "{} paths from {}",
            joined.len(),
            separate.len()
        );
        // Every end is joined once at most, so the links cover no more of
        // the outline than half of it, however generous the limit.
        let links = length(&joined) - length(&separate);
        assert!(
            links > 0.0 && links <= 80.0,
            "links come to {links} mm of a 160 mm outline"
        );
        // Nothing is lost: joined, the lines are as long as before plus links.
        let stretches =
            |paths: &[Vec<Point2>]| paths.iter().map(|path| path.len() / 2).sum::<usize>();
        assert!(stretches(&joined) >= separate.len());
    }

    #[test]
    fn holes_split_rows_and_short_pieces_are_dropped() {
        let mut hole = rect(4.0, 4.5, 6.0, 5.5);
        hole.points.reverse();
        let ring = ExPolygon {
            contour: rect(0.0, 0.5, 10.0, 9.5),
            holes: vec![hole],
        };
        let rows = paths(&ring, scaled(1.0), &[0.0], 0, 0);
        // Rows 1..9; row 5 is cut in two by the hole.
        assert_eq!(rows.len(), 10);
        assert_eq!(length(&rows), 9.0 * 10.0 - 2.0);
        assert_eq!(paths(&ring, scaled(1.0), &[0.0], scaled(5.0), 0).len(), 8);

        // Joined, nothing is laid across the hole. At most eight 1 mm links
        // go up the sides and one 3 mm link goes round the hole's end.
        let joined = paths(&ring, scaled(1.0), &[0.0], 0, scaled(3.0));
        let rows_length: f64 = 9.0 * 10.0 - 2.0;
        assert!(
            length(&joined) > rows_length && length(&joined) <= rows_length + 11.0,
            "{}",
            length(&joined)
        );
        let crosses_hole = |pair: &[Point2]| {
            let on_row_five = pair[0].y == scaled(5.0) && pair[1].y == scaled(5.0);
            on_row_five
                && pair[0].x.min(pair[1].x) < scaled(4.0)
                && pair[0].x.max(pair[1].x) > scaled(6.0)
        };
        assert!(
            !joined
                .iter()
                .flat_map(|path| path.windows(2))
                .any(crosses_hole)
        );
    }

    #[test]
    fn diagonal_lines_cover_the_area_at_the_spacing() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 30.0, 30.0),
            holes: Vec::new(),
        };
        let lines = paths(&square, scaled(0.5), &[45f64.to_radians()], 0, 0);
        // Total length times spacing approximates the area.
        assert!(
            (length(&lines) * 0.5 - 900.0).abs() < 1.0,
            "covered {}",
            length(&lines) * 0.5
        );
        assert!(paths(&square, 0, &[0.0], 0, 0).is_empty());
    }
}
