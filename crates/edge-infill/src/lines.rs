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
/// of zero joins nothing, and every line then runs edge to edge.
pub fn paths_turned(
    outer: &[ExPolygon],
    inner: &[ExPolygon],
    spacing: f64,
    first: f64,
    angle: f64,
    max_link: Coord,
) -> Vec<Vec<Point2>> {
    let rings: Vec<&Polygon> = inner.iter().flat_map(ExPolygon::rings).collect();
    let edge: Vec<&Polygon> = outer.iter().flat_map(ExPolygon::rings).collect();
    let joining = Joining {
        max_link,
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
            let (radius, clip) = (joining.beside as f64, joining.beside_clip as f64);
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
                for stretch in &segments {
                    let (c, d) = (
                        as_pair(crossings[stretch[0]].point),
                        as_pair(crossings[stretch[1]].point),
                    );
                    let run = (d.0 - c.0).hypot(d.1 - c.1);
                    if run <= 2.0 * clip {
                        continue;
                    }
                    // The line less its two ends.
                    let (ux, uy) = ((d.0 - c.0) / run, (d.1 - c.1) / run);
                    let (c, d) = (
                        (c.0 + ux * clip, c.1 + uy * clip),
                        (d.0 - ux * clip, d.1 - uy * clip),
                    );
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
        for ends in &segments {
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
    for &start in &by_row {
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
    fn turned_lines_run_edge_to_edge_and_turn_inside() {
        let whole = |polygon| ExPolygon {
            contour: polygon,
            holes: Vec::new(),
        };
        let outer = [whole(rect(0.0, 0.0, 10.0, 10.0))];
        let inner = [whole(rect(0.5, 0.5, 9.5, 9.5))];
        // Rows at y = 1 .. 9.
        let joined = paths_turned(&outer, &inner, 1e6, 1e6, 0.0, scaled(3.0));
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
        let separate = paths_turned(&outer, &inner, 1e6, 1e6, 0.0, 0);
        assert_eq!(separate.len(), 9);
        assert_eq!(length(&separate), 90.0);

        // A row that does not reach the inner outline is not laid.
        let low = [whole(rect(0.5, 2.0, 9.5, 8.0))];
        let rows = paths_turned(&outer, &low, 1e6, 0.5e6, 0.0, 0);
        assert_eq!(rows.len(), 6);
        assert!(
            rows.iter()
                .all(|row| row[0].y >= scaled(2.0) && row[0].y < scaled(8.0))
        );
        assert!(paths_turned(&outer, &[], 1e6, 0.0, 0.0, 0).is_empty());
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
