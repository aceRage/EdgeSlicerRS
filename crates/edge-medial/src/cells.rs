//! What part of the plane lies nearest each of a set of edges: the cells
//! of their Voronoi diagram, as polygons.

use boostvoronoi::prelude::{Builder, Diagram, EdgeIndex, SourceCategory};
use edge_geom::{Point2, Polygon};

use crate::MedialError;

/// How far apart the points along a curved side of a cell are put, in
/// the units of the edges.
const ARC_STEP: f64 = 50_000.0;
/// The most pieces a curved side is cut into.
const ARC_PIECES: usize = 24;

/// For each of `edges` (each given as the start and end of a segment),
/// the points nearer to it than to any other, as polygons: one for the
/// edge itself and one for each of its ends that has a cell of its own,
/// each with the edge's index. The edges must not cross; they may share
/// ends, as the edges of a region's outline do. Sides that are arcs of
/// parabolas are cut into short straight pieces.
///
/// The cells of an outline's edges reach outwards without end, so a
/// frame is put round everything first, a little way off. No point
/// inside the outline is any nearer to the frame than to the outline, so
/// within the outline the cells are what they would be without it;
/// beyond the outline they stop short of the frame, and are of no use.
pub fn cells(edges: &[[i64; 4]]) -> Result<Vec<(usize, Polygon)>, MedialError> {
    if edges.is_empty() {
        return Ok(Vec::new());
    }
    let xs = || edges.iter().flat_map(|edge| [edge[0], edge[2]]);
    let ys = || edges.iter().flat_map(|edge| [edge[1], edge[3]]);
    let (min_x, max_x) = (xs().min().unwrap_or(0), xs().max().unwrap_or(0));
    let (min_y, max_y) = (ys().min().unwrap_or(0), ys().max().unwrap_or(0));
    let (width, height) = (max_x - min_x, max_y - min_y);
    let margin = (width.max(height) / 8).max(1_000_000);
    if width + 2 * margin > i64::from(i32::MAX / 2) || height + 2 * margin > i64::from(i32::MAX / 2)
    {
        return Err(MedialError::TooLarge);
    }
    let origin = (min_x - margin, min_y - margin);
    let mut all: Vec<[i64; 4]> = edges
        .iter()
        .map(|edge| {
            [
                edge[0] - origin.0,
                edge[1] - origin.1,
                edge[2] - origin.0,
                edge[3] - origin.1,
            ]
        })
        .collect();
    let (right, top) = (width + 2 * margin, height + 2 * margin);
    all.extend([
        [0, 0, right, 0],
        [right, 0, right, top],
        [right, top, 0, top],
        [0, top, 0, 0],
    ]);
    let diagram = Builder::<i64>::default()
        .with_segments(all.iter())?
        .build()?;

    let mut found = Vec::new();
    for cell in diagram.cells() {
        let source = cell.source_index().usize();
        if source >= edges.len() || cell.is_degenerate() {
            continue;
        }
        let Some(first) = cell.get_incident_edge() else {
            continue;
        };
        let mut points: Vec<[f64; 2]> = Vec::new();
        let mut edge = first;
        let mut whole = true;
        loop {
            match side(&diagram, &all, edge)? {
                Some(run) => points.extend(run),
                None => {
                    whole = false;
                    break;
                }
            }
            edge = diagram.edge_get_next(edge)?;
            if edge == first {
                break;
            }
        }
        if !whole {
            continue;
        }
        let mut polygon = Polygon {
            points: points
                .into_iter()
                .map(|[x, y]| Point2 {
                    x: x.round() as i64 + origin.0,
                    y: y.round() as i64 + origin.1,
                })
                .collect(),
        };
        polygon.points.dedup();
        if polygon.points.len() > 1 && polygon.points.first() == polygon.points.last() {
            polygon.points.pop();
        }
        if polygon.points.len() < 3 {
            continue;
        }
        if !polygon.is_ccw() {
            polygon.points.reverse();
        }
        found.push((source, polygon));
    }
    Ok(found)
}

/// Where a side of a cell starts and ends.
type Ends = ([f64; 2], [f64; 2]);

/// The points along one side of a cell, from its start up to but not
/// including its end; `None` when the side runs off without end.
fn side(
    diagram: &Diagram,
    sites: &[[i64; 4]],
    edge: EdgeIndex,
) -> Result<Option<Vec<[f64; 2]>>, MedialError> {
    let at = |edge: EdgeIndex| -> Result<Option<Ends>, MedialError> {
        let (Some(from), Some(to)) = (
            diagram.edge_get_vertex0(edge)?,
            diagram.edge_get_vertex1(edge)?,
        ) else {
            return Ok(None);
        };
        let (from, to) = (diagram.vertex(from)?, diagram.vertex(to)?);
        Ok(Some(([from.x(), from.y()], [to.x(), to.y()])))
    };
    let Some((from, _)) = at(edge)? else {
        return Ok(None);
    };
    if !diagram.edge(edge)?.is_curved() {
        return Ok(Some(vec![from]));
    }
    // An arc is worked out one way round for both cells it lies between,
    // so that they meet along exactly the same points.
    let twin = diagram.edge_get_twin(edge)?;
    let (forward, reversed) = if edge.usize() < twin.usize() {
        (edge, false)
    } else {
        (twin, true)
    };
    let Some((start, end)) = at(forward)? else {
        return Ok(None);
    };
    // Between a point and a segment: the point is the focus.
    let one = diagram.cell(diagram.edge_get_cell(forward)?)?;
    let other = diagram.cell(diagram.edge_get_cell(diagram.edge_get_twin(forward)?)?)?;
    let (point, segment) = if one.contains_point() {
        (one, other)
    } else {
        (other, one)
    };
    let end_of = |cell: &boostvoronoi::prelude::Cell| {
        let [ax, ay, bx, by] = sites[cell.source_index().usize()];
        match cell.source_category() {
            SourceCategory::SegmentEnd => [bx as f64, by as f64],
            _ => [ax as f64, ay as f64],
        }
    };
    let focus = end_of(point);
    let [ax, ay, bx, by] = sites[segment.source_index().usize()].map(|c| c as f64);
    let length = (bx - ax).hypot(by - ay);
    let mut run = vec![start];
    if length > 0.0 && point.contains_point() && segment.contains_segment() {
        let (dx, dy) = ((bx - ax) / length, (by - ay) / length);
        // Along the segment's line and square to it.
        let along = |p: [f64; 2]| (p[0] - ax) * dx + (p[1] - ay) * dy;
        let (focus_along, focus_off) = (along(focus), (focus[1] - ay) * dx - (focus[0] - ax) * dy);
        if focus_off.abs() > 1.0 {
            let (from_along, to_along) = (along(start), along(end));
            let pieces =
                (((to_along - from_along).abs() / ARC_STEP).ceil() as usize).clamp(1, ARC_PIECES);
            for piece in 1..pieces {
                let u = from_along + (to_along - from_along) * piece as f64 / pieces as f64;
                let off = ((u - focus_along).powi(2) + focus_off * focus_off) / (2.0 * focus_off);
                run.push([ax + u * dx - off * dy, ay + u * dy + off * dx]);
            }
        }
    }
    if reversed {
        // From this side's own start: the other's end, then back along it.
        let mut back = vec![end];
        back.extend(run.into_iter().skip(1).rev());
        run = back;
    }
    Ok(Some(run))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(polygon: &Polygon) -> f64 {
        polygon.signed_area_mm2()
    }

    fn ring(points: &[(f64, f64)]) -> Vec<[i64; 4]> {
        (0..points.len())
            .map(|at| {
                let (a, b) = (points[at], points[(at + 1) % points.len()]);
                [a.0, a.1, b.0, b.1].map(|c| (c * 1e6) as i64)
            })
            .collect()
    }

    #[test]
    fn a_squares_sides_share_it_out_in_four_triangles() {
        let square = ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let found = cells(&square).unwrap();
        // Each side's cell holds the triangle from it to the middle, and
        // nothing of the square beyond that.
        for (side, inside) in [
            (0, (5.0, 2.0)),
            (1, (8.0, 5.0)),
            (2, (5.0, 8.0)),
            (3, (2.0, 5.0)),
        ] {
            let holds = |polygon: &Polygon, (x, y): (f64, f64)| {
                let (x, y) = ((x * 1e6) as i64, (y * 1e6) as i64);
                let points = &polygon.points;
                let mut within = false;
                for at in 0..points.len() {
                    let (c, d) = (points[at], points[(at + 1) % points.len()]);
                    if (c.y > y) != (d.y > y)
                        && ((d.x - c.x) as f64 * (y - c.y) as f64 / (d.y - c.y) as f64 + c.x as f64
                            > x as f64)
                    {
                        within = !within;
                    }
                }
                within
            };
            let own: Vec<&Polygon> = found
                .iter()
                .filter(|(edge, _)| *edge == side)
                .map(|(_, polygon)| polygon)
                .collect();
            assert!(
                own.iter().any(|polygon| holds(polygon, inside)),
                "side {side}"
            );
            let across = (10.0 - inside.0, 10.0 - inside.1);
            assert!(
                !own.iter().any(|polygon| holds(polygon, across)),
                "side {side}"
            );
        }
        assert!(found.iter().all(|(_, polygon)| area(polygon) > 0.0));
    }

    #[test]
    fn cells_round_an_inside_corner_meet_along_the_same_arc() {
        // An L: its inside corner has a cell of its own, with curved sides.
        let l = ring(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 10.0),
            (10.0, 10.0),
            (10.0, 20.0),
            (0.0, 20.0),
        ]);
        let found = cells(&l).unwrap();
        assert!(found.len() > l.len(), "{} cells", found.len());
        // Curved sides are cut into more than their two ends.
        assert!(found.iter().any(|(_, polygon)| polygon.points.len() > 6));
        // Every point of a curved side is on both cells beside it:
        // every point inside the L is on two cells at least. (Outside it
        // the cells beside the frame's are not given.)
        let inside = |point: &Point2| {
            let (x, y) = (point.x, point.y);
            let within = |c: i64, high: i64| c > 0 && c < high;
            within(x, 20_000_000) && within(y, 20_000_000) && !(x >= 10_000_000 && y >= 10_000_000)
        };
        let mut points: Vec<Point2> = found
            .iter()
            .flat_map(|(_, polygon)| polygon.points.iter().copied())
            .filter(inside)
            .collect();
        points.sort_by_key(|point| (point.x, point.y));
        assert!(points.len() > 20);
        let alone = points
            .chunk_by(|a, b| a == b)
            .filter(|same| same.len() == 1)
            .count();
        assert_eq!(alone, 0, "points on one cell only");
    }

    #[test]
    fn nothing_gives_nothing() {
        assert!(cells(&[]).unwrap().is_empty());
    }
}
