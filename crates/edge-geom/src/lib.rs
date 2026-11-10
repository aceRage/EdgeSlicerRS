//! Fixed-point 2D geometry shared by every pipeline stage.

/// Planar coordinate in nanometres. This is the resolution libslic3r uses, so
/// contours can be compared against the reference slicer without rescaling.
pub type Coord = i64;

pub const UNITS_PER_MM: f64 = 1_000_000.0;

pub fn scaled(mm: f64) -> Coord {
    (mm * UNITS_PER_MM).round() as Coord
}

pub fn unscaled(units: Coord) -> f64 {
    units as f64 / UNITS_PER_MM
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Point2 {
    pub x: Coord,
    pub y: Coord,
}

impl Point2 {
    pub fn from_mm(x: f64, y: f64) -> Self {
        Self {
            x: scaled(x),
            y: scaled(y),
        }
    }
}

/// A closed contour; the last point connects back to the first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Polygon {
    pub points: Vec<Point2>,
}

impl Polygon {
    /// Area in mm², positive for counter-clockwise contours (outer boundaries)
    /// and negative for clockwise ones (holes).
    pub fn signed_area_mm2(&self) -> f64 {
        let pts = &self.points;
        // A metre-sized part squared overflows i64, so accumulate in i128.
        let doubled: i128 = (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
            })
            .sum();
        doubled as f64 / (2.0 * UNITS_PER_MM * UNITS_PER_MM)
    }

    pub fn is_ccw(&self) -> bool {
        self.signed_area_mm2() > 0.0
    }

    /// Length in mm of the closed outline.
    pub fn perimeter_mm(&self) -> f64 {
        let pts = &self.points;
        (0..pts.len())
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
            })
            .sum::<f64>()
            / UNITS_PER_MM
    }
}

/// The smallest convex contour around a set of points, counter-clockwise,
/// with no three corners in a line. Fewer than three distinct points, or
/// points all in one line, give back what there is.
pub fn convex_hull(points: impl IntoIterator<Item = Point2>) -> Polygon {
    let mut points: Vec<Point2> = points.into_iter().collect();
    points.sort_unstable_by_key(|point| (point.x, point.y));
    points.dedup();
    if points.len() < 3 {
        return Polygon { points };
    }
    // Which way the turn a -> b -> c goes; products of two coordinates
    // overflow i64.
    let turn = |a: Point2, b: Point2, c: Point2| {
        i128::from(b.x - a.x) * i128::from(c.y - a.y)
            - i128::from(b.y - a.y) * i128::from(c.x - a.x)
    };
    // Andrew's monotone chain: the lower side left to right, then the
    // upper side back again.
    let mut hull: Vec<Point2> = Vec::with_capacity(points.len() + 1);
    for pass in 0..2 {
        // Points of this side that a new one may still push off the hull.
        let floor = if pass == 0 { 2 } else { hull.len() + 1 };
        let side: Box<dyn Iterator<Item = &Point2>> = if pass == 0 {
            Box::new(points.iter())
        } else {
            Box::new(points.iter().rev().skip(1))
        };
        for &point in side {
            while hull.len() >= floor
                && turn(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0
            {
                hull.pop();
            }
            hull.push(point);
        }
    }
    hull.pop();
    Polygon { points: hull }
}

/// A region of the plane: one outer boundary and the holes cut out of it.
/// The contour runs counter-clockwise and the holes clockwise. Named as in
/// libslic3r, where the same shape is the unit every later stage works on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExPolygon {
    pub contour: Polygon,
    pub holes: Vec<Polygon>,
}

impl ExPolygon {
    /// Area in mm² of the material: the contour less its holes.
    pub fn area_mm2(&self) -> f64 {
        self.rings().map(Polygon::signed_area_mm2).sum()
    }

    /// The contour, then each hole.
    pub fn rings(&self) -> impl Iterator<Item = &Polygon> {
        std::iter::once(&self.contour).chain(&self.holes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(side: f64) -> Polygon {
        let corners = [(0.0, 0.0), (side, 0.0), (side, side), (0.0, side)];
        Polygon {
            points: corners
                .iter()
                .map(|&(x, y)| Point2::from_mm(x, y))
                .collect(),
        }
    }

    #[test]
    fn area_sign_follows_winding() {
        let mut sq = square(10.0);
        assert_eq!(sq.signed_area_mm2(), 100.0);
        assert!(sq.is_ccw());
        sq.points.reverse();
        assert_eq!(sq.signed_area_mm2(), -100.0);
    }

    #[test]
    fn perimeter_closes_the_outline() {
        assert_eq!(square(10.0).perimeter_mm(), 40.0);
        assert_eq!(Polygon::default().perimeter_mm(), 0.0);
    }

    #[test]
    fn area_survives_metre_scale_parts() {
        assert_eq!(square(2000.0).signed_area_mm2(), 4_000_000.0);
    }

    #[test]
    fn expolygon_area_subtracts_holes() {
        let mut hole = square(4.0);
        hole.points.reverse();
        let region = ExPolygon {
            contour: square(10.0),
            holes: vec![hole],
        };
        assert_eq!(region.area_mm2(), 84.0);
        assert_eq!(region.rings().count(), 2);
    }

    #[test]
    fn scaling_round_trips() {
        assert_eq!(scaled(0.2), 200_000);
        assert_eq!(unscaled(scaled(123.456789)), 123.456789);
    }

    #[test]
    fn convex_hull_wraps_the_outermost_points() {
        let mm = |points: &[(f64, f64)]| -> Vec<Point2> {
            points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect()
        };
        // An L with a point inside and one on an edge: neither is a corner.
        let hull = convex_hull(mm(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (5.0, 5.0),
            (5.0, 10.0),
            (0.0, 10.0),
            (2.0, 2.0),
            (5.0, 0.0),
        ]));
        assert_eq!(
            hull.points,
            mm(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0, 5.0),
                (5.0, 10.0),
                (0.0, 10.0)
            ])
        );
        assert!(hull.is_ccw());
        // Too few points to enclose anything come back as they are.
        assert_eq!(convex_hull(mm(&[(1.0, 1.0), (1.0, 1.0)])).points.len(), 1);
        assert_eq!(
            convex_hull(mm(&[(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)])).points,
            mm(&[(0.0, 0.0), (2.0, 2.0)])
        );
    }
}
