//! What the hole options ask of one ring of a layer's outline: where its
//! middle is, how far a point is from it, and whether two rings are one.
//!
//! A ring is judged round by the points it has. The reference's slices
//! have been simplified at 0.0025 mm as they were cut (PrintObjectSlice.cpp,
//! "0.0025mm is safe enough to simplify the data"), which this engine's
//! are not, so a ring drawn finer than that is judged here by more points
//! than there; what the compensations do to a ring afterwards is judged
//! as it is in both.

use edge_geom::{Coord, Point2, Polygon};

/// The reference's `scale_`: millimetres to coordinate units, unrounded.
pub(crate) const fn scale(mm: f64) -> f64 {
    mm / SCALING_FACTOR
}

/// libslic3r.h `SCALING_FACTOR`: millimetres in one coordinate unit.
pub(crate) const SCALING_FACTOR: f64 = 0.000_001;

/// The middle of the area a ring encloses, to the nearest unit
/// (Polygon.cpp `Polygon::centroid`). Worked out from the ring's first
/// point, which the reference does not do: its coordinates are an
/// object's own and small, where these are the bed's.
pub fn centroid(points: &[Point2]) -> Point2 {
    let Some(&origin) = points.first() else {
        return Point2::default();
    };
    if points.len() < 3 {
        return origin;
    }
    let local = |p: Point2| ((p.x - origin.x) as f64, (p.y - origin.y) as f64);
    let (mut area_sum, mut cx, mut cy) = (0.0, 0.0, 0.0);
    let mut p1 = local(points[points.len() - 1]);
    for &point in points {
        let p2 = local(point);
        let a = p1.0 * p2.1 - p1.1 * p2.0;
        area_sum += a;
        cx += (p1.0 + p2.0) * a;
        cy += (p1.1 + p2.1) * a;
        p1 = p2;
    }
    if area_sum == 0.0 {
        return origin;
    }
    Point2 {
        x: origin.x + (cx / (3.0 * area_sum)).round() as Coord,
        y: origin.y + (cy / (3.0 * area_sum)).round() as Coord,
    }
}

/// `Point::distance_to`.
pub(crate) fn distance(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
    (dx * dx + dy * dy).sqrt()
}

/// Whether two rings are one ring, whichever point each starts at.
pub fn same_ring(a: &[Point2], b: &[Point2]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let Some(first) = a.first() else {
        return true;
    };
    (0..b.len())
        .filter(|&start| b[start] == *first)
        .any(|start| (0..a.len()).all(|i| a[i] == b[(start + i) % b.len()]))
}

/// A ring put in place of another on a layer: a hole made a polyhole, or
/// a round outline or hole made larger or smaller.
#[derive(Clone, Debug, PartialEq)]
pub struct Replaced {
    pub layer: usize,
    pub from: Polygon,
    pub to: Polygon,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_of_a_ring_is_the_middle_of_its_area() {
        // An L: a 20 x 10 bar with a 10 x 10 block on its left end.
        let l = [(0, 0), (20, 0), (20, 10), (10, 10), (10, 20), (0, 20)]
            .map(|(x, y)| Point2::from_mm(100.0 + f64::from(x), 100.0 + f64::from(y)));
        // Areas 200 and 100, middles (10, 5) and (5, 15).
        assert_eq!(
            centroid(&l),
            Point2::from_mm(100.0 + 25.0 / 3.0, 100.0 + 25.0 / 3.0)
        );
        // Either way round.
        let mut back = l;
        back.reverse();
        assert_eq!(centroid(&back), centroid(&l));
    }

    #[test]
    fn rings_are_told_alike_from_any_start() {
        let ring = [(0, 0), (4, 0), (4, 4), (0, 4)].map(|(x, y)| Point2 { x, y });
        let mut turned = ring;
        turned.rotate_left(3);
        assert!(same_ring(&ring, &turned));
        let mut other = turned;
        other[1].x += 1;
        assert!(!same_ring(&ring, &other));
        assert!(!same_ring(&ring, &ring[..3]));
    }
}
