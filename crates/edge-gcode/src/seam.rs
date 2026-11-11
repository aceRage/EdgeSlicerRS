//! Where a wall loop starts and ends: its seam.
//!
//! Which point of a wall its seam goes to is settled in `seam_plan.rs`,
//! as the reference's SeamPlacer settles it. Here are the setting, and
//! what is done with a loop once its seam is known: opening it there,
//! moving a start on round it, and stopping it short of where it began.

use edge_geom::{Point2, unscaled};

/// The `seam_position` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seam {
    /// The best corner near where the nozzle already is.
    Nearest,
    /// The best corner, kept in line from layer to layer.
    Aligned,
    /// As `Aligned`, leaning to the back, the front, the left or the
    /// right of the bed: what faces the other way counts as easier seen.
    AlignedBack,
    AlignedFront,
    Left,
    Right,
    /// The point furthest back (highest Y) that is fit for a seam.
    Back,
    /// Anywhere along the loop, a different place on every layer.
    Random,
}

fn mm(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// The place `distance` scaled units further round a closed loop from
/// `start`, which lies on the edge leaving corner `edge`: forward in the
/// order of `points`, or `backward`. The answer is given as [`project`]
/// gives one, a point and the corner its edge leaves from.
pub fn advance(
    points: &[Point2],
    edge: usize,
    start: Point2,
    distance: f64,
    backward: bool,
) -> (usize, Point2) {
    let count = points.len();
    let apart = |a: Point2, b: Point2| ((b.x - a.x) as f64).hypot((b.y - a.y) as f64);
    let (mut edge, mut at, mut left) = (edge, start, distance);
    // No further than once round.
    for _ in 0..=count {
        if left <= 0.0 {
            break;
        }
        let next = if backward {
            points[edge]
        } else {
            points[(edge + 1) % count]
        };
        let step = apart(at, next);
        if step > left {
            let share = left / step;
            at = Point2 {
                x: at.x + ((next.x - at.x) as f64 * share).round() as i64,
                y: at.y + ((next.y - at.y) as f64 * share).round() as i64,
            };
            break;
        }
        left -= step;
        at = next;
        edge = if backward {
            (edge + count - 1) % count
        } else {
            (edge + 1) % count
        };
    }
    (edge, at)
}

/// The point of a closed loop nearest `target`, and the index of the corner
/// its edge starts from.
pub fn project(points: &[Point2], target: Point2) -> (usize, Point2) {
    let mut best = (0, points[0], f64::INFINITY);
    for index in 0..points.len() {
        let (a, b) = (points[index], points[(index + 1) % points.len()]);
        let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
        let along = if dx == 0.0 && dy == 0.0 {
            0.0
        } else {
            (((target.x - a.x) as f64 * dx + (target.y - a.y) as f64 * dy) / (dx * dx + dy * dy))
                .clamp(0.0, 1.0)
        };
        let point = Point2 {
            x: a.x + (along * dx).round() as i64,
            y: a.y + (along * dy).round() as i64,
        };
        let distance = mm(point, target);
        if distance < best.2 {
            best = (index, point, distance);
        }
    }
    (best.0, best.1)
}

/// The loop as a path once round from `start`, which lies on the edge
/// leaving corner `edge`: it begins and ends at `start`.
pub fn open_at(points: &[Point2], edge: usize, start: Point2) -> Vec<Point2> {
    let count = points.len();
    // A start within 1.5 µm of a corner is that corner, as in the reference.
    let near = |corner: Point2| mm(corner, start) < 0.0015;
    let first = if near(points[edge]) {
        edge
    } else {
        (edge + 1) % count
    };
    let mut path = Vec::with_capacity(count + 2);
    if !near(points[first]) {
        path.push(start);
    }
    path.extend((0..count).map(|step| points[(first + step) % count]));
    path.push(path[0]);
    path
}

/// A loop's width at each point of the path [`open_at`] makes of it:
/// `widths` holds the width at each of `points`, and the width at
/// `start` is that of its place along its edge.
pub fn open_widths_at(points: &[Point2], widths: &[f64], edge: usize, start: Point2) -> Vec<f64> {
    let count = points.len();
    let near = |corner: Point2| mm(corner, start) < 0.0015;
    let first = if near(points[edge]) {
        edge
    } else {
        (edge + 1) % count
    };
    let mut found = Vec::with_capacity(count + 2);
    if !near(points[first]) {
        let (a, b) = (points[edge], points[(edge + 1) % count]);
        let whole = mm(a, b);
        let along = if whole > 0.0 {
            mm(a, start) / whole
        } else {
            0.0
        };
        found.push(widths[edge] + along * (widths[(edge + 1) % count] - widths[edge]));
    }
    found.extend((0..count).map(|step| widths[(first + step) % count]));
    found.push(found[0]);
    found
}

/// The widths of a path that [`clip_end`] has shortened to `clipped`
/// from `whole`, whose widths were `widths`.
pub fn clip_widths(whole: &[Point2], widths: &[f64], clipped: &[Point2]) -> Vec<f64> {
    let kept = clipped.len();
    let mut found = widths[..kept.min(widths.len())].to_vec();
    if kept >= 2 && kept <= whole.len() && clipped[kept - 1] != whole[kept - 1] {
        let (a, b) = (whole[kept - 2], whole[kept - 1]);
        let full = mm(a, b);
        let along = if full > 0.0 {
            mm(a, clipped[kept - 1]) / full
        } else {
            0.0
        };
        found[kept - 1] = widths[kept - 2] + along * (widths[kept - 1] - widths[kept - 2]);
    }
    found
}

/// Shortens a path by `length` mm from its end. A path no longer than that
/// is left as its first point.
pub fn clip_end(path: &mut Vec<Point2>, length: f64) {
    let mut left = length;
    while left > 0.0 && path.len() >= 2 {
        let (a, b) = (path[path.len() - 2], path[path.len() - 1]);
        let segment = mm(a, b);
        if segment <= left {
            path.pop();
            left -= segment;
        } else {
            let keep = 1.0 - left / segment;
            let end = path.len() - 1;
            path[end] = Point2 {
                x: a.x + ((b.x - a.x) as f64 * keep).round() as i64,
                y: a.y + ((b.y - a.y) as f64 * keep).round() as i64,
            };
            return;
        }
    }
}

/// The first `length` mm of a path.
pub fn head(path: &[Point2], length: f64) -> Vec<Point2> {
    let mut out = vec![path[0]];
    let mut left = length;
    for pair in path.windows(2) {
        let segment = mm(pair[0], pair[1]);
        if segment == 0.0 {
            continue;
        }
        if segment >= left {
            let share = left / segment;
            out.push(Point2 {
                x: pair[0].x + ((pair[1].x - pair[0].x) as f64 * share).round() as i64,
                y: pair[0].y + ((pair[1].y - pair[0].y) as f64 * share).round() as i64,
            });
            break;
        }
        out.push(pair[1]);
        left -= segment;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(corners: &[(f64, f64)]) -> Vec<Point2> {
        corners
            .iter()
            .map(|&(x, y)| Point2::from_mm(x, y))
            .collect()
    }

    #[test]
    fn a_loop_opens_at_a_point_on_an_edge() {
        let square = points(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let (edge, start) = project(&square, Point2::from_mm(4.0, -3.0));
        assert_eq!((edge, start), (0, Point2::from_mm(4.0, 0.0)));
        let path = open_at(&square, edge, start);
        assert_eq!(
            path,
            points(&[
                (4.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
                (4.0, 0.0)
            ])
        );
        // On a corner, no point is added.
        let (edge, start) = project(&square, Point2::from_mm(11.0, 11.0));
        assert_eq!(start, Point2::from_mm(10.0, 10.0));
        let path = open_at(&square, edge, start);
        assert_eq!((path.len(), path[0], path[4]), (5, start, start));
    }

    #[test]
    fn paths_are_trimmed_from_the_end_and_sampled_from_the_start() {
        let mut path = points(&[(0.0, 0.0), (10.0, 0.0), (10.0, 1.0)]);
        clip_end(&mut path, 0.5);
        assert_eq!(path, points(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.5)]));
        clip_end(&mut path, 1.5);
        assert_eq!(path, points(&[(0.0, 0.0), (9.0, 0.0)]));
        clip_end(&mut path, 100.0);
        assert_eq!(path, points(&[(0.0, 0.0)]));

        let path = points(&[(0.0, 0.0), (1.0, 0.0), (1.0, 5.0)]);
        assert_eq!(
            head(&path, 2.0),
            points(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)])
        );
        assert_eq!(head(&path, 50.0), path);
    }

    #[test]
    fn a_start_can_be_moved_on_round_the_loop_either_way() {
        let square: Vec<Point2> = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        let from = Point2::from_mm(8.0, 0.0);
        // Along the edge it is on.
        assert_eq!(
            advance(&square, 0, from, 1e6, false),
            (0, Point2::from_mm(9.0, 0.0))
        );
        assert_eq!(
            advance(&square, 0, from, 1e6, true),
            (0, Point2::from_mm(7.0, 0.0))
        );
        // Round a corner, forward and back.
        assert_eq!(
            advance(&square, 0, from, 5e6, false),
            (1, Point2::from_mm(10.0, 3.0))
        );
        assert_eq!(
            advance(&square, 0, from, 11e6, true),
            (3, Point2::from_mm(0.0, 3.0))
        );
        // Nowhere, for no distance.
        assert_eq!(advance(&square, 0, from, 0.0, false), (0, from));
        // The loop opened there starts and ends at the place found.
        let (edge, start) = advance(&square, 0, from, 5e6, false);
        let path = open_at(&square, edge, start);
        assert_eq!((path[0], path[path.len() - 1]), (start, start));
        assert_eq!(path.len(), 6);
    }
}
