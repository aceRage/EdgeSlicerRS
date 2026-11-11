//! Arc fitting: finds the stretches of a path that lie on a circle, so
//! the writer can lay each as one `G2` or `G3` instead of many short
//! straight moves (`enable_arc_fitting`).
//!
//! Ported from the reference: the fitter is ArcFitter.cpp
//! (`do_arc_fitting`, `do_arc_fitting_and_simplify`), the geometry
//! Circle.cpp (`Circle`, `ArcSegment`), and what carries a fitted path
//! through being turned about, cut, joined and shortened is Polyline.cpp
//! (`reverse`, `clip_end`, `split_at`, `split_at_index`, `append`) with
//! `ExtrusionLoop::split_at` from ExtrusionEntity.cpp. Constants,
//! tolerances and the order things are tried in are the reference's, and
//! so is every place a value is cut to a whole number.
//!
//! Where this differs from the reference on purpose, each said again at
//! its place:
//!
//! - the circle through the path's middle point is among those tried
//!   when the first guess fails ([`circle_of`]);
//! - an arc found by going the other way round keeps the angle of the
//!   way it goes, and a counter-clockwise arc may have its middle point
//!   due east of its centre ([`arc_on`]);
//! - two points in the same place have no foot of a perpendicular
//!   between them ([`perpendicular_foot`]).

use std::f64::consts::PI;

use edge_geom::Point2;

/// What one unit of a coordinate is in mm (libslic3r.h `SCALING_FACTOR`).
/// The thresholds below are worked out from it the way the reference's
/// macros work them out, so they come to the same numbers.
const SCALING_FACTOR: f64 = 0.000001;
/// libslic3r.h `EPSILON`, and the same in coordinate units.
const EPSILON: f64 = 1e-4;
const SCALED_EPSILON: f64 = EPSILON / SCALING_FACTOR;
/// Circle.hpp `ZERO_TOLERANCE`.
const ZERO_TOLERANCE: f64 = 0.000005;
/// Circle.cpp `Parallel_area_threshold`, in mm²: three points whose
/// triangle is no bigger are taken to be in line.
const PARALLEL_AREA_THRESHOLD: f64 = 0.0001;
/// Circle.hpp `DEFAULT_SCALED_MAX_RADIUS`: 2 m.
const MAX_RADIUS: f64 = 2000.0 / SCALING_FACTOR;
/// Circle.hpp `DEFAULT_ARC_LENGTH_PERCENT_TOLERANCE`: an arc may differ
/// in length from the path it replaces by less than this share.
const LENGTH_TOLERANCE: f64 = 0.05;

/// How far an arc may stray from sparse infill and from supports, in mm
/// (libslic3r.h `SPARSE_INFILL_RESOLUTION`, `SUPPORT_RESOLUTION`).
/// Everything else is held to the `resolution` setting.
pub(crate) const SPARSE_INFILL_RESOLUTION: f64 = 0.04;
pub(crate) const SUPPORT_RESOLUTION: f64 = 0.0375;

/// A tolerance in mm as the fitter takes it.
pub(crate) fn tolerance(mm: f64) -> f64 {
    mm / SCALING_FACTOR
}

fn point(x: i64, y: i64) -> Point2 {
    Point2 { x, y }
}

/// Distance between two points in coordinate units, as Eigen's `norm`.
fn span(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    (dx * dx + dy * dy).sqrt()
}

/// Circle.hpp `Circle::is_equal` and its kin, at [`ZERO_TOLERANCE`].
fn is_equal(x: f64, y: f64) -> bool {
    (x - y).abs() < ZERO_TOLERANCE
}

fn less_than_or_equal(x: f64, y: f64) -> bool {
    x < y || is_equal(x, y)
}

fn greater_than_or_equal(x: f64, y: f64) -> bool {
    x > y || is_equal(x, y)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Circle {
    center: Point2,
    /// In coordinate units.
    radius: f64,
}

impl Circle {
    /// `Circle::get_polar_radians`: the angle of a point seen from the
    /// centre, from 0 up to a full turn.
    fn polar(&self, p: Point2) -> f64 {
        let polar = ((p.y - self.center.y) as f64).atan2((p.x - self.center.x) as f64);
        if polar < 0.0 { 2.0 * PI + polar } else { polar }
    }

    /// `Circle::get_closest_point`: the point of the circle nearest
    /// `input`, cut to whole units as the reference's cast cuts it.
    fn closest_point(&self, input: Point2) -> Point2 {
        let (dx, dy) = (
            (input.x - self.center.x) as f64,
            (input.y - self.center.y) as f64,
        );
        let norm = (dx * dx + dy * dy).sqrt();
        // Eigen's `normalized` leaves a vector of no length as it is.
        let (ux, uy) = if norm > 0.0 {
            (dx / norm, dy / norm)
        } else {
            (dx, dy)
        };
        point(
            self.center.x + (ux * self.radius) as i64,
            self.center.y + (uy * self.radius) as i64,
        )
    }

    fn distance_from_center(&self, p: Point2) -> f64 {
        span(self.center, p)
    }

    /// `Circle::is_over_deviation`: whether any point, or the place on
    /// any segment nearest the centre, is further than `tolerance` from
    /// the circle. The first and last points are on it by construction.
    fn is_over_deviation(&self, points: &[Point2], tolerance: f64) -> bool {
        for index in 0..points.len() - 1 {
            if index != 0
                && (self.distance_from_center(points[index]) - self.radius).abs() > tolerance
            {
                return true;
            }
            if let Some(foot) = perpendicular_foot(points[index], points[index + 1], self.center)
                && (self.distance_from_center(foot) - self.radius).abs() > tolerance
            {
                return true;
            }
        }
        false
    }

    /// `Circle::get_deviation_sum_squared`: the sum of the squares of
    /// the same distances, or none when one of them is over `tolerance`.
    /// Also none once the sum has reached `least`, the sum to beat: it
    /// only grows, so the circle cannot be the one strayed from least,
    /// and the reference's answer is the same without the rest of the
    /// sum being worked out.
    fn deviation_sum_squared(
        &self,
        points: &[Point2],
        tolerance: f64,
        least: Option<f64>,
    ) -> Option<f64> {
        let least = least.unwrap_or(f64::INFINITY);
        let mut total = 0.0;
        for p in &points[1..points.len() - 1] {
            let deviation = (self.distance_from_center(*p) - self.radius).abs();
            total += deviation * deviation;
            if deviation > tolerance || total >= least {
                return None;
            }
        }
        for pair in points.windows(2) {
            if let Some(foot) = perpendicular_foot(pair[0], pair[1], self.center) {
                let deviation = (self.distance_from_center(foot) - self.radius).abs();
                total += deviation * deviation;
                if deviation > tolerance || total >= least {
                    return None;
                }
            }
        }
        Some(total)
    }
}

/// `Circle::get_closest_perpendicular_point`: where the perpendicular
/// from `c` meets the segment from `p1` to `p2`, cut to whole units;
/// none when it meets it at an end or beyond.
fn perpendicular_foot(p1: Point2, p2: Point2, c: Point2) -> Option<Point2> {
    let (x1, y1) = (p1.x as f64, p1.y as f64);
    let (x2, y2) = (p2.x as f64, p2.y as f64);
    let (x_dif, y_dif) = (x2 - x1, y2 - y1);
    let num = (c.x as f64 - x1) * x_dif + (c.y as f64 - y1) * y_dif;
    let denom = x_dif * x_dif + y_dif * y_dif;
    // Not as the reference: there two points in the same place divide
    // nothing by nothing, the comparisons below all fail on the result
    // and a point is made from it, which no rule defines. A segment of
    // no length has no foot.
    if denom == 0.0 {
        return None;
    }
    let t = num / denom;
    if less_than_or_equal(t, 0.0) || greater_than_or_equal(t, 1.0) {
        return None;
    }
    Some(point(
        (x1 + t * (x2 - x1)) as i64,
        (y1 + t * (y2 - y1)) as i64,
    ))
}

/// `Circle::try_create_circle` for three points: the circle through
/// them, unless they are nearly in line or it is larger than `max_radius`.
fn circle_through(p1: Point2, p2: Point2, p3: Point2, max_radius: f64) -> Option<Circle> {
    let (x1, y1) = (p1.x as f64, p1.y as f64);
    let (x2, y2) = (p2.x as f64, p2.y as f64);
    let (x3, y3) = (p3.x as f64, p3.y as f64);
    // The points are scaled once, so an area is scaled twice.
    if ((y1 - y2) * (x1 - x3) - (y1 - y3) * (x1 - x2)).abs()
        <= PARALLEL_AREA_THRESHOLD / SCALING_FACTOR / SCALING_FACTOR
    {
        return None;
    }
    let a = x1 * (y2 - y3) - y1 * (x2 - x3) + x2 * y3 - x3 * y2;
    if a.abs() < SCALED_EPSILON {
        return None;
    }
    let b = (x1 * x1 + y1 * y1) * (y3 - y2)
        + (x2 * x2 + y2 * y2) * (y1 - y3)
        + (x3 * x3 + y3 * y3) * (y2 - y1);
    let c = (x1 * x1 + y1 * y1) * (x2 - x3)
        + (x2 * x2 + y2 * y2) * (x3 - x1)
        + (x3 * x3 + y3 * y3) * (x1 - x2);
    let center_x = -b / (2.0 * a);
    let center_y = -c / (2.0 * a);
    let (delta_x, delta_y) = (center_x - x1, center_y - y1);
    let radius = (delta_x * delta_x + delta_y * delta_y).sqrt();
    if radius > max_radius {
        return None;
    }
    Some(Circle {
        center: point(center_x.round() as i64, center_y.round() as i64),
        radius,
    })
}

/// `Circle::try_create_circle` for a path: a circle through its ends
/// that every point and segment of it lies within `tolerance` of. The
/// one through the middle is tried first; failing that, of the circles
/// through each point between the ends, the one the path strays from
/// least.
fn circle_of(points: &[Point2], max_radius: f64, tolerance: f64) -> Option<Circle> {
    let count = points.len();
    let middle_index = count / 2;
    let (first, last) = (points[0], points[count - 1]);
    let fits = |circle: &Circle| !circle.is_over_deviation(points, tolerance);
    if count == 3 {
        return circle_through(first, points[middle_index], last, max_radius).filter(fits);
    }
    // Between the two middle points of an even number, and between the
    // neighbours of the middle point of an odd one; halved as whole
    // numbers are, toward zero.
    let (a, b) = if count.is_multiple_of(2) {
        (points[middle_index], points[middle_index - 1])
    } else {
        (points[middle_index - 1], points[middle_index + 1])
    };
    let middle_point = point((a.x + b.x) / 2, (a.y + b.y) / 2);
    if let Some(circle) = circle_through(first, middle_point, last, max_radius).filter(fits) {
        return Some(circle);
    }
    // Not as the reference, which leaves the path's own middle point out
    // of this search as "already checked": that was so in the code it
    // took this from (ArcWelder), which tried the middle point first,
    // but what it tries first now is the point made above, so the middle
    // point itself was never tried. It is tried here with the rest.
    let mut best: Option<(f64, Circle)> = None;
    for index in 1..count - 1 {
        let Some(circle) = circle_through(first, points[index], last, max_radius) else {
            continue;
        };
        let least = best.map(|(least, _)| least);
        if let Some(deviation) = circle.deviation_sum_squared(points, tolerance, least) {
            best = Some((deviation, circle));
        }
    }
    best.map(|(_, circle)| circle)
}

/// A circular move (Circle.hpp `ArcSegment`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Arc {
    pub(crate) center: Point2,
    /// In coordinate units.
    pub(crate) radius: f64,
    pub(crate) start: Point2,
    pub(crate) end: Point2,
    /// Counter-clockwise, a `G3`; otherwise clockwise, a `G2`.
    pub(crate) ccw: bool,
    /// Along the arc, in coordinate units.
    pub(crate) length: f64,
    /// The angle turned through: positive counter-clockwise, negative
    /// clockwise.
    angle: f64,
    polar_start: f64,
    polar_end: f64,
}

impl Arc {
    fn circle(&self) -> Circle {
        Circle {
            center: self.center,
            radius: self.radius,
        }
    }

    /// Length along the arc in mm.
    pub(crate) fn length_mm(&self) -> f64 {
        self.length * SCALING_FACTOR
    }

    /// `ArcSegment::reverse`.
    fn reverse(&mut self) {
        std::mem::swap(&mut self.start, &mut self.end);
        self.ccw = !self.ccw;
        self.angle *= -1.0;
        std::mem::swap(&mut self.polar_start, &mut self.polar_end);
    }

    /// `ArcSegment::update_angle_and_length`.
    fn update_angle_and_length(&mut self) {
        let circle = self.circle();
        self.polar_start = circle.polar(self.start);
        self.polar_end = circle.polar(self.end);
        self.angle = self.polar_end - self.polar_start;
        if self.angle < 0.0 && self.ccw {
            self.angle += 2.0 * PI;
        } else if self.angle > 0.0 && !self.ccw {
            self.angle -= 2.0 * PI;
        }
        self.length = self.angle.abs() * self.radius;
    }

    /// `ArcSegment::is_point_inside`: whether a point is, by its angle
    /// from the centre, between the arc's ends.
    fn is_point_inside(&self, p: Point2) -> bool {
        let mut delta = self.circle().polar(p) - self.polar_start;
        if delta > 0.0 && !self.ccw {
            delta -= 2.0 * PI;
        } else if delta < 0.0 && self.ccw {
            delta += 2.0 * PI;
        }
        if self.ccw {
            delta > 0.0 && delta < self.angle
        } else {
            delta < 0.0 && delta > self.angle
        }
    }

    /// `ArcSegment::clip_start`: starts the arc abreast of `p` instead.
    /// False, and nothing done, when `p` is not abreast of the arc.
    fn clip_start(&mut self, p: Point2) -> bool {
        if p == self.center || !self.is_point_inside(p) {
            return false;
        }
        self.start = self.circle().closest_point(p);
        self.update_angle_and_length();
        true
    }

    /// `ArcSegment::clip_end`: the same for its end.
    fn clip_end(&mut self, p: Point2) -> bool {
        if p == self.center || !self.is_point_inside(p) {
            return false;
        }
        self.end = self.circle().closest_point(p);
        self.update_angle_and_length();
        true
    }

    /// The box round the arc in coordinate units, bulge and all: min x,
    /// min y, max x, max y.
    pub(crate) fn bounds(&self) -> [f64; 4] {
        let (cx, cy) = (self.center.x as f64, self.center.y as f64);
        let (sx, sy) = (self.start.x as f64, self.start.y as f64);
        let (ex, ey) = (self.end.x as f64, self.end.y as f64);
        let mut bounds = [sx.min(ex), sy.min(ey), sx.max(ex), sy.max(ey)];
        // Where the circle is furthest east, north, west and south, if
        // the arc passes there. A little is added to the sweep so that an
        // arc ending just short of such a place still counts it.
        let from = (sy - cy).atan2(sx - cx);
        let sweep = self.angle.abs() + 1e-6;
        for (quarter, (dx, dy)) in [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]
            .into_iter()
            .enumerate()
        {
            let at = quarter as f64 * PI / 2.0;
            let turned = if self.ccw { at - from } else { from - at }.rem_euclid(2.0 * PI);
            if turned <= sweep {
                let (x, y) = (cx + dx * self.radius, cy + dy * self.radius);
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x),
                    bounds[3].max(y),
                ];
            }
        }
        bounds
    }
}

/// `ArcSegment::try_create_arc` for a path: the arc from its first point
/// to its last that it lies within `tolerance` of, if there is one.
/// `approximate_length` is the path's own length.
fn arc_of(points: &[Point2], approximate_length: f64, tolerance: f64) -> Option<Arc> {
    let circle = circle_of(points, MAX_RADIUS, tolerance)?;
    let mid_point_index = (points.len() - 2) / 2 + 1;
    let arc = arc_on(
        &circle,
        points[0],
        points[mid_point_index],
        points[points.len() - 1],
        approximate_length,
    )?;
    within_slice(&arc, points).then_some(arc)
}

/// `ArcSegment::try_create_arc` on a known circle: which way round the
/// arc from `start` to `end` goes, told by where `mid` is, and that its
/// length is that of the path it stands for.
fn arc_on(
    circle: &Circle,
    start: Point2,
    mid: Point2,
    end: Point2,
    approximate_length: f64,
) -> Option<Arc> {
    let polar_start = circle.polar(start);
    let polar_mid = circle.polar(mid);
    let polar_end = circle.polar(end);

    let mut angle = 0.0;
    let mut ccw: Option<bool> = None;
    if polar_end > polar_start {
        if polar_start < polar_mid && polar_mid < polar_end {
            ccw = Some(true);
            angle = polar_end - polar_start;
        } else if (0.0 <= polar_mid && polar_mid < polar_start)
            || (polar_end < polar_mid && polar_mid < 2.0 * PI)
        {
            ccw = Some(false);
            angle = polar_start + (2.0 * PI - polar_end);
        }
    } else if polar_start > polar_end {
        // Not as the reference, which has `0.0 < polar_mid` here where
        // the clockwise case above has `0.0 <=`: a counter-clockwise arc
        // through the angle nought whose middle point is exactly there,
        // due east of the centre, is then no arc at all, and the path's
        // arc is cut in two at whatever point came before. A round wall
        // drawn square to the axes does it on every layer.
        if (polar_start < polar_mid && polar_mid < 2.0 * PI)
            || (0.0 <= polar_mid && polar_mid < polar_end)
        {
            ccw = Some(true);
            angle = polar_end + (2.0 * PI - polar_start);
        } else if polar_end < polar_mid && polar_mid < polar_start {
            ccw = Some(false);
            angle = polar_start - polar_end;
        }
    }
    let mut ccw = ccw?;
    if f64::abs(angle) < EPSILON {
        return None;
    }

    // The length is checked against the path's. A difference can simply
    // be the two lengths differing, or the direction found above being
    // the wrong one, which very close points can cause: then the arc the
    // other way round is tried.
    let mut length = circle.radius * angle;
    let difference = (length - approximate_length) / approximate_length;
    if difference.abs() >= LENGTH_TOLERANCE {
        let other = (angle - 2.0 * PI).abs();
        let other_length = circle.radius * other;
        let difference = (other_length - approximate_length) / approximate_length;
        if difference.abs() >= LENGTH_TOLERANCE {
            return None;
        }
        length = other_length;
        ccw = !ccw;
        // Not as the reference, which turns the direction about "but not
        // the angle" and so keeps, with the length of this arc, the
        // angle of the one it gave up. The angle is what says whether a
        // point is abreast of the arc when the arc is cut at a seam
        // (`is_point_inside`), so it is this arc's own here.
        angle = other;
    }
    if !ccw {
        angle *= -1.0;
    }
    Some(Arc {
        center: circle.center,
        radius: circle.radius,
        start,
        end,
        ccw,
        length,
        angle,
        polar_start,
        polar_end,
    })
}

/// `ArcSegment::are_points_within_slice`: that the path's last stretch
/// runs the way the arc does and does not cross the lines from the
/// centre to the arc's ends. Only the last two points are looked at:
/// the fitter grows a path a point at a time and has checked the rest.
fn within_slice(arc: &Arc, points: &[Point2]) -> bool {
    let circle = arc.circle();
    let mut previous_polar = arc.polar_start;
    let mut crossed_zero = false;
    let count = points.len();

    let start_norm = (
        (arc.start.x as f64 - arc.center.x as f64) / arc.radius,
        (arc.start.y as f64 - arc.center.y as f64) / arc.radius,
    );
    let end_norm = (
        (arc.end.x as f64 - arc.center.x as f64) / arc.radius,
        (arc.end.y as f64 - arc.center.y as f64) / arc.radius,
    );
    let will_cross_zero = if arc.ccw {
        arc.polar_start > arc.polar_end
    } else {
        arc.polar_start < arc.polar_end
    };

    for index in count - 2..count {
        let polar_test = if index < count - 1 {
            circle.polar(points[index])
        } else {
            arc.polar_end
        };
        if arc.ccw {
            // A point that is not the end has to be within the arc.
            if index < count - 1 {
                if will_cross_zero {
                    if !(polar_test > arc.polar_start || polar_test < arc.polar_end) {
                        return false;
                    }
                } else if !(arc.polar_start < polar_test && polar_test < arc.polar_end) {
                    return false;
                }
            }
            // The angles rise, but for one passing of zero.
            if previous_polar > polar_test {
                if !will_cross_zero || crossed_zero {
                    return false;
                }
                crossed_zero = true;
            }
        } else {
            if index < count - 1 {
                if will_cross_zero {
                    if !(polar_test < arc.polar_start || polar_test > arc.polar_end) {
                        return false;
                    }
                } else if !(arc.polar_start > polar_test && polar_test > arc.polar_end) {
                    return false;
                }
            }
            if previous_polar < polar_test {
                if !will_cross_zero || crossed_zero {
                    return false;
                }
                crossed_zero = true;
            }
        }

        let (a, b) = (points[index - 1], points[index]);
        if (index != 1 && ray_intersects_segment(arc.center, start_norm, a, b))
            || (index != count - 1 && ray_intersects_segment(arc.center, end_norm, a, b))
        {
            return false;
        }
        previous_polar = polar_test;
    }
    will_cross_zero == crossed_zero
}

/// `ArcSegment::ray_intersects_segment`.
fn ray_intersects_segment(origin: Point2, direction: (f64, f64), a: Point2, b: Point2) -> bool {
    let v1 = ((origin.x - a.x) as f64, (origin.y - a.y) as f64);
    let v2 = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let v3 = (-direction.1, direction.0);
    let dot = v2.0 * v3.0 + v2.1 * v3.1;
    if dot.abs() < SCALED_EPSILON {
        return false;
    }
    let t1 = (v2.0 * v1.1 - v2.1 * v1.0) / dot;
    let t2 = (v1.0 * v3.0 + v1.1 * v3.1) / dot;
    t1 >= 0.0 && (0.0..=1.0).contains(&t2)
}

/// How a stretch of a path is laid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    /// Straight from point to point.
    Line,
    /// As one arc from its first point to its last; the points between
    /// are kept for whatever follows the path itself, a wipe for one.
    Arc(Arc),
}

/// A stretch of a path: from the point at `start` to the one at `end`
/// (ArcFitter.hpp `PathFittingData`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Part {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) kind: Kind,
}

impl Part {
    fn line(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            kind: Kind::Line,
        }
    }

    fn is_arc(&self) -> bool {
        matches!(self.kind, Kind::Arc(_))
    }
}

/// `ArcFitter::do_arc_fitting`: goes along the path adding a point at a
/// time to the stretch in hand for as long as an arc fits it; when one
/// no longer does, the stretch up to the point before is an arc, or with
/// three points only, its first segment is a straight move.
fn fit_parts(points: &[Point2], tolerance: f64) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    if points.len() < 3 {
        parts.push(Part::line(0, points.len() - 1));
        return parts;
    }
    let add_line = |parts: &mut Vec<Part>, start: usize, end: usize| match parts.last_mut() {
        Some(last) if !last.is_arc() => last.end = end,
        _ => parts.push(Part::line(start, end)),
    };
    let mut front = 0;
    let mut back = 0;
    let mut last_arc: Option<Arc> = None;
    // The length of the stretch in hand, summed from its start as the
    // reference sums it afresh each time.
    let mut length = 0.0;
    for (index, &p) in points.iter().enumerate() {
        back = index;
        if back > front {
            length += span(points[back - 1], p);
        }
        if back - front < 2 {
            continue;
        }
        match arc_of(&points[front..=back], length, tolerance) {
            Some(arc) => {
                last_arc = Some(arc);
                if back == points.len() - 1 {
                    parts.push(Part {
                        start: front,
                        end: back,
                        kind: Kind::Arc(arc),
                    });
                    front = back;
                }
            }
            None => {
                match last_arc.filter(|_| back - front > 2) {
                    // The stretch fitted until this point was added.
                    Some(arc) => parts.push(Part {
                        start: front,
                        end: back - 1,
                        kind: Kind::Arc(arc),
                    }),
                    None => add_line(&mut parts, front, front + 1),
                }
                front = back - 1;
                length = span(points[front], points[back]);
            }
        }
    }
    if front != back {
        add_line(&mut parts, front, back);
    }
    parts
}

/// `MultiPoint::_douglas_peucker`: an open path with the points left out
/// that are within `tolerance` of the line between those kept, measured
/// to the segment. (edge-clip has the same for outlines.) Also what a
/// path is thinned with when no arcs are fitted to it (`simplify.rs`).
pub(crate) fn douglas_peucker(points: &[Point2], tolerance: f64) -> Vec<Point2> {
    let Some(&first) = points.first() else {
        return Vec::new();
    };
    let mut kept = vec![first];
    if points.len() == 1 {
        return kept;
    }
    let tolerance_sq = tolerance * tolerance;
    let mut anchor = 0;
    let mut floaters = vec![points.len() - 1];
    while let Some(&floater) = floaters.last() {
        let (mut furthest, mut furthest_sq) = (anchor, 0.0);
        for index in anchor + 1..floater {
            let distance_sq =
                distance_to_segment_sq(points[index], points[anchor], points[floater]);
            if distance_sq > furthest_sq {
                (furthest, furthest_sq) = (index, distance_sq);
            }
        }
        if furthest_sq <= tolerance_sq {
            kept.push(points[floater]);
            anchor = floater;
            floaters.pop();
        } else {
            floaters.push(furthest);
        }
    }
    kept
}

/// Line.hpp `distance_to_squared`.
fn distance_to_segment_sq(p: Point2, a: Point2, b: Point2) -> f64 {
    let (vx, vy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let (ax, ay) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
    let l2 = vx * vx + vy * vy;
    if l2 == 0.0 {
        return ax * ax + ay * ay;
    }
    let t = (ax * vx + ay * vy) / l2;
    if t <= 0.0 {
        ax * ax + ay * ay
    } else if t >= 1.0 {
        let (bx, by) = ((p.x - b.x) as f64, (p.y - b.y) as f64);
        bx * bx + by * by
    } else {
        let (ox, oy) = (ax - t * vx, ay - t * vy);
        ox * ox + oy * oy
    }
}

/// `Point::projection_onto` a segment.
fn projection_onto(p: Point2, a: Point2, b: Point2) -> Point2 {
    if a == b {
        return a;
    }
    let (lx, ly) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let theta = ((b.x - p.x) as f64 * lx + (b.y - p.y) as f64 * ly) / (lx * lx + ly * ly);
    if (0.0..=1.0).contains(&theta) {
        return point(
            (theta * a.x as f64 + (1.0 - theta) * b.x as f64) as i64,
            (theta * a.y as f64 + (1.0 - theta) * b.y as f64) as i64,
        );
    }
    let to = |q: Point2| {
        let (dx, dy) = ((q.x - p.x) as f64, (q.y - p.y) as f64);
        dx * dx + dy * dy
    };
    if to(a) < to(b) { a } else { b }
}

/// A path and how each stretch of it is laid (Polyline.hpp `Polyline`
/// with its `fitting_result`). The parts follow one another from the
/// first point to the last. A path of fewer than two points has none.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Fitted {
    pub(crate) points: Vec<Point2>,
    pub(crate) parts: Vec<Part>,
}

impl Fitted {
    /// `ArcFitter::do_arc_fitting_and_simplify`: fits arcs to `points`
    /// within `tolerance` (see [`tolerance`]), then drops from each
    /// stretch, straight or arc, the points that are within the same
    /// tolerance of the line between their neighbours. An arc keeps
    /// those that outline it, which a wipe follows.
    pub(crate) fn new(points: &[Point2], tolerance: f64) -> Self {
        if points.len() < 2 {
            return Self {
                points: points.to_vec(),
                parts: Vec::new(),
            };
        }
        // A tolerance of next to nothing fits no arcs.
        let mut parts = if tolerance.abs() > SCALED_EPSILON {
            fit_parts(points, tolerance)
        } else {
            vec![Part::line(0, points.len() - 1)]
        };
        if let [only] = parts.as_mut_slice()
            && !only.is_arc()
        {
            let points = douglas_peucker(points, tolerance);
            only.end = points.len() - 1;
            return Self { points, parts };
        }
        let mut simplified = Vec::with_capacity(points.len());
        simplified.push(points[0]);
        let mut at = 0;
        for part in &mut parts {
            let kept = douglas_peucker(&points[part.start..=part.end], tolerance);
            simplified.extend_from_slice(&kept[1..]);
            part.start = at;
            at += kept.len() - 1;
            part.end = at;
        }
        Self {
            points: simplified,
            parts,
        }
    }

    /// A path laid straight throughout.
    pub(crate) fn straight(points: Vec<Point2>) -> Self {
        let parts = if points.len() < 2 {
            Vec::new()
        } else {
            vec![Part::line(0, points.len() - 1)]
        };
        Self { points, parts }
    }

    fn is_valid(&self) -> bool {
        self.points.len() >= 2
    }

    #[cfg(test)]
    fn has_arcs(&self) -> bool {
        self.parts.iter().any(Part::is_arc)
    }

    /// `Polyline::reverse`: the same path from its other end.
    pub(crate) fn reverse(&mut self) {
        self.points.reverse();
        let last = self.points.len().saturating_sub(1);
        for part in &mut self.parts {
            (part.start, part.end) = (last - part.end, last - part.start);
            if let Kind::Arc(arc) = &mut part.kind {
                arc.reverse();
            }
        }
        self.parts.reverse();
    }

    /// `Polyline::clip_end`: shortens the path by `distance` coordinate
    /// units from its end, measured along its points. Cut within an arc,
    /// the arc ends abreast of the cut; one that cannot be ended there
    /// becomes straight moves. A path no longer than `distance` is left
    /// with fewer than two points and no parts.
    pub(crate) fn clip_end(&mut self, mut distance: f64) {
        // `ExtrusionLoop::clip_end` asks for nothing to be done with
        // nothing to clip.
        if distance <= 0.0 {
            return;
        }
        let mut remove_after = self.points.len();
        while distance > 0.0 {
            let Some(last) = self.points.pop() else {
                break;
            };
            remove_after -= 1;
            let Some(&before) = self.points.last() else {
                self.parts.clear();
                return;
            };
            let (vx, vy) = ((before.x - last.x) as f64, (before.y - last.y) as f64);
            let lsqr = vx * vx + vy * vy;
            if lsqr > distance * distance {
                let share = distance / lsqr.sqrt();
                self.points.push(point(
                    (last.x as f64 + vx * share) as i64,
                    (last.y as f64 + vy * share) as i64,
                ));
                break;
            }
            distance -= lsqr.sqrt();
        }
        while self
            .parts
            .last()
            .is_some_and(|part| part.start >= remove_after)
        {
            self.parts.pop();
        }
        let end = self.points.len() - 1;
        let end_point = self.points[end];
        if let Some(part) = self.parts.last_mut() {
            if let Kind::Arc(arc) = &mut part.kind {
                if arc.clip_end(end_point) {
                    self.points[end] = arc.end;
                } else {
                    part.kind = Kind::Line;
                }
            }
            part.end = end;
        }
    }

    /// `Polyline::split_at_index`: the path up to the point at `index`
    /// and the path from it on. An arc the cut falls within is ended, and
    /// begun again, abreast of that point.
    pub(crate) fn split_at_index(&self, index: usize) -> (Self, Self) {
        let last = self.points.len() - 1;
        if index == 0 {
            return (Self::straight(vec![self.points[0]]), self.clone());
        }
        if index == last {
            return (self.clone(), Self::straight(vec![self.points[last]]));
        }
        // `split_fitting_result_before_index`
        let mut head = Self {
            points: self.points[..=index].to_vec(),
            parts: self
                .parts
                .iter()
                .take_while(|part| part.start < index)
                .copied()
                .collect(),
        };
        if let Some(part) = head.parts.last_mut() {
            if let Kind::Arc(arc) = &mut part.kind
                && part.end > index
            {
                if arc.clip_end(self.points[index]) {
                    head.points[index] = arc.end;
                } else {
                    part.kind = Kind::Line;
                }
            }
            part.end = index;
        }
        // `split_fitting_result_after_index`
        let mut tail = Self {
            points: self.points[index..].to_vec(),
            parts: self
                .parts
                .iter()
                .filter(|part| part.end > index)
                .copied()
                .collect(),
        };
        for (at, part) in tail.parts.iter_mut().enumerate() {
            part.end -= index;
            if at != 0 {
                part.start -= index;
                continue;
            }
            if let Kind::Arc(arc) = &mut part.kind
                && part.start < index
            {
                if arc.clip_start(self.points[index]) {
                    tail.points[0] = arc.start;
                } else {
                    part.kind = Kind::Line;
                }
            }
            part.start = 0;
        }
        (head, tail)
    }

    /// `Polyline::append` of one point: a straight move on to it.
    fn push(&mut self, p: Point2) {
        if self.points.last() == Some(&p) {
            return;
        }
        self.points.push(p);
        let end = self.points.len() - 1;
        if end == 0 {
            return;
        }
        match self.parts.last_mut() {
            Some(part) if !part.is_arc() => part.end = end,
            Some(part) => {
                let start = part.end;
                self.parts.push(Part::line(start, end));
            }
            None => self.parts.push(Part::line(0, end)),
        }
    }

    /// `Polyline::append_before`: a straight move from a new first point.
    pub(crate) fn push_front(&mut self, p: Point2) {
        if self.points.first() == Some(&p) {
            return;
        }
        self.reverse();
        self.push(p);
        self.reverse();
    }

    /// `Polyline::append` of a path, which starts where this one ends.
    fn append(&mut self, other: Self) {
        if !other.is_valid() {
            return;
        }
        if self.points.is_empty() {
            *self = other;
            return;
        }
        self.push(other.points[0]);
        let offset = self.points.len() - 1;
        self.points.extend_from_slice(&other.points[1..]);
        self.parts.extend(other.parts.into_iter().map(|part| Part {
            start: part.start + offset,
            end: part.end + offset,
            kind: part.kind,
        }));
    }

    /// `Polyline::split_at`: the path up to `p` and the path from it on.
    /// A `p` that is not one of the path's points is put on the segment
    /// nearest it, and the two halves of that segment are straight moves
    /// even where an arc ran through it.
    fn split_at(&self, p: Point2) -> (Self, Self) {
        if let Some(index) = self.points.iter().position(|q| *q == p) {
            return self.split_at_index(index);
        }
        let mut line_idx = 0;
        let mut nearest = self.points[0];
        let mut min = span(nearest, p);
        for (index, pair) in self.points.windows(2).enumerate() {
            let projected = projection_onto(p, pair[0], pair[1]);
            if span(projected, p) < min {
                nearest = projected;
                min = span(nearest, p);
                line_idx = index;
            }
        }
        if let Some(index) = self.points.iter().position(|q| *q == nearest) {
            let (mut head, mut tail) = self.split_at_index(index);
            head.push(p);
            tail.push_front(p);
            return (head, tail);
        }
        let (mut head, _) = self.split_at_index(line_idx);
        head.push(p);
        let (_, mut tail) = self.split_at_index(line_idx + 1);
        tail.push_front(p);
        (head, tail)
    }

    /// `Polyline::split_at_length`: the path up to `length` coordinate
    /// units along its points and the path from there on. The segment
    /// the cut falls on becomes two straight moves, even where an arc
    /// ran through it. None for a length the path does not have.
    pub(crate) fn split_at_length(&self, length: f64) -> Option<(Self, Self)> {
        let (first, last) = (*self.points.first()?, *self.points.last()?);
        let whole: f64 = self
            .points
            .windows(2)
            .map(|pair| span(pair[0], pair[1]))
            .sum();
        if length < 0.0 || length > whole {
            return None;
        }
        if length < SCALED_EPSILON {
            return Some((Self::straight(vec![first]), self.clone()));
        }
        if (length - whole).abs() < SCALED_EPSILON {
            return Some((self.clone(), Self::straight(vec![last])));
        }
        let (mut line_idx, mut done, mut p) = (0, 0.0, first);
        for pair in self.points.windows(2) {
            p = pair[1];
            let current = span(pair[0], pair[1]);
            if done + current >= length {
                // Point.hpp `lerp`, which drops the fraction of a unit.
                let t = (length - done) / current;
                p = point(
                    ((1.0 - t) * pair[0].x as f64 + t * pair[1].x as f64) as i64,
                    ((1.0 - t) * pair[0].y as f64 + t * pair[1].y as f64) as i64,
                );
                break;
            }
            done += current;
            line_idx += 1;
        }
        if let Some(index) = self.points.iter().position(|q| *q == p) {
            return Some(self.split_at_index(index));
        }
        let (mut head, _) = self.split_at_index(line_idx);
        head.push(p);
        let (_, mut tail) = self.split_at_index(line_idx + 1);
        tail.push_front(p);
        Some((head, tail))
    }

    /// `ExtrusionLoop::split_at` for a loop of one path: the closed path
    /// (its last point is its first) taken once round from `p` instead,
    /// which is one of its points or a point on it.
    pub(crate) fn start_at(&mut self, p: Point2) {
        let (head, mut tail) = self.split_at(p);
        *self = if !head.is_valid() {
            tail
        } else if !tail.is_valid() {
            head
        } else {
            tail.append(head);
            tail
        };
    }
}

#[cfg(test)]
mod tests {
    use edge_geom::scaled;

    use super::*;

    const RESOLUTION: f64 = 0.012;

    fn mm(x: f64, y: f64) -> Point2 {
        Point2 {
            x: scaled(x),
            y: scaled(y),
        }
    }

    /// Points on a circle about (`cx`, `cy`) from angle `from` to `to`,
    /// in degrees, `steps` segments.
    fn on_circle(cx: f64, cy: f64, r: f64, from: f64, to: f64, steps: usize) -> Vec<Point2> {
        (0..=steps)
            .map(|step| {
                let angle = (from + (to - from) * step as f64 / steps as f64).to_radians();
                mm(cx + r * angle.cos(), cy + r * angle.sin())
            })
            .collect()
    }

    fn fit(points: &[Point2]) -> Fitted {
        Fitted::new(points, tolerance(RESOLUTION))
    }

    fn arcs(fitted: &Fitted) -> Vec<Arc> {
        fitted
            .parts
            .iter()
            .filter_map(|part| match part.kind {
                Kind::Arc(arc) => Some(arc),
                Kind::Line => None,
            })
            .collect()
    }

    /// The parts follow one another from the first point to the last,
    /// and every arc starts and ends on the points it is said to.
    fn assert_whole(fitted: &Fitted) {
        assert_eq!(fitted.parts.first().map(|part| part.start), Some(0));
        assert_eq!(
            fitted.parts.last().map(|part| part.end),
            Some(fitted.points.len() - 1)
        );
        for pair in fitted.parts.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        for part in &fitted.parts {
            assert!(part.start < part.end);
            if let Kind::Arc(arc) = part.kind {
                assert_eq!(arc.start, fitted.points[part.start]);
                assert_eq!(arc.end, fitted.points[part.end]);
            }
        }
    }

    fn length_mm(points: &[Point2]) -> f64 {
        points
            .windows(2)
            .map(|pair| span(pair[0], pair[1]))
            .sum::<f64>()
            * SCALING_FACTOR
    }

    /// What a fitted path comes to as it is laid, in mm.
    fn laid_mm(fitted: &Fitted) -> f64 {
        fitted
            .parts
            .iter()
            .map(|part| match part.kind {
                Kind::Arc(arc) => arc.length_mm(),
                Kind::Line => length_mm(&fitted.points[part.start..=part.end]),
            })
            .sum()
    }

    #[test]
    fn a_quarter_circle_is_one_arc_each_way_round() {
        let points = on_circle(100.0, 100.0, 10.0, 0.0, 90.0, 64);
        let fitted = fit(&points);
        assert_whole(&fitted);
        let [arc] = arcs(&fitted)[..] else {
            panic!("{:?}", fitted.parts);
        };
        assert!(arc.ccw);
        assert_eq!((arc.start, arc.end), (points[0], points[64]));
        // The circle is the one through the ends and the point between
        // the two points either side of the middle, which is a little
        // inside the curve: within the tolerance, not on it.
        assert!((arc.radius * SCALING_FACTOR - 10.0).abs() < 0.05);
        assert!(span(arc.center, mm(100.0, 100.0)) * SCALING_FACTOR < 0.05);
        let quarter = 10.0 * PI / 2.0;
        assert!((arc.length_mm() - quarter).abs() < 0.01);
        assert!((arc.angle - PI / 2.0).abs() < 5e-3);
        // The points between are thinned to what outlines the arc.
        assert!(fitted.points.len() < points.len() && fitted.points.len() > 2);

        let mut back = points.clone();
        back.reverse();
        let fitted = fit(&back);
        let [arc] = arcs(&fitted)[..] else {
            panic!("{:?}", fitted.parts);
        };
        assert!(!arc.ccw);
        assert!((arc.angle + PI / 2.0).abs() < 5e-3);
        assert!((arc.length_mm() - quarter).abs() < 0.01);
    }

    #[test]
    fn a_closed_circle_is_an_arc_and_its_last_side() {
        // 128 sides, as a loop: the last point is the first again, which
        // no arc can end on.
        let mut points = on_circle(128.0, 128.0, 10.0, 0.0, 360.0, 128);
        points[128] = points[0];
        let fitted = fit(&points);
        assert_whole(&fitted);
        let found = arcs(&fitted);
        assert_eq!(found.len(), 1, "{:?}", fitted.parts);
        assert!(found[0].ccw);
        assert_eq!((found[0].start, found[0].end), (points[0], points[127]));
        assert_eq!(fitted.parts.len(), 2);
        assert_eq!(fitted.parts[1].kind, Kind::Line);
        let whole = 2.0 * PI * 10.0;
        assert!((found[0].length_mm() - whole * 127.0 / 128.0).abs() < 0.02);
    }

    #[test]
    fn an_s_curve_is_two_arcs_turning_opposite_ways() {
        // A half circle to the left, then one to the right.
        let mut points = on_circle(10.0, 50.0, 10.0, 180.0, 0.0, 48);
        points.extend(
            on_circle(30.0, 50.0, 10.0, 180.0, 360.0, 48)
                .into_iter()
                .skip(1),
        );
        let fitted = fit(&points);
        assert_whole(&fitted);
        let found = arcs(&fitted);
        assert_eq!(found.len(), 2, "{:?}", fitted.parts);
        assert!(!found[0].ccw && found[1].ccw);
        assert_eq!(found[0].start, points[0]);
        assert_eq!(found[1].end, points[96]);
        // They meet within a segment of where the curve turns over.
        assert!(span(found[0].end, mm(20.0, 50.0)) * SCALING_FACTOR < 1.0);
        let total: f64 = found.iter().map(Arc::length_mm).sum();
        assert!((total - 2.0 * PI * 10.0).abs() < 1.0, "{total}");
    }

    #[test]
    fn a_straight_line_with_noise_stays_straight_and_is_thinned() {
        // 20 mm in 40 steps, each point up to 20 nm off the line, which
        // is what rounding leaves: three points so nearly in line make
        // no circle.
        let noisy = |off: [f64; 5]| -> Vec<Point2> {
            (0..=40)
                .map(|step| mm(50.0 + 0.5 * step as f64, 80.0 + off[step % 5]))
                .collect()
        };
        let points = noisy([0.0, 0.00002, -0.00001, 0.00001, -0.00002]);
        let fitted = fit(&points);
        assert_whole(&fitted);
        assert!(!fitted.has_arcs(), "{:?}", fitted.parts);
        assert_eq!(fitted.parts, [Part::line(0, 1)]);
        assert_eq!(fitted.points, [points[0], points[40]]);

        // Noise of a few µm is another matter: it is within the
        // tolerance of some very shallow curves, and the reference's
        // fitter takes it for them. Whatever is fitted is shallow, and
        // nothing is laid longer or shorter than the path was.
        let points = noisy([0.0, 0.003, -0.002, 0.001, -0.003]);
        let fitted = fit(&points);
        assert_whole(&fitted);
        assert!(
            arcs(&fitted)
                .iter()
                .all(|arc| arc.radius * SCALING_FACTOR > 20.0)
        );
        assert!((laid_mm(&fitted) - length_mm(&points)).abs() < 0.01);
    }

    #[test]
    fn a_path_part_arc_and_part_straight_is_fitted_as_each() {
        // 20 mm straight, a quarter turn of 8 mm radius, 20 mm straight.
        let mut points = vec![mm(60.0, 100.0), mm(70.0, 100.0), mm(80.0, 100.0)];
        points.extend(
            on_circle(80.0, 108.0, 8.0, -90.0, 0.0, 40)
                .into_iter()
                .skip(1),
        );
        points.extend([mm(88.0, 118.0), mm(88.0, 128.0)]);
        let fitted = fit(&points);
        assert_whole(&fitted);
        let found = arcs(&fitted);
        assert_eq!(found.len(), 1, "{:?}", fitted.parts);
        assert!(found[0].ccw);
        assert!((found[0].radius * SCALING_FACTOR - 8.0).abs() < 0.02);
        // The straight runs either side are straight moves, thinned.
        assert_eq!(fitted.parts.first().unwrap().kind, Kind::Line);
        assert_eq!(fitted.parts.last().unwrap().kind, Kind::Line);
        assert_eq!(fitted.points[0], points[0]);
        assert_eq!(*fitted.points.last().unwrap(), *points.last().unwrap());
        // The arc takes up the turn, give or take a segment at each end.
        let turn = 8.0 * PI / 2.0;
        assert!((found[0].length_mm() - turn).abs() < 1.0);
        // Nothing is moved by more than the tolerance: the length holds.
        let laid = laid_mm(&fitted);
        assert!((laid - length_mm(&points)).abs() < 0.01, "{laid}");
    }

    #[test]
    fn tiny_arcs_and_sharp_corners_stay_lines() {
        // Three points nearly in line: the triangle they make is under
        // the area that counts as a turn.
        let nearly = [mm(10.0, 10.0), mm(10.005, 10.0001), mm(10.01, 10.0)];
        assert!(circle_through(nearly[0], nearly[1], nearly[2], MAX_RADIUS).is_none());
        assert!(!fit(&nearly).has_arcs());
        // A corner rounded to 0.05 mm in eight steps: every three points
        // of it are as near in line.
        let fillet = on_circle(50.0, 50.0, 0.05, 0.0, 90.0, 8);
        assert!(!fit(&fillet).has_arcs());
        // A square corner: a circle goes through any three points, but
        // the sides stray from it by far more than the tolerance.
        let corner = [mm(10.0, 10.0), mm(20.0, 10.0), mm(20.0, 20.0)];
        let fitted = fit(&corner);
        assert!(!fitted.has_arcs());
        assert_eq!(fitted.points, corner);
        // A coarse circle, 12 sides of a 10 mm radius: each side is
        // 0.34 mm from the circle at its middle.
        let coarse = on_circle(50.0, 50.0, 10.0, 0.0, 180.0, 6);
        assert!(!fit(&coarse).has_arcs());
        // A curve gentler than the largest radius is not an arc.
        let flat = on_circle(0.0, -2_500.0, 2_500.0, 89.0, 91.0, 40);
        assert!(circle_through(flat[0], flat[20], flat[40], MAX_RADIUS).is_none());
        // Two points, and one, are what they are.
        assert_eq!(fit(&corner[..2]).parts, [Part::line(0, 1)]);
        assert!(fit(&corner[..1]).parts.is_empty());
        // With next to no tolerance nothing is fitted.
        let round = on_circle(100.0, 100.0, 10.0, 0.0, 90.0, 32);
        assert!(!Fitted::new(&round, tolerance(0.00005)).has_arcs());
    }

    #[test]
    fn the_circle_through_the_middle_point_is_tried_too() {
        // Five points 2 degrees apart on a 10 mm circle, the second
        // 8 µm outside it and the fourth 8 µm inside. The first guess,
        // through the point between those two, is flatter than the
        // circle and leaves the second point out by more than the 10 µm
        // allowed; the circle through the second point leaves the fourth
        // out, and the other way about. Only the circle through the
        // middle point has them all within reach, and the reference
        // never tries it.
        let at = |degrees: f64, r: f64| {
            let angle = degrees.to_radians();
            mm(100.0 + r * angle.cos(), 100.0 + r * angle.sin())
        };
        let points = [
            at(0.0, 10.0),
            at(2.0, 10.008),
            at(4.0, 10.0),
            at(6.0, 9.992),
            at(8.0, 10.0),
        ];
        let allowed = tolerance(0.010);
        let through =
            |index: usize| circle_through(points[0], points[index], points[4], MAX_RADIUS).unwrap();
        assert!(
            through(1)
                .deviation_sum_squared(&points, allowed, None)
                .is_none()
        );
        assert!(
            through(3)
                .deviation_sum_squared(&points, allowed, None)
                .is_none()
        );
        let sum = through(2)
            .deviation_sum_squared(&points, allowed, None)
            .unwrap();
        // A sum that does not beat the least so far is not finished.
        assert!(
            through(2)
                .deviation_sum_squared(&points, allowed, Some(sum))
                .is_none()
        );
        assert!(
            through(2)
                .deviation_sum_squared(&points, allowed, Some(sum * 2.0))
                .is_some()
        );
        let found = circle_of(&points, MAX_RADIUS, allowed).unwrap();
        assert_eq!(found, through(2));
        assert!((found.radius * SCALING_FACTOR - 10.0).abs() < 0.01);
    }

    #[test]
    fn an_arc_may_have_its_middle_point_due_east_of_its_centre() {
        // A ring of 128 sides about a centre on whole units, begun just
        // past its lowest point and taken counter-clockwise: when the
        // arc has grown to a little over half the ring, its middle
        // point is the corner due east of the centre and the centre is
        // level with it to the unit, so the angle of the one from the
        // other is exactly nought. The reference takes that for no arc
        // and cuts the ring's arc in two at the top; it is one arc here.
        let ring: Vec<Point2> = (0..=128)
            .map(|step| {
                let angle = (((97 + step) % 128) as f64 * 2.8125).to_radians();
                mm(128.0 + 9.398 * angle.cos(), 128.0 + 9.398 * angle.sin())
            })
            .collect();
        let half = &ring[..63];
        let circle = circle_of(half, MAX_RADIUS, tolerance(RESOLUTION)).unwrap();
        assert_eq!(circle.polar(half[31]), 0.0);
        assert!(
            arc_of(
                half,
                length_mm(half) / SCALING_FACTOR,
                tolerance(RESOLUTION)
            )
            .is_some()
        );
        let fitted = fit(&ring);
        assert_whole(&fitted);
        let found = arcs(&fitted);
        assert_eq!(found.len(), 1, "{:?}", fitted.parts);
        assert_eq!((found[0].start, found[0].end), (ring[0], ring[127]));
        // The same ring the other way round never had the trouble.
        let mut back = ring.clone();
        back.reverse();
        assert_eq!(arcs(&fit(&back)).len(), 1);
    }

    #[test]
    fn reversing_turns_every_arc_about() {
        let mut points = vec![mm(60.0, 100.0), mm(80.0, 100.0)];
        points.extend(
            on_circle(80.0, 108.0, 8.0, -90.0, 0.0, 40)
                .into_iter()
                .skip(1),
        );
        points.push(mm(88.0, 128.0));
        let fitted = fit(&points);
        let mut turned = fitted.clone();
        turned.reverse();
        assert_whole(&turned);
        assert_eq!(turned.points[0], *points.last().unwrap());
        let (was, now) = (arcs(&fitted)[0], arcs(&turned)[0]);
        assert_eq!((now.start, now.end), (was.end, was.start));
        assert_eq!((now.ccw, now.angle), (!was.ccw, -was.angle));
        assert_eq!(now.length, was.length);
        turned.reverse();
        assert_eq!(turned, fitted);
    }

    #[test]
    fn a_loop_started_at_its_seam_cuts_the_arc_there() {
        let mut points = on_circle(128.0, 128.0, 10.0, 0.0, 360.0, 128);
        points[128] = points[0];
        let whole = fit(&points);
        let around = 2.0 * PI * 10.0;

        // A seam at one of the points the fit kept: the arc is cut in
        // two there and the loop goes once round from it.
        let seam = whole.points[whole.points.len() / 2];
        let mut fitted = whole.clone();
        fitted.start_at(seam);
        assert_whole(&fitted);
        // The loop starts and ends abreast of the seam on the circle,
        // which the point itself is a little off.
        let start = fitted.points[0];
        assert_eq!(*fitted.points.last().unwrap(), start);
        assert!(span(start, seam) * SCALING_FACTOR < 0.005);
        let found = arcs(&fitted);
        assert_eq!(found.len(), 2, "{:?}", fitted.parts);
        assert_eq!(found[0].start, start);
        assert_eq!(found[1].end, start);
        assert!(found.iter().all(|arc| arc.ccw));
        let total: f64 = found.iter().map(Arc::length_mm).sum();
        assert!((total - around * 127.0 / 128.0).abs() < 0.01, "{total}");

        // A seam between two points: it is joined to the arcs either
        // side by straight moves along the segment it is on.
        let (a, b) = (
            whole.points[whole.points.len() / 2],
            whole.points[whole.points.len() / 2 + 1],
        );
        let seam = Point2 {
            x: (a.x + b.x) / 2,
            y: (a.y + b.y) / 2,
        };
        let mut fitted = whole.clone();
        fitted.start_at(seam);
        assert_whole(&fitted);
        assert_eq!(
            (fitted.points[0], *fitted.points.last().unwrap()),
            (seam, seam)
        );
        assert_eq!(fitted.parts.first().unwrap().kind, Kind::Line);
        assert_eq!(fitted.parts.last().unwrap().kind, Kind::Line);
        assert_eq!(arcs(&fitted).len(), 2);
        assert!(span(fitted.points[1], b) * SCALING_FACTOR < 0.005);
        assert!(span(fitted.points[fitted.points.len() - 2], a) * SCALING_FACTOR < 0.005);

        // A seam where the loop already starts changes nothing.
        let mut fitted = whole.clone();
        fitted.start_at(points[0]);
        assert_eq!(fitted, whole);
    }

    #[test]
    fn clipping_the_end_shortens_the_last_arc() {
        let points = on_circle(100.0, 100.0, 10.0, 0.0, 180.0, 96);
        let mut fitted = fit(&points);
        let before = arcs(&fitted)[0];
        fitted.clip_end(tolerance(2.0));
        assert_whole(&fitted);
        let after = arcs(&fitted)[0];
        assert_eq!(after.start, before.start);
        assert!((before.length_mm() - after.length_mm() - 2.0).abs() < 0.01);
        // The new end is on the arc's circle.
        let end = *fitted.points.last().unwrap();
        assert_eq!(after.end, end);
        assert!((span(end, after.center) - after.radius).abs() < 2.0);
        assert_eq!((after.center, after.radius), (before.center, before.radius));

        // Clipped back past an arc, the arc goes and the line before it
        // is shortened.
        let mut points = vec![mm(0.0, 50.0), mm(20.0, 50.0)];
        points.extend(
            on_circle(20.0, 52.0, 2.0, -90.0, 0.0, 24)
                .into_iter()
                .skip(1),
        );
        let mut fitted = fit(&points);
        assert!(fitted.has_arcs());
        fitted.clip_end(tolerance(PI + 5.0));
        assert_whole(&fitted);
        assert!(!fitted.has_arcs());
        assert!((length_mm(&fitted.points) - 15.0).abs() < 0.01);

        // Clipped by its whole length or more, nothing is left to lay.
        let mut fitted = fit(&points);
        fitted.clip_end(tolerance(100.0));
        assert!(fitted.points.len() < 2 && fitted.parts.is_empty());
        // And by nothing, it is as it was.
        let mut fitted = fit(&points);
        let was = fitted.clone();
        fitted.clip_end(0.0);
        assert_eq!(fitted, was);
    }

    #[test]
    fn an_arcs_box_counts_its_bulge() {
        // A half circle over the top: its ends are at y 100, its top at 110.
        let points = on_circle(100.0, 100.0, 10.0, 0.0, 180.0, 96);
        let arc = arcs(&fit(&points))[0];
        let [min_x, min_y, max_x, max_y] = arc.bounds().map(|v| v * SCALING_FACTOR);
        assert!((min_x - 90.0).abs() < 1e-3 && (max_x - 110.0).abs() < 1e-3);
        assert!((min_y - 100.0).abs() < 1e-3 && (max_y - 110.0).abs() < 0.01);
        // The top is the circle's, which none of the path's points is.
        assert_eq!(arc.bounds()[3], arc.center.y as f64 + arc.radius);
        // The same the other way round.
        let mut turned = fit(&points);
        turned.reverse();
        assert_eq!(arcs(&turned)[0].bounds(), arc.bounds());
        // A short arc that passes none of the four is the box of its ends.
        let points = on_circle(100.0, 100.0, 10.0, 20.0, 60.0, 32);
        let arc = arcs(&fit(&points))[0];
        let [min_x, min_y, max_x, max_y] = arc.bounds();
        assert_eq!(
            [min_x, min_y, max_x, max_y],
            [
                points[32].x as f64,
                points[0].y as f64,
                points[0].x as f64,
                points[32].y as f64
            ]
        );
    }

    #[test]
    fn fitting_is_the_same_every_time() {
        let mut points = on_circle(128.0, 128.0, 9.4, 0.0, 360.0, 128);
        points[128] = points[0];
        assert_eq!(fit(&points), fit(&points));
    }
}
