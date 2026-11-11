//! Shapes and measures the tests of several modules share.

use edge_geom::{ExPolygon, Point2, Polygon};

pub(crate) fn mm(x: f64, y: f64) -> Point2 {
    Point2::from_mm(x, y)
}

/// A counter-clockwise rectangle, corners in mm, starting bottom left.
pub(crate) fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
    Polygon {
        points: vec![mm(x0, y0), mm(x1, y0), mm(x1, y1), mm(x0, y1)],
    }
}

pub(crate) fn rect_region(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
    ExPolygon {
        contour: rect(x0, y0, x1, y1),
        holes: Vec::new(),
    }
}

/// The distance in units from a point to a segment.
pub(crate) fn segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let (px, py) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let length_sq = dx * dx + dy * dy;
    let t = if length_sq > 0.0 {
        ((px * dx + py * dy) / length_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (px - t * dx).hypot(py - t * dy)
}

/// The distance in units from a point to a closed outline.
pub(crate) fn outline_distance(p: Point2, outline: &[Point2]) -> f64 {
    (0..outline.len())
        .map(|i| segment_distance(p, outline[i], outline[(i + 1) % outline.len()]))
        .fold(f64::INFINITY, f64::min)
}

/// How far round a closed outline, in units, the point of it nearest to
/// `p` lies, measured from its first point.
pub(crate) fn outline_arc(p: Point2, outline: &[Point2]) -> f64 {
    let mut best = (f64::INFINITY, 0.0);
    let mut walked = 0.0;
    for i in 0..outline.len() {
        let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
        let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
        let length = dx.hypot(dy);
        let distance = segment_distance(p, a, b);
        if distance < best.0 {
            let along = ((p.x - a.x) as f64 * dx + (p.y - a.y) as f64 * dy) / length;
            best = (distance, walked + along.clamp(0.0, length));
        }
        walked += length;
    }
    best.1
}
