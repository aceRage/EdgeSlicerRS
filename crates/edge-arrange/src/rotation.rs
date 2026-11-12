//! Turning an item before it is placed, when turning is allowed: the turn
//! that gives the smallest box round it, and for an item the bed cannot
//! hold otherwise, a turn that makes it fit (libnest2d
//! `utils/rotcalipers.hpp`: `rotcalipers`, `minAreaBoundingBox`,
//! `fitIntoBoxRotation`; Arrange.cpp `min_area_boundingbox_rotation`,
//! `fit_into_box_rotation`).
//!
//! The reference compares its ratios exactly (boost rationals of 128-bit
//! integers); here the same comparisons are made by cross-multiplying
//! into 256 bits, so ties fall the same way.

use std::cmp::Ordering;

use edge_geom::{Point2, Polygon};

use crate::geom::{Bbox, rotated, sub};

/// A 256-bit product, as a sign and a magnitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wide {
    negative: bool,
    hi: u128,
    lo: u128,
}

fn wide(a: i128, b: i128) -> Wide {
    let (x, y) = (a.unsigned_abs(), b.unsigned_abs());
    let mask = u128::from(u64::MAX);
    let (x1, x0, y1, y0) = (x >> 64, x & mask, y >> 64, y & mask);
    let low = x0 * y0;
    let mid1 = x1 * y0;
    let mid2 = x0 * y1;
    let high = x1 * y1;
    // Sum the middle terms with carries.
    let (mid, mid_carry) = mid1.overflowing_add(mid2);
    let (lo, lo_carry) = low.overflowing_add(mid << 64);
    let hi = high + (mid >> 64) + (u128::from(mid_carry) << 64) + u128::from(lo_carry);
    let zero = hi == 0 && lo == 0;
    Wide {
        negative: (a < 0) != (b < 0) && !zero,
        hi,
        lo,
    }
}

fn compare(a: Wide, b: Wide) -> Ordering {
    match (a.negative, b.negative) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => (a.hi, a.lo).cmp(&(b.hi, b.lo)),
        (true, true) => (b.hi, b.lo).cmp(&(a.hi, a.lo)),
    }
}

/// An exact fraction with a positive denominator.
#[derive(Clone, Copy, Debug)]
struct Ratio {
    num: i128,
    den: i128,
}

impl Ratio {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(wide(self.num, other.den), wide(other.num, self.den))
    }
}

fn dot(a: Point2, b: Point2) -> i128 {
    i128::from(a.x) * i128::from(b.x) + i128::from(a.y) * i128::from(b.y)
}

fn magnsq(a: Point2) -> i128 {
    dot(a, a)
}

/// libnest2d's `perp`: (y, -x).
fn perp(a: Point2) -> Point2 {
    Point2 { x: a.y, y: -a.x }
}

fn neg(a: Point2) -> Point2 {
    Point2 { x: -a.x, y: -a.y }
}

/// A box turned to an axis: its extents along the axis (`bottom`) and
/// across it (`right`), each scaled by the axis's length.
#[derive(Clone, Copy, Debug, Default)]
struct RotatedBox {
    axis: Point2,
    bottom: i128,
    right: i128,
}

impl RotatedBox {
    fn area(&self) -> Ratio {
        Ratio {
            num: self.bottom * self.right,
            den: magnsq(self.axis),
        }
    }

    fn width(&self) -> f64 {
        (self.bottom.abs() as f64) / (magnsq(self.axis) as f64).sqrt()
    }

    fn height(&self) -> f64 {
        (self.right.abs() as f64) / (magnsq(self.axis) as f64).sqrt()
    }

    /// The turn that lays the axis along x (`RotatedBox::angleToX`).
    fn angle_to_x(&self) -> f64 {
        let mut angle = (self.axis.y as f64).atan2(self.axis.x as f64);
        if angle.is_sign_negative() {
            angle += 2.0 * std::f64::consts::PI;
        }
        -angle
    }
}

/// The rotating calipers over a convex outline: each box with a side on
/// one of the outline's sides, starting with the box square to the axes,
/// until `visit` says stop (libnest2d `rotcalipers`).
fn rotcalipers(shape: &Polygon, mut visit: impl FnMut(&RotatedBox) -> bool) -> Option<()> {
    let mut pts: Vec<Point2> = shape.points.clone();
    if pts.len() < 2 {
        return Some(());
    }
    if pts.first() == pts.last() {
        pts.pop();
    }
    if pts.len() < 3 {
        return Some(());
    }
    {
        let (p, q, r) = (pts[0], pts[1], pts[pts.len() - 1]);
        let d = (i128::from(q.y) - i128::from(p.y)) * (i128::from(r.x) - i128::from(p.x))
            - (i128::from(q.x) - i128::from(p.x)) * (i128::from(r.y) - i128::from(p.y));
        if d > 0 {
            // Clockwise: walk it the other way, from the same first corner.
            let mut flipped: Vec<Point2> = pts[1..].iter().rev().copied().collect();
            flipped.push(pts[0]);
            pts = flipped;
        }
    }
    let n = pts.len();
    let next = |i: usize| if i == n - 1 { 0 } else { i + 1 };
    let prev = |i: usize| if i == 0 { n - 1 } else { i - 1 };

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (0, 0, 0, 0);
    for (i, v) in pts.iter().enumerate() {
        let d = sub(*v, pts[min_x]);
        if d.x < 0 || (d.x == 0 && d.y < 0) {
            min_x = i;
        }
        let d = sub(*v, pts[max_x]);
        if d.x > 0 || (d.x == 0 && d.y > 0) {
            max_x = i;
        }
        let d = sub(*v, pts[min_y]);
        if d.y < 0 || (d.y == 0 && d.x > 0) {
            min_y = i;
        }
        let d = sub(*v, pts[max_y]);
        if d.y > 0 || (d.y == 0 && d.x < 0) {
            max_y = i;
        }
    }

    let boxed = |w: Point2, rect: &[usize; 4]| RotatedBox {
        axis: w,
        bottom: dot(w, sub(pts[rect[1]], pts[rect[3]])),
        right: dot(neg(perp(w)), sub(pts[rect[2]], pts[rect[0]])),
    };

    // The next sides to lay the box on (libnest2d's `update`); false when
    // the outline turns out to be a rectangle already done.
    let update = |w: Point2, rect: &mut [usize; 4]| -> Option<bool> {
        let sides = rect.map(|i| sub(pts[next(i)], pts[i]));
        let pw = perp(w);
        let dots = [
            dot(w, sides[0]),
            dot(neg(pw), sides[1]),
            dot(neg(w), sides[2]),
            dot(pw, sides[3]),
        ];
        let dw = magnsq(w);
        let mut angles = [Ratio { num: 0, den: 1 }; 4];
        for k in 0..4 {
            let den = magnsq(sides[k]);
            if den == 0 {
                // The reference's rationals throw on this, and it then
                // answers no turn.
                return None;
            }
            angles[k] = Ratio {
                num: dots[k] * dots[k],
                den,
            };
        }
        let dw = Ratio { num: dw, den: 1 };
        // Sorted from the greatest, a new one before those it equals.
        let mut sorted: Vec<(Ratio, usize)> = Vec::with_capacity(4);
        let mut i = 3;
        for j in 0..4 {
            if rect[i] != rect[j] && angles[i].cmp(&dw) == Ordering::Less {
                let at = sorted
                    .iter()
                    .position(|(a, _)| a.cmp(&angles[i]) != Ordering::Greater)
                    .unwrap_or(sorted.len());
                sorted.insert(at, (angles[i], i));
            }
            i = j;
        }
        let Some(&(least, first)) = sorted.first() else {
            return Some(false);
        };
        for (angle, k) in &sorted {
            if angle.cmp(&least) == Ordering::Equal {
                rect[*k] = next(rect[*k]);
            }
        }
        rect.rotate_left(first);
        Some(true)
    };

    let mut w = Point2 { x: 1, y: 0 };
    let mut rect = [min_y, max_x, max_y, min_x];
    if !visit(&boxed(w, &rect)) {
        return Some(());
    }
    let mut edgemask = vec![false; n];
    let mut c = 0;
    while c < n {
        c += 1;
        if !update(w, &mut rect)? {
            break;
        }
        let eidx = rect[0];
        if edgemask[eidx] {
            break;
        }
        edgemask[eidx] = true;
        w = sub(pts[rect[0]], pts[prev(rect[0])]);
        if !visit(&boxed(w, &rect)) {
            break;
        }
    }
    Some(())
}

/// The turn that gives a convex outline the smallest box round it
/// (Arrange.cpp `min_area_boundingbox_rotation`); among boxes alike in
/// area the last found, as the reference keeps it. No turn when the
/// outline is degenerate.
pub fn min_area_rotation(shape: &Polygon) -> f64 {
    let mut least = Ratio {
        num: i128::from(i64::MAX),
        den: 1,
    };
    let mut best = RotatedBox::default();
    let done = rotcalipers(shape, |rbox| {
        let area = rbox.area();
        if area.cmp(&least) != Ordering::Greater {
            least = area;
            best = *rbox;
        }
        true
    });
    if done.is_none() {
        return 0.0;
    }
    best.angle_to_x()
}

/// A further turn that makes a convex outline fit the box `bed` when it
/// is too wide or deep for it as it stands (Arrange.cpp
/// `fit_into_box_rotation`, libnest2d `fitIntoBoxRotation`): the calipers
/// find two boxes between which the outline's aspect ratio passes the
/// bed's, and the turn between them is bisected until it fits.
///
/// The reference never moves its "previous box" along as the calipers
/// turn (rotcalipers.hpp l.300: `prev_rbox` is left as it was made), so it
/// always bisects from no turn at all; here the previous box is the one
/// visited before, as the comment over the function says it is.
pub fn fit_into_box_rotation(shape: &Polygon, bed: &Bbox) -> f64 {
    let aspect = |w: f64, h: f64| w / h;
    let target = aspect(bed.width() as f64, bed.height() as f64);
    let mut previous: Option<RotatedBox> = None;
    let (mut from, mut to) = (0.0, 0.0);
    rotcalipers(shape, |rbox| {
        let lower_previous = previous.is_some_and(|p| aspect(p.width(), p.height()) < target);
        let lower_current = aspect(rbox.width(), rbox.height()) < target;
        if previous.is_some() && lower_previous != lower_current {
            from = previous.map(|p| p.angle_to_x()).unwrap_or(0.0);
            to = rbox.angle_to_x();
            return false;
        }
        previous = Some(*rbox);
        true
    });
    let box_at = |r: f64| Bbox::of(&rotated(shape, r).points);
    let ratio_at = |r: f64| {
        let b = box_at(r);
        aspect(b.width() as f64, b.height() as f64)
    };
    let fits = |b: &Bbox| b.width() < bed.width() && b.height() < bed.height();
    let mut ratio_from = ratio_at(from);
    let mut middle = (from + to) / 2.0;
    let mut box_middle = box_at(middle);
    while !fits(&box_middle) && (to - from).abs() > 1e-4 {
        let ratio_middle = aspect(box_middle.width() as f64, box_middle.height() as f64);
        if (ratio_from < target) != (ratio_middle < target) {
            to = middle;
        } else {
            from = middle;
        }
        ratio_from = ratio_at(from);
        middle = (from + to) / 2.0;
        box_middle = box_at(middle);
    }
    middle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn polygon(points: &[(i64, i64)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| Point2 { x, y }).collect(),
        }
    }

    #[test]
    fn wide_products_compare_exactly() {
        let big = i128::from(i64::MAX) * 1000;
        assert_eq!(
            compare(wide(big, big), wide(big, big - 1)),
            Ordering::Greater
        );
        assert_eq!(compare(wide(-big, big), wide(big, 1)), Ordering::Less);
        assert!(!wide(0, -5).negative);
        assert_eq!(
            wide(3, 4),
            Wide {
                negative: false,
                hi: 0,
                lo: 12
            }
        );
    }

    #[test]
    fn a_tilted_bar_is_turned_square() {
        // A 100 by 10 bar turned 30 degrees.
        let bar = polygon(&[(0, 0), (100_000, 0), (100_000, 10_000), (0, 10_000)]);
        let tilted = rotated(&bar, 30f64.to_radians());
        let turn = min_area_rotation(&tilted);
        let back = Bbox::of(&rotated(&tilted, turn).points);
        let area = back.width() as f64 * back.height() as f64;
        assert!((area - 1.0e9).abs() / 1.0e9 < 1e-3, "{back:?}");
    }

    #[test]
    fn a_box_too_long_for_the_bed_is_turned_to_fit() {
        let bar = polygon(&[(0, 0), (120, 0), (120, 10), (0, 10)]);
        let bed = Bbox::new(Point2 { x: 0, y: 0 }, Point2 { x: 100, y: 100 });
        let turn = fit_into_box_rotation(&bar, &bed);
        let b = Bbox::of(&rotated(&bar, turn).points);
        assert!(b.width() < 100 && b.height() < 100, "{turn} {b:?}");
    }
}
