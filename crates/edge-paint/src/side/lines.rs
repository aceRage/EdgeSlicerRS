//! Which edges of a layer's outline each painted triangle's cut lies
//! along, and the painted stretches of each edge that come of it.

use edge_geom::{ExPolygon, Point2};
use rustc_hash::FxHashMap;

use super::{Cut, Line, SCALED_EPSILON, dist, dist2, scale};

/// The side of the squares the outline's edges are sorted into.
const CELL: i64 = 2_000_000;

/// A layer's rings, outer ones and holes as they come (the reference's
/// `EdgeGrid::Grid::contours`), with the squares each edge passes
/// through.
pub(crate) struct Rings {
    pub rings: Vec<Vec<Point2>>,
    grid: FxHashMap<(i64, i64), Vec<(u32, u32)>>,
}

impl Rings {
    pub fn new(outline: &[ExPolygon]) -> Self {
        let rings: Vec<Vec<Point2>> = outline
            .iter()
            .flat_map(ExPolygon::rings)
            .filter(|ring| !ring.points.is_empty())
            .map(|ring| ring.points.clone())
            .collect();
        let mut grid: FxHashMap<(i64, i64), Vec<(u32, u32)>> = FxHashMap::default();
        for (ring, points) in rings.iter().enumerate() {
            for at in 0..points.len() {
                let (a, b) = (points[at], points[(at + 1) % points.len()]);
                for x in cell(a.x.min(b.x))..=cell(a.x.max(b.x)) {
                    for y in cell(a.y.min(b.y))..=cell(a.y.max(b.y)) {
                        grid.entry((x, y))
                            .or_default()
                            .push((ring as u32, at as u32));
                    }
                }
            }
        }
        Self { rings, grid }
    }

    /// Edge `at` of ring `ring` (`EdgeGrid::Contour::get_segment`).
    pub fn segment(&self, ring: usize, at: usize) -> Line {
        let points = &self.rings[ring];
        Line::new(points[at], points[(at + 1) % points.len()])
    }

    /// Every edge that passes through a square within `margin` of
    /// `line`'s box, once each.
    fn near(&self, line: &Line, margin: i64, found: &mut Vec<(u32, u32)>) {
        found.clear();
        let (x0, x1) = (
            line.a.x.min(line.b.x) - margin,
            line.a.x.max(line.b.x) + margin,
        );
        let (y0, y1) = (
            line.a.y.min(line.b.y) - margin,
            line.a.y.max(line.b.y) + margin,
        );
        for x in cell(x0)..=cell(x1) {
            for y in cell(y0)..=cell(y1) {
                if let Some(edges) = self.grid.get(&(x, y)) {
                    found.extend(edges);
                }
            }
        }
        found.sort_unstable();
        found.dedup();
    }
}

fn cell(c: i64) -> i64 {
    c.div_euclid(CELL)
}

/// A cut put onto an edge of the outline (the reference's
/// `PaintedLine`): which ring and edge, the stretch of the edge, its
/// colour and the triangle it came from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaintedLine {
    pub ring: usize,
    pub edge: usize,
    pub line: Line,
    pub colour: u8,
    pub facet: u32,
}

/// The squared distance from `p` to the segment `line`
/// (`line_alg::distance_to_squared`).
pub(crate) fn distance_to_squared(line: &Line, p: Point2) -> f64 {
    let v = ((line.b.x - line.a.x) as f64, (line.b.y - line.a.y) as f64);
    let va = ((p.x - line.a.x) as f64, (p.y - line.a.y) as f64);
    let l2 = v.0 * v.0 + v.1 * v.1;
    if l2 == 0.0 {
        return va.0 * va.0 + va.1 * va.1;
    }
    let t = (va.0 * v.0 + va.1 * v.1) / l2;
    if t <= 0.0 {
        va.0 * va.0 + va.1 * va.1
    } else if t >= 1.0 {
        dist2(p, line.b)
    } else {
        let (dx, dy) = (t * v.0 - va.0, t * v.1 - va.1);
        dx * dx + dy * dy
    }
}

/// `projected` put onto `onto`, each end square to it and kept within
/// it; none when `onto` has no length (`project_line_on_line`).
fn project_line_on_line(onto: &Line, projected: &Line) -> Option<Line> {
    let v1 = ((onto.b.x - onto.a.x) as f64, (onto.b.y - onto.a.y) as f64);
    let l2 = v1.0 * v1.0 + v1.1 * v1.1;
    if l2 == 0.0 {
        return None;
    }
    let along = |p: Point2| {
        let t = (((p.x - onto.a.x) as f64) * v1.0 + ((p.y - onto.a.y) as f64) * v1.1) / l2;
        let t = t.clamp(0.0, 1.0);
        // `(t * v1).cast<coord_t>()`: towards zero.
        Point2 {
            x: onto.a.x + (t * v1.0) as i64,
            y: onto.a.y + (t * v1.1) as i64,
        }
    };
    Some(Line::new(along(projected.a), along(projected.b)))
}

/// Every edge of the outline each cut lies along, with the stretch of
/// it the cut covers (`PaintedLineVisitor`): an edge within 30 degrees
/// of the cut's direction with an end of either within 50
/// `SCALED_EPSILON` of the other. The cut is projected onto the edge and
/// turned to run the edge's way.
pub(crate) fn painted(rings: &Rings, cuts: &[Cut]) -> Vec<PaintedLine> {
    const APPEND: f64 = 50.0 * SCALED_EPSILON as f64;
    let cos_threshold2 = (std::f64::consts::PI * 30.0 / 180.0).cos().powi(2);
    let mut found = Vec::new();
    let mut near = Vec::new();
    for cut in cuts {
        let line = cut.line;
        rings.near(&line, APPEND.ceil() as i64 + 1, &mut near);
        let v1 = ((line.b.x - line.a.x) as f64, (line.b.y - line.a.y) as f64);
        let v1_norm = v1.0 * v1.0 + v1.1 * v1.1;
        let heuristic = line.length() + APPEND;
        for &(ring, edge) in &near {
            let (ring, edge) = (ring as usize, edge as usize);
            let grid_line = rings.segment(ring, edge);
            let v2 = (
                (grid_line.b.x - grid_line.a.x) as f64,
                (grid_line.b.y - grid_line.a.y) as f64,
            );
            let heuristic2 = (heuristic + grid_line.length()).powi(2);
            if dist2(grid_line.a, line.a) > heuristic2
                || dist2(grid_line.b, line.a) > heuristic2
                || dist2(grid_line.a, line.b) > heuristic2
                || dist2(grid_line.b, line.b) > heuristic2
            {
                continue;
            }
            let dot = v1.0 * v2.0 + v1.1 * v2.1;
            if dot * dot <= cos_threshold2 * v1_norm * (v2.0 * v2.0 + v2.1 * v2.1) {
                continue;
            }
            let close = APPEND * APPEND;
            if distance_to_squared(&grid_line, line.a) < close
                || distance_to_squared(&grid_line, line.b) < close
                || distance_to_squared(&line, grid_line.a) < close
                || distance_to_squared(&line, grid_line.b) < close
            {
                let Some(mut projected) = project_line_on_line(&grid_line, &line) else {
                    continue;
                };
                if dist2(projected.a, grid_line.a) > dist2(projected.b, grid_line.a) {
                    std::mem::swap(&mut projected.a, &mut projected.b);
                }
                found.push(PaintedLine {
                    ring,
                    edge,
                    line: projected,
                    colour: cut.colour,
                    facet: cut.facet,
                });
            }
        }
    }
    found
}

/// The reference's `filter_eps_value`, `scale_(0.1f * run_scale)` cut
/// to a whole number.
pub(crate) fn filter_eps(run_scale: f32) -> f64 {
    scale(f64::from(0.1f32 * run_scale)) as i64 as f64
}

/// The painted lines sorted by ring, edge, where they start along the
/// edge and how long they are, and each edge's joined
/// (`post_process_painted_lines`): one list for each ring.
pub(crate) fn post_process(
    rings: &Rings,
    mut painted: Vec<PaintedLine>,
    run_scale: f32,
) -> Vec<Vec<PaintedLine>> {
    let mut by_ring = vec![Vec::new(); rings.rings.len()];
    if painted.is_empty() {
        return by_ring;
    }
    painted.sort_by(|first, second| {
        (first.ring, first.edge)
            .cmp(&(second.ring, second.edge))
            .then_with(|| {
                let start = rings.rings[first.ring][first.edge];
                dist2(first.line.a, start).total_cmp(&dist2(second.line.a, start))
            })
            .then_with(|| {
                dist2(first.line.a, first.line.b).total_cmp(&dist2(second.line.a, second.line.b))
            })
            .then_with(|| first.colour.cmp(&second.colour))
            .then_with(|| first.facet.cmp(&second.facet))
    });
    for same in painted.chunk_by(|a, b| (a.ring, a.edge) == (b.ring, b.edge)) {
        let edge = rings.segment(same[0].ring, same[0].edge);
        by_ring[same[0].ring].extend(filter_painted_lines(&edge, same, run_scale));
    }
    by_ring
}

/// One edge's painted lines, sorted along it, joined where one colour
/// overlaps itself or leaves a gap of no more than 0.1 mm, and cut where
/// another overlaps it; the first and last pulled to the edge's ends
/// when that near them (`filter_painted_lines`).
fn filter_painted_lines(edge: &Line, painted: &[PaintedLine], run_scale: f32) -> Vec<PaintedLine> {
    let eps = filter_eps(run_scale);
    let mut filtered = vec![painted[0]];
    for curr in &painted[1..] {
        // The edge is all coloured already.
        if filtered[filtered.len() - 1].line.b == edge.b {
            break;
        }
        let prev = filtered.last_mut().expect("one at least");
        let prev_length = prev.line.length();
        let dist_between = dist(curr.line.a, prev.line.a) - prev_length;
        if dist_between >= 0.0 {
            if prev.colour == curr.colour && dist_between <= eps {
                prev.line.b = curr.line.b;
            } else {
                filtered.push(*curr);
            }
        } else if dist(curr.line.b, prev.line.a) > prev_length {
            if prev.colour == curr.colour {
                prev.line.b = curr.line.b;
            } else {
                let from = prev.line.b;
                filtered.push(PaintedLine {
                    line: Line::new(from, curr.line.b),
                    ..*curr
                });
            }
        }
    }
    if dist(filtered[0].line.a, edge.a) <= eps {
        filtered[0].line.a = edge.a;
    }
    let last = filtered.len() - 1;
    if dist(filtered[last].line.b, edge.b) <= eps {
        filtered[last].line.b = edge.b;
    }
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f64, y: f64) -> Point2 {
        Point2::from_mm(x, y)
    }

    #[test]
    fn a_cut_along_an_edge_is_put_onto_it_and_one_across_is_not() {
        let square = vec![ExPolygon {
            contour: edge_geom::Polygon {
                points: vec![
                    point(0.0, 0.0),
                    point(10.0, 0.0),
                    point(10.0, 10.0),
                    point(0.0, 10.0),
                ],
            },
            holes: Vec::new(),
        }];
        let rings = Rings::new(&square);
        let cut = |a: Point2, b: Point2| Cut {
            line: Line::new(a, b),
            colour: 2,
            facet: 0,
        };
        // 3 µm off the bottom edge, running against it: put onto it,
        // the edge's way round.
        let along = painted(&rings, &[cut(point(6.0, -0.003), point(2.0, -0.003))]);
        assert_eq!(along.len(), 1);
        assert_eq!((along[0].ring, along[0].edge), (0, 0));
        assert_eq!(along[0].line, Line::new(point(2.0, 0.0), point(6.0, 0.0)));
        // 10 µm off is too far; square to the edge does not count.
        assert!(painted(&rings, &[cut(point(2.0, -0.01), point(6.0, -0.01))]).is_empty());
        assert!(painted(&rings, &[cut(point(5.0, 0.0), point(5.0, 3.0))]).is_empty());
    }

    #[test]
    fn lines_of_a_colour_nearly_meeting_are_joined_and_another_is_cut_where_it_overlaps() {
        let edge = Line::new(point(0.0, 0.0), point(10.0, 0.0));
        let line = |a: f64, b: f64, colour: u8| PaintedLine {
            ring: 0,
            edge: 0,
            line: Line::new(point(a, 0.0), point(b, 0.0)),
            colour,
            facet: 0,
        };
        let filtered = filter_painted_lines(
            &edge,
            &[
                line(0.05, 3.0, 1),
                line(3.08, 5.0, 1),
                line(4.0, 7.0, 2),
                line(9.0, 9.95, 2),
            ],
            1.0,
        );
        let got: Vec<(i64, i64, u8)> = filtered
            .iter()
            .map(|l| (l.line.a.x / 1000, l.line.b.x / 1000, l.colour))
            .collect();
        assert_eq!(got, [(0, 5000, 1), (5000, 7000, 2), (9000, 10_000, 2)]);
    }
}
