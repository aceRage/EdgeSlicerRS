//! A branch's walls (SupportCommon.cpp `tree_supports_generate_paths`):
//! hollow loops, one or two, the innermost started from a hook laid
//! inside it so that it has something to start on.

use edge_clip::Join;
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2};

use crate::SupportError;

fn mm(a: Point2, b: Point2) -> f64 {
    edge_geom::unscaled(b.x - a.x).hypot(edge_geom::unscaled(b.y - a.y))
}

/// The point `share` of the way from `from` to `to`.
fn toward(from: Point2, to: Point2, share: f64) -> Point2 {
    Point2 {
        x: from.x + ((to.x - from.x) as f64 * share).round() as i64,
        y: from.y + ((to.y - from.y) as f64 * share).round() as i64,
    }
}

/// A wall's start is kept this much of a line's spacing short of its
/// end, and its hook is this many spacings long
/// (SupportCommon.cpp `tree_supports_generate_paths`).
const SEAM_CLIP: f64 = 0.15;
const HOOK_SPACINGS: f64 = 6.0;

/// One wall round each island, and a second inside it when `double`. With
/// `hooked`, the innermost starts from a hook laid inside it.
pub(crate) fn walls(
    islands: &[ExPolygon],
    flow: &Flow,
    double: bool,
    hooked: bool,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    let innermost = if hooked { Start::Hooked } else { Start::Whole };
    if double {
        // The outer of two is cut short of its start all the same.
        let outer = if hooked { Start::Short } else { Start::Whole };
        loops(islands, flow.scaled_width() * 3 / 2, flow, innermost, paths)?;
        return loops(islands, flow.scaled_width() / 2, flow, outer, paths);
    }
    loops(islands, flow.scaled_width() / 2, flow, innermost, paths)
}

/// How a wall begins.
#[derive(Clone, Copy, PartialEq)]
enum Start {
    /// A closed loop.
    Whole,
    /// Cut a little short of where it began.
    Short,
    /// Cut short, and begun from a hook laid inside it.
    Hooked,
}

fn clockwise(points: &mut [Point2]) {
    let doubled: i128 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| a.x as i128 * b.y as i128 - b.x as i128 * a.y as i128)
        .sum();
    if doubled > 0 {
        points.reverse();
    }
}

/// The outlines of some islands, `inset` in from their edges. Whole, each
/// is a closed path. Hooked, each is cut short of its start and begins
/// with a hook: from a little way round the next outline in, across to
/// the wall, so the wall does not start on nothing.
fn loops(
    islands: &[ExPolygon],
    inset: i64,
    flow: &Flow,
    start: Start,
    paths: &mut Vec<ExtrusionPath>,
) -> Result<(), SupportError> {
    let hooked = start == Start::Hooked;
    for island in edge_clip::offset(islands, -inset, Join::MITER)? {
        // Where a hook may lie: one line's width further in.
        let mut within: Vec<Vec<Point2>> = Vec::new();
        if hooked {
            for inner in edge_clip::offset(
                std::slice::from_ref(&island),
                -flow.scaled_width(),
                Join::MITER,
            )? {
                for ring in inner.rings() {
                    if ring.points.len() >= 3 {
                        let mut points = ring.points.clone();
                        clockwise(&mut points);
                        within.push(points);
                    }
                }
            }
        }
        for ring in island.rings() {
            if ring.points.len() < 3 {
                continue;
            }
            let mut points = ring.points.clone();
            if start == Start::Whole {
                points.push(points[0]);
                paths.push(ExtrusionPath {
                    points,
                    role: Role::Support,
                    flow: *flow,
                    in_turn: false,
                    part: 0,
                });
                continue;
            }
            // Clockwise, and short of its own start.
            clockwise(&mut points);
            points.push(points[0]);
            let mut spare = SEAM_CLIP * flow.spacing;
            while points.len() >= 2 {
                let (end, before) = (points[points.len() - 1], points[points.len() - 2]);
                let length = mm(end, before);
                if length > spare {
                    let last = points.len() - 1;
                    points[last] = toward(end, before, spare / length);
                    break;
                }
                spare -= length;
                points.pop();
            }
            if points.len() < 2 {
                continue;
            }
            // The hook starts at the nearest place on the next outline
            // in, if that is within three lines' width, and runs on
            // round it.
            let seam = points[points.len() - 1];
            let mut nearest: Option<(f64, usize, usize, Point2)> = None;
            for (which, inner) in within.iter().enumerate() {
                for index in 0..inner.len() {
                    let (a, b) = (inner[index], inner[(index + 1) % inner.len()]);
                    let (vx, vy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
                    let squared = vx * vx + vy * vy;
                    let share = if squared == 0.0 {
                        0.0
                    } else {
                        (((seam.x - a.x) as f64 * vx + (seam.y - a.y) as f64 * vy) / squared)
                            .clamp(0.0, 1.0)
                    };
                    let foot = toward(a, b, share);
                    let distance = mm(seam, foot);
                    if nearest.is_none_or(|(least, ..)| distance < least) {
                        nearest = Some((distance, which, index, foot));
                    }
                }
            }
            if let Some((distance, which, index, foot)) = nearest
                && distance < 3.0 * flow.width
            {
                let inner = &within[which];
                points.push(foot);
                let mut left = HOOK_SPACINGS * flow.spacing - distance;
                let mut from = foot;
                for step in 1..=inner.len() {
                    if left <= 0.0 {
                        break;
                    }
                    let to = inner[(index + step) % inner.len()];
                    let length = mm(from, to);
                    if length >= left {
                        points.push(toward(from, to, left / length));
                        break;
                    }
                    points.push(to);
                    left -= length;
                    from = to;
                }
            }
            // The hook first, then the wall.
            points.reverse();
            paths.push(ExtrusionPath {
                points,
                role: Role::Support,
                flow: *flow,
                in_turn: false,
                part: 0,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_geom::{Polygon, unscaled};

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    #[test]
    fn a_branch_wall_starts_from_a_hook_inside_it() {
        let flow = Flow::new(0.42, 0.2, 0.4).unwrap();
        let island = [square(0.0, 0.0, 6.0, 6.0)];
        let length = |path: &ExtrusionPath| path.length_mm();
        // Plain, a closed loop half a line in from the edge.
        let mut plain = Vec::new();
        walls(&island, &flow, false, false, &mut plain).unwrap();
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].points.first(), plain[0].points.last());
        assert!((length(&plain[0]) - 4.0 * 5.58).abs() < 1e-6);
        // Hooked, it stops 0.15 of a spacing short of its start, and
        // before it comes six spacings of hook, the first of them the
        // step across from the next outline in.
        let mut hooked = Vec::new();
        walls(&island, &flow, false, true, &mut hooked).unwrap();
        assert_eq!(hooked.len(), 1);
        let path = &hooked[0];
        let wanted = 4.0 * 5.58 - 0.15 * flow.spacing + 6.0 * flow.spacing;
        assert!(
            (length(path) - wanted).abs() < 1e-3,
            "{} for {wanted}",
            length(path)
        );
        // It starts inside the loop, a line's width in or more.
        let start = path.points[0];
        let (x, y) = (unscaled(start.x), unscaled(start.y));
        assert!(
            (0.62..=5.38).contains(&x) && (0.62..=5.38).contains(&y),
            "{x} x {y}"
        );
        // And ends on the loop, beside where the loop began.
        let end = path.points[path.points.len() - 1];
        let on_loop = |p: Point2| {
            let (x, y) = (unscaled(p.x), unscaled(p.y));
            [x - 0.21, 5.79 - x, y - 0.21, 5.79 - y]
                .iter()
                .any(|gap| gap.abs() < 1e-6)
        };
        assert!(on_loop(end));
        let hook = path
            .points
            .iter()
            .position(|p| on_loop(*p))
            .expect("the wall follows the hook");
        assert!(mm(path.points[hook], end) < 0.15 * flow.spacing + 1e-6);
        // With two walls the hook goes with the inner one, and the
        // outer is cut short of its start all the same.
        let mut two = Vec::new();
        walls(&island, &flow, true, true, &mut two).unwrap();
        assert_eq!(two.len(), 2);
        assert!(length(&two[0]) < length(&two[1]));
        assert!((length(&two[1]) - (4.0 * 5.58 - 0.15 * flow.spacing)).abs() < 1e-3);
        // An island too small for anything inside it gets its wall and no hook.
        let mut small = Vec::new();
        walls(
            &[square(0.0, 0.0, 1.0, 1.0)],
            &flow,
            false,
            true,
            &mut small,
        )
        .unwrap();
        assert_eq!(small.len(), 1);
        assert!((length(&small[0]) - (4.0 * 0.58 - 0.15 * flow.spacing)).abs() < 1e-3);
    }
}
