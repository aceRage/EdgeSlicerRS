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
    /// Anywhere along the loop, a different place on every layer.
    Random,
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

/// How far the loop turns at one of its corners, in radians, positive to
/// the left: as [`choose`] measures it.
pub fn turn_at(points: &[Point2], corner: usize, nozzle_diameter: f64) -> f64 {
    turns(points, nozzle_diameter)
        .get(corner)
        .copied()
        .unwrap_or(0.0)
}

/// A point somewhere along a closed loop, by a figure made from the
/// loop's first point and the height `z` of the layer, in mm: the same
/// for the same loop, and scattered from layer to layer (SeamPlacer.cpp
/// `pick_random_seam_point`, in single precision as it is there).
///
/// Not as the reference in one thing: it picks among the points it
/// counts as equally good, which leaves out those over a steep overhang
/// or painted against; here any point of the loop may be picked.
pub fn random_point(points: &[Point2], z: f64) -> Point2 {
    let Some(&first) = points.first() else {
        return Point2 { x: 0, y: 0 };
    };
    let at = |point: Point2| (unscaled(point.x) as f32, unscaled(point.y) as f32);
    let (x, y) = at(first);
    let mut rand = ((x * 12.9898 + y * 78.233 + z as f32 * 133.3333).sin() * 43_758.547).abs();
    rand -= rand.trunc();
    let count = points.len();
    let edges: Vec<(f32, f32, f32)> = (0..count)
        .map(|index| {
            let (ax, ay) = at(points[index]);
            let (bx, by) = at(points[(index + 1) % count]);
            let (dx, dy) = (bx - ax, by - ay);
            (dx, dy, dx.hypot(dy))
        })
        .collect();
    let mut picked = edges.iter().map(|edge| edge.2).sum::<f32>() * rand;
    let mut index = 0;
    while index + 1 < count && picked - edges[index].2 > 0.0 {
        picked -= edges[index].2;
        index += 1;
    }
    let (dx, dy, length) = edges[index];
    if length <= 0.0 {
        return points[index];
    }
    let (ax, ay) = at(points[index]);
    Point2::from_mm(
        f64::from(ax + dx / length * picked),
        f64::from(ay + dy / length * picked),
    )
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

    #[test]
    fn a_random_seam_is_on_the_loop_and_moves_from_layer_to_layer() {
        let square: Vec<Point2> = [(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        let on_loop = |point: Point2| {
            let (x, y) = (unscaled(point.x), unscaled(point.y));
            let edge = |v: f64| v.abs() < 1e-3 || (v - 20.0).abs() < 1e-3;
            (edge(x) && (-1e-3..=20.001).contains(&y)) || (edge(y) && (-1e-3..=20.001).contains(&x))
        };
        let picked: Vec<Point2> = (0..40)
            .map(|layer| random_point(&square, 0.1 + 0.2 * f64::from(layer)))
            .collect();
        assert!(picked.iter().all(|&point| on_loop(point)));
        // The same for the same loop and height; not all in one place.
        assert_eq!(random_point(&square, 0.5), random_point(&square, 0.5));
        let mut distinct = picked.clone();
        distinct.sort_by_key(|point| (point.x, point.y));
        distinct.dedup();
        assert!(distinct.len() > 30, "{}", distinct.len());
        // Spread over all four sides.
        for side in [
            |p: &Point2| p.y < 1_000,
            |p: &Point2| p.x > 19_999_000,
            |p: &Point2| p.y > 19_999_000,
            |p: &Point2| p.x < 1_000,
        ] {
            assert!(picked.iter().any(side));
        }
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
