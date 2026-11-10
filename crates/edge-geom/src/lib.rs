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
    fn area_survives_metre_scale_parts() {
        assert_eq!(square(2000.0).signed_area_mm2(), 4_000_000.0);
    }

    #[test]
    fn scaling_round_trips() {
        assert_eq!(scaled(0.2), 200_000);
        assert_eq!(unscaled(scaled(123.456789)), 123.456789);
    }
}
