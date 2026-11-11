//! A contoured line's heights, by where its points are
//! (ContourZ.hpp `ContourZSamples`).

use std::collections::HashMap;

use edge_geom::{Point2, unscaled};

/// The points a line was contoured at, each with how far above its
/// layer's top it is laid (negative below), in mm.
///
/// As in the reference, the heights are kept apart from the line and
/// looked up by where a point is, not by its place in the line: between
/// contouring and writing, a loop is opened at its seam, turned about,
/// and cut short by the seam gap, which leaves its points where they
/// were and adds at most one between two of them, and that one is found
/// along the stretch it lies on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    points: Vec<Point2>,
    heights: Vec<f32>,
    index: HashMap<Point2, f32>,
}

impl Profile {
    /// From the points and their heights, in the line's order. Where a
    /// line passes the same place twice (a loop's two ends), the first
    /// height is kept (`ContourZSamples::build_index`).
    pub fn new(points: Vec<Point2>, heights: Vec<f32>) -> Self {
        debug_assert_eq!(points.len(), heights.len());
        let mut index = HashMap::with_capacity(points.len());
        for (point, height) in points.iter().zip(&heights) {
            index.entry(*point).or_insert(*height);
        }
        Self {
            points,
            heights,
            index,
        }
    }

    pub fn points(&self) -> &[Point2] {
        &self.points
    }

    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// How far above the layer's top the line is at `point`, in mm: the
    /// height kept for it, or for a point that is none of the line's
    /// own, the height along the nearest stretch between two of them
    /// (`ContourZSamples::z_at`).
    pub fn at(&self, point: Point2) -> f64 {
        if let Some(height) = self.index.get(&point) {
            return f64::from(*height);
        }
        let Some(first) = self.heights.first() else {
            return 0.0;
        };
        let (qx, qy) = (unscaled(point.x), unscaled(point.y));
        let mut best = (f64::MAX, f64::from(*first));
        for (pair, heights) in self.points.windows(2).zip(self.heights.windows(2)) {
            let (ax, ay) = (unscaled(pair[0].x), unscaled(pair[0].y));
            let (bx, by) = (unscaled(pair[1].x), unscaled(pair[1].y));
            let (abx, aby) = (bx - ax, by - ay);
            let length = abx * abx + aby * aby;
            let t = if length > 0.0 {
                (((qx - ax) * abx + (qy - ay) * aby) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (fx, fy) = (ax + abx * t, ay + aby * t);
            let away = (qx - fx) * (qx - fx) + (qy - fy) * (qy - fy);
            if away < best.0 {
                let (a, b) = (f64::from(heights[0]), f64::from(heights[1]));
                best = (away, a + t * (b - a));
            }
        }
        best.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_are_found_by_place_and_between_places() {
        let points = vec![
            Point2::from_mm(0.0, 0.0),
            Point2::from_mm(1.0, 0.0),
            Point2::from_mm(1.0, 1.0),
            Point2::from_mm(0.0, 0.0),
        ];
        let profile = Profile::new(points, vec![-0.1, -0.05, 0.0, -0.08]);
        // The loop's two ends are one place: the first height stands.
        assert!((profile.at(Point2::from_mm(0.0, 0.0)) + 0.1).abs() < 1e-6);
        assert!((profile.at(Point2::from_mm(1.0, 1.0))).abs() < 1e-9);
        // A point a seam split put in, part way along a stretch.
        assert!((profile.at(Point2::from_mm(0.25, 0.0)) + 0.0875).abs() < 1e-6);
        assert!((profile.at(Point2::from_mm(1.0, 0.5)) + 0.025).abs() < 1e-6);
        assert_eq!(Profile::default().at(Point2::from_mm(1.0, 1.0)), 0.0);
    }
}
