//! From an outline to its skeleton: the Voronoi diagram of the outline's
//! edges, cell by cell, as a graph of trapezoids, each lying against one
//! edge or corner of the outline with a rib from every node on the
//! skeleton out to it.
//!
//! A port of `SkeletalTrapezoidation::constructFromPolygons` with
//! `transferEdge`, `discretize` and `separatePointyQuadEndNodes`
//! (SkeletalTrapezoidation.cpp), and of what they use from
//! Geometry/VoronoiUtils.cpp. Now and then a diagram comes out broken
//! for an outline that has nothing wrong with it; as the reference
//! does, the diagram is then built for the outline turned by a few odd
//! angles in turn and turned back (Geometry/Voronoi.cpp
//! `try_to_repair_degenerated_voronoi_diagram`). Of the reference's
//! checks for a broken diagram the one that needs exact arithmetic
//! (`is_voronoi_diagram_planar_angle`) is not made.
//!
//! The reference's outlines are in their object's coordinates, measured
//! from the middle of the object's box (`PrintObject::m_center_offset`),
//! and it makes the diagram of them as they are. Here outlines are in
//! the plate's coordinates; the diagram is made of the outline measured
//! from the middle of its own box (`edge_medial::diagram`), its
//! vertices are rounded to the grid there, and the outline is turned
//! about that middle when it has to be turned. A diagram, repaired or
//! not, is then the same wherever the object stands. What is made of
//! the diagram below is still worked out in the plate's coordinates,
//! where a whole-number division of a sum is cut off downwards and the
//! reference's, left of or below its object's middle, upwards: a
//! skeleton's nodes can lie a nanometre or two from where they would
//! with the object standing elsewhere.

use std::collections::HashMap;
use std::f64::consts::PI;

use edge_geom::{Coord, Point2, Polygon};
use edge_medial::{MedialError, Source, VoronoiCell, VoronoiDiagram};

use crate::graph::{EdgeId, EdgeType, Graph, NodeId, length, nearest_on_line};

/// Edges of the graph shorter than this are taken out
/// (`collapseSmallEdges`'s default).
const SNAP_DIST: Coord = 5;

struct Builder<'a> {
    polys: &'a [Polygon],
    /// (polygon, point) of each segment's start, in the order given to
    /// the diagram.
    segments: Vec<(usize, usize)>,
    diagram: VoronoiDiagram,
    graph: Graph,
    /// The last graph edge made for each diagram edge.
    edge_of: HashMap<usize, EdgeId>,
    node_of: HashMap<usize, NodeId>,
    transitioning_angle: f64,
    discretization_step_size: Coord,
}

fn sub(a: Point2, b: Point2) -> Point2 {
    Point2 {
        x: a.x - b.x,
        y: a.y - b.y,
    }
}

fn dot(a: Point2, b: Point2) -> i64 {
    a.x * b.x + a.y * b.y
}

/// `Point::rotated(cos, sin)`.
fn rotated(x: i64, y: i64, cos: f64, sin: f64) -> Point2 {
    let (x, y) = (x as f64, y as f64);
    Point2 {
        x: (cos * x - sin * y).round() as Coord,
        y: (cos * y + sin * x).round() as Coord,
    }
}

/// Whether `query` lies inside the corner the outline turns at `b`,
/// coming from `a` and going on to `c` (Geometry.cpp
/// `is_point_inside_polygon_corner`).
fn inside_corner(a: Point2, b: Point2, c: Point2, query: Point2) -> bool {
    let unit = |p: Point2| {
        let (x, y) = ((p.x - b.x) as f64, (p.y - b.y) as f64);
        let norm = x.hypot(y);
        (x / norm, y / norm)
    };
    let (ba, bc, bq) = (unit(a), unit(c), unit(query));
    let bq_left = (-bq.1, bq.0);
    let a_on_normal = ba.0 * bq_left.0 + ba.1 * bq_left.1;
    let c_on_normal = bc.0 * bq_left.0 + bc.1 * bq_left.1;
    if (a_on_normal > 0.0 && c_on_normal <= 0.0) || (a_on_normal <= 0.0 && c_on_normal > 0.0) {
        a_on_normal > 0.0
    } else {
        let a_on_bq = ba.0 * bq.0 + ba.1 * bq.1;
        let c_on_bq = bc.0 * bq.0 + bc.1 * bq.1;
        (a_on_normal > 0.0 && c_on_bq < a_on_bq) || (a_on_normal <= 0.0 && c_on_bq >= a_on_bq)
    }
}

impl Builder<'_> {
    /// Whether the diagram is broken in one of the ways the reference
    /// knows of (Voronoi.cpp `detect_known_voronoi_edge_issues` and
    /// `detect_known_voronoi_cell_issues`).
    fn has_known_issue(&self) -> bool {
        let finite = |vertex: usize| self.diagram.vertices[vertex].iter().all(|c| c.is_finite());
        for edge in &self.diagram.edges {
            let (Some(v0), Some(v1)) = (edge.vertex0, edge.vertex1) else {
                continue;
            };
            if !finite(v0) || !finite(v1) {
                return true;
            }
            // A parabola with no point to be the focus of it.
            if edge.curved
                && self.diagram.cells[edge.cell].source == Source::Segment
                && self.diagram.cells[self.diagram.edges[edge.twin].cell].source == Source::Segment
            {
                return true;
            }
        }
        for cell in &self.diagram.cells {
            if cell.incident_edge.is_none() || cell.source != Source::Segment {
                continue;
            }
            // A vertex is missing where the cell does not reach both
            // ends of its edge of the outline.
            let Some((begin, end)) = self.segment_cell_range(cell) else {
                return true;
            };
            let (from, to) = self.segment(cell.segment);
            let along = ((to.x - from.x) as f64, (to.y - from.y) as f64);
            // The edge's start measured as the diagram's vertices are.
            let [origin_x, origin_y] = self.diagram.origin;
            let start = ((from.x - origin_x) as f64, (from.y - origin_y) as f64);
            let mut edge = begin;
            let mut steps = 0;
            while edge != end {
                let Some(v1) = self.diagram.edges[edge].vertex1 else {
                    return true;
                };
                if self.diagram.edges[edge].vertex0.is_none() {
                    return true;
                }
                // No edge of the cell may lie beyond the outline.
                let [x, y] = self.diagram.vertices[v1];
                if along.0 * (y - start.1) - along.1 * (x - start.0) < 0.0 {
                    return true;
                }
                edge = self.diagram.edges[edge].next;
                steps += 1;
                if steps > self.diagram.edges.len() {
                    return true;
                }
            }
        }
        false
    }

    fn point(&self, (poly, at): (usize, usize)) -> Point2 {
        let points = &self.polys[poly].points;
        points[at % points.len()]
    }

    fn segment(&self, index: usize) -> (Point2, Point2) {
        let (poly, at) = self.segments[index];
        (self.point((poly, at)), self.point((poly, at + 1)))
    }

    /// A vertex of the diagram on the grid (`VoronoiUtils::to_point`).
    fn vertex(&self, vertex: usize) -> Point2 {
        self.diagram.point(vertex)
    }

    /// Both ends of a diagram edge, where it has both.
    fn ends(&self, edge: usize) -> Option<(Point2, Point2)> {
        let edge = &self.diagram.edges[edge];
        let (v0, v1) = (edge.vertex0?, edge.vertex1?);
        let finite = |v: usize| self.diagram.vertices[v].iter().all(|c| c.is_finite());
        (finite(v0) && finite(v1)).then(|| (self.vertex(v0), self.vertex(v1)))
    }

    fn source_point(&self, cell: &VoronoiCell) -> Point2 {
        let (from, to) = self.segment(cell.segment);
        match cell.source {
            Source::SegmentEnd => to,
            _ => from,
        }
    }

    /// The edges of a segment's cell from the one leaving the segment's
    /// end to the one arriving at its start
    /// (`compute_segment_cell_range`).
    fn segment_cell_range(&self, cell: &VoronoiCell) -> Option<(usize, usize)> {
        let (from, to) = self.segment(cell.segment);
        let first = cell.incident_edge?;
        let (mut begin, mut end) = (None, None);
        let mut seen_possible_start = false;
        let mut after_start = false;
        let mut end_set_before_start = false;
        let mut edge = first;
        loop {
            if let Some((v0, v1)) = self.ends(edge) {
                if v0 == to && !after_start {
                    // The last edge that starts at the segment's end.
                    begin = Some(edge);
                    seen_possible_start = true;
                } else if seen_possible_start {
                    after_start = true;
                }
                if v1 == from && (end.is_none() || end_set_before_start) {
                    end_set_before_start = !after_start;
                    end = Some(edge);
                }
            }
            edge = self.diagram.edges[edge].next;
            if edge == first {
                break;
            }
        }
        match (begin, end) {
            (Some(begin), Some(end)) if begin != end => Some((begin, end)),
            _ => None,
        }
    }

    /// The edges of a corner's cell from the one leaving the corner to
    /// the one coming back to it; none for a cell outside the outline
    /// (`compute_point_cell_range`).
    fn point_cell_range(&self, cell: &VoronoiCell) -> Option<(usize, usize)> {
        let source = self.source_point(cell);
        let first = cell.incident_edge?;
        // An edge without end is outside the outline.
        let (v0, v1) = self.ends(first)?;
        let (poly, at) = self.segments[cell.segment];
        let count = self.polys[poly].points.len();
        let at = match cell.source {
            Source::SegmentEnd => (at + 1) % count,
            _ => at,
        };
        let query = if v0 == source { v1 } else { v0 };
        if !inside_corner(
            self.point((poly, at + count - 1)),
            self.point((poly, at)),
            self.point((poly, at + 1)),
            query,
        ) {
            return None;
        }
        let (mut begin, mut end) = (None, None);
        let mut edge = first;
        loop {
            if let Some((_, v1)) = self.ends(edge)
                && v1 == source
            {
                begin = Some(self.diagram.edges[edge].next);
                end = Some(edge);
            }
            edge = self.diagram.edges[edge].next;
            if edge == first {
                break;
            }
        }
        match (begin, end) {
            (Some(begin), Some(end)) if begin != end => Some((begin, end)),
            _ => None,
        }
    }

    fn make_node(&mut self, vertex: usize, p: Point2) -> NodeId {
        if let Some(&node) = self.node_of.get(&vertex) {
            return node;
        }
        let node = self.graph.new_node_front(p);
        self.node_of.insert(vertex, node);
        node
    }

    /// A curved or narrowing edge of the diagram as short straight ones
    /// (`SkeletalTrapezoidation::discretize`).
    fn discretize(&self, vd_edge: usize, start: Point2, end: Point2) -> Vec<Point2> {
        let edge = &self.diagram.edges[vd_edge];
        let left = &self.diagram.cells[edge.cell];
        let right = &self.diagram.cells[self.diagram.edges[edge.twin].cell];
        let point_left = left.source != Source::Segment;
        let point_right = right.source != Source::Segment;
        if (!point_left && !point_right) || !edge.primary {
            return vec![start, end];
        }
        if point_left != point_right {
            // A parabola, between a point and a line.
            let (point_cell, segment_cell) = if point_left {
                (left, right)
            } else {
                (right, left)
            };
            return discretize_parabola(
                self.source_point(point_cell),
                self.segment(segment_cell.segment),
                start,
                end,
                self.discretization_step_size,
                self.transitioning_angle as f32,
            );
        }
        // Straight, between two points, but the part narrows towards the
        // line through them, so it may want different lines along the way.
        let left_point = self.source_point(left);
        let right_point = self.source_point(right);
        let d = length(right_point, left_point);
        let middle = Point2 {
            x: (left_point.x + right_point.x) / 2,
            y: (left_point.y + right_point.y) / 2,
        };
        let across = sub(right_point, left_point);
        let x_axis_dir = Point2 {
            x: -across.y,
            y: across.x,
        };
        let x_axis_length = length(x_axis_dir, Point2 { x: 0, y: 0 });
        let projected_x = |from: Point2| dot(sub(from, middle), x_axis_dir) / x_axis_length;
        let (start_x, end_x) = (projected_x(start), projected_x(end));

        // How far along the edge the markings at its ends reach.
        let bound = (0.5 / ((PI - self.transitioning_angle) * 0.5).tan()) as f32;
        let mut marking_start_x = ((-d) as f32 * bound) as i64;
        let mut marking_end_x = (d as f32 * bound) as i64;
        let along = |x: i64| Point2 {
            x: middle.x + x_axis_dir.x * x / x_axis_length,
            y: middle.y + x_axis_dir.y * x / x_axis_length,
        };
        let mut marking_start = along(marking_start_x);
        let mut marking_end = along(marking_end_x);
        let mut direction = 1;
        if start_x > end_x {
            direction = -1;
            std::mem::swap(&mut marking_start, &mut marking_end);
            std::mem::swap(&mut marking_start_x, &mut marking_end_x);
        }

        let (a, b) = (start, end);
        let mut points = vec![a];
        let mut add_marking_start = marking_start_x * direction > start_x * direction;
        let mut add_marking_end = marking_end_x * direction > start_x * direction;

        let ab = sub(b, a);
        let ab_size = length(b, a);
        let step_size = self.discretization_step_size;
        let mut step_count = (ab_size + step_size / 2) / step_size;
        if step_count % 2 == 1 {
            // A point in the middle, always.
            step_count += 1;
        }
        for step in 1..step_count {
            let here = Point2 {
                x: a.x + ab.x * step / step_count,
                y: a.y + ab.y * step / step_count,
            };
            let x_here = projected_x(here);
            if add_marking_start && marking_start_x * direction < x_here * direction {
                points.push(marking_start);
                add_marking_start = false;
            }
            if add_marking_end && marking_end_x * direction < x_here * direction {
                points.push(marking_end);
                add_marking_end = false;
            }
            points.push(here);
        }
        if add_marking_end && marking_end_x * direction < end_x * direction {
            points.push(marking_end);
        }
        points.push(b);
        points
    }

    /// Copies one edge of the diagram into the graph, after `prev_edge`,
    /// with a rib wherever it is cut in pieces. The other side's pieces
    /// are followed where that side was copied first.
    fn transfer_edge(
        &mut self,
        from: Point2,
        to: Point2,
        vd_edge: usize,
        prev_edge: &mut Option<EdgeId>,
        start_source: Point2,
        end_source: Point2,
    ) {
        let vd = self.diagram.edges[vd_edge];
        if let Some(&source_twin) = self.edge_of.get(&vd.twin) {
            let end_node = vd
                .vertex1
                .and_then(|vertex| self.node_of.get(&vertex).copied());
            let mut twin = source_twin;
            loop {
                let edge = self.graph.new_edge_front(EdgeType::Normal);
                self.graph.edge[edge].from = self.graph.edge[twin].to;
                self.graph.edge[edge].to = self.graph.edge[twin].from;
                self.graph.edge[edge].twin = Some(twin);
                self.graph.edge[twin].twin = Some(edge);
                let edge_from = self.graph.edge[edge].from;
                self.graph.node[edge_from].incident_edge = Some(edge);
                if let Some(prev) = *prev_edge {
                    self.graph.edge[edge].prev = Some(prev);
                    self.graph.edge[prev].next = Some(edge);
                }
                *prev_edge = Some(edge);
                if Some(self.graph.edge[edge].to) == end_node {
                    return;
                }
                // The piece before, on the other side: back over the rib.
                let before = self.graph.edge[twin]
                    .prev
                    .and_then(|forth| self.graph.edge[forth].twin)
                    .and_then(|back| self.graph.edge[back].prev);
                let Some(before) = before else { return };
                let mut last = edge;
                self.graph.make_rib(&mut last, start_source, end_source);
                *prev_edge = Some(last);
                twin = before;
            }
        }

        let (Some(vertex0), Some(vertex1)) = (vd.vertex0, vd.vertex1) else {
            return;
        };
        let discretized = self.discretize(vd_edge, self.vertex(vertex0), self.vertex(vertex1));
        let mut v0 = match *prev_edge {
            Some(prev) => self.graph.edge[prev].to,
            None => self.make_node(vertex0, from),
        };
        for (index, &p1) in discretized.iter().enumerate().skip(1) {
            let last = index == discretized.len() - 1;
            let v1 = if last {
                self.make_node(vertex1, to)
            } else {
                self.graph.new_node_front(p1)
            };
            let edge = self.graph.new_edge_front(EdgeType::Normal);
            self.graph.edge[edge].from = v0;
            self.graph.edge[edge].to = v1;
            self.graph.node[v0].incident_edge = Some(edge);
            if let Some(prev) = *prev_edge {
                self.graph.edge[edge].prev = Some(prev);
                self.graph.edge[prev].next = Some(edge);
            }
            *prev_edge = Some(edge);
            v0 = v1;
            // The rib at the edge's own end is the caller's to make.
            if !last {
                let mut at = edge;
                self.graph.make_rib(&mut at, start_source, end_source);
                *prev_edge = Some(at);
            }
        }
        if let Some(prev) = *prev_edge {
            self.edge_of.insert(vd_edge, prev);
        }
    }

    /// A node where two cells' first edges both start is made two
    /// (`separatePointyQuadEndNodes`).
    fn separate_pointy_quad_end_nodes(&mut self) {
        let mut visited: std::collections::HashSet<NodeId> = Default::default();
        for quad_start in self.graph.edges() {
            if self.graph.edge[quad_start].prev.is_some() {
                continue;
            }
            let from = self.graph.edge[quad_start].from;
            if visited.insert(from) {
                continue;
            }
            let new_node = self.graph.copy_node_back(from);
            self.graph.node[new_node].incident_edge = Some(quad_start);
            self.graph.edge[quad_start].from = new_node;
            if let Some(twin) = self.graph.edge[quad_start].twin {
                self.graph.edge[twin].to = new_node;
            }
        }
    }
}

/// The angles an outline is turned by, in turn, when its diagram comes
/// out broken.
const FIX_ANGLES: [f64; 4] = [PI / 6.0, PI / 5.0, PI / 7.0, PI / 11.0];

/// A builder that is only asked about its diagram.
fn checker<'a>(
    polys: &'a [Polygon],
    segments: &[(usize, usize)],
    diagram: VoronoiDiagram,
) -> Builder<'a> {
    Builder {
        polys,
        segments: segments.to_vec(),
        diagram,
        graph: Graph::default(),
        edge_of: HashMap::new(),
        node_of: HashMap::new(),
        transitioning_angle: 0.0,
        discretization_step_size: 1,
    }
}

/// The diagram of `polys`' edges, repaired if it came out broken or did
/// not come out at all.
fn sound_diagram(
    polys: &[Polygon],
    segments: &[(usize, usize)],
) -> Result<VoronoiDiagram, MedialError> {
    let ends_of = |polys: &[Polygon]| -> Vec<[i64; 4]> {
        segments
            .iter()
            .map(|&(poly, at)| {
                let points = &polys[poly].points;
                let (from, to) = (points[at], points[(at + 1) % points.len()]);
                [from.x, from.y, to.x, to.y]
            })
            .collect()
    };
    // The reference's library hands back a diagram whatever happened, and
    // a broken one is found by the checks. This one gives up on some of
    // those with an error instead, so an error is one more way for the
    // first attempt to have gone wrong, and the outline is turned for it
    // as for a flaw that was found. The error comes back only when no
    // attempt gave a diagram at all.
    let ends = ends_of(polys);
    let mut best = match edge_medial::diagram(&ends) {
        Ok(diagram) => {
            let first = checker(polys, segments, diagram);
            if !first.has_known_issue() {
                return Ok(first.diagram);
            }
            Ok(first.diagram)
        }
        Err(error) => Err(error),
    };
    // The outline is turned about the point its diagram is measured
    // from, the middle of the box round it. The reference turns its
    // outline about the origin of its coordinates, which is the middle
    // of the object's box; about the plate's origin the turned points
    // would be rounded to the grid elsewhere for every place the object
    // could stand.
    let Some([middle_x, middle_y]) = edge_medial::diagram_origin(&ends) else {
        return best;
    };
    for angle in FIX_ANGLES {
        let (sin, cos) = angle.sin_cos();
        let turned: Vec<Polygon> = polys
            .iter()
            .map(|poly| Polygon {
                points: poly
                    .points
                    .iter()
                    .map(|p| rotated(p.x - middle_x, p.y - middle_y, cos, sin))
                    .collect(),
            })
            .collect();
        let Ok(diagram) = edge_medial::diagram(&ends_of(&turned)) else {
            continue;
        };
        let turned_check = checker(&turned, segments, diagram);
        let broken = turned_check.has_known_issue();
        // The vertices that are ends of the outline's edges go back to
        // exactly where those are; the rest are turned back.
        let mut exact: Vec<Option<Point2>> = vec![None; turned_check.diagram.vertices.len()];
        for cell in &turned_check.diagram.cells {
            if cell.incident_edge.is_none() || cell.source != Source::Segment {
                continue;
            }
            let Some((begin, end)) = turned_check.segment_cell_range(cell) else {
                continue;
            };
            let (poly, at) = segments[cell.segment];
            let points = &polys[poly].points;
            let (from, to) = (points[at], points[(at + 1) % points.len()]);
            if let Some(vertex) = turned_check.diagram.edges[end].vertex1 {
                exact[vertex].get_or_insert(from);
            }
            if let Some(vertex) = turned_check.diagram.edges[begin].vertex0 {
                exact[vertex].get_or_insert(to);
            }
        }
        // The turned outline's diagram is measured from the middle of
        // the turned outline's own box; turned back, from the middle
        // the outline was turned about.
        let mut diagram = turned_check.diagram;
        let [turned_x, turned_y] = diagram.origin.map(|c| c as f64);
        for (vertex, exact) in diagram.vertices.iter_mut().zip(exact) {
            *vertex = match exact {
                Some(point) => [(point.x - middle_x) as f64, (point.y - middle_y) as f64],
                None => {
                    let (x, y) = (vertex[0] + turned_x, vertex[1] + turned_y);
                    [cos * x + sin * y, cos * y - sin * x]
                }
            };
        }
        diagram.origin = [middle_x, middle_y];
        best = Ok(diagram);
        if !broken {
            break;
        }
    }
    best
}

/// VoronoiUtils.cpp `discretize_parabola`: the curve between `start` and
/// `end` that is as far from `source_point` as from `source_segment`, in
/// steps of about `step_size`, with a point at its apex and at either
/// marking where they fall between.
fn discretize_parabola(
    source_point: Point2,
    source_segment: (Point2, Point2),
    start: Point2,
    end: Point2,
    step_size: Coord,
    transitioning_angle: f32,
) -> Vec<Point2> {
    let (a, b) = source_segment;
    let ab = sub(b, a);
    let ab_size = length(b, a);
    let sx = dot(sub(start, a), ab) / ab_size;
    let ex = dot(sub(end, a), ab) / ab_size;
    let sxex = ex - sx;
    let px = dot(sub(source_point, a), ab) / ab_size;

    let pxx = nearest_on_line(a, b, source_point);
    let ppxx = sub(pxx, source_point);
    let d = length(pxx, source_point);
    if d == 0 {
        return vec![start, end];
    }
    let (rot_x, rot_y) = (-ppxx.y as f64, ppxx.x as f64);
    let norm = rot_x.hypot(rot_y);
    let (cos, sin) = (rot_x / norm, rot_y / norm);

    let marking_bound = (f64::from(transitioning_angle) * 0.5).atan();
    let mut msx = (-marking_bound * d as f64) as i64;
    let mut mex = (marking_bound * d as f64) as i64;
    let marking_h = msx * msx / (2 * d) + d / 2;
    let place = |x: i64, y: i64| {
        let turned = rotated(x, y, cos, sin);
        Point2 {
            x: turned.x + pxx.x,
            y: turned.y + pxx.y,
        }
    };
    let mut marking_start = place(msx, marking_h);
    let mut marking_end = place(mex, marking_h);
    let dir: i64 = if sx > ex { -1 } else { 1 };
    if dir < 0 {
        std::mem::swap(&mut marking_start, &mut marking_end);
        std::mem::swap(&mut msx, &mut mex);
    }
    let mut add_marking_start = msx * dir > (sx - px) * dir && msx * dir < (ex - px) * dir;
    let mut add_marking_end = mex * dir > (sx - px) * dir && mex * dir < (ex - px) * dir;
    let apex = place(0, d / 2);
    let mut add_apex = (sx - px) * dir < 0 && (ex - px) * dir > 0;

    let step_count = ((ex - sx).abs() as f64 / step_size as f64).round() as i64;
    let mut points = vec![start];
    for step in 1..step_count {
        let x = sx + sxex * step / step_count - px;
        let y = x * x / (2 * d) + d / 2;
        if add_marking_start && msx * dir < x * dir {
            points.push(marking_start);
            add_marking_start = false;
        }
        if add_apex && x * dir > 0 {
            points.push(apex);
            add_apex = false;
        }
        if add_marking_end && mex * dir < x * dir {
            points.push(marking_end);
            add_marking_end = false;
        }
        points.push(place(x, y));
    }
    if add_apex {
        points.push(apex);
    }
    if add_marking_end {
        points.push(marking_end);
    }
    points.push(end);
    points
}

/// The skeleton of `polys`: outlines wound counter-clockwise and holes
/// clockwise, none touching or crossing itself or another, and none
/// with three points in a row on one line.
pub(crate) fn from_polygons(
    polys: &[Polygon],
    transitioning_angle: f64,
    discretization_step_size: Coord,
) -> Result<Graph, MedialError> {
    let segments: Vec<(usize, usize)> = polys
        .iter()
        .enumerate()
        .flat_map(|(poly, polygon)| (0..polygon.points.len()).map(move |at| (poly, at)))
        .collect();
    let diagram = sound_diagram(polys, &segments)?;
    let mut builder = Builder {
        polys,
        segments,
        diagram,
        graph: Graph::default(),
        edge_of: HashMap::new(),
        node_of: HashMap::new(),
        transitioning_angle,
        discretization_step_size,
    };

    for index in 0..builder.diagram.cells.len() {
        let cell = builder.diagram.cells[index];
        if cell.incident_edge.is_none() {
            continue;
        }
        let (start_source, end_source, range) = if cell.source == Source::Segment {
            let (from, to) = builder.segment(cell.segment);
            (to, from, builder.segment_cell_range(&cell))
        } else {
            let source = builder.source_point(&cell);
            (source, source, builder.point_cell_range(&cell))
        };
        let Some((starting, ending)) = range else {
            continue;
        };
        let Some((_, starting_v1)) = builder.ends(starting) else {
            continue;
        };

        let mut prev_edge = None;
        builder.transfer_edge(
            start_source,
            starting_v1,
            starting,
            &mut prev_edge,
            start_source,
            end_source,
        );
        let Some(mut prev) = prev_edge else { continue };
        if let Some(&starting_node) = builder.diagram.edges[starting]
            .vertex0
            .and_then(|vertex| builder.node_of.get(&vertex))
        {
            builder.graph.node[starting_node].distance_to_boundary = 0;
        }
        builder.graph.make_rib(&mut prev, start_source, end_source);

        let mut vd_edge = builder.diagram.edges[starting].next;
        let mut steps = 0;
        while vd_edge != ending {
            let Some((v1, v2)) = builder.ends(vd_edge) else {
                break;
            };
            let mut prev_edge = Some(prev);
            builder.transfer_edge(v1, v2, vd_edge, &mut prev_edge, start_source, end_source);
            prev = prev_edge.unwrap_or(prev);
            builder.graph.make_rib(&mut prev, start_source, end_source);
            vd_edge = builder.diagram.edges[vd_edge].next;
            steps += 1;
            if steps > builder.diagram.edges.len() {
                break;
            }
        }

        let Some((ending_v0, _)) = builder.ends(ending) else {
            continue;
        };
        let mut prev_edge = Some(prev);
        builder.transfer_edge(
            ending_v0,
            end_source,
            ending,
            &mut prev_edge,
            start_source,
            end_source,
        );
        let last = prev_edge.unwrap_or(prev);
        let last_to = builder.graph.edge[last].to;
        builder.graph.node[last_to].distance_to_boundary = 0;
    }

    builder.separate_pointy_quad_end_nodes();
    builder.graph.collapse_small_edges(SNAP_DIST);

    // From a node's `incident_edge` every edge at the node is reached
    // going forwards only if it is the first of its cell.
    for edge in builder.graph.edges() {
        if builder.graph.edge[edge].prev.is_none() {
            let from = builder.graph.edge[edge].from;
            builder.graph.node[from].incident_edge = Some(edge);
        }
    }
    Ok(builder.graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(points: &[(f64, f64)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
        }
    }

    #[test]
    fn every_edge_of_a_skeleton_has_its_other_side() {
        // A square with a notch, and a hole in it.
        let outline = [
            poly(&[
                (0.0, 0.0),
                (20.0, 0.0),
                (20.0, 20.0),
                (12.0, 20.0),
                (12.0, 15.0),
                (8.0, 15.0),
                (8.0, 20.0),
                (0.0, 20.0),
            ]),
            poly(&[(4.0, 4.0), (4.0, 8.0), (8.0, 8.0), (8.0, 4.0)]),
        ];
        let graph = from_polygons(&outline, 10f64.to_radians(), 800_000).unwrap();
        let edges = graph.edges();
        assert!(edges.len() > 40);
        for &edge in &edges {
            let twin = graph.edge[edge].twin.expect("a twin");
            assert_eq!(graph.edge[twin].twin, Some(edge));
            assert_eq!(graph.edge[twin].from, graph.edge[edge].to);
            // A cell starts and ends on the outline.
            if graph.edge[edge].prev.is_none() {
                assert_eq!(graph.node[graph.edge[edge].from].distance_to_boundary, 0);
            }
            if graph.edge[edge].next.is_none() {
                assert_eq!(graph.node[graph.edge[edge].to].distance_to_boundary, 0);
            }
        }
        // The furthest any point is from the outline: 6 mm, between the
        // hole and the far sides.
        let deepest = graph
            .nodes()
            .into_iter()
            .map(|node| graph.node[node].distance_to_boundary)
            .max()
            .unwrap();
        assert!((5_000_000..=8_500_000).contains(&deepest), "{deepest}");
    }

    /// Two edges of the Voron design cube's outline, in the plate's
    /// coordinates: upright, 5.35 mm apart, one starting a nanometre
    /// above the other. They are what is left when the outline the
    /// sweep goes wrong on is cut down to the least that still does.
    const JOG: [[i64; 4]; 2] = [
        [65_788_257, 76_173_144, 65_788_257, 78_978_540],
        [60_438_147, 76_173_145, 60_438_147, 78_978_540],
    ];

    /// An outline of the Voron design cube with the notches along its
    /// sides, as the walls are given it, in the plate's coordinates
    /// (nanometres): the one in `edge-medial`'s `tests/deterministic.rs`.
    /// Two of its edges are a nanometre out of line, and swept where it
    /// lies, 49 to 79 mm from the origin, its diagram came out broken.
    const VORON_OUTLINE: [(i64, i64); 44] = [
        (60_186_098, 55_673_323),
        (63_132_079, 55_673_322),
        (63_132_079, 49_021_460),
        (65_535_743, 49_021_460),
        (65_535_739, 55_673_323),
        (68_482_148, 55_673_323),
        (68_482_148, 49_021_460),
        (78_978_540, 49_021_460),
        (78_978_540, 54_474_779),
        (76_978_540, 54_474_779),
        (76_978_540, 60_211_277),
        (78_978_540, 60_211_277),
        (78_978_540, 63_071_416),
        (76_978_540, 63_071_416),
        (76_978_540, 65_560_936),
        (78_978_540, 65_560_935),
        (78_978_540, 68_421_487),
        (76_978_540, 68_421_489),
        (76_978_540, 73_766_928),
        (78_978_540, 73_766_928),
        (78_978_540, 78_978_540),
        (67_813_901, 78_978_540),
        (67_813_901, 72_326_676),
        (64_867_921, 72_326_678),
        (64_867_921, 78_978_540),
        (62_464_257, 78_978_540),
        (62_464_258, 72_326_677),
        (59_517_853, 72_326_677),
        (59_517_853, 78_978_540),
        (49_021_460, 78_978_540),
        (49_021_460, 73_525_221),
        (51_021_460, 73_525_221),
        (51_021_460, 67_788_721),
        (49_021_460, 67_788_720),
        (49_021_460, 64_928_582),
        (51_021_460, 64_928_585),
        (51_021_460, 62_439_064),
        (49_021_460, 62_439_063),
        (49_021_460, 59_578_511),
        (51_021_460, 59_578_511),
        (51_021_460, 54_233_072),
        (49_021_460, 54_233_072),
        (49_021_460, 49_021_460),
        (60_186_098, 49_021_460),
    ];

    /// The round hole through it.
    const VORON_HOLE: [(i64, i64); 56] = [
        (63_185_132, 57_428_894),
        (62_479_353, 57_555_507),
        (61_791_068, 57_757_827),
        (61_128_455, 58_033_576),
        (60_168_549, 58_599_637),
        (59_587_813, 59_062_746),
        (59_062_746, 59_587_813),
        (58_599_781, 60_168_355),
        (58_204_739, 60_797_062),
        (57_882_558, 61_466_082),
        (57_637_320, 62_166_923),
        (57_472_087, 62_890_864),
        (57_388_950, 63_628_744),
        (57_388_950, 64_371_254),
        (57_472_089, 65_109_156),
        (57_637_318, 65_833_073),
        (57_882_558, 66_533_916),
        (58_204_739, 67_202_936),
        (58_599_781, 67_831_643),
        (59_062_754, 68_412_194),
        (59_587_805, 68_937_240),
        (60_168_352, 69_400_219),
        (60_797_058, 69_795_257),
        (61_466_082, 70_117_440),
        (62_166_923, 70_362_678),
        (62_890_866, 70_527_907),
        (63_628_931, 70_611_064),
        (64_250_586, 70_616_521),
        (64_814_920, 70_571_093),
        (65_520_658, 70_444_485),
        (66_208_934, 70_242_166),
        (66_871_512, 69_966_435),
        (67_522_827, 69_606_560),
        (68_128_432, 69_176_863),
        (68_682_081, 68_682_081),
        (69_176_863, 68_128_432),
        (69_606_561, 67_522_827),
        (69_965_734, 66_872_952),
        (70_249_892, 66_186_930),
        (70_455_451, 65_473_417),
        (70_579_831, 64_741_391),
        (70_621_465, 63_999_999),
        (70_579_831, 63_258_609),
        (70_455_451, 62_526_578),
        (70_249_894, 61_813_073),
        (69_965_734, 61_127_046),
        (69_606_561, 60_477_171),
        (69_176_871, 59_871_581),
        (68_682_082, 59_317_914),
        (68_128_424, 58_823_130),
        (67_522_827, 58_393_437),
        (66_872_952, 58_034_264),
        (66_186_930, 57_750_106),
        (65_473_400, 57_544_543),
        (64_461_962, 57_394_906),
        (63_810_719, 57_381_756),
    ];

    fn voron(x: i64, y: i64) -> Vec<Polygon> {
        [&VORON_OUTLINE[..], &VORON_HOLE[..]]
            .iter()
            .map(|ring| Polygon {
                points: ring
                    .iter()
                    .map(|&(px, py)| Point2 {
                        x: px + x,
                        y: py + y,
                    })
                    .collect(),
            })
            .collect()
    }

    fn segments_of(polys: &[Polygon]) -> Vec<(usize, usize)> {
        polys
            .iter()
            .enumerate()
            .flat_map(|(poly, polygon)| (0..polygon.points.len()).map(move |at| (poly, at)))
            .collect()
    }

    #[test]
    fn an_outline_needs_no_repair_wherever_it_stands() {
        // Where it lay on the plate when its diagram came out broken,
        // and elsewhere: the first diagram is sound, and is the same
        // diagram, measured from the middle of the outline's box.
        let home = voron(0, 0);
        let segments = segments_of(&home);
        let ends = |polys: &[Polygon]| -> Vec<[i64; 4]> {
            segments
                .iter()
                .map(|&(poly, at)| {
                    let points = &polys[poly].points;
                    let (from, to) = (points[at], points[(at + 1) % points.len()]);
                    [from.x, from.y, to.x, to.y]
                })
                .collect()
        };
        let first = checker(
            &home,
            &segments,
            edge_medial::diagram(&ends(&home)).unwrap(),
        );
        assert!(!first.has_known_issue());
        assert_eq!(first.diagram.origin, [64_000_000, 64_000_000]);
        let at_home = sound_diagram(&home, &segments).unwrap();
        assert_eq!(at_home, first.diagram);
        for (x, y) in [
            (-64_000_000, -64_000_000),
            (71_500_000, 72_000_000),
            (-250_000_000, 1),
        ] {
            let away = voron(x, y);
            let mut found = sound_diagram(&away, &segments).unwrap();
            assert_eq!(found.origin, [64_000_000 + x, 64_000_000 + y]);
            found.origin = at_home.origin;
            assert_eq!(found, at_home, "moved by ({x}, {y})");
        }
        // The skeleton is made of it without trouble.
        let graph = from_polygons(&home, 10f64.to_radians(), 800_000).unwrap();
        assert!(graph.edges().len() > 400);
    }

    #[test]
    fn an_outline_the_library_gives_up_on_is_turned_like_a_broken_one() {
        // The two edges and the same two through the origin, so that
        // they lie where they lie wherever a diagram is measured from.
        let lines: Vec<Polygon> = JOG
            .iter()
            .flat_map(|&[x0, y0, x1, y1]| [[x0, y0, x1, y1], [-x0, -y0, -x1, -y1]])
            .map(|[x0, y0, x1, y1]| Polygon {
                points: vec![Point2 { x: x0, y: y0 }, Point2 { x: x1, y: y1 }],
            })
            .collect();
        let segments: Vec<(usize, usize)> = (0..lines.len()).map(|line| (line, 0)).collect();
        let ends: Vec<[i64; 4]> = lines
            .iter()
            .map(|line| {
                let (from, to) = (line.points[0], line.points[1]);
                [from.x, from.y, to.x, to.y]
            })
            .collect();
        // What this stands on: as they lie, the library has no diagram
        // for them. Should it come to have one, this needs other edges.
        assert!(matches!(
            edge_medial::diagram(&ends),
            Err(MedialError::Voronoi(_))
        ));

        let found = sound_diagram(&lines, &segments).expect("a diagram of the edges turned");
        // A cell for each edge and for each end of one, every one with
        // edges round it, and nothing left where it is not a number.
        assert_eq!(found.cells.len(), 12);
        assert!(found.cells.iter().all(|cell| cell.incident_edge.is_some()));
        assert!(found.vertices.iter().flatten().all(|c| c.is_finite()));
        // Turned back, the vertices lie between the edges again: the
        // nearest of them to the first pair is midway between the two.
        let midway = (JOG[0][0] + JOG[1][0]) as f64 / 2.0;
        assert!(
            found
                .vertices
                .iter()
                .any(|v| (v[0] - midway).abs() < 2.0 && v[1] > 0.0)
        );
    }

    #[test]
    fn a_repair_is_the_same_wherever_the_outline_stands() {
        // The edges of the test above, which are turned for want of a
        // first diagram, and the same edges elsewhere: turned about the
        // middle of their own box, so the same diagram comes back.
        let lines = |x: i64, y: i64| -> Vec<Polygon> {
            JOG.iter()
                .flat_map(|&[x0, y0, x1, y1]| [[x0, y0, x1, y1], [-x0, -y0, -x1, -y1]])
                .map(|[x0, y0, x1, y1]| Polygon {
                    points: vec![
                        Point2 {
                            x: x0 + x,
                            y: y0 + y,
                        },
                        Point2 {
                            x: x1 + x,
                            y: y1 + y,
                        },
                    ],
                })
                .collect()
        };
        let segments: Vec<(usize, usize)> = (0..4).map(|line| (line, 0)).collect();
        let at_home = sound_diagram(&lines(0, 0), &segments).unwrap();
        assert_eq!(at_home.origin, [0, 0]);
        for (x, y) in [(135_500_000, 136_000_000), (-7, 3_000_000_000)] {
            let mut found = sound_diagram(&lines(x, y), &segments).unwrap();
            assert_eq!(found.origin, [x, y]);
            found.origin = [0, 0];
            assert_eq!(found, at_home, "moved by ({x}, {y})");
        }
    }

    #[test]
    fn an_error_from_every_attempt_is_passed_up() {
        // Too wide for the library however it is turned.
        let wide = i64::from(i32::MAX) * 4;
        let outline = [Polygon {
            points: vec![
                Point2 { x: 0, y: 0 },
                Point2 { x: wide, y: 0 },
                Point2 { x: wide, y: wide },
                Point2 { x: 0, y: wide },
            ],
        }];
        let segments: Vec<(usize, usize)> = (0..4).map(|at| (0, at)).collect();
        assert!(matches!(
            sound_diagram(&outline, &segments),
            Err(MedialError::TooLarge)
        ));
    }

    #[test]
    fn a_point_is_told_inside_a_corner_from_outside_it() {
        let (a, b, c) = (
            Point2::from_mm(10.0, 0.0),
            Point2::from_mm(0.0, 0.0),
            Point2::from_mm(0.0, 10.0),
        );
        // Going a, b, c with the inside on the left: the corner is the
        // quarter between the two.
        assert!(!inside_corner(a, b, c, Point2::from_mm(5.0, 5.0)));
        assert!(inside_corner(c, b, a, Point2::from_mm(5.0, 5.0)));
        assert!(inside_corner(a, b, c, Point2::from_mm(-5.0, -5.0)));
    }

    #[test]
    fn a_parabola_is_cut_into_steps_through_its_apex() {
        // Between the point (0, 2) and the x axis, from x = -4 to x = 4.
        let at = |x: f64| Point2::from_mm(x, x * x / 4.0 + 1.0);
        let points = discretize_parabola(
            Point2::from_mm(0.0, 2.0),
            (Point2::from_mm(-10.0, 0.0), Point2::from_mm(10.0, 0.0)),
            at(-4.0),
            at(4.0),
            800_000,
            10f32.to_radians(),
        );
        assert_eq!((points[0], points[points.len() - 1]), (at(-4.0), at(4.0)));
        assert!(points.contains(&Point2::from_mm(0.0, 1.0)));
        assert!(points.len() >= 10);
        // Every point is as far from the focus as from the line.
        for p in &points {
            let to_focus = (p.x as f64).hypot(p.y as f64 - 2e6);
            assert!((to_focus - p.y as f64).abs() < 5.0, "{p:?}");
        }
        // In order along the line (the apex may be there twice, as in the
        // reference, where a step lands on it).
        assert!(points.windows(2).all(|pair| pair[1].x >= pair[0].x));
    }
}
