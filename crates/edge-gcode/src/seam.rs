//! Where a wall loop starts and ends: its seam.
//!
//! The reference's SeamPlacer scores every corner of the outer wall by how
//! visible it is from outside (by casting rays at the model), by its angle
//! and by overhang, then lines seams up between layers along a curve. This
//! keeps the angle score and its weights and replaces the rest with
//! something much simpler: among corners that score about the same, take
//! the one nearest the seam on the layer below. Visibility and overhangs
//! are not looked at.

use edge_geom::{Point2, unscaled};

/// The `seam_position` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seam {
    /// The best corner near where the nozzle already is.
    Nearest,
    /// The best corner, kept in line from layer to layer.
    Aligned,
    /// As `Aligned`, preferring the back (or front) among equals.
    AlignedBack,
    AlignedFront,
    /// The corner furthest back (highest Y), or furthest to one side.
    Back,
    Left,
    Right,
}

/// Corners whose scores are this close count as equally good.
const SIMILAR: f64 = 0.05;

/// The reference's bell curve (SeamPlacer.cpp `gauss`): `peak` at `mean`,
/// falling off faster the larger `falloff` is.
fn gauss(value: f64, mean: f64, peak: f64, falloff: f64) -> f64 {
    let offset = value - mean;
    peak * ((1.0 / (falloff * offset * offset + 1.0)).exp() - 1.0) / (std::f64::consts::E - 1.0)
}

/// How bad a corner is as a seam, by the angle the wall turns through
/// there: negative for a corner that folds inward, where a seam hides best,
/// zero on a straight run, where it shows most.
pub fn angle_penalty(turn: f64) -> f64 {
    gauss(turn, 0.0, 1.0, 3.0) + 1.0 / (2.0 + (-turn).exp())
}

fn mm(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// The angle the loop turns through at each corner, in radians, positive
/// to the left. Measured between points at least `arm` mm either side, so
/// a curve cut into many short edges still reads as a curve.
fn turns(points: &[Point2], arm: f64) -> Vec<f64> {
    let count = points.len();
    let reach = |from: usize, step: usize| {
        let mut at = from;
        for _ in 1..count {
            at = (at + step) % count;
            if mm(points[from], points[at]) >= arm {
                break;
            }
        }
        points[at]
    };
    (0..count)
        .map(|index| {
            let (before, here, after) = (reach(index, count - 1), points[index], reach(index, 1));
            let (ax, ay) = (unscaled(here.x - before.x), unscaled(here.y - before.y));
            let (bx, by) = (unscaled(after.x - here.x), unscaled(after.y - here.y));
            (ax * by - ay * bx).atan2(ax * bx + ay * by)
        })
        .collect()
}

/// The corner of a loop to start at. `points` run counter-clockwise around
/// material and clockwise around a hole, which makes a corner that folds
/// into the part a right turn either way. `nozzle` is where the nozzle is
/// and `previous` the seams of the layer below.
pub fn choose(
    points: &[Point2],
    seam: Seam,
    nozzle_diameter: f64,
    nozzle: Option<Point2>,
    previous: &[Point2],
) -> usize {
    if points.len() < 3 {
        return 0;
    }
    let extreme = |key: fn(&Point2) -> i64| {
        (0..points.len())
            .max_by_key(|&index| (key(&points[index]), std::cmp::Reverse(index)))
            .unwrap_or(0)
    };
    match seam {
        Seam::Back => return extreme(|point| point.y),
        Seam::Left => return extreme(|point| -point.x),
        Seam::Right => return extreme(|point| point.x),
        _ => {}
    }

    let turns = turns(points, nozzle_diameter);
    // The angle counts for everything when the seam goes to the nearest
    // corner and for 0.6 when it is aligned (SeamPlacer.hpp).
    let weight = if seam == Seam::Nearest { 1.0 } else { 0.6 };
    let scores: Vec<f64> = (0..points.len())
        .map(|index| {
            let distance = match (seam, nozzle) {
                (Seam::Nearest, Some(at)) => 1.0 - gauss(mm(at, points[index]), 0.0, 1.0, 0.005),
                _ => 0.0,
            };
            weight * angle_penalty(turns[index]) + distance
        })
        .collect();
    let best = scores.iter().copied().fold(f64::INFINITY, f64::min);
    let lowest = |key: &dyn Fn(usize) -> f64| {
        (0..points.len())
            .filter(|&index| scores[index] <= best + SIMILAR)
            .min_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)))
            .unwrap_or(0)
    };
    match seam {
        Seam::Nearest => lowest(&|index| scores[index]),
        Seam::AlignedBack => lowest(&|index| -(points[index].y as f64)),
        Seam::AlignedFront => lowest(&|index| points[index].y as f64),
        _ if previous.is_empty() => lowest(&|index| scores[index]),
        _ => lowest(&|index| {
            previous
                .iter()
                .map(|seam| mm(*seam, points[index]))
                .fold(f64::INFINITY, f64::min)
        }),
    }
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

    /// An L: the corner at (5, 5) folds inward, the other five outward.
    fn ell() -> Vec<Point2> {
        points(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (5.0, 5.0),
            (5.0, 10.0),
            (0.0, 10.0),
        ])
    }

    #[test]
    fn inward_corners_score_best_and_straight_runs_worst() {
        let quarter = std::f64::consts::FRAC_PI_2;
        assert!(angle_penalty(-quarter) < angle_penalty(quarter));
        assert!(angle_penalty(quarter) < angle_penalty(0.0));
        assert!((angle_penalty(0.0) - (1.0 + 1.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn the_seam_hides_in_the_inward_corner() {
        for seam in [Seam::Aligned, Seam::AlignedBack, Seam::Nearest] {
            assert_eq!(choose(&ell(), seam, 0.4, None, &[]), 3, "{seam:?}");
        }
        // A hole runs clockwise; its corners all fold into the part, so
        // with nothing to choose between them the first is taken.
        let hole = points(&[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]);
        assert_eq!(choose(&hole, Seam::Aligned, 0.4, None, &[]), 0);
    }

    #[test]
    fn equal_corners_follow_the_layer_below_or_the_named_side() {
        let square = points(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        assert_eq!(choose(&square, Seam::Aligned, 0.4, None, &[]), 0);
        let below = points(&[(50.0, 50.0), (10.1, 9.9)]);
        assert_eq!(choose(&square, Seam::Aligned, 0.4, None, &below), 2);
        // Back is the highest Y; of two equally far back, the earlier corner.
        assert_eq!(choose(&square, Seam::AlignedBack, 0.4, None, &below), 2);
        assert_eq!(choose(&square, Seam::Back, 0.4, None, &[]), 2);
        assert_eq!(choose(&square, Seam::AlignedFront, 0.4, None, &below), 0);
        assert_eq!(choose(&square, Seam::Right, 0.4, None, &[]), 1);
        assert_eq!(choose(&square, Seam::Left, 0.4, None, &[]), 0);
        // Nearest goes to the corner by the nozzle.
        let at = Some(Point2::from_mm(-1.0, 11.0));
        assert_eq!(choose(&square, Seam::Nearest, 0.4, at, &[]), 3);
        // The directional seams ignore the angle: on the L the back wins
        // over the inward corner.
        assert_eq!(choose(&ell(), Seam::Back, 0.4, None, &[]), 4);
    }

    #[test]
    fn a_round_wall_keeps_its_seam_where_it_was() {
        let circle = |turn: f64| -> Vec<Point2> {
            (0..90)
                .map(|step| {
                    let angle = turn + f64::from(step) * std::f64::consts::TAU / 90.0;
                    Point2::from_mm(20.0 * angle.cos(), 20.0 * angle.sin())
                })
                .collect()
        };
        let first = circle(0.0);
        let seam = first[choose(&first, Seam::Aligned, 0.4, None, &[])];
        // The layer above has its corners elsewhere; the seam stays close.
        let second = circle(0.3);
        let above = second[choose(&second, Seam::Aligned, 0.4, None, &[seam])];
        assert!(mm(seam, above) < 0.75, "moved {} mm", mm(seam, above));
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
}
