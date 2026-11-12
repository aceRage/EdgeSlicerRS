//! Points along a region's outline by how far round it they are, 0 at the
//! first corner and 1 back there (libnest2d `nfpplacer.hpp` `EdgeCache`):
//! what the optimiser slides an item along.
//!
//! The reference walks a ring from its first corner to its last and never
//! back to the first (its rings are open, and `createCache` stops at the
//! last corner), so the side from the last corner to the first is never
//! searched and the last corner is no starting point. A free region's
//! every side is a place an item may stand; here every side is walked.

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

struct Ring {
    /// Each side's first corner and its angle to the x axis.
    sides: Vec<(Point2, f64)>,
    /// How far round each side ends.
    distances: Vec<f64>,
    full: f64,
}

impl Ring {
    fn of(ring: &Polygon) -> Self {
        let pts = &ring.points;
        let n = pts.len();
        let mut sides = Vec::with_capacity(n);
        let mut distances = Vec::with_capacity(n);
        let mut full = 0.0;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            // `_Segment::angleToXaxis`: 0 to 2 pi.
            let mut angle = (dy as f64).atan2(dx as f64);
            if angle.is_sign_negative() {
                angle += 2.0 * std::f64::consts::PI;
            }
            full += ((dx as f64) * (dx as f64) + (dy as f64) * (dy as f64)).sqrt();
            sides.push((a, angle));
            distances.push(full);
        }
        Self {
            sides,
            distances,
            full,
        }
    }

    /// Where on the ring `at` (0..=1) of the way round is, rounded to the
    /// nanometre as the reference rounds it.
    fn point(&self, at: f64) -> Point2 {
        if self.sides.is_empty() {
            return Point2::default();
        }
        let at = if at > 1.0 { at % 1.0 } else { at };
        let d = at * self.full;
        let idx = self
            .distances
            .partition_point(|&end| end < d)
            .min(self.sides.len() - 1);
        let before = if idx > 0 {
            self.distances[idx - 1]
        } else {
            0.0
        };
        let along = d - before;
        let (first, angle) = self.sides[idx];
        Point2 {
            x: first.x + (along * angle.cos()).round() as Coord,
            y: first.y + (along * angle.sin()).round() as Coord,
        }
    }

    /// Where the search starts: each corner (`fetchCorners` with the
    /// accuracy at 1, every corner).
    fn corners(&self) -> Vec<f64> {
        let mut corners = Vec::with_capacity(self.sides.len());
        corners.push(0.0);
        if self.full > 0.0 {
            for end in &self.distances[..self.distances.len().saturating_sub(1)] {
                corners.push(end / self.full);
            }
        }
        corners
    }
}

/// A free region's rings: its contour, then its holes.
pub struct EdgeCache {
    rings: Vec<Ring>,
}

impl EdgeCache {
    pub fn of(region: &ExPolygon) -> Self {
        Self {
            rings: region.rings().map(Ring::of).collect(),
        }
    }

    pub fn ring_count(&self) -> usize {
        self.rings.len()
    }

    pub fn point(&self, ring: usize, at: f64) -> Point2 {
        self.rings[ring].point(at)
    }

    pub fn corners(&self, ring: usize) -> Vec<f64> {
        self.rings[ring].corners()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_is_walked_round_every_side() {
        let square = ExPolygon {
            contour: Polygon {
                points: vec![
                    Point2 { x: 0, y: 0 },
                    Point2 { x: 10, y: 0 },
                    Point2 { x: 10, y: 10 },
                    Point2 { x: 0, y: 10 },
                ],
            },
            holes: Vec::new(),
        };
        let cache = EdgeCache::of(&square);
        assert_eq!(cache.point(0, 0.0), Point2 { x: 0, y: 0 });
        assert_eq!(cache.point(0, 0.125), Point2 { x: 5, y: 0 });
        assert_eq!(cache.point(0, 0.5), Point2 { x: 10, y: 10 });
        // The closing side, which the reference leaves out.
        assert_eq!(cache.point(0, 0.875), Point2 { x: 0, y: 5 });
        assert_eq!(cache.corners(0), vec![0.0, 0.25, 0.5, 0.75]);
    }
}
