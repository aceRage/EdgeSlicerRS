use edge_geom::{Coord, ExPolygon, Point2};

/// Parallel segments across `area`, `spacing` apart, running at `angle`
/// radians counter-clockwise from +x. Lines sit on one grid for the whole
/// plane, so separate areas and successive layers line up. Segments shorter
/// than `min_length` are dropped. Rows come in order with alternate rows
/// reversed, so the end of one segment is near the start of the next.
pub(crate) fn segments(
    area: &ExPolygon,
    spacing: Coord,
    angle: f64,
    min_length: Coord,
) -> Vec<[Point2; 2]> {
    if spacing <= 0 {
        return Vec::new();
    }
    let (sin, cos) = angle.sin_cos();
    let spacing = spacing as f64;
    // In the rotated frame `u` runs along the lines and `v` across them.
    let rotated = |p: Point2| {
        (
            p.x as f64 * cos + p.y as f64 * sin,
            p.y as f64 * cos - p.x as f64 * sin,
        )
    };

    // Every place a line crosses the outline: (row, position along it).
    let mut crossings: Vec<(i64, f64)> = Vec::new();
    for ring in area.rings() {
        for (index, &a) in ring.points.iter().enumerate() {
            let b = ring.points[(index + 1) % ring.points.len()];
            let ((ua, va), (ub, vb)) = (rotated(a), rotated(b));
            // A row crosses this edge when exactly one end is at or below
            // it, so a row through a vertex counts the two edges meeting
            // there once between them, not twice or never.
            let first = (va.min(vb) / spacing).ceil() as i64;
            let end = (va.max(vb) / spacing).ceil() as i64;
            for row in first..end {
                let v = row as f64 * spacing;
                crossings.push((row, ua + (v - va) / (vb - va) * (ub - ua)));
            }
        }
    }
    crossings.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));

    let unrotated = |u: f64, v: f64| Point2 {
        x: (u * cos - v * sin).round() as Coord,
        y: (u * sin + v * cos).round() as Coord,
    };
    let mut segments = Vec::new();
    for row in crossings.chunk_by(|a, b| a.0 == b.0) {
        let v = row[0].0 as f64 * spacing;
        let start = segments.len();
        // Inside between the 1st and 2nd crossing, the 3rd and 4th, and so on.
        for pair in row.as_chunks::<2>().0 {
            if pair[1].1 - pair[0].1 >= min_length as f64 {
                segments.push([unrotated(pair[0].1, v), unrotated(pair[1].1, v)]);
            }
        }
        if row[0].0 % 2 != 0 {
            segments[start..].reverse();
            for segment in &mut segments[start..] {
                segment.reverse();
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use edge_geom::{Polygon, scaled};

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn length(segments: &[[Point2; 2]]) -> f64 {
        segments
            .iter()
            .map(|[a, b]| ((b.x - a.x) as f64).hypot((b.y - a.y) as f64))
            .sum::<f64>()
            / 1e6
    }

    #[test]
    fn rows_cross_a_square_on_the_global_grid() {
        let square = ExPolygon {
            contour: rect(0.25, 0.0, 10.25, 10.0),
            holes: Vec::new(),
        };
        let rows = segments(&square, scaled(1.0), 0.0, 0);
        // Rows at y = 0 .. 9; the top edge at y = 10 is not inside.
        assert_eq!(rows.len(), 10);
        assert_eq!(
            rows[0],
            [Point2::from_mm(0.25, 0.0), Point2::from_mm(10.25, 0.0)]
        );
        // Odd rows run the other way.
        assert_eq!(
            rows[1],
            [Point2::from_mm(10.25, 1.0), Point2::from_mm(0.25, 1.0)]
        );
        assert_eq!(length(&rows), 100.0);

        let columns = segments(&square, scaled(1.0), FRAC_PI_2, 0);
        // Columns at x = 1 .. 10, shifted by nothing: the grid is the plane's.
        assert_eq!(columns.len(), 10);
        assert!(
            columns
                .iter()
                .all(|[a, b]| a.x == b.x && a.x % scaled(1.0) == 0)
        );
    }

    #[test]
    fn holes_split_rows_and_short_pieces_are_dropped() {
        let mut hole = rect(4.0, 4.5, 6.0, 5.5);
        hole.points.reverse();
        let ring = ExPolygon {
            contour: rect(0.0, 0.5, 10.0, 9.5),
            holes: vec![hole],
        };
        let rows = segments(&ring, scaled(1.0), 0.0, 0);
        // Rows 1..9; row 5 is cut in two by the hole.
        assert_eq!(rows.len(), 10);
        assert_eq!(length(&rows), 9.0 * 10.0 - 2.0);
        assert_eq!(segments(&ring, scaled(1.0), 0.0, scaled(5.0)).len(), 8);
    }

    #[test]
    fn diagonal_lines_cover_the_area_at_the_spacing() {
        let square = ExPolygon {
            contour: rect(0.0, 0.0, 30.0, 30.0),
            holes: Vec::new(),
        };
        let lines = segments(&square, scaled(0.5), 45f64.to_radians(), 0);
        // Total length times spacing approximates the area.
        assert!(
            (length(&lines) * 0.5 - 900.0).abs() < 1.0,
            "covered {}",
            length(&lines) * 0.5
        );
        assert!(segments(&square, 0, 0.0, 0).is_empty());
    }
}
