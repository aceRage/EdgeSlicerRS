//! Polygon booleans and offsetting on the engine's fixed-point geometry.
//!
//! This is the only crate that knows which clipping library does the work
//! (clipper2-rs, a port of Clipper2), so replacing it touches nothing else.
//! The reference slicer uses the older Clipper 6; results agree in area and
//! shape but not vertex for vertex.

use clipper2_rs::{
    ClipType, Clipper64, ClipperOffset, EndType, FillRule, JoinType, Paths64, Point64, PolyTree64,
};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

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
}
