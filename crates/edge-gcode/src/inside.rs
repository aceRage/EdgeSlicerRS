#[cfg(test)]
use edge_geom::ExPolygon;
use edge_geom::{Point2, Polygon};

/// Whether the straight move from `a` to `b` stays within `areas`: it
/// crosses none of their outlines and its midpoint is inside one of them.
/// A move that only touches an outline counts as staying inside.
///
/// Every edge is tried: the writer asks `edge_avoid::Shelter`, which has
/// the areas sorted into cells, and this is what it is checked against.
#[cfg(test)]
pub(crate) fn stays_inside(areas: &[ExPolygon], a: Point2, b: Point2) -> bool {
    let edges = || {
        areas.iter().flat_map(ExPolygon::rings).flat_map(|ring| {
            let points = &ring.points;
            (0..points.len()).map(move |i| (points[i], points[(i + 1) % points.len()]))
        })
    };
    if edges().any(|(c, d)| crosses(a, b, c, d)) {
        return false;
    }
    // Even-odd count of outline edges to the right of the midpoint. Doubled
    // coordinates keep the midpoint exact.
    let (mx, my) = (
        i128::from(a.x) + i128::from(b.x),
        i128::from(a.y) + i128::from(b.y),
    );
    let mut inside = false;
    for (c, d) in edges() {
        let (cx, cy, dx, dy) = (
            2 * i128::from(c.x),
            2 * i128::from(c.y),
            2 * i128::from(d.x),
            2 * i128::from(d.y),
        );
        if (cy > my) != (dy > my) {
            // Which side of the edge the midpoint is on, without dividing.
            let side = (dx - cx) * (my - cy) - (mx - cx) * (dy - cy);
            if (side > 0) == (dy > cy) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Whether `point` is inside `outline`, by counting the edges to its right.
pub(crate) fn within(outline: &Polygon, point: Point2) -> bool {
    let points = &outline.points;
    let mut inside = false;
    for i in 0..points.len() {
        let (c, d) = (points[i], points[(i + 1) % points.len()]);
        if (c.y > point.y) != (d.y > point.y) {
            let side = i128::from(d.x - c.x) * i128::from(point.y - c.y)
                - i128::from(point.x - c.x) * i128::from(d.y - c.y);
            if (side > 0) == (d.y > c.y) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Whether segments `ab` and `cd` cross at a point interior to both.
#[cfg(test)]
fn crosses(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let side = |p: Point2, q: Point2, r: Point2| {
        let cross = i128::from(q.x - p.x) * i128::from(r.y - p.y)
            - i128::from(q.y - p.y) * i128::from(r.x - p.x);
        cross.signum()
    };
    side(a, b, c) * side(a, b, d) < 0 && side(c, d, a) * side(c, d, b) < 0
}

#[cfg(test)]
mod tests {
    use edge_geom::Polygon;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    #[test]
    fn moves_are_inside_only_when_they_cross_no_outline() {
        let mut hole = rect(4.0, 4.0, 6.0, 6.0);
        hole.points.reverse();
        let areas = [
            ExPolygon {
                contour: rect(0.0, 0.0, 10.0, 10.0),
                holes: vec![hole],
            },
            ExPolygon {
                contour: rect(20.0, 0.0, 30.0, 10.0),
                holes: Vec::new(),
            },
        ];
        let inside = |a: (f64, f64), b: (f64, f64)| {
            stays_inside(&areas, Point2::from_mm(a.0, a.1), Point2::from_mm(b.0, b.1))
        };
        assert!(inside((1.0, 1.0), (9.0, 1.0)));
        assert!(inside((1.0, 1.0), (1.0, 9.0)));
        // Through the hole, between the two areas, and wholly outside.
        assert!(!inside((1.0, 5.0), (9.0, 5.0)));
        assert!(!inside((9.0, 5.0), (21.0, 5.0)));
        assert!(!inside((12.0, 1.0), (18.0, 1.0)));
        // Entirely within the hole: no outline is crossed, but it is not inside.
        assert!(!inside((4.5, 5.0), (5.5, 5.0)));
        assert!(inside((21.0, 1.0), (29.0, 9.0)));
        assert!(!stays_inside(
            &[],
            Point2::from_mm(0.0, 0.0),
            Point2::from_mm(1.0, 1.0)
        ));
    }
}
