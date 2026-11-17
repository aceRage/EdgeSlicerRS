//! Rounding the corners of a sparse pattern's lines
//! (`sparse_infill_smooth_factor`), ported from the reference's
//! Fill/FillCornerSmoothing.cpp: each sharp vertex is replaced by the
//! polygon of tangents to a quintic Bézier curve joining the two legs,
//! taken at equal steps of direction, so every vertex of a rounded corner
//! turns by the same angle and a planner keeps more of its speed.

use std::collections::HashMap;

use edge_geom::{Coord, Point2};

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;
/// Turns sharper than this are left alone: the curve's ends nearly meet.
const MIN_SMOOTHED_TURN_COSINE: f64 = -0.9;
/// Spacing of the curve's control points along each leg, as a share of
/// the reach.
const CONTROL_SPACING: f64 = 0.3;
/// The most vertices one corner gets.
const MAX_CORNER_VERTICES: usize = 256;

type Vec2 = (f64, f64);

fn sub(a: Vec2, b: Vec2) -> Vec2 {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: Vec2, b: Vec2) -> Vec2 {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(k: f64, a: Vec2) -> Vec2 {
    (k * a.0, k * a.1)
}
fn norm(a: Vec2) -> f64 {
    a.0.hypot(a.1)
}
fn dot(a: Vec2, b: Vec2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn cross(a: Vec2, b: Vec2) -> f64 {
    a.0 * b.1 - a.1 * b.0
}

/// The factor, as the reference takes it: NaN or infinite is none, the
/// rest clamped to 0..=1 (`sanitize_smooth_factor`).
pub(crate) fn sanitize(factor: f64) -> f64 {
    if factor.is_finite() {
        factor.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

type Control = [Vec2; 6];

/// The control points of the corner curve of `reach`, the incoming leg
/// along +x into the corner at the origin and the outgoing leg leaving it
/// along `outgoing`.
fn control_points(reach: f64, outgoing: Vec2) -> Control {
    let spacing = CONTROL_SPACING * reach;
    [
        (-reach, 0.0),
        (-(reach - spacing), 0.0),
        (-(reach - 2.0 * spacing), 0.0),
        mul(reach - 2.0 * spacing, outgoing),
        mul(reach - spacing, outgoing),
        mul(reach, outgoing),
    ]
}

/// The first and second derivatives, by de Casteljau down to the last
/// three points.
fn derivatives(control: &Control, t: f64) -> (Vec2, Vec2) {
    let mut work = *control;
    let mut count = work.len();
    while count > 3 {
        for i in 0..count - 1 {
            work[i] = add(mul(1.0 - t, work[i]), mul(t, work[i + 1]));
        }
        count -= 1;
    }
    let degree = (work.len() - 1) as f64;
    (
        mul(
            degree,
            add(
                mul(1.0 - t, sub(work[1], work[0])),
                mul(t, sub(work[2], work[1])),
            ),
        ),
        mul(
            degree * (degree - 1.0),
            add(sub(work[2], mul(2.0, work[1])), work[0]),
        ),
    )
}

fn point_at(control: &Control, t: f64) -> Vec2 {
    let mut work = *control;
    let mut count = work.len();
    while count > 1 {
        for i in 0..count - 1 {
            work[i] = add(mul(1.0 - t, work[i]), mul(t, work[i + 1]));
        }
        count -= 1;
    }
    work[0]
}

/// A rounded corner of unit reach: its vertices in the (incoming,
/// outgoing) basis, and how far they stray from the curve.
struct UnitCorner {
    vertices: Vec<Vec2>,
    deviation: f64,
}

/// The shapes worked out so far, by the turn's cosine (as bits) and the
/// vertex count. They depend on nothing else, so keeping them changes
/// nothing but the time.
#[derive(Default)]
struct Shapes {
    corners: HashMap<(u64, usize), UnitCorner>,
    peaks: HashMap<u64, f64>,
}

impl Shapes {
    /// The peak curvature of the curve times its reach.
    fn peak_by_reach(&mut self, cosine: f64) -> f64 {
        if let Some(peak) = self.peaks.get(&cosine.to_bits()) {
            return *peak;
        }
        let turn = cosine.clamp(-1.0, 1.0).acos();
        let control = control_points(1.0, (cosine, turn.sin()));
        const SAMPLES: usize = 256;
        let mut peak = 0.0f64;
        for i in 0..=SAMPLES {
            let (first, second) = derivatives(&control, i as f64 / SAMPLES as f64);
            let speed = norm(first);
            if speed > EPSILON {
                peak = peak.max(cross(first, second).abs() / (speed * speed * speed));
            }
        }
        self.peaks.insert(cosine.to_bits(), peak);
        peak
    }

    fn unit_corner(&mut self, cosine: f64, count: usize) -> &UnitCorner {
        self.corners
            .entry((cosine.to_bits(), count))
            .or_insert_with(|| tangent_polygon(cosine, count))
    }
}

/// The polygon of `count` vertices of the corner of unit reach.
fn tangent_polygon(cosine: f64, count: usize) -> UnitCorner {
    let turn = cosine.clamp(-1.0, 1.0).acos();
    let sine = turn.sin();
    let outgoing = (cosine, sine);
    // Tangents at equal steps of direction, both legs included, and
    // halfway between them.
    let mut tangents: Vec<(Vec2, Vec2)> = Vec::with_capacity(2 * count + 1);
    tangents.push(((-1.0, 0.0), (1.0, 0.0)));
    let control = control_points(1.0, outgoing);
    let mut from = 0.0;
    for i in 1..2 * count {
        let angle = 0.5 * turn * i as f64 / count as f64;
        let (mut low, mut high) = (from, 1.0f64);
        let mut t = 0.5 * (low + high);
        let mut iteration = 0;
        while iteration < 64 && high - low > 1e-15 {
            let (first, second) = derivatives(&control, t);
            let error = first.1.atan2(first.0) - angle;
            if error.abs() < 1e-14 {
                break;
            }
            if error < 0.0 {
                low = t;
            } else {
                high = t;
            }
            let rate = cross(first, second) / dot(first, first);
            let next = if rate > 0.0 { t - error / rate } else { low };
            t = if next > low && next < high {
                next
            } else {
                0.5 * (low + high)
            };
            iteration += 1;
        }
        from = low;
        let direction = derivatives(&control, t).0;
        let length = norm(direction);
        tangents.push((
            point_at(&control, t),
            (direction.0 / length, direction.1 / length),
        ));
    }
    tangents.push((outgoing, outgoing));
    let mut vertices = Vec::with_capacity(count);
    let mut deviation = 0.0f64;
    for i in 0..count {
        let (point, direction) = tangents[2 * i];
        let (next_point, next_direction) = tangents[2 * i + 2];
        let vertex = add(
            point,
            mul(
                cross(sub(next_point, point), next_direction) / cross(direction, next_direction),
                direction,
            ),
        );
        deviation = deviation.max(norm(sub(vertex, tangents[2 * i + 1].0)));
        let along_outgoing = vertex.1 / sine;
        vertices.push((vertex.0 - along_outgoing * cosine, along_outgoing));
    }
    UnitCorner {
        vertices,
        deviation,
    }
}

/// Rounds the corners of a path given point by point (`CornerSmoother`).
pub(crate) struct Smoother {
    /// How far a curve reaches along the shorter leg, as a share of it.
    ratio: f64,
    /// The G-code resolution, in the units of the points.
    tolerance: f64,
    window: [Vec2; 3],
    held: usize,
    shapes: Shapes,
}

impl Smoother {
    /// `factor` as set, `tolerance` in the units the points come in.
    pub(crate) fn new(factor: f64, tolerance: f64) -> Self {
        Self {
            ratio: 0.5 * sanitize(factor),
            tolerance,
            window: [(0.0, 0.0); 3],
            held: 0,
            shapes: Shapes::default(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.ratio > 0.0
    }

    pub(crate) fn push(&mut self, point: Vec2, emit: &mut impl FnMut(Vec2)) {
        if self.held == 0 {
            // The first point is an end, not a corner.
            emit(point);
            self.window[0] = point;
            self.held = 1;
            return;
        }
        if self.held > 1
            && on_straight_run(
                self.window[self.held - 2],
                self.window[self.held - 1],
                point,
            )
        {
            self.window[self.held - 1] = point;
            return;
        }
        if self.held < 3 {
            self.window[self.held] = point;
            self.held += 1;
            return;
        }
        let [previous, corner, next] = self.window;
        self.round(previous, corner, next, emit);
        self.window = [corner, next, point];
    }

    /// Emits the last point and gets ready for a new path.
    pub(crate) fn flush(&mut self, emit: &mut impl FnMut(Vec2)) {
        if self.held > 2 {
            let [previous, corner, next] = self.window;
            self.round(previous, corner, next, emit);
        }
        if self.held > 1 {
            emit(self.window[self.held - 1]);
        }
        self.held = 0;
    }

    fn round(&mut self, previous: Vec2, corner: Vec2, next: Vec2, emit: &mut impl FnMut(Vec2)) {
        let (incoming_leg, outgoing_leg) = (sub(corner, previous), sub(next, corner));
        let (incoming_length, outgoing_length) = (norm(incoming_leg), norm(outgoing_leg));
        if incoming_length < EPSILON || outgoing_length < EPSILON {
            emit(corner);
            return;
        }
        let incoming = mul(1.0 / incoming_length, incoming_leg);
        let outgoing = mul(1.0 / outgoing_length, outgoing_leg);
        let turning = cross(incoming, outgoing);
        // Snapped, so the corners of one turn share their shapes.
        let cosine = (dot(incoming, outgoing) * 1e12).round() * 1e-12;
        if turning.abs() < EPSILON || cosine < MIN_SMOOTHED_TURN_COSINE {
            emit(corner);
            return;
        }
        let reach = self.ratio * incoming_length.min(outgoing_length);
        let Some(vertices) = self.vertices(reach, cosine) else {
            emit(corner);
            return;
        };
        for (along_in, along_out) in vertices {
            emit(add(
                corner,
                mul(
                    reach,
                    add(mul(along_in, incoming), mul(along_out, outgoing)),
                ),
            ));
        }
    }

    /// The vertices of the corner of `reach`, or none where the sharp
    /// corner keeps within the tolerance.
    fn vertices(&mut self, reach: f64, cosine: f64) -> Option<Vec<Vec2>> {
        let tolerance = if self.tolerance > 0.0 && self.tolerance.is_finite() {
            self.tolerance
        } else {
            EPSILON
        };
        let turn = cosine.clamp(-1.0, 1.0).acos();
        let curvature = self.shapes.peak_by_reach(cosine) / reach;
        let most =
            (MAX_CORNER_VERTICES as f64).min(turn / (2.0 * curvature * tolerance).sqrt()) as usize;
        let mut count =
            most.min((turn / (8.0 * curvature * tolerance).sqrt() - EPSILON).ceil() as usize);
        if count < 2 {
            return None;
        }
        loop {
            let deviation = reach * self.shapes.unit_corner(cosine, count).deviation;
            if deviation <= tolerance || count >= most {
                break;
            }
            count = most.min(
                (count + 1).max((count as f64 * (deviation / tolerance).sqrt()).ceil() as usize),
            );
        }
        Some(self.shapes.unit_corner(cosine, count).vertices.clone())
    }
}

fn on_straight_run(previous: Vec2, vertex: Vec2, next: Vec2) -> bool {
    let (incoming_leg, outgoing_leg) = (sub(vertex, previous), sub(next, vertex));
    let (incoming_length, outgoing_length) = (norm(incoming_leg), norm(outgoing_leg));
    if incoming_length < EPSILON || outgoing_length < EPSILON {
        return true;
    }
    let incoming = mul(1.0 / incoming_length, incoming_leg);
    let outgoing = mul(1.0 / outgoing_length, outgoing_leg);
    dot(incoming, outgoing) > 0.0 && cross(incoming, outgoing).abs() < EPSILON
}

/// Rounds the corners of scaled open paths in place, both ends kept
/// (`smooth_polylines_corners`); `tolerance` in units. Points that come
/// to the same whole unit one after the other are kept once.
pub(crate) fn smooth_lines(lines: &mut [Vec<Point2>], factor: f64, tolerance: f64) {
    let mut smoother = Smoother::new(factor, tolerance);
    if !smoother.enabled() {
        return;
    }
    for line in lines.iter_mut().filter(|line| line.len() >= 3) {
        let mut smoothed: Vec<Point2> = Vec::with_capacity(2 * line.len());
        let mut emit = |p: Vec2| {
            smoothed.push(Point2 {
                x: (p.0 + 0.5).floor() as Coord,
                y: (p.1 + 0.5).floor() as Coord,
            });
        };
        for p in line.iter() {
            smoother.push((p.x as f64, p.y as f64), &mut emit);
        }
        smoother.flush(&mut emit);
        smoothed.dedup();
        *line = smoothed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_right_angle_is_rounded_within_its_legs_and_the_ends_stay() {
        let mut lines = vec![vec![
            Point2 { x: 0, y: 0 },
            Point2 { x: 1_000_000, y: 0 },
            Point2 {
                x: 1_000_000,
                y: 1_000_000,
            },
        ]];
        let sharp = lines.clone();
        smooth_lines(&mut lines, 0.5, 12_000.0);
        let line = &lines[0];
        assert!(line.len() > 4, "{line:?}");
        assert_eq!(line[0], sharp[0][0]);
        assert_eq!(line[line.len() - 1], sharp[0][2]);
        // The curve leaves the first leg a quarter of it before the
        // corner (half the factor of the shorter leg) and keeps inside
        // the turn.
        assert_eq!(line[1].y, 0);
        assert!(line[1].x >= 750_000 - 1 && line[1].x < 1_000_000);
        for p in &line[1..line.len() - 1] {
            assert!(p.x <= 1_000_000 && p.y >= 0 && (p.x, p.y) != (1_000_000, 0));
        }
        // Each vertex turns by the same step.
        let turns: Vec<f64> = line
            .windows(3)
            .map(|w| {
                let a = ((w[1].x - w[0].x) as f64).atan2((w[1].y - w[0].y) as f64);
                let b = ((w[2].x - w[1].x) as f64).atan2((w[2].y - w[1].y) as f64);
                (a - b).abs()
            })
            .collect();
        let mean = turns.iter().sum::<f64>() / turns.len() as f64;
        assert!(
            turns.iter().all(|turn| (turn - mean).abs() < 0.02),
            "{turns:?}"
        );
    }

    #[test]
    fn nothing_is_rounded_without_a_factor_or_at_a_hairpin() {
        let line = vec![
            Point2 { x: 0, y: 0 },
            Point2 { x: 1_000_000, y: 0 },
            Point2 { x: 0, y: 100_000 },
        ];
        let mut lines = vec![line.clone()];
        smooth_lines(&mut lines, 0.0, 12_000.0);
        assert_eq!(lines[0], line);
        smooth_lines(&mut lines, 1.0, 12_000.0);
        assert_eq!(lines[0], line);
        assert_eq!(sanitize(f64::NAN), 0.0);
        assert_eq!(sanitize(3.0), 1.0);
    }
}
