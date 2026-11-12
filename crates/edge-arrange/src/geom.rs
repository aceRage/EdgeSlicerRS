//! The little geometry the arranger needs beyond the shared types: boxes
//! as libnest2d keeps them, turning an outline as libslic3r turns it, and
//! where a point lies against a region.

use edge_geom::{Coord, ExPolygon, Point2, Polygon};

/// A box with a flag for whether it holds anything (libnest2d `_Box`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bbox {
    pub min: Point2,
    pub max: Point2,
    pub defined: bool,
}

impl Bbox {
    pub fn new(min: Point2, max: Point2) -> Self {
        Self {
            min,
            max,
            defined: true,
        }
    }

    /// The box round some points; undefined when there are none.
    pub fn of<'a>(points: impl IntoIterator<Item = &'a Point2>) -> Self {
        let mut points = points.into_iter();
        let Some(first) = points.next() else {
            return Self::default();
        };
        let (mut min, mut max) = (*first, *first);
        for p in points {
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x);
            max.y = max.y.max(p.y);
        }
        Self::new(min, max)
    }

    /// The box round some regions' contours.
    pub fn of_regions(regions: &[ExPolygon]) -> Self {
        Self::of(regions.iter().flat_map(|region| &region.contour.points))
    }

    /// The box round both (`sl::boundingBox(bb1, bb2)`): an undefined one
    /// adds nothing.
    pub fn merge(self, other: Self) -> Self {
        if !self.defined {
            return other;
        }
        if !other.defined {
            return self;
        }
        Self::new(
            Point2 {
                x: self.min.x.min(other.min.x),
                y: self.min.y.min(other.min.y),
            },
            Point2 {
                x: self.max.x.max(other.max.x),
                y: self.max.y.max(other.max.y),
            },
        )
    }

    pub fn width(&self) -> Coord {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> Coord {
        self.max.y - self.min.y
    }

    pub fn area(&self) -> f64 {
        self.width() as f64 * self.height() as f64
    }

    /// The middle, each half taken apart and truncated, as libnest2d's
    /// `_Box::center` does it in integers.
    pub fn center(&self) -> Point2 {
        Point2 {
            x: self.min.x / 2 + self.max.x / 2,
            y: self.min.y / 2 + self.max.y / 2,
        }
    }

    pub fn translated(&self, by: Point2) -> Self {
        Self {
            min: add(self.min, by),
            max: add(self.max, by),
            defined: self.defined,
        }
    }

    /// Whether two closed boxes meet (boost's `intersects` on boxes).
    pub fn intersects(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && other.min.x <= self.max.x
            && self.min.y <= other.max.y
            && other.min.y <= self.max.y
    }

    /// What two boxes share, an empty box at the corner when nothing
    /// (`_Box::intersection`).
    pub fn intersection(&self, other: &Self) -> Self {
        let mut min = Point2 {
            x: self.min.x.max(other.min.x),
            y: self.min.y.max(other.min.y),
        };
        let mut max = Point2 {
            x: self.max.x.min(other.max.x),
            y: self.max.y.min(other.max.y),
        };
        if max.y < min.y || max.x < min.x {
            max = min;
        }
        if max == min {
            min = max;
        }
        Self::new(min, max)
    }

    pub fn polygon(&self) -> Polygon {
        Polygon {
            points: vec![
                self.min,
                Point2 {
                    x: self.max.x,
                    y: self.min.y,
                },
                self.max,
                Point2 {
                    x: self.min.x,
                    y: self.max.y,
                },
            ],
        }
    }
}

pub fn add(a: Point2, b: Point2) -> Point2 {
    Point2 {
        x: a.x + b.x,
        y: a.y + b.y,
    }
}

pub fn sub(a: Point2, b: Point2) -> Point2 {
    Point2 {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}

/// libnest2d's `pl::distance`, in floating point.
pub fn distance(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    (dx * dx + dy * dy).sqrt()
}

/// Turns an outline about the origin, rounding each corner to the
/// nanometre as libslic3r's `MultiPoint::rotate` does.
pub fn rotated(polygon: &Polygon, angle: f64) -> Polygon {
    let (s, c) = angle.sin_cos();
    Polygon {
        points: polygon
            .points
            .iter()
            .map(|p| {
                let (x, y) = (p.x as f64, p.y as f64);
                Point2 {
                    x: (c * x - s * y).round() as Coord,
                    y: (c * y + s * x).round() as Coord,
                }
            })
            .collect(),
    }
}

pub fn translated(polygon: &Polygon, by: Point2) -> Polygon {
    Polygon {
        points: polygon.points.iter().map(|p| add(*p, by)).collect(),
    }
}

/// Where a point lies against a closed ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Inside,
    On,
    Outside,
}

/// Where `p` lies against a ring, whichever way it runs, in exact integer
/// arithmetic.
pub fn ring_side(ring: &Polygon, p: Point2) -> Side {
    let pts = &ring.points;
    let n = pts.len();
    if n == 0 {
        return Side::Outside;
    }
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        // On the side itself?
        let cross = i128::from(b.x - a.x) * i128::from(p.y - a.y)
            - i128::from(b.y - a.y) * i128::from(p.x - a.x);
        if cross == 0
            && p.x >= a.x.min(b.x)
            && p.x <= a.x.max(b.x)
            && p.y >= a.y.min(b.y)
            && p.y <= a.y.max(b.y)
        {
            return Side::On;
        }
        // Crossing number, half-open in y.
        if (a.y > p.y) != (b.y > p.y) {
            // x of the side at p.y, compared without dividing.
            let lhs = i128::from(p.x - a.x) * i128::from(b.y - a.y);
            let rhs = i128::from(b.x - a.x) * i128::from(p.y - a.y);
            let left_of = if b.y > a.y { lhs < rhs } else { lhs > rhs };
            if left_of {
                inside = !inside;
            }
        }
    }
    if inside { Side::Inside } else { Side::Outside }
}

/// Where `p` lies against a region with holes: inside its material, on
/// one of its rings, or outside.
pub fn region_side(region: &ExPolygon, p: Point2) -> Side {
    match ring_side(&region.contour, p) {
        Side::Outside => Side::Outside,
        Side::On => Side::On,
        Side::Inside => {
            for hole in &region.holes {
                match ring_side(hole, p) {
                    Side::Inside => return Side::Outside,
                    Side::On => return Side::On,
                    Side::Outside => {}
                }
            }
            Side::Inside
        }
    }
}

/// The point of some regions' rings nearest `from` (libslic3r
/// `projection_onto(ExPolygons, Point)`), truncated to the nanometre as
/// its `Point::projection_onto(Line)` casts it.
pub fn projection_onto(regions: &[ExPolygon], from: Point2) -> Point2 {
    let mut projected = from;
    let mut least = f64::MAX;
    for region in regions {
        for ring in region.rings() {
            let p = project_onto_ring(ring, from);
            let (dx, dy) = ((from.x - p.x) as f64, (from.y - p.y) as f64);
            let dist = dx * dx + dy * dy;
            if dist < least {
                projected = p;
                least = dist;
            }
        }
    }
    projected
}

fn project_onto_ring(ring: &Polygon, from: Point2) -> Point2 {
    let pts = &ring.points;
    let Some(&first) = pts.first() else {
        return from;
    };
    let norm = |p: Point2| distance(p, from);
    let mut running = first;
    let mut least = norm(first);
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let p = project_onto_line(a, b, from);
        if norm(p) < least {
            running = p;
            least = norm(p);
        }
    }
    running
}

fn project_onto_line(a: Point2, b: Point2, p: Point2) -> Point2 {
    if a == b {
        return a;
    }
    let (lx, ly) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let theta = ((b.x - p.x) as f64 * lx + (b.y - p.y) as f64 * ly) / (lx * lx + ly * ly);
    if (0.0..=1.0).contains(&theta) {
        return Point2 {
            x: (theta * a.x as f64 + (1.0 - theta) * b.x as f64) as Coord,
            y: (theta * a.y as f64 + (1.0 - theta) * b.y as f64) as Coord,
        };
    }
    let sq = |q: Point2| {
        let (dx, dy) = ((q.x - p.x) as f64, (q.y - p.y) as f64);
        dx * dx + dy * dy
    };
    if sq(a) < sq(b) { a } else { b }
}

/// The signed area of a ring in square nanometres, positive when it runs
/// counter-clockwise.
pub fn signed_area(ring: &Polygon) -> f64 {
    let pts = &ring.points;
    let doubled: i128 = (0..pts.len())
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            i128::from(a.x) * i128::from(b.y) - i128::from(b.x) * i128::from(a.y)
        })
        .sum();
    doubled as f64 / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Polygon {
        Polygon {
            points: vec![
                Point2 { x: 0, y: 0 },
                Point2 { x: 10, y: 0 },
                Point2 { x: 10, y: 10 },
                Point2 { x: 0, y: 10 },
            ],
        }
    }

    #[test]
    fn a_point_is_inside_on_or_outside() {
        let ring = square();
        assert_eq!(ring_side(&ring, Point2 { x: 5, y: 5 }), Side::Inside);
        assert_eq!(ring_side(&ring, Point2 { x: 10, y: 5 }), Side::On);
        assert_eq!(ring_side(&ring, Point2 { x: 0, y: 0 }), Side::On);
        assert_eq!(ring_side(&ring, Point2 { x: 11, y: 5 }), Side::Outside);
        let mut clockwise = square();
        clockwise.points.reverse();
        assert_eq!(ring_side(&clockwise, Point2 { x: 5, y: 5 }), Side::Inside);
    }

    #[test]
    fn a_hole_is_outside_the_region() {
        let mut hole = Polygon {
            points: vec![
                Point2 { x: 2, y: 2 },
                Point2 { x: 8, y: 2 },
                Point2 { x: 8, y: 8 },
                Point2 { x: 2, y: 8 },
            ],
        };
        hole.points.reverse();
        let region = ExPolygon {
            contour: square(),
            holes: vec![hole],
        };
        assert_eq!(region_side(&region, Point2 { x: 5, y: 5 }), Side::Outside);
        assert_eq!(region_side(&region, Point2 { x: 1, y: 5 }), Side::Inside);
        assert_eq!(region_side(&region, Point2 { x: 2, y: 5 }), Side::On);
    }

    #[test]
    fn the_nearest_point_of_a_ring() {
        let region = ExPolygon {
            contour: square(),
            holes: Vec::new(),
        };
        assert_eq!(
            projection_onto(&[region], Point2 { x: 15, y: 4 }),
            Point2 { x: 10, y: 4 }
        );
    }

    #[test]
    fn a_box_centre_halves_each_corner() {
        let bb = Bbox::new(Point2 { x: -3, y: 1 }, Point2 { x: 4, y: 6 });
        assert_eq!(bb.center(), Point2 { x: -1 + 2, y: 3 });
    }
}
