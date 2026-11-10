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
pub(crate) fn paths(
    area: &ExPolygon,
    spacing: Coord,
    angles: &[f64],
    min_length: Coord,
    max_link: Coord,
) -> Vec<Vec<Point2>> {
    if spacing <= 0 {
        return Vec::new();
    }
    let spacing = spacing as f64;
    let rings: Vec<&Polygon> = area.rings().collect();
    let mut ring_lengths = vec![0.0f64; rings.len()];
    let mut crossings: Vec<Crossing> = Vec::new();
    for (direction, angle) in angles.iter().enumerate() {
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
                let first = (va.min(vb) / spacing).ceil() as i64;
                let end = (va.max(vb) / spacing).ceil() as i64;
                for row in first..end {
                    let v = row as f64 * spacing;
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
                    });
                }
                travelled += length;
            }
            ring_lengths[ring] = travelled;
        }
    }

    // Along each row the area is inside between the 1st and 2nd crossing,
    // the 3rd and 4th, and so on.
    let mut by_row: Vec<usize> = (0..crossings.len()).collect();
    by_row.sort_by(|&a, &b| {
        let (a, b) = (&crossings[a], &crossings[b]);
        (a.direction, a.row)
            .cmp(&(b.direction, b.row))
            .then(a.u.total_cmp(&b.u))
    });
    let same_row = |&a: &usize, &b: &usize| {
        (crossings[a].direction, crossings[a].row) == (crossings[b].direction, crossings[b].row)
    };
    let segments: Vec<[usize; 2]> = by_row
        .chunk_by(same_row)
        .flat_map(|row| row.as_chunks::<2>().0)
        .filter(|pair| crossings[pair[1]].u - crossings[pair[0]].u >= min_length as f64)
        .copied()
        .collect();
    for (segment, ends) in segments.iter().enumerate() {
        for &end in ends {
            crossings[end].segment = Some(segment);
        }
    }

    // Ends that are neighbours along a ring, with the outline distance
    // between them: (distance, from, to), `to` lying forwards of `from`.
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for (ring, &ring_length) in ring_lengths.iter().enumerate() {
        let mut ends: Vec<usize> = (0..crossings.len())
            .filter(|&c| crossings[c].ring == ring && crossings[c].segment.is_some())
            .collect();
        ends.sort_by(|&a, &b| crossings[a].at.total_cmp(&crossings[b].at));
        for (index, &from) in ends.iter().enumerate() {
            let to = ends[(index + 1) % ends.len()];
            let distance = (crossings[to].at - crossings[from].at).rem_euclid(ring_length);
            if from != to && distance <= max_link as f64 {
                candidates.push((distance, from, to));
            }
        }
    }
    // Nearest pairs first. Each end is joined once at most, and never to a
    // path it already belongs to, which would close a loop. Distances are
    // compared to the micrometre and equal ones keep their order along the
    // ring: evenly spaced rows then pair up in sequence and form one zigzag,
    // where rounding noise would pair them haphazardly and leave gaps.
    candidates.sort_by(|a, b| (a.0 / 1000.0).round().total_cmp(&(b.0 / 1000.0).round()));
    let mut group: Vec<usize> = (0..segments.len()).collect();
    fn root(group: &mut [usize], mut segment: usize) -> usize {
        while group[segment] != segment {
            group[segment] = group[group[segment]];
            segment = group[segment];
        }
        segment
    }
    for (_, from, to) in candidates {
        if crossings[from].link.is_some() || crossings[to].link.is_some() {
            continue;
        }
        let (a, b) = (
            crossings[from].segment.expect("kept"),
            crossings[to].segment.expect("kept"),
        );
        let (a, b) = (root(&mut group, a), root(&mut group, b));
        if a != b {
            group[a] = b;
            crossings[from].link = Some((to, true));
            crossings[to].link = Some((from, false));
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
        let mut points = vec![crossings[start].point];
        let (mut at, mut segment) = (start, first);
        loop {
            walked[segment] = true;
            let [a, b] = segments[segment];
            let far = if a == at { b } else { a };
            points.push(crossings[far].point);
            let Some((next, is_forward)) = crossings[far].link else {
                break;
            };
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
