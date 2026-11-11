//! Lines of changing width as extrusions: a G-code move has one width, so
//! a line is cut where its width changes by more than a twentieth of a
//! millimetre (VariableWidth.cpp).

use std::f64::consts::PI;

use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Point2, UNITS_PER_MM, scaled};

use crate::{EPSILON, ThickPolyline, as_f64, distance};

#[derive(Clone, Copy)]
struct ThickLine {
    a: Point2,
    b: Point2,
    a_width: f64,
    b_width: f64,
}

impl ThickLine {
    fn length(&self) -> f64 {
        distance(as_f64(self.a), as_f64(self.b))
    }
}

fn thick_lines(line: &ThickPolyline) -> Vec<ThickLine> {
    line.points
        .windows(2)
        .enumerate()
        .map(|(index, pair)| ThickLine {
            a: pair[0],
            b: pair[1],
            a_width: line.widths[2 * index],
            b_width: line.widths[2 * index + 1],
        })
        .collect()
}

/// A path along `points` as wide as the gap is, `width` being the gap's
/// width in scaled units: the line's own width is that plus what its
/// rounded edges leave unfilled. None when no line is that thin.
fn path(points: Vec<Point2>, width: f64, role: Role, flow: &Flow) -> Option<ExtrusionPath> {
    let flow = flow
        .with_width(width / UNITS_PER_MM + flow.height * (1.0 - 0.25 * PI))
        .ok()?;
    (points.len() >= 2).then_some(ExtrusionPath {
        points,
        role,
        flow,
        in_turn: false,
        part: 0,
    })
}

/// A closed line is one path at its average width
/// (`closed_gap_fill_loop_to_extrusion_paths`).
fn closed(line: &ThickPolyline, role: Role, flow: &Flow) -> Option<ExtrusionPath> {
    let (mut total, mut weighted) = (0.0, 0.0);
    for part in thick_lines(line) {
        let length = part.length();
        if length >= EPSILON {
            total += length;
            weighted += length * 0.5 * (part.a_width + part.b_width);
        }
    }
    if total <= EPSILON {
        return None;
    }
    path(line.points.clone(), weighted / total, role, flow)
}

/// An open line, cut wherever its width has drifted more than `tolerance`
/// (`thick_polyline_to_extrusion_paths_2`).
fn open(line: &ThickPolyline, role: Role, flow: &Flow, tolerance: f64) -> Vec<ExtrusionPath> {
    let mut paths = Vec::new();
    let mut lines = thick_lines(line);
    let mut start = 0;
    let (mut widest, mut thinnest) = (0.0_f64, 0.0_f64);
    let mut index = 0;
    while index < lines.len() {
        let part = lines[index];
        if index == 0 {
            (widest, thinnest) = (part.a_width, part.a_width);
        }
        let length = part.length();
        if length < EPSILON {
            index += 1;
            continue;
        }
        let drift = (widest - part.b_width)
            .abs()
            .max((thinnest - part.b_width).abs());
        if drift > tolerance {
            // What came before is one path, at its average width.
            if start != index {
                let (mut total, mut weighted) = (0.0, 0.0);
                let mut points = Vec::with_capacity(index - start + 1);
                for before in &lines[start..index] {
                    let length = before.length();
                    total += length;
                    weighted += length * 0.5 * (before.a_width + before.b_width);
                    points.push(before.a);
                }
                points.push(part.a);
                if total > EPSILON {
                    paths.extend(path(points, weighted / total, role, flow));
                }
            }
            start = index;
            (widest, thinnest) = (part.a_width, part.a_width);
            // A segment that itself changes too much is cut into steps.
            let change = (part.a_width - part.b_width).abs();
            if change > tolerance {
                let steps = (change / tolerance).ceil() as usize;
                let at = |step: usize| {
                    let along = step as f64 / steps as f64;
                    let point = if step == steps {
                        part.b
                    } else {
                        Point2 {
                            x: part.a.x + ((part.b.x - part.a.x) as f64 * along) as i64,
                            y: part.a.y + ((part.b.y - part.a.y) as f64 * along) as i64,
                        }
                    };
                    (point, part.a_width + along * (part.b_width - part.a_width))
                };
                let pieces: Vec<ThickLine> = (0..steps)
                    .map(|step| {
                        let ((a, a_width), (b, b_width)) = (at(step), at(step + 1));
                        ThickLine {
                            a,
                            b,
                            a_width,
                            b_width,
                        }
                    })
                    .collect();
                lines.splice(index..=index, pieces);
                // The first step is looked at again, as a segment of its own.
                continue;
            }
        } else {
            widest = widest.max(part.a_width).max(part.b_width);
            thinnest = thinnest.min(part.a_width).min(part.b_width);
        }
        index += 1;
    }
    if start < lines.len() {
        let (mut total, mut weighted) = (0.0, 0.0);
        let mut points = Vec::with_capacity(lines.len() - start + 1);
        for rest in &lines[start..] {
            let length = rest.length();
            total += length;
            weighted += length * rest.a_width;
            points.push(rest.a);
        }
        points.extend(lines.last().map(|last| last.b));
        if total > EPSILON {
            paths.extend(path(points, weighted / total, role, flow));
        }
    }
    paths
}

/// Whether two closed lines trace the same loop to within a twentieth of
/// a millimetre (`are_near_duplicate_closed_gap_fill_loops`).
fn same_loop(a: &ThickPolyline, b: &ThickPolyline) -> bool {
    if !a.is_closed() || !b.is_closed() {
        return false;
    }
    let (a, b) = (
        &a.points[..a.points.len() - 1],
        &b.points[..b.points.len() - 1],
    );
    if a.len() < 3 || a.len() != b.len() {
        return false;
    }
    let near = scaled(0.05) as f64;
    let close = |p: Point2, q: Point2| distance(as_f64(p), as_f64(q)) <= near;
    let count = a.len();
    (0..count).any(|shift| {
        (0..count).all(|index| close(a[index], b[(shift + index) % count]))
            || (0..count).all(|index| close(a[index], b[(shift + count - index) % count]))
    })
}

/// One line as extrusions end to end, cut wherever its width has drifted
/// by more than a twentieth of a millimetre, whether or not it comes
/// round to where it began: what a wall of changing width is laid as.
pub fn varying_width(line: &ThickPolyline, role: Role, flow: &Flow) -> Vec<ExtrusionPath> {
    if line.points.len() < 2 {
        return Vec::new();
    }
    open(line, role, flow, scaled(0.05) as f64)
}

/// Extrusions for a set of centre lines, in the order given. `flow` gives
/// the layer height and nozzle; each path takes its width from the line.
pub fn variable_width(lines: &[ThickPolyline], role: Role, flow: &Flow) -> Vec<ExtrusionPath> {
    let tolerance = scaled(0.05) as f64;
    let mut kept: Vec<&ThickPolyline> = Vec::with_capacity(lines.len());
    let mut paths = Vec::new();
    for line in lines {
        if line.points.len() < 2 || kept.iter().any(|earlier| same_loop(line, earlier)) {
            continue;
        }
        kept.push(line);
        let whole = line.is_closed().then(|| closed(line, role, flow)).flatten();
        match whole {
            Some(path) => paths.push(path),
            None => paths.extend(open(line, role, flow, tolerance)),
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[(f64, f64)], widths: &[f64]) -> ThickPolyline {
        ThickPolyline {
            points: points
                .iter()
                .map(|(x, y)| Point2::from_mm(*x, *y))
                .collect(),
            widths: widths.iter().map(|width| width * UNITS_PER_MM).collect(),
            open_ends: (true, true),
        }
    }

    #[test]
    fn an_even_line_is_one_path_filling_its_gap() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let even = line(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)], &[0.3; 4]);
        let paths = variable_width(&[even], Role::GapFill, &flow);
        let [path] = paths.as_slice() else {
            panic!("expected one path, got {}", paths.len())
        };
        assert_eq!(path.points.len(), 3);
        // A 0.3 mm gap takes 0.3 x 0.2 mm² of plastic along it.
        assert!((path.flow.mm3_per_mm() - 0.3 * 0.2).abs() < 1e-9);
        assert!((path.flow.spacing - 0.3).abs() < 1e-9);
    }

    #[test]
    fn a_tapering_line_is_cut_into_steps() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let taper = line(&[(0.0, 0.0), (10.0, 0.0)], &[0.2, 0.6]);
        let paths = variable_width(&[taper], Role::GapFill, &flow);
        // 0.4 mm of change in steps of 0.05 mm.
        assert_eq!(paths.len(), 8);
        let widths: Vec<f64> = paths.iter().map(|path| path.flow.spacing).collect();
        assert!(widths.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(widths[0] >= 0.2 && widths[7] <= 0.6);
        // End to end with no breaks.
        assert!(
            paths
                .windows(2)
                .all(|pair| pair[0].points.last() == pair[1].points.first())
        );
    }

    #[test]
    fn a_loop_traced_twice_is_printed_once() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let square = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (0.0, 0.0)];
        let mut once = line(&square, &[0.3; 8]);
        once.open_ends = (false, false);
        let mut again = line(
            &[(4.0, 4.0), (0.0, 4.0), (0.0, 0.0), (4.0, 0.0), (4.0, 4.0)],
            &[0.3; 8],
        );
        again.open_ends = (false, false);
        let paths = variable_width(&[once, again], Role::GapFill, &flow);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].points.len(), 5);
    }
}
