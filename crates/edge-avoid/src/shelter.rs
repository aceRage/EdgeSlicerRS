//! Areas made ready to be asked, many times over, whether a move stays
//! within them: what a way round the walls is asked stretch by stretch
//! when it may go unretracted for staying inside the part.

use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};

use crate::grid::Grid;

/// Areas a move may stay within.
pub struct Shelter {
    rings: Vec<Polygon>,
    grid: Grid,
}

impl Shelter {
    pub fn new(areas: &[ExPolygon]) -> Self {
        let rings: Vec<Polygon> = areas.iter().flat_map(ExPolygon::rings).cloned().collect();
        Shelter {
            grid: Grid::new(&rings, scaled(1.0)),
            rings,
        }
    }

    fn edge(&self, ring: usize, edge: usize) -> (Point2, Point2) {
        let points = &self.rings[ring].points;
        (points[edge], points[(edge + 1) % points.len()])
    }

    /// Whether the straight move from `a` to `b` stays within the areas:
    /// it crosses none of their outlines and its midpoint is inside one
    /// of them. A move that only touches an outline counts as staying
    /// inside.
    pub fn holds(&self, a: Point2, b: Point2) -> bool {
        let side = |p: Point2, q: Point2, r: Point2| {
            (i128::from(q.x - p.x) * i128::from(r.y - p.y)
                - i128::from(q.y - p.y) * i128::from(r.x - p.x))
            .signum()
        };
        let crossed = self.grid.any_along(a, b, |ring, edge| {
            let (c, d) = self.edge(ring, edge);
            side(a, b, c) * side(a, b, d) < 0 && side(c, d, a) * side(c, d, b) < 0
        });
        if crossed {
            return false;
        }
        // Even-odd count of outline edges to the right of the midpoint.
        // Doubled coordinates keep the midpoint exact.
        let (mx, my) = (
            i128::from(a.x) + i128::from(b.x),
            i128::from(a.y) + i128::from(b.y),
        );
        let doubled = |p: Point2| (2 * i128::from(p.x), 2 * i128::from(p.y));
        // Where an edge crosses the level of the midpoint, which says in
        // which one cell of those it lies in it is to be counted.
        let crossing = |ring: usize, edge: usize| {
            let (c, d) = self.edge(ring, edge);
            if c.y == d.y {
                return c.x;
            }
            let t = (my as f64 / 2.0 - c.y as f64) / (d.y - c.y) as f64;
            (c.x as f64 + t.clamp(0.0, 1.0) * (d.x - c.x) as f64) as Coord
        };
        let middle = Point2 {
            x: mx.div_euclid(2) as Coord,
            y: my.div_euclid(2) as Coord,
        };
        let mut inside = false;
        self.grid
            .each_rightward(middle, crossing, |ring, edge, counted| {
                let (c, d) = self.edge(ring, edge);
                let ((cx, cy), (dx, dy)) = (doubled(c), doubled(d));
                if counted && (cy > my) != (dy > my) {
                    // Which side of the edge the midpoint is on, without dividing.
                    let side = (dx - cx) * (my - cy) - (mx - cx) * (dy - cy);
                    if (side > 0) == (dy > cy) {
                        inside = !inside;
                    }
                }
            });
        inside
    }
}
