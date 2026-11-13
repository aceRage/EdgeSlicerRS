//! Each ring of a layer's outline cut into lines of one colour each.

use super::lines::{PaintedLine, Rings, filter_eps};
use super::{Line, dist, scale};

/// A stretch of a ring in one colour, 0 for unpainted (the reference's
/// `ColoredLine`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColouredLine {
    pub line: Line,
    pub colour: u8,
}

/// Each ring's coloured lines (`colorize_contours`). `painted` holds each
/// ring's painted lines, as [`super::lines::post_process`] leaves them.
pub(crate) fn colorize_contours(
    rings: &Rings,
    painted: &[Vec<PaintedLine>],
    run_scale: f32,
) -> Vec<Vec<ColouredLine>> {
    (0..rings.rings.len())
        .map(|ring| colorize_contour(rings, ring, &painted[ring], run_scale))
        .collect()
}

/// One ring's coloured lines: its painted edges cut as their paint
/// says, the rest unpainted, then filtered (`colorize_contour`).
fn colorize_contour(
    rings: &Rings,
    ring: usize,
    painted: &[PaintedLine],
    run_scale: f32,
) -> Vec<ColouredLine> {
    let count = rings.rings[ring].len();
    let bare = |edge: usize| ColouredLine {
        line: rings.segment(ring, edge),
        colour: 0,
    };
    if painted.is_empty() {
        return (0..count).map(bare).collect();
    }
    let mut coloured: Vec<ColouredLine> = (0..painted[0].edge).map(bare).collect();
    let groups: Vec<&[PaintedLine]> = painted.chunk_by(|a, b| a.edge == b.edge).collect();
    for (at, group) in groups.iter().enumerate() {
        coloured.extend(colorize_line(
            &rings.segment(ring, group[0].edge),
            group,
            run_scale,
        ));
        if let Some(next) = groups.get(at + 1) {
            coloured.extend((group[0].edge + 1..next[0].edge).map(bare));
        }
    }
    let last = painted[painted.len() - 1].edge;
    coloured.extend((last + 1..count).map(bare));
    filter_colorized_polygon(coloured, run_scale)
}

/// One edge cut into coloured lines by its painted lines, gaps between
/// them unpainted, then runs of 0.2 mm or less given to their
/// neighbours (`colorize_line`).
fn colorize_line(edge: &Line, painted: &[PaintedLine], run_scale: f32) -> Vec<ColouredLine> {
    let eps = filter_eps(run_scale);
    let short = scale(0.2 * f64::from(run_scale));
    let mut lines: Vec<ColouredLine> = Vec::new();
    let first = &painted[0];
    if dist(first.line.a, edge.a) > eps {
        lines.push(ColouredLine {
            line: Line::new(edge.a, first.line.a),
            colour: 0,
        });
    }
    lines.push(ColouredLine {
        line: first.line,
        colour: first.colour,
    });
    for curr in &painted[1..] {
        let prev = lines.last_mut().expect("one at least");
        if dist(curr.line.a, prev.line.b) <= eps {
            if prev.colour == curr.colour {
                prev.line.b = curr.line.b;
            } else {
                prev.line.b = curr.line.a;
                lines.push(ColouredLine {
                    line: curr.line,
                    colour: curr.colour,
                });
            }
        } else {
            let from = prev.line.b;
            lines.push(ColouredLine {
                line: Line::new(from, curr.line.a),
                colour: 0,
            });
            lines.push(ColouredLine {
                line: curr.line,
                colour: curr.colour,
            });
        }
    }
    let end = lines[lines.len() - 1].line.b;
    if dist(end, edge.b) > eps {
        lines.push(ColouredLine {
            line: Line::new(end, edge.b),
            colour: 0,
        });
    }

    for at in 2..lines.len() {
        let (c0, c2) = (lines[at - 2].colour, lines[at].colour);
        if c0 == c2 && c0 != lines[at - 1].colour && lines[at - 1].line.length() <= short {
            lines[at - 1].colour = c0;
        }
    }

    let mut simple: Vec<ColouredLine> = vec![lines[0]];
    for line in &lines[1..] {
        let last = simple.last_mut().expect("one at least");
        if last.colour == line.colour {
            last.line.b = line.line.b;
        } else {
            simple.push(*line);
        }
    }
    let mut lines = simple;

    if lines.len() > 1 && lines[0].colour != lines[1].colour && lines[0].line.length() <= short {
        lines[1].line.a = lines[0].line.a;
        lines.remove(0);
    }
    let count = lines.len();
    if count > 1
        && lines[count - 1].colour != lines[count - 2].colour
        && lines[count - 1].line.length() <= short
    {
        lines[count - 2].line.b = lines[count - 1].line.b;
        lines.pop();
    }
    lines
}

/// The runs of one colour round a ring, as the first and last line of
/// each, going round from the first change of colour; the whole ring as
/// one when it is all one colour (`get_segments`).
pub(crate) fn get_segments(ring: &[ColouredLine]) -> Vec<(usize, usize)> {
    let count = ring.len();
    let mut end = 0;
    while end + 1 < count && ring[end].colour == ring[end + 1].colour {
        end += 1;
    }
    if end == count - 1 {
        return vec![(0, count - 1)];
    }
    let first_different = (end + 1) % count;
    let mut segments = Vec::new();
    let mut offset = 0;
    while offset < count {
        let start = (first_different + offset) % count;
        let mut stop = start;
        while offset + 1 < count
            && ring[start].colour == ring[(first_different + offset + 1) % count].colour
        {
            stop = (first_different + offset + 1) % count;
            offset += 1;
        }
        segments.push((start, stop));
        offset += 1;
    }
    segments
}

/// Short runs round a ring given to the colour about them
/// (`filter_colorized_polygon`): one or two lines of 0.5 mm or less
/// between two lines of one painted colour, and then a run of 0.2 mm or
/// less after a run of another colour at least 0.1 mm long, and again
/// after any run of a painted colour; last, a run of 0.5 mm or less
/// between two runs of one painted colour.
fn filter_colorized_polygon(mut lines: Vec<ColouredLine>, run_scale: f32) -> Vec<ColouredLine> {
    let half = scale(0.5 * f64::from(run_scale));
    let short = scale(0.2 * f64::from(run_scale));
    let tenth = scale(0.1 * f64::from(run_scale));
    for at in 2..lines.len() {
        let (c0, c1, c2) = (lines[at - 2].colour, lines[at - 1].colour, lines[at].colour);
        if c0 == c2 && c0 != c1 && c0 >= 1 && lines[at - 1].line.length() <= half {
            lines[at - 1].colour = c0;
        }
    }
    for at in 3..lines.len() {
        let (c0, c3) = (lines[at - 3].colour, lines[at].colour);
        if c0 == c3
            && (c0 != lines[at - 2].colour || c0 != lines[at - 1].colour)
            && c0 >= 1
            && c3 >= 1
            && lines[at - 2].line.length() + lines[at - 1].line.length() <= half
        {
            lines[at - 2].colour = c0;
            lines[at - 1].colour = c0;
        }
    }

    let count = lines.len();
    let length = |lines: &[ColouredLine], (first, last): (usize, usize)| {
        let mut total = 0.0;
        let mut at = first;
        while at != last {
            total += lines[at].line.length();
            at = (at + 1) % count;
        }
        total + lines[last].line.length()
    };
    let paint = |lines: &mut [ColouredLine], (first, last): (usize, usize), colour: u8| {
        let mut at = first;
        while at != last {
            lines[at].colour = colour;
            at = (at + 1) % count;
        }
        lines[last].colour = colour;
    };

    let segments = get_segments(&lines);
    if segments.len() >= 2 {
        for curr in 0..segments.len() {
            let next = (curr + 1) % segments.len();
            let c0 = lines[segments[curr].0].colour;
            let c1 = lines[segments[next].0].colour;
            if c0 != c1
                && length(&lines, segments[curr]) >= tenth
                && length(&lines, segments[next]) <= short
            {
                paint(&mut lines, segments[next], c0);
            }
        }
    }

    let segments = get_segments(&lines);
    if segments.len() >= 2 {
        for curr in 0..segments.len() {
            let next = (curr + 1) % segments.len();
            let c0 = lines[segments[curr].0].colour;
            let c1 = lines[segments[next].0].colour;
            if c0 >= 1 && c0 != c1 && length(&lines, segments[next]) <= short {
                paint(&mut lines, segments[next], c0);
            }
        }
    }

    // Last, round the ring with wrap-around: a run of 0.5 mm or less
    // between two runs of one painted colour takes that colour
    // (PrusaSlicer #7112). The reference's pass recolours the run after
    // the short one (`segments[next_next_idx]`), which has the colour
    // already, so it does nothing there; the short run is what is meant,
    // and that is done here (fix: aceRage/EdgeSlicer#1017).
    let segments = get_segments(&lines);
    if segments.len() >= 3 {
        for curr in 0..segments.len() {
            let next = (curr + 1) % segments.len();
            let after = (next + 1) % segments.len();
            let c0 = lines[segments[curr].0].colour;
            let c1 = lines[segments[next].0].colour;
            let c2 = lines[segments[after].0].colour;
            if c0 > 0 && c0 == c2 && c0 != c1 && length(&lines, segments[next]) <= half {
                paint(&mut lines, segments[next], c0);
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use edge_geom::Point2;

    use super::*;

    fn run(lengths_and_colours: &[(f64, u8)]) -> Vec<ColouredLine> {
        let mut x = 0.0;
        lengths_and_colours
            .iter()
            .map(|&(length, colour)| {
                let line = Line::new(Point2::from_mm(x, 0.0), Point2::from_mm(x + length, 0.0));
                x += length;
                ColouredLine { line, colour }
            })
            .collect()
    }

    fn colours(lines: &[ColouredLine]) -> Vec<u8> {
        lines.iter().map(|line| line.colour).collect()
    }

    #[test]
    fn runs_round_a_ring_start_at_the_first_change() {
        let lines = run(&[(1.0, 1), (1.0, 1), (1.0, 2), (1.0, 0), (1.0, 1)]);
        assert_eq!(get_segments(&lines), [(2, 2), (3, 3), (4, 1)]);
        let one = run(&[(1.0, 3), (1.0, 3)]);
        assert_eq!(get_segments(&one), [(0, 1)]);
    }

    #[test]
    fn short_runs_go_to_the_colour_about_them() {
        // 0.4 mm unpainted between two of colour 2 goes to 2; 0.15 mm of
        // 3 after a long run of 1 goes to 1.
        let lines = filter_colorized_polygon(
            run(&[(5.0, 2), (0.4, 0), (5.0, 2), (5.0, 1), (0.15, 3), (5.0, 0)]),
            1.0,
        );
        assert_eq!(colours(&lines), [2, 2, 2, 1, 1, 0]);
        // 0.4 mm of colour between two unpainted runs stays: only paint
        // closes over a gap.
        let lines = filter_colorized_polygon(run(&[(5.0, 0), (0.4, 2), (5.0, 0)]), 1.0);
        assert_eq!(colours(&lines), [0, 2, 0]);
    }

    /// The C++ fix's test (aceRage/EdgeSlicer#1017,
    /// tests/libslic3r/test_mm_filter_colorized.cpp): a 20 x 10 mm
    /// rectangle, the bottom edge in colour 1, then up the right edge a
    /// short run of `mid` cut into three lines, the rest of the right edge
    /// and the top in colour 1, the left edge in colour 2.
    fn short_run_between_painted(mid: u8, run_mm: f64) -> Vec<ColouredLine> {
        let s = run_mm / 3.0;
        let points = [
            ((0.0, 0.0), 1),
            ((20.0, 0.0), mid),
            ((20.0, s), mid),
            ((20.0, 2.0 * s), mid),
            ((20.0, run_mm), 1),
            ((20.0, 10.0), 1),
            ((0.0, 10.0), 2),
        ];
        (0..points.len())
            .map(|at| {
                let ((ax, ay), colour) = points[at];
                let ((bx, by), _) = points[(at + 1) % points.len()];
                ColouredLine {
                    line: Line::new(Point2::from_mm(ax, ay), Point2::from_mm(bx, by)),
                    colour,
                }
            })
            .collect()
    }

    #[test]
    fn a_short_run_between_two_runs_of_one_painted_colour_takes_it() {
        let middle = |lines: &[ColouredLine]| colours(&lines[1..4]);
        // 0.4 mm unpainted, and 0.4 mm of colour 3, both go to colour 1.
        for mid in [0, 3] {
            let out = filter_colorized_polygon(short_run_between_painted(mid, 0.4), 1.0);
            assert_eq!(out.len(), 7);
            assert_eq!(middle(&out), [1, 1, 1], "mid {mid}");
            assert_eq!(out[6].colour, 2);
        }
        // 0.6 mm is over the limit.
        let out = filter_colorized_polygon(short_run_between_painted(0, 0.6), 1.0);
        assert_eq!(middle(&out), [0, 0, 0]);
        // The limit scales: 0.4 mm is over it at a 0.2 mm nozzle.
        let out = filter_colorized_polygon(short_run_between_painted(0, 0.4), 0.5);
        assert_eq!(middle(&out), [0, 0, 0]);
        // Between two runs of the default colour nothing changes.
        let mut lines = short_run_between_painted(3, 0.4);
        for line in &mut lines {
            if line.colour == 1 {
                line.colour = 0;
            }
        }
        let out = filter_colorized_polygon(lines, 1.0);
        assert_eq!(middle(&out), [3, 3, 3]);
    }
}
