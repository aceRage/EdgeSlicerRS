//! Sparse lines joined along the outline they are cut to, as the
//! reference joins them: a port of FillBase.cpp `Fill::connect_infill`
//! (the version that sorts the arcs between neighbouring line ends by
//! length), with `create_boundary_infill_graph`,
//! `mark_boundary_segments_touching_infill`, `take` and `take_limited`.
//!
//! Which ends get joined, which way round each joined path runs and the
//! order the paths come out in all follow from the order the lines are
//! given in (`infill_ordered`): the paths keep the place of the lowest
//! numbered line in them. The reference hands the lines over in the
//! order `make_fill_lines` makes them, and the G-code writer then
//! orders the paths from where the nozzle is, so getting these right is
//! what makes the nozzle travel the same way between them.
//!
//! The reference finds the outline near a line with a grid of cells; here
//! a grid of cells does the same, and since marking an outline as taken
//! does not depend on the order the line pieces are looked at in, the
//! outcome is the same.

use edge_geom::{Coord, Point2, Polygon};

/// libslic3r's `SCALED_EPSILON`, in scaled units (1e-4 mm).
const SCALED_EPSILON: f64 = 100.0;
const NONE: usize = usize::MAX;

/// FillBase.cpp `ContourIntersectionPoint`: where a line end meets the
/// outline.
#[derive(Clone, Copy, Debug)]
struct Cp {
    contour: usize,
    point: usize,
    param: f64,
    prev: usize,
    next: usize,
    not_taken_prev: f64,
    not_taken_next: f64,
    consumed: bool,
    prev_trimmed: bool,
    next_trimmed: bool,
}

impl Cp {
    fn unconnected() -> Self {
        Self {
            contour: NONE,
            point: NONE,
            param: 0.0,
            prev: NONE,
            next: NONE,
            not_taken_prev: f64::MAX,
            not_taken_next: f64::MAX,
            consumed: false,
            prev_trimmed: false,
            next_trimmed: false,
        }
    }
}

struct Graph {
    boundary: Vec<Vec<Point2>>,
    params: Vec<Vec<f64>>,
    cps: Vec<Cp>,
    /// The ends on each ring, in order along it.
    ring_ends: Vec<Vec<usize>>,
}

impl Graph {
    fn consume_prev(&mut self, at: usize) {
        let cp = &mut self.cps[at];
        cp.not_taken_prev = 0.0;
        cp.prev_trimmed = true;
        cp.consumed = true;
    }
    fn consume_next(&mut self, at: usize) {
        let cp = &mut self.cps[at];
        cp.not_taken_next = 0.0;
        cp.next_trimmed = true;
        cp.consumed = true;
    }
    fn trim_prev(&mut self, at: usize, length: f64) {
        let cp = &mut self.cps[at];
        if length < cp.not_taken_prev {
            cp.not_taken_prev = length;
            cp.prev_trimmed = true;
        }
    }
    fn trim_next(&mut self, at: usize, length: f64) {
        let cp = &mut self.cps[at];
        if length < cp.not_taken_next {
            cp.not_taken_next = length;
            cp.next_trimmed = true;
        }
    }
    fn could_take_prev(&self, at: usize) -> bool {
        let cp = &self.cps[at];
        !cp.consumed && cp.not_taken_prev > SCALED_EPSILON
    }
    fn could_take_next(&self, at: usize) -> bool {
        let cp = &self.cps[at];
        !cp.consumed && cp.not_taken_next > SCALED_EPSILON
    }
    fn could_connect_prev(&self, at: usize) -> bool {
        let cp = &self.cps[at];
        !cp.consumed
            && cp.prev != at
            && !self.cps[cp.prev].consumed
            && !cp.prev_trimmed
            && !self.cps[cp.prev].next_trimmed
    }
    fn could_connect_next(&self, at: usize) -> bool {
        let cp = &self.cps[at];
        !cp.consumed
            && cp.next != at
            && !self.cps[cp.next].consumed
            && !cp.next_trimmed
            && !self.cps[cp.next].prev_trimmed
    }
}

fn ccw(param1: f64, param2: f64, length: f64) -> f64 {
    let d = param2 - param1;
    if d < 0.0 { d + length } else { d }
}

fn cw(param1: f64, param2: f64, length: f64) -> f64 {
    ccw(param2, param1, length)
}

fn as_f(p: Point2) -> (f64, f64) {
    (p.x as f64, p.y as f64)
}

/// libslic3r `lerp` of two points, rounded back to whole units the way
/// `Point(Vec2d)` does (to nearest).
fn lerp(a: Point2, b: Point2, t: f64) -> Point2 {
    let (ax, ay) = as_f(a);
    let (bx, by) = as_f(b);
    Point2 {
        x: (ax * (1.0 - t) + bx * t).round() as Coord,
        y: (ay * (1.0 - t) + by * t).round() as Coord,
    }
}

/// Lines joined and anchored along `boundary` (FillBase.cpp
/// `Fill::connect_infill`). `spacing`, `anchor` and `anchor_max` are in
/// scaled units; every line end is expected on one of the rings.
pub(crate) fn connect_infill(
    mut lines: Vec<Vec<Point2>>,
    boundary: &[&Polygon],
    spacing: f64,
    anchor: f64,
    anchor_max: f64,
) -> Vec<Vec<Point2>> {
    lines.retain(|line| line.len() >= 2);
    if lines.is_empty() {
        return lines;
    }
    let mut graph = boundary_graph(&lines, boundary, spacing);
    let mut merged_with: Vec<usize> = (0..lines.len()).collect();
    fn merged(merged_with: &mut [usize], index: usize) -> usize {
        let mut last = index;
        loop {
            let lower = merged_with[last];
            if lower == last {
                merged_with[index] = last;
                return last;
            }
            last = lower;
        }
    }
    let half_width = 0.5 * spacing;

    // The arcs from each end to the next along its ring, shortest first.
    let mut arches: Vec<(usize, f64)> = Vec::with_capacity(graph.cps.len());
    for at in 0..graph.cps.len() {
        let cp = graph.cps[at];
        if cp.contour != NONE && cp.next != at && graph.could_connect_next(at) {
            let length = *graph.params[cp.contour].last().expect("parametrised");
            arches.push((at, ccw(cp.param, graph.cps[cp.next].param, length)));
        }
    }
    // The reference's `std::sort`, ties and all.
    crate::cxx_sort::sort_by(&mut arches, |a, b| a.1 < b.1);

    for &(cp1, arc_length) in &arches {
        let cp2 = graph.cps[cp1].next;
        if graph.cps[cp1].consumed || graph.cps[cp2].consumed {
            continue;
        }
        let index1 = merged(&mut merged_with, cp1 / 2);
        let index2 = merged(&mut merged_with, cp2 / 2);
        if index1 == index2 {
            continue;
        }
        let contour = graph.cps[cp1].contour;
        if arc_length < anchor_max {
            let (point1, point2) = (
                graph.boundary[contour][graph.cps[cp1].point],
                graph.boundary[contour][graph.cps[cp2].point],
            );
            if lines[index1][0] == point1 {
                lines[index1].reverse();
            }
            if lines[index2].last() == Some(&point2) {
                lines[index2].reverse();
            }
            let second = std::mem::take(&mut lines[index2]);
            take(&mut graph, &mut lines[index1], &second, cp1, cp2, false);
            if index2 < index1 {
                lines[index2] = std::mem::take(&mut lines[index1]);
                merged_with[index1] = merged_with[index2];
            } else {
                merged_with[index2] = merged_with[index1];
            }
        } else if anchor > SCALED_EPSILON {
            take_limited(
                &mut graph,
                &mut lines[index1],
                cp1,
                cp2,
                false,
                anchor,
                half_width,
            );
            take_limited(
                &mut graph,
                &mut lines[index2],
                cp2,
                cp1,
                true,
                anchor,
                half_width,
            );
        }
    }

    // Join what is still open to a neighbour, or anchor it.
    for at in 0..graph.cps.len() {
        let cp = graph.cps[at];
        if cp.consumed || cp.contour == NONE {
            continue;
        }
        let length = *graph.params[cp.contour].last().expect("parametrised");
        let lprev = if graph.could_connect_prev(at) {
            ccw(graph.cps[cp.prev].param, cp.param, length)
        } else {
            f64::MAX
        };
        let lnext = if graph.could_connect_next(at) {
            ccw(cp.param, graph.cps[cp.next].param, length)
        } else {
            f64::MAX
        };
        let index = merged(&mut merged_with, at / 2);
        let mut connected = false;
        for l in [lprev.min(lnext), lprev.max(lnext)] {
            if l == f64::MAX || l > anchor_max {
                break;
            }
            let reversed = l == lprev;
            let cp2 = if reversed { cp.prev } else { cp.next };
            let index2 = merged(&mut merged_with, cp2 / 2);
            if index == index2 {
                continue;
            }
            let point = graph.boundary[cp.contour][cp.point];
            let point2 = graph.boundary[cp.contour][graph.cps[cp2].point];
            if lines[index][0] == point {
                lines[index].reverse();
            }
            if lines[index2].last() == Some(&point2) {
                lines[index2].reverse();
            }
            let second = std::mem::take(&mut lines[index2]);
            take(&mut graph, &mut lines[index], &second, at, cp2, reversed);
            if index < index2 {
                merged_with[index2] = index;
            } else {
                merged_with[index] = index2;
                lines[index2] = std::mem::take(&mut lines[index]);
            }
            connected = true;
            break;
        }
        if !connected && anchor > SCALED_EPSILON {
            // The longer side, which leaves the better chance of an anchor
            // on the other side of this end.
            let cp = graph.cps[at];
            let l = cp.not_taken_prev.max(cp.not_taken_next);
            if l > SCALED_EPSILON {
                if cp.not_taken_prev > cp.not_taken_next {
                    take_limited(
                        &mut graph,
                        &mut lines[index],
                        at,
                        cp.prev,
                        true,
                        anchor,
                        half_width,
                    );
                } else {
                    take_limited(
                        &mut graph,
                        &mut lines[index],
                        at,
                        cp.next,
                        false,
                        anchor,
                        half_width,
                    );
                }
            }
        }
    }
    lines.retain(|line| !line.is_empty());
    lines
}

/// FillBase.cpp `create_boundary_infill_graph`: the rings with every line
/// end put in as a point of its own, each end's place along its ring and
/// its neighbours either way, and the outline that lines run beside
/// marked as taken.
fn boundary_graph(lines: &[Vec<Point2>], boundary: &[&Polygon], spacing: f64) -> Graph {
    let mut graph = Graph {
        boundary: vec![Vec::new(); boundary.len()],
        params: vec![Vec::new(); boundary.len()],
        cps: vec![Cp::unconnected(); lines.len() * 2],
        ring_ends: vec![Vec::new(); boundary.len()],
    };
    // Each end projected onto the rings: (ring, edge, t, end).
    let mut found: Vec<(usize, usize, f64, usize)> = Vec::with_capacity(lines.len() * 2);
    for (index, line) in lines.iter().enumerate() {
        for (end, point) in [line[0], line[line.len() - 1]].into_iter().enumerate() {
            if let Some((ring, edge, t)) = closest_on(boundary, point, SCALED_EPSILON) {
                found.push((ring, edge, t, index * 2 + end));
            }
        }
    }
    // The reference's `std::sort`, ties and all.
    crate::cxx_sort::sort_by(&mut found, |a, b| {
        a.0 < b.0 || (a.0 == b.0 && (a.1 < b.1 || (a.1 == b.1 && a.2 < b.2)))
    });
    let mut it = 0;
    for (ring_index, ring) in boundary.iter().enumerate() {
        let dst = &mut graph.boundary[ring_index];
        let mut on_ring: Vec<usize> = Vec::new();
        let (mut first, mut prev) = (NONE, NONE);
        let count = ring.points.len();
        for (index, &vertex) in ring.points.iter().enumerate() {
            if dst.last() != Some(&vertex) {
                dst.push(vertex);
            }
            while it < found.len() && found[it].0 == ring_index && found[it].1 == index {
                let end = found[it].3;
                let line = &lines[end / 2];
                let point = if end & 1 == 1 {
                    line[line.len() - 1]
                } else {
                    line[0]
                };
                let mut at = 0;
                if index + 1 < count || Some(&point) != dst.first() {
                    if dst.last() != Some(&point) {
                        dst.push(point);
                    }
                    at = dst.len() - 1;
                }
                let mut cp = Cp::unconnected();
                cp.contour = ring_index;
                cp.point = at;
                graph.cps[end] = cp;
                if prev != NONE {
                    graph.cps[prev].next = end;
                    graph.cps[end].prev = prev;
                } else {
                    first = end;
                }
                on_ring.push(end);
                prev = end;
                it += 1;
            }
            if first != NONE {
                graph.cps[prev].next = first;
                graph.cps[first].prev = prev;
            }
        }
        // The ring parametrised by length, with the ends put in.
        let params = &mut graph.params[ring_index];
        *params = vec![0.0; dst.len() + 1];
        for i in 1..dst.len() {
            let (a, b) = (as_f(dst[i - 1]), as_f(dst[i]));
            params[i] = params[i - 1] + (b.0 - a.0).hypot(b.1 - a.1);
        }
        if !dst.is_empty() {
            let (a, b) = (as_f(dst[dst.len() - 1]), as_f(dst[0]));
            let n = dst.len();
            params[n] = params[n - 1] + (b.0 - a.0).hypot(b.1 - a.1);
        }
        let length = *params.last().unwrap_or(&0.0);
        for &end in &on_ring {
            graph.cps[end].param = params[graph.cps[end].point];
        }
        for &end in &on_ring {
            let cp = graph.cps[end];
            if cp.next == end {
                graph.cps[end].not_taken_prev = length;
                graph.cps[end].not_taken_next = length;
            } else {
                graph.cps[end].not_taken_prev = ccw(graph.cps[cp.prev].param, cp.param, length);
                graph.cps[end].not_taken_next = ccw(cp.param, graph.cps[cp.next].param, length);
            }
        }
        graph.ring_ends[ring_index] = on_ring;
    }
    mark_touching(&mut graph, lines, 1.7 * spacing, 0.8 * spacing);
    graph
}

/// EdgeGrid.cpp `Grid::closest_point_signed_distance`: the place on the
/// rings nearest `point`, nearer than `radius`, as (ring, edge, t).
fn closest_on(boundary: &[&Polygon], point: Point2, radius: f64) -> Option<(usize, usize, f64)> {
    let mut d_min = radius;
    let mut best: Option<(usize, usize, f64)> = None;
    for (ring, polygon) in boundary.iter().enumerate() {
        let points = &polygon.points;
        let count = points.len();
        if count < 2 {
            continue;
        }
        for edge in 0..count {
            let p1 = points[edge];
            let p2 = points[(edge + 1) % count];
            let (sx, sy) = ((p2.x - p1.x) as i128, (p2.y - p1.y) as i128);
            let (vx, vy) = ((point.x - p1.x) as i128, (point.y - p1.y) as i128);
            let t_pt = sx * vx + sy * vy;
            let l2 = sx * sx + sy * sy;
            if l2 == 0 {
                continue;
            }
            if t_pt < 0 {
                let dabs = ((vx * vx + vy * vy) as f64).sqrt();
                if dabs < d_min {
                    let p0 = points[(edge + count - 1) % count];
                    let (qx, qy) = ((p1.x - p0.x) as i128, (p1.y - p0.y) as i128);
                    if qx * vx + qy * vy > 0 {
                        d_min = dabs;
                        best = Some((ring, edge, 0.0));
                    }
                }
            } else if t_pt > l2 {
                continue;
            } else {
                let d_seg = sy * vx - sx * vy;
                let dabs = (d_seg as f64 / (l2 as f64).sqrt()).abs();
                if dabs < d_min {
                    d_min = dabs;
                    best = Some((ring, edge, t_pt as f64 / l2 as f64));
                }
            }
        }
    }
    best.filter(|_| d_min <= radius)
}

/// FillBase.cpp `clip_start_segment_and_point` and
/// `clip_end_segment_and_point`: the point `distance` in from one end of
/// a polyline, as (segment, t along it from its start, point).
fn clip_end(line: &[Point2], distance: f64, from_start: bool) -> Option<(usize, f64, (f64, f64))> {
    let mut distance = distance;
    let n = line.len();
    if from_start {
        let mut prev = as_f(line[0]);
        for (i, &p) in line.iter().enumerate().skip(1) {
            let pt = as_f(p);
            let (vx, vy) = (pt.0 - prev.0, pt.1 - prev.1);
            let l = vx.hypot(vy);
            if l > distance {
                let t = distance / l;
                return Some((i - 1, t, (prev.0 + t * vx, prev.1 + t * vy)));
            }
            distance -= l;
            prev = pt;
        }
    } else {
        let mut next = as_f(line[n - 1]);
        for i in (0..n - 1).rev() {
            let pt = as_f(line[i]);
            let (vx, vy) = (pt.0 - next.0, pt.1 - next.1);
            let l = vx.hypot(vy);
            if l > distance {
                let t = distance / l;
                return Some((i, 1.0 - t, (next.0 + t * vx, next.1 + t * vy)));
            }
            distance -= l;
            next = pt;
        }
    }
    None
}

/// Geometry.hpp `ray_circle_intersections_r2_lv2_c`.
fn ray_circle(r2: f64, a: f64, b: f64, lv2: f64, c: f64) -> Option<((f64, f64), (f64, f64))> {
    let x0 = -a * c / lv2;
    let y0 = -b * c / lv2;
    let d2 = r2 - c * c / lv2;
    if d2 < 0.0 {
        return None;
    }
    let mult = (d2 / lv2).sqrt();
    Some((
        (x0 + b * mult, y0 - a * mult),
        (x0 - b * mult, y0 + a * mult),
    ))
}

/// Geometry.hpp `liang_barsky_line_clipping_interval`.
fn liang_barsky(
    p0: (f64, f64),
    v: (f64, f64),
    min: (f64, f64),
    max: (f64, f64),
) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let clip = |p: f64, q: f64, t0: &mut f64, t1: &mut f64| -> bool {
        if p == 0.0 {
            return q >= 0.0;
        }
        let r = q / p;
        if p < 0.0 {
            if r > *t1 {
                return false;
            }
            if r > *t0 {
                *t0 = r;
            }
        } else {
            if r < *t0 {
                return false;
            }
            if r < *t1 {
                *t1 = r;
            }
        }
        true
    };
    if clip(-v.0, p0.0 - min.0, &mut t0, &mut t1)
        && clip(v.0, max.0 - p0.0, &mut t0, &mut t1)
        && clip(-v.1, p0.1 - min.1, &mut t0, &mut t1)
        && clip(v.1, max.1 - p0.1, &mut t0, &mut t1)
    {
        Some((t0, t1))
    } else {
        None
    }
}

/// FillBase.cpp `line_rounded_thick_segment_collision`: where along the
/// line a→b it comes within `offset` of the segment c→d, as distances
/// from a.
fn thick_collision(
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    d: (f64, f64),
    offset: f64,
) -> Option<(f64, f64)> {
    let v0 = (b.0 - a.0, b.1 - a.1);
    let lv = v0.0 * v0.0 + v0.1 * v0.1;
    let sv = (d.0 - c.0, d.1 - c.1);
    let sl = sv.0.hypot(sv.1);
    let offset2 = offset * offset;
    if lv < SCALED_EPSILON * SCALED_EPSILON {
        let mid = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
        let hit = if sl > SCALED_EPSILON {
            let t = (((mid.0 - c.0) * sv.0 + (mid.1 - c.1) * sv.1) / (sl * sl)).clamp(0.0, 1.0);
            let f = (c.0 + t * sv.0 - mid.0, c.1 + t * sv.1 - mid.1);
            f.0 * f.0 + f.1 * f.1 < offset2
        } else {
            let m = (0.5 * (c.0 + d.0) - mid.0, 0.5 * (c.1 + d.1) - mid.1);
            m.0 * m.0 + m.1 * m.1 < offset2
        };
        return hit.then_some((0.0, lv.sqrt()));
    }
    let (mut tmin, mut tmax) = (f64::MAX, -f64::MAX);
    let mut extend = |lo: f64, hi: f64| {
        tmin = tmin.min(lo);
        tmax = tmax.max(hi);
    };
    let mut circle = |centre: (f64, f64), r2: f64| {
        let p0 = (a.0 - centre.0, a.1 - centre.1);
        if let Some((p1, p2)) = ray_circle(r2, v0.1, -v0.0, lv, -v0.1 * p0.0 + v0.0 * p0.1) {
            let mut lo = ((p1.0 - p0.0) * v0.0 + (p1.1 - p0.1) * v0.1) / lv;
            let mut hi = ((p2.0 - p0.0) * v0.0 + (p2.1 - p0.1) * v0.1) / lv;
            if lo > hi {
                std::mem::swap(&mut lo, &mut hi);
            }
            lo = lo.max(0.0);
            hi = hi.min(1.0);
            if lo <= hi {
                extend(lo, hi);
            }
        }
    };
    if sl > SCALED_EPSILON {
        circle(c, offset2);
        circle(d, offset2);
        let dx = (sv.0 / sl, sv.1 / sl);
        let dy = (-dx.1, dx.0);
        let p0 = (a.0 - c.0, a.1 - c.1);
        if let Some((lo, hi)) = liang_barsky(
            (p0.0 * dx.0 + p0.1 * dx.1, p0.0 * dy.0 + p0.1 * dy.1),
            (v0.0 * dx.0 + v0.1 * dx.1, v0.0 * dy.0 + v0.1 * dy.1),
            (0.0, -offset),
            (sl, offset),
        ) {
            extend(lo, hi);
        }
    } else {
        // The reference passes the radius here where the squared radius
        // is meant; a segment this short never comes from a fill line.
        circle((0.5 * (c.0 + d.0), 0.5 * (c.1 + d.1)), offset2);
    }
    (tmin <= tmax).then(|| {
        let l = lv.sqrt();
        (tmin * l, tmax * l)
    })
}

/// FillBase.cpp `mark_boundary_segments_touching_infill`: where a line
/// runs within `colliding` of the outline (leaving out `clip` at each of
/// its ends), that stretch of outline is taken: no end is joined or
/// anchored across it.
fn mark_touching(graph: &mut Graph, lines: &[Vec<Point2>], clip: f64, colliding: f64) {
    let ring_ends = std::mem::take(&mut graph.ring_ends);
    // The outline's edges, sorted into square cells.
    const CELL: f64 = 2_000_000.0;
    let mut cells: std::collections::HashMap<(i64, i64), Vec<(usize, usize)>> =
        std::collections::HashMap::new();
    for (ring, points) in graph.boundary.iter().enumerate() {
        if ring_ends.get(ring).is_none_or(|ends| ends.is_empty()) {
            continue;
        }
        let n = points.len();
        for edge in 0..n {
            let (a, b) = (as_f(points[edge]), as_f(points[(edge + 1) % n]));
            let (x0, x1) = (
                ((a.0.min(b.0) - colliding) / CELL).floor() as i64,
                ((a.0.max(b.0) + colliding) / CELL).floor() as i64,
            );
            let (y0, y1) = (
                ((a.1.min(b.1) - colliding) / CELL).floor() as i64,
                ((a.1.max(b.1) + colliding) / CELL).floor() as i64,
            );
            for x in x0..=x1 {
                for y in y0..=y1 {
                    cells.entry((x, y)).or_default().push((ring, edge));
                }
            }
        }
    }
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for line in lines {
        let (Some(start), Some(end)) = (clip_end(line, clip, true), clip_end(line, clip, false))
        else {
            continue;
        };
        if !(start.0 < end.0 || (start.0 == end.0 && start.1 < end.1)) {
            continue;
        }
        for index in start.0..=end.0 {
            let p1 = if index == start.0 {
                start.2
            } else {
                as_f(line[index])
            };
            let p2 = if index == end.0 {
                end.2
            } else {
                as_f(line[index + 1])
            };
            let reach = colliding + SCALED_EPSILON;
            let (lo, hi) = (
                (p1.0.min(p2.0) - reach, p1.1.min(p2.1) - reach),
                (p1.0.max(p2.0) + reach, p1.1.max(p2.1) + reach),
            );
            seen.clear();
            let (x0, x1) = ((lo.0 / CELL).floor() as i64, (hi.0 / CELL).floor() as i64);
            let (y0, y1) = ((lo.1 / CELL).floor() as i64, (hi.1 / CELL).floor() as i64);
            let mut candidates: Vec<(usize, usize)> = Vec::new();
            for x in x0..=x1 {
                for y in y0..=y1 {
                    if let Some(edges) = cells.get(&(x, y)) {
                        for &edge in edges {
                            if seen.insert(edge) {
                                candidates.push(edge);
                            }
                        }
                    }
                }
            }
            candidates.sort_unstable();
            for (ring, edge) in candidates {
                let points = &graph.boundary[ring];
                let n = points.len();
                let (a, b) = (as_f(points[edge]), as_f(points[(edge + 1) % n]));
                if a.0.max(b.0) < lo.0
                    || a.0.min(b.0) > hi.0
                    || a.1.max(b.1) < lo.1
                    || a.1.min(b.1) > hi.1
                {
                    continue;
                }
                let Some(interval) = thick_collision(a, b, p1, p2, colliding) else {
                    continue;
                };
                mark_interval(graph, &ring_ends[ring], ring, edge, interval);
            }
        }
    }
}

/// The marking of one stretch of outline (the visitor's body in
/// `mark_boundary_segments_touching_infill`).
fn mark_interval(
    graph: &mut Graph,
    ends: &[usize],
    ring: usize,
    edge: usize,
    interval: (f64, f64),
) {
    if ends.is_empty() {
        return;
    }
    let params = &graph.params[ring];
    let length = *params.last().expect("parametrised");
    let (p1, p2) = (params[edge], params[edge + 1]);
    let overlap1 = p2.min(p1 + interval.0);
    let overlap2 = p2.min(p1 + interval.1);
    let (low, high) = if ends.len() == 1 {
        (ends[0], ends[0])
    } else {
        let at_least = |x: f64| ends.iter().position(|&e| graph.cps[e].param >= x);
        let mut low = at_least(overlap1).map_or(ends[0], |i| ends[i]);
        let high = at_least(overlap2).map_or(ends[0], |i| ends[i]);
        if graph.cps[low].param != overlap1 {
            low = graph.cps[low].prev;
        }
        (low, high)
    };
    if graph.cps[low].next != high {
        let mut at = graph.cps[low].next;
        while at != high {
            graph.consume_prev(at);
            graph.consume_next(at);
            at = graph.cps[at].next;
        }
    }
    let trim = ccw(graph.cps[low].param, overlap1, length);
    graph.trim_next(low, trim);
    let trim = ccw(overlap2, graph.cps[high].param, length);
    graph.trim_prev(high, trim);
}

/// FillBase.cpp `take_cw_full` / `take_ccw_full`: the ring's points after
/// `start` up to and with `end`.
fn take_full(
    line: &mut Vec<Point2>,
    contour: &[Point2],
    start: usize,
    end: usize,
    clockwise: bool,
) {
    let n = contour.len();
    let step = |i: usize| {
        if clockwise {
            (i + n - 1) % n
        } else {
            (i + 1) % n
        }
    };
    let mut i = step(start);
    while i != end {
        line.push(contour[i]);
        i = step(i);
    }
    line.push(contour[i]);
}

/// FillBase.cpp `take`: `second` joined onto the end of `line` round the
/// ring from `start` to `end`, and the ring between them taken.
fn take(
    graph: &mut Graph,
    line: &mut Vec<Point2>,
    second: &[Point2],
    start: usize,
    end: usize,
    clockwise: bool,
) {
    let contour = graph.cps[start].contour;
    take_full(
        line,
        &graph.boundary[contour],
        graph.cps[start].point,
        graph.cps[end].point,
        clockwise,
    );
    line.extend_from_slice(&second[1..]);
    let (start, end) = if clockwise {
        (end, start)
    } else {
        (start, end)
    };
    if graph.cps[start].next != end {
        let mut at = graph.cps[start].next;
        while graph.cps[at].next != end {
            graph.consume_prev(at);
            graph.consume_next(at);
            at = graph.cps[at].next;
        }
    }
    graph.consume_next(start);
    graph.consume_prev(end);
}

/// FillBase.cpp `take_cw_limited` / `take_ccw_limited`.
fn take_limited_run(
    line: &mut Vec<Point2>,
    contour: &[Point2],
    params: &[f64],
    start: usize,
    end: usize,
    to_take: f64,
    clockwise: bool,
) -> f64 {
    let n = contour.len();
    let length = params[params.len() - 1];
    let p0 = params[start];
    let step = |i: usize| {
        if clockwise {
            (i + n - 1) % n
        } else {
            (i + 1) % n
        }
    };
    let mut i = step(start);
    let mut iprev = start;
    let mut lprev = 0.0;
    loop {
        let l = if clockwise {
            cw(p0, params[i], length)
        } else {
            ccw(p0, params[i], length)
        };
        if l >= to_take {
            let t = (to_take - lprev) / (l - lprev);
            line.push(lerp(contour[iprev], contour[i], t));
            return to_take;
        }
        line.push(contour[i]);
        if i == end {
            return l;
        }
        iprev = i;
        lprev = l;
        i = step(i);
    }
}

/// FillBase.cpp `take_limited`: an anchor from `start` towards `end`
/// along the ring, at most `max_length` long and stopping half a line
/// short of the next end.
fn take_limited(
    graph: &mut Graph,
    line: &mut Vec<Point2>,
    start: usize,
    end: usize,
    clockwise: bool,
    max_length: f64,
    half_width: f64,
) {
    let could = if clockwise {
        graph.could_take_prev(start)
    } else {
        graph.could_take_next(start)
    };
    if !could || line.is_empty() {
        return;
    }
    let contour_index = graph.cps[start].contour;
    let contour = graph.boundary[contour_index].clone();
    let params = graph.params[contour_index].clone();
    let at_start = line[0] == contour[graph.cps[start].point];
    let kept = if at_start {
        std::mem::take(line)
    } else {
        Vec::new()
    };
    let length = params[params.len() - 1];
    let mut to_go = max_length;
    graph.cps[start].consumed = true;
    if start == end {
        to_go = 0f64.max(to_go.min(length - half_width));
        to_go = to_go.min(if clockwise {
            graph.cps[start].not_taken_prev
        } else {
            graph.cps[start].not_taken_next
        });
        graph.consume_prev(start);
        graph.consume_next(start);
        if to_go > SCALED_EPSILON {
            let point = graph.cps[start].point;
            take_limited_run(line, &contour, &params, point, point, to_go, clockwise);
        }
    } else {
        let mut at = start;
        while at != end {
            let cp = graph.cps[at];
            let other = if clockwise { cp.prev } else { cp.next };
            let l = if clockwise {
                cw(cp.param, graph.cps[other].param, length)
            } else {
                ccw(cp.param, graph.cps[other].param, length)
            };
            to_go = to_go.min(if clockwise {
                cp.not_taken_prev
            } else {
                cp.not_taken_next
            });
            to_go = 0f64.max(to_go.min(l - half_width));
            if clockwise {
                graph.consume_prev(at);
            } else {
                graph.consume_next(at);
            }
            if l >= to_go {
                if to_go > SCALED_EPSILON {
                    if clockwise {
                        graph.trim_next(other, l - to_go);
                    } else {
                        graph.trim_prev(other, l - to_go);
                    }
                    take_limited_run(
                        line,
                        &contour,
                        &params,
                        cp.point,
                        graph.cps[other].point,
                        to_go,
                        clockwise,
                    );
                }
                break;
            }
            if clockwise {
                graph.trim_next(other, 0.0);
            } else {
                graph.trim_prev(other, 0.0);
            }
            take_full(line, &contour, cp.point, graph.cps[other].point, clockwise);
            to_go -= l;
            at = other;
        }
    }
    if at_start {
        line.reverse();
        line.extend(kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point2 {
        Point2::from_mm(x, y)
    }

    fn rectangle(w: f64, h: f64) -> Polygon {
        Polygon {
            points: vec![p(0.0, 0.0), p(w, 0.0), p(w, h), p(0.0, h)],
        }
    }

    fn mm(line: &[Point2]) -> Vec<(f64, f64)> {
        line.iter()
            .map(|q| (q.x as f64 / 1e6, q.y as f64 / 1e6))
            .collect()
    }

    #[test]
    fn rows_are_joined_shortest_arc_first_into_a_zigzag_and_anchored() {
        // Five rows down a 10 x 4 mm rectangle, 2 mm apart, each run from
        // its top. Every arc along the top and bottom is 2 mm: equal, they
        // are taken in the order of their ends (libc++ sorts so few by
        // insertion, which keeps ties in order), and each joins the path
        // of the lowest line it touches, which keeps that line's place.
        // The two ends left over are anchored 1 mm round the corner.
        let ring = rectangle(10.0, 4.0);
        let lines: Vec<Vec<Point2>> = [1.0, 3.0, 5.0, 7.0, 9.0]
            .iter()
            .map(|&x| vec![p(x, 4.0), p(x, 0.0)])
            .collect();
        let out = connect_infill(lines, &[&ring], 0.4e6, 1.0e6, 10.0e6);
        assert_eq!(out.len(), 1);
        assert_eq!(
            mm(&out[0]),
            [
                (10.0, 0.0),
                (9.0, 0.0),
                (9.0, 4.0),
                (7.0, 4.0),
                (7.0, 0.0),
                (5.0, 0.0),
                (5.0, 4.0),
                (3.0, 4.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (1.0, 4.0),
                (0.0, 4.0),
            ]
        );
    }

    #[test]
    fn ends_too_far_apart_are_anchored_and_not_joined() {
        // Two rows 6 mm apart with joins allowed only up to 5 mm: each
        // end takes a 1 mm anchor along the outline instead.
        let ring = rectangle(10.0, 4.0);
        let lines = vec![
            vec![p(2.0, 4.0), p(2.0, 0.0)],
            vec![p(8.0, 4.0), p(8.0, 0.0)],
        ];
        let out = connect_infill(lines, &[&ring], 0.4e6, 1.0e6, 5.0e6);
        assert_eq!(out.len(), 2);
        for line in &out {
            let length: f64 = line
                .windows(2)
                .map(|w| ((w[1].x - w[0].x) as f64).hypot((w[1].y - w[0].y) as f64))
                .sum();
            assert!((length - 6.0e6).abs() < 2.0, "{:?}", mm(line));
        }
    }

    #[test]
    fn no_join_runs_along_outline_a_line_lies_beside() {
        // Rows at x = 1 and 3, and a line along the bottom 0.2 mm off the
        // edge: the bottom edge is taken, so the rows meet over the top.
        let ring = rectangle(10.0, 4.0);
        let lines = vec![
            vec![p(1.0, 4.0), p(1.0, 0.0)],
            vec![p(0.0, 0.2), p(10.0, 0.2)],
            vec![p(3.0, 4.0), p(3.0, 0.0)],
        ];
        let out = connect_infill(lines, &[&ring], 0.4e6, 0.0, 10.0e6);
        let joined = out
            .iter()
            .find(|line| line.contains(&p(1.0, 0.0)))
            .expect("the first row");
        let pairs: Vec<(Point2, Point2)> = joined.windows(2).map(|w| (w[0], w[1])).collect();
        let along = |a: Point2, b: Point2| pairs.contains(&(a, b)) || pairs.contains(&(b, a));
        assert!(along(p(1.0, 4.0), p(3.0, 4.0)));
        assert!(!along(p(1.0, 0.0), p(3.0, 0.0)));
    }
}
