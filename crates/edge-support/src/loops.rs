//! The loop that edges a normal support's top contact where it faces the
//! overhang (`support_interface_loop_pattern`): a port of the reference's
//! `LoopInterfaceProcessor` (Support/SupportCommon.cpp), which it runs
//! with one loop. Along each outline of the contact that comes near the
//! overhang, small hexagons are set no closer than three of their radii
//! apart and cut out of it, and what is left of the outline near the
//! overhang is laid as interface; the contact's own fill keeps clear of it.

use std::collections::HashMap;

use edge_clip::Join;
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, Polygon, scaled};

use crate::SupportError;

/// How far past the overhang the loop may run (Support/SupportCommon.hpp
/// `SUPPORT_MATERIAL_MARGIN`).
const MARGIN: f64 = 1.5;
/// The reference's `SCALED_EPSILON`.
const SCALED_EPSILON: f64 = 100.0;

/// The loop for a top contact `contact` under `overhang`, laid with
/// `flow`, and the ground the contact's fill keeps off.
pub(crate) fn contact_loops(
    contact: &[ExPolygon],
    overhang: &[ExPolygon],
    flow: &Flow,
) -> Result<(Vec<ExtrusionPath>, Vec<ExPolygon>), SupportError> {
    if contact.is_empty() || overhang.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let width = flow.scaled_width();
    let radius = 1.5 * width as f64;
    let spread = 3.0 * radius;
    // The hexagon each circle stands for.
    let hexagon: Vec<Point2> = (0..6)
        .map(|i| {
            let angle = f64::from(i) * std::f64::consts::PI / 3.0;
            Point2 {
                x: (radius * angle.cos()).round() as i64,
                y: (radius * angle.sin()).round() as i64,
            }
        })
        .collect();
    // Centre line of the outermost loop.
    let inner = edge_clip::offset(&edge_clip::union_ex(contact)?, -width / 2, Join::MITER)?;
    let near = edge_clip::offset(&edge_clip::union_ex(overhang)?, width / 2, Join::MITER)?;
    let mut outlines: Vec<Polygon> = Vec::new();
    let mut circles: Vec<ExPolygon> = Vec::new();
    for region in &inner {
        let mut centres: Vec<Point2> = Vec::new();
        let mut last = Point2 { x: 0, y: 0 };
        for ring in region.rings() {
            let points = &ring.points;
            if points.len() < 2 {
                continue;
            }
            if edge_clip::clip_lines(&[points.clone()], &near)?.is_empty() {
                continue;
            }
            // Where along the ring the walk is: the segment's first point
            // and how far along it.
            let mut at: Option<(usize, f64)> = None;
            if centres.is_empty() {
                at = Some((0, 0.0));
                last = points[0];
                centres.push(last);
            }
            let mut index = 1;
            while index < points.len() {
                let (p1, p2) = (points[index - 1], points[index]);
                let seg = [(p2.x - p1.x) as f64, (p2.y - p1.y) as f64];
                let from = [(p1.x - last.x) as f64, (p1.y - last.y) as f64];
                let a = seg[0] * seg[0] + seg[1] * seg[1];
                let b = 2.0 * (seg[0] * from[0] + seg[1] * from[1]);
                let c = from[0] * from[0] + from[1] * from[1] - spread * spread;
                let disc = b * b - 4.0 * a * c;
                if disc > 0.0 {
                    // Leave out the stretch inside the last circle's reach.
                    let t1 = (-b - disc.sqrt()) / (2.0 * a);
                    let t2 = (-b + disc.sqrt()) / (2.0 * a);
                    let t0 = match at {
                        Some((segment, t)) if segment == index - 1 => t,
                        _ => 0.0,
                    };
                    let t = if t0 <= t1 {
                        t0
                    } else if t2 <= 1.0 {
                        t2
                    } else {
                        at = None;
                        index += 1;
                        continue;
                    };
                    at = Some((index - 1, t));
                    last = Point2 {
                        x: p1.x + (seg[0] * t) as i64,
                        y: p1.y + (seg[1] * t) as i64,
                    };
                    // Too near a circle set already: on along the segment.
                    let crowded = centres.iter().any(|centre| {
                        let (dx, dy) = ((centre.x - last.x) as f64, (centre.y - last.y) as f64);
                        dx.hypot(dy) < spread - SCALED_EPSILON
                    });
                    if crowded {
                        continue;
                    }
                } else {
                    at = Some((index - 1, 0.0));
                    last = p1;
                }
                centres.push(last);
                index += 1;
            }
            outlines.push(ring.clone());
        }
        circles.extend(centres.iter().map(|centre| {
            ExPolygon {
                contour: Polygon {
                    points: hexagon
                        .iter()
                        .map(|p| Point2 {
                            x: p.x + centre.x,
                            y: p.y + centre.y,
                        })
                        .collect(),
                },
                holes: Vec::new(),
            }
        }));
    }
    if outlines.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    // Each outline taken as an area of its own, whichever way round it
    // goes (the reference's nonzero `diff` of bare polygons), the
    // circles cut out of it; every ring left is a loop.
    let mut loops: Vec<Polygon> = Vec::new();
    for outline in outlines {
        let mut contour = outline;
        if !contour.is_ccw() {
            contour.points.reverse();
        }
        let area = [ExPolygon {
            contour,
            holes: Vec::new(),
        }];
        for region in edge_clip::difference(&area, &circles)? {
            loops.extend(region.rings().cloned());
        }
    }
    // Opened at their first points and kept near the overhang; pieces
    // the opening cut apart are joined again.
    let opened: Vec<Vec<Point2>> = loops
        .iter()
        .map(|ring| {
            let mut points = ring.points.clone();
            points.push(ring.points[0]);
            points
        })
        .collect();
    let mut splits: HashMap<Point2, Option<usize>> =
        loops.iter().map(|ring| (ring.points[0], None)).collect();
    let reach = edge_clip::offset(&edge_clip::union_ex(overhang)?, scaled(MARGIN), Join::MITER)?;
    let mut lines = edge_clip::clip_lines(&opened, &reach)?;
    let mut joined = vec![false; lines.len()];
    for index in 0..lines.len() {
        for at_start in [true, false] {
            let end = if at_start {
                lines[index][0]
            } else {
                *lines[index].last().expect("two points")
            };
            let Some(seen) = splits.get_mut(&end) else {
                continue;
            };
            match *seen {
                None => *seen = Some(index),
                Some(first) => {
                    let mut piece = std::mem::take(&mut lines[index]);
                    if !at_start {
                        piece.reverse();
                    }
                    if lines[first][0] == end {
                        lines[first].reverse();
                    }
                    lines[first].extend(piece);
                    joined[index] = true;
                    splits.remove(&end);
                }
            }
            break;
        }
    }
    let lines: Vec<Vec<Point2>> = lines
        .into_iter()
        .zip(joined)
        .filter(|(line, joined)| !joined && line.len() >= 2)
        .map(|(line, _)| line)
        .collect();
    let covered = edge_clip::thicken(lines.iter().map(Vec::as_slice), (1.1 * radius) as i64)?;
    let paths = lines
        .into_iter()
        .map(|points| ExtrusionPath {
            points,
            role: Role::SupportInterface,
            flow: *flow,
            in_turn: false,
            part: 0,
        })
        .collect();
    Ok((paths, covered))
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_geom::unscaled;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    /// The contact under a 30 mm cap on a 10 mm stem: a frame round the
    /// stem. Its loop follows both of its outlines half a line in, with
    /// hexagons cut out of it, and the fill keeps clear of it; under no
    /// overhang there is none.
    #[test]
    fn the_contact_is_edged_where_it_faces_the_overhang() {
        let mut hole = square(0.0, 0.0, 10.0, 10.0);
        hole.points.reverse();
        let frame = vec![ExPolygon {
            contour: square(-10.0, -10.0, 20.0, 20.0),
            holes: vec![hole],
        }];
        let flow = Flow::new(0.42, 0.2, 0.4).unwrap();
        let (loops, covered) = contact_loops(&frame, &frame, &flow).unwrap();
        assert!(!loops.is_empty() && !covered.is_empty());
        let on_outline = |x: f64, y: f64| {
            let near = |v: f64, at: f64| (v - at).abs() < 0.22;
            let between = |v: f64, a: f64, b: f64| v > a - 0.22 && v < b + 0.22;
            let ring = |a: f64, b: f64| {
                ((near(x, a) || near(x, b)) && between(y, a, b))
                    || ((near(y, a) || near(y, b)) && between(x, a, b))
            };
            ring(-0.21, 10.21) || ring(-9.79, 19.79)
        };
        let mut notched = 0;
        let mut length = 0.0;
        for path in &loops {
            assert_eq!(path.role, Role::SupportInterface);
            for pair in path.points.windows(2) {
                length += unscaled(pair[1].x - pair[0].x).hypot(unscaled(pair[1].y - pair[0].y));
            }
            for p in &path.points {
                let (x, y) = (unscaled(p.x), unscaled(p.y));
                if !on_outline(x, y) {
                    notched += 1;
                }
            }
        }
        // Both outlines, less what the hexagons take.
        assert!(length > 0.8 * (4.0 * 10.42 + 4.0 * 29.58), "{length}");
        assert!(notched > 0, "no hexagon cut out");
        // Nothing to hold up: no loop.
        let (none, _) = contact_loops(
            &frame,
            &[ExPolygon {
                contour: square(50.0, 50.0, 60.0, 60.0),
                holes: Vec::new(),
            }],
            &flow,
        )
        .unwrap();
        assert!(none.is_empty());
    }
}
