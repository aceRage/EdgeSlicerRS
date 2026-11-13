//! The borders between colours across a layer, and each colour's faces
//! traced round them: a port of the reference's `MMU_Graph` and the
//! functions that build and read it (`build_graph`,
//! `remove_multiple_edges_in_vertices`, `extract_colored_segments`,
//! MultiMaterialSegmentation.cpp).
//!
//! The graph's nodes are the corners of the coloured lines round the
//! rings and the vertices of the lines' Voronoi diagram. Its arcs are
//! the coloured lines themselves (border arcs, one way, the ring's way
//! round) and the Voronoi edges that part two colours (both ways). A
//! colour's faces are traced from each of its border arcs, always
//! turning as far left as an arc not yet walked allows.

use edge_geom::{Point2, Polygon};
use edge_medial::{MedialError, Source, VoronoiDiagram};
use rustc_hash::FxHashMap;

use super::colour::{ColouredLine, get_segments};
use super::{Line, SCALED_EPSILON, dist2};

/// Why a layer's colours could not be traced.
#[derive(Debug, thiserror::Error)]
pub enum SideError {
    #[error(transparent)]
    Diagram(#[from] MedialError),
    #[error("a ring of the outline goes back along itself")]
    Ring,
}

/// No node: a Voronoi vertex outside the outline's box (the
/// reference's colour -1).
const NONE: usize = usize::MAX;
/// The reference's `EPSILON`, in scaled units where it is used so.
const EPSILON: f64 = 1e-4;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Arc {
    from: usize,
    to: usize,
    /// The colour of a border arc; -1 for the rest.
    colour: i32,
    border: bool,
}

#[derive(Clone, Debug)]
struct Node {
    point: [f64; 2],
    /// The arcs that leave it, in the order they were added.
    arcs: Vec<usize>,
}

/// The reference's `MMU_Graph`.
struct Graph {
    nodes: Vec<Node>,
    arcs: Vec<Arc>,
    /// Nodes below this are the rings' corners, one for each coloured
    /// line, in order; the arc of line `i` is arc `i`.
    border_points: usize,
    offsets: Vec<usize>,
}

impl Graph {
    fn global(&self, ring: usize, at: usize) -> usize {
        self.offsets[ring] + at
    }

    /// `MMU_Graph::append_edge`: none where the two are joined already,
    /// either way; both ways unless it is a border arc.
    fn append_edge(&mut self, from: usize, to: usize, colour: i32, border: bool) {
        let joined = |node: usize, other: usize| {
            self.nodes[node]
                .arcs
                .iter()
                .any(|&arc| self.arcs[arc].to == other)
        };
        if joined(from, to) || joined(to, from) {
            return;
        }
        self.nodes[from].arcs.push(self.arcs.len());
        self.arcs.push(Arc {
            from,
            to,
            colour,
            border,
        });
        if !border {
            self.nodes[to].arcs.push(self.arcs.len());
            self.arcs.push(Arc {
                from: to,
                to: from,
                colour,
                border,
            });
        }
    }

    fn append(&mut self, from: usize, to: usize) {
        self.append_edge(from, to, -1, false);
    }

    /// `MMU_Graph::remove_edge`: the arc each way between the two.
    fn remove_edge(&mut self, from: usize, to: usize) {
        for (node, other) in [(from, to), (to, from)] {
            if let Some(at) = self.nodes[node]
                .arcs
                .iter()
                .position(|&arc| self.arcs[arc].to == other)
            {
                self.nodes[node].arcs.remove(at);
            }
        }
    }

    /// `MMU_Graph::remove_nodes_with_one_arc`: dead ends off the rings
    /// cut back until none is left.
    fn remove_nodes_with_one_arc(&mut self) {
        let mut queue: std::collections::VecDeque<usize> = (self.border_points..self.nodes.len())
            .filter(|&node| self.nodes[node].arcs.len() == 1)
            .collect();
        while let Some(from) = queue.pop_front() {
            if self.nodes[from].arcs.is_empty() {
                continue;
            }
            let to = self.arcs[self.nodes[from].arcs[0]].to;
            self.remove_edge(from, to);
            if self.nodes[to].arcs.len() == 1 && to >= self.border_points {
                queue.push_back(to);
            }
        }
    }

    fn point(&self, node: usize) -> [f64; 2] {
        self.nodes[node].point
    }
}

/// Boost's `ulp_comparison`, as `vertex_equal_to_point` uses it: within
/// 128 units in the last place.
fn ulp_equal(a: f64, b: f64) -> bool {
    const SIGN: u64 = 0x8000_0000_0000_0000;
    let key = |x: f64| {
        let bits = x.to_bits();
        if bits < SIGN { SIGN - bits } else { bits }
    };
    let (a, b) = (key(a), key(b));
    a.abs_diff(b) <= 128
}

/// A point in a [`Lookup`]: where it is on the grid, where exactly, and
/// its node.
type Entry = (Point2, [f64; 2], usize);

/// The nearest of a set of points closer than a radius
/// (`ClosestPointInRadiusLookup`).
struct Lookup {
    radius: i64,
    cells: FxHashMap<(i64, i64), Vec<Entry>>,
}

impl Lookup {
    fn new(radius: i64) -> Self {
        Self {
            radius,
            cells: FxHashMap::default(),
        }
    }

    fn key(&self, p: Point2) -> (i64, i64) {
        (p.x.div_euclid(self.radius), p.y.div_euclid(self.radius))
    }

    fn insert(&mut self, at: Point2, exact: [f64; 2], node: usize) {
        let key = self.key(at);
        self.cells.entry(key).or_default().push((at, exact, node));
    }

    /// All within the radius, with their squared distances.
    fn all(&self, p: Point2) -> impl Iterator<Item = (&Entry, f64)> {
        let (kx, ky) = self.key(p);
        let limit = (self.radius as f64).powi(2);
        (-1..=1)
            .flat_map(move |dx| (-1..=1).map(move |dy| (kx + dx, ky + dy)))
            .filter_map(|key| self.cells.get(&key))
            .flatten()
            .map(move |entry| (entry, dist2(p, entry.0)))
            .filter(move |(_, d2)| *d2 < limit)
    }

    fn find(&self, p: Point2) -> Option<(&Entry, f64)> {
        let mut best: Option<(&Entry, f64)> = None;
        for (entry, d2) in self.all(p) {
            if best.is_none_or(|(_, least)| d2 < least) {
                best = Some((entry, d2));
            }
        }
        best
    }
}

/// `mk_point` of a vertex: towards zero.
fn truncated(p: [f64; 2]) -> Point2 {
    Point2 {
        x: p[0] as i64,
        y: p[1] as i64,
    }
}

/// `mk_point(const Vec2d&)`: to the nearest.
fn rounded(p: [f64; 2]) -> Point2 {
    Point2 {
        x: p[0].round() as i64,
        y: p[1].round() as i64,
    }
}

fn to_f64(p: Point2) -> [f64; 2] {
    [p.x as f64, p.y as f64]
}

fn normalized(v: [f64; 2]) -> [f64; 2] {
    let norm = v[0].hypot(v[1]);
    if norm > 0.0 {
        [v[0] / norm, v[1] / norm]
    } else {
        v
    }
}

fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// Where two segments cross, cut towards zero; none where they are
/// parallel or miss (`line_alg::intersection`).
fn intersection(l1: &Line, l2: &Line) -> Option<Point2> {
    let v1 = [(l1.b.x - l1.a.x) as f64, (l1.b.y - l1.a.y) as f64];
    let v2 = [(l2.b.x - l2.a.x) as f64, (l2.b.y - l2.a.y) as f64];
    let denom = cross2(v1, v2);
    if denom.abs() < EPSILON {
        return None;
    }
    let v12 = [(l1.a.x - l2.a.x) as f64, (l1.a.y - l2.a.y) as f64];
    let t1 = cross2(v2, v12) / denom;
    let t2 = cross2(v1, v12) / denom;
    if (0.0..=1.0).contains(&t1) && (0.0..=1.0).contains(&t2) {
        Some(Point2 {
            x: (l1.a.x as f64 + t1 * v1[0]) as i64,
            y: (l1.a.y as f64 + t1 * v1[1]) as i64,
        })
    } else {
        None
    }
}

/// `line_intersection_with_epsilon`: `line` made 15 `SCALED_EPSILON`
/// longer at each end (`Line::extend`) and crossed with `other`.
fn crossing_with_epsilon(line: &Line, other: &Line) -> Option<Point2> {
    let v = normalized([(line.b.x - line.a.x) as f64, (line.b.y - line.a.y) as f64]);
    let offset = 15.0 * SCALED_EPSILON as f64;
    let (ox, oy) = ((offset * v[0]) as i64, (offset * v[1]) as i64);
    let extended = Line::new(
        Point2 {
            x: line.a.x - ox,
            y: line.a.y - oy,
        },
        Point2 {
            x: line.b.x + ox,
            y: line.b.y + oy,
        },
    );
    intersection(&extended, other)
}

fn closer_to_start(line: &Line, p: Point2) -> bool {
    dist2(p, line.a) < dist2(p, line.b)
}

/// Whether a line from the corner where `first` meets `second` towards
/// `to` points into the ring (`points_inside`).
fn points_inside(first: &Line, second: &Line, to: Point2) -> bool {
    let perp = |v: [f64; 2]| [-v[1], v[0]];
    let (left, middle, right) = (to_f64(first.a), to_f64(first.b), to_f64(second.b));
    let a = normalized(perp([middle[0] - left[0], middle[1] - left[1]]));
    let b = normalized(perp([right[0] - middle[0], right[1] - middle[1]]));
    let inward = normalized([a[0] + b[0], a[1] + b[1]]);
    let towards = normalized([to.x as f64 - middle[0], to.y as f64 - middle[1]]);
    dot(inward, towards) > 0.0
}

/// The reference's `BoundingBox` of integer points.
#[derive(Clone, Copy)]
struct Bounds {
    min: Point2,
    max: Point2,
}

impl Bounds {
    fn of(points: impl Iterator<Item = Point2>) -> Self {
        let mut bounds = Bounds {
            min: Point2 {
                x: i64::MAX,
                y: i64::MAX,
            },
            max: Point2 {
                x: i64::MIN,
                y: i64::MIN,
            },
        };
        for p in points {
            bounds.min.x = bounds.min.x.min(p.x);
            bounds.min.y = bounds.min.y.min(p.y);
            bounds.max.x = bounds.max.x.max(p.x);
            bounds.max.y = bounds.max.y.max(p.y);
        }
        bounds
    }

    fn offset(self, by: i64) -> Self {
        Bounds {
            min: Point2 {
                x: self.min.x - by,
                y: self.min.y - by,
            },
            max: Point2 {
                x: self.max.x + by,
                y: self.max.y + by,
            },
        }
    }

    fn contains(&self, p: Point2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    fn contains_f64(&self, p: [f64; 2]) -> bool {
        p[0] >= self.min.x as f64
            && p[0] <= self.max.x as f64
            && p[1] >= self.min.y as f64
            && p[1] <= self.max.y as f64
    }
}

/// The diagram with what the graph asks of it.
struct Diagram<'a> {
    vd: &'a VoronoiDiagram,
    /// The coloured lines, which are its segments.
    lines: &'a [ColouredLine],
}

impl Diagram<'_> {
    fn vertex(&self, vertex: usize) -> [f64; 2] {
        let [x, y] = self.vd.vertices[vertex];
        [x + self.vd.origin[0] as f64, y + self.vd.origin[1] as f64]
    }

    fn source(&self, edge: usize) -> usize {
        self.vd.cells[self.vd.edges[edge].cell].segment
    }

    fn twin(&self, edge: usize) -> usize {
        self.vd.edges[edge].twin
    }

    fn ends(&self, edge: usize) -> (Option<usize>, Option<usize>) {
        let edge = &self.vd.edges[edge];
        (edge.vertex0, edge.vertex1)
    }

    /// The point a point cell is the cell of (`retrieve_point`).
    fn site_point(&self, cell: usize) -> [f64; 2] {
        let cell = &self.vd.cells[cell];
        let line = &self.lines[cell.segment].line;
        to_f64(match cell.source {
            Source::SegmentEnd => line.b,
            _ => line.a,
        })
    }

    /// An edge that runs off without end, cut where it has gone `size`
    /// on from its one vertex (`Voronoi::Internal::clip_infinite_edge`).
    fn clip_infinite_edge(&self, edge: usize, size: f64) -> Option<[[f64; 2]; 2]> {
        let e = &self.vd.edges[edge];
        let (cell1, cell2) = (e.cell, self.vd.edges[e.twin].cell);
        let point_cell = |cell: usize| self.vd.cells[cell].source != Source::Segment;
        let direction = if point_cell(cell1) && point_cell(cell2) {
            let (mut p1, mut p2) = (self.site_point(cell1), self.site_point(cell2));
            if e.vertex0.is_none() {
                std::mem::swap(&mut p1, &mut p2);
            }
            [p1[1] - p2[1], p2[0] - p1[0]]
        } else if point_cell(cell1) || point_cell(cell2) {
            let segment = if point_cell(cell1) { cell2 } else { cell1 };
            let line = &self.lines[self.vd.cells[segment].segment].line;
            // Boost's `low` and `high` of the segment as it was given.
            [(line.b.y - line.a.y) as f64, (line.a.x - line.b.x) as f64]
        } else {
            return None;
        };
        let koef = size / direction[0].abs().max(direction[1].abs());
        match (e.vertex0, e.vertex1) {
            (None, Some(v1)) => {
                let v = self.vertex(v1);
                Some([[v[0] + direction[0] * koef, v[1] + direction[1] * koef], v])
            }
            (Some(v0), None) => {
                let v = self.vertex(v0);
                Some([v, [v[0] + direction[0] * koef, v[1] + direction[1] * koef]])
            }
            _ => None,
        }
    }
}

/// Each colour's faces on a layer whose rings are coloured as `rings`
/// says (`build_graph`, `remove_multiple_edges_in_vertices`,
/// `remove_nodes_with_one_arc`, `extract_colored_segments`), as polygons
/// that may overlap or touch: one list for each of `colours`.
pub(crate) fn segments(
    rings: &[Vec<ColouredLine>],
    colours: usize,
) -> Result<Vec<Vec<Polygon>>, SideError> {
    // A line of no length has no place in the diagram.
    let rings: Vec<Vec<ColouredLine>> = rings
        .iter()
        .map(|ring| {
            ring.iter()
                .copied()
                .filter(|line| line.line.a != line.line.b)
                .collect::<Vec<_>>()
        })
        .filter(|ring| ring.len() >= 3)
        .collect();
    if rings.is_empty() {
        return Ok(vec![Vec::new(); colours]);
    }
    let mut graph = build_graph(&rings)?;
    remove_multiple_edges_in_vertices(&mut graph, &rings);
    graph.remove_nodes_with_one_arc();
    Ok(extract_colored_segments(&graph, colours))
}

/// `build_graph`.
fn build_graph(rings: &[Vec<ColouredLine>]) -> Result<Graph, SideError> {
    let lines: Vec<ColouredLine> = rings.iter().flatten().copied().collect();
    // Which ring each line is of, and where in it.
    let mut place = Vec::with_capacity(lines.len());
    let mut offsets = Vec::with_capacity(rings.len());
    for (ring, coloured) in rings.iter().enumerate() {
        offsets.push(place.len());
        place.extend((0..coloured.len()).map(|at| (ring, at)));
    }
    // A ring all one colour has to add an edge of its own, or a hole
    // of another colour than the outside would not be told apart.
    let mut force: Vec<bool> = rings
        .iter()
        .map(|ring| ring.iter().all(|line| line.colour == ring[0].colour))
        .collect();

    let segments: Vec<[i64; 4]> = lines
        .iter()
        .map(|l| [l.line.a.x, l.line.a.y, l.line.b.x, l.line.b.y])
        .collect();
    let vd = edge_medial::diagram(&segments)?;
    let diagram = Diagram {
        vd: &vd,
        lines: &lines,
    };

    let mut graph = Graph {
        nodes: lines
            .iter()
            .map(|l| Node {
                point: to_f64(l.line.a),
                arcs: Vec::new(),
            })
            .collect(),
        arcs: Vec::new(),
        border_points: lines.len(),
        offsets: offsets.clone(),
    };
    // `MMU_Graph::add_contours`.
    for (ring, coloured) in rings.iter().enumerate() {
        for (at, line) in coloured.iter().enumerate() {
            let (from, to) = (
                graph.global(ring, at),
                graph.global(ring, (at + 1) % coloured.len()),
            );
            graph.append_edge(from, to, i32::from(line.colour), true);
        }
    }
    if graph.arcs.len() != lines.len() {
        // A ring that goes back along itself: two of its lines join the
        // same two corners.
        return Err(SideError::Ring);
    }

    let bounds = Bounds::of(lines.iter().map(|l| l.line.a));
    let colour = append_voronoi_vertices(&mut graph, &diagram, bounds);
    let count = graph.nodes.len();
    let border_points = graph.border_points;
    let on_contour = |vertex: usize| colour[vertex] < border_points;
    let colour_of = |vertex: Option<usize>| vertex.map_or(NONE, |v| colour[v]);

    let prev_line = |line: usize| {
        let (ring, at) = place[line];
        let size = rings[ring].len();
        lines[offsets[ring] + if at > 0 { at - 1 } else { size - 1 }]
    };
    let next_line = |line: usize| {
        let (ring, at) = place[line];
        lines[offsets[ring] + (at + 1) % rings[ring].len()]
    };

    let clip_bounds = bounds.offset(scale_mm(10.0));
    let size_x = (clip_bounds.max.x - clip_bounds.min.x) as f64;
    let size_y = (clip_bounds.max.y - clip_bounds.min.y) as f64;
    let dim_max = size_x.max(size_y);
    let diagonal = size_x.hypot(size_y);
    // `clip_finite_voronoi_edge`.
    let clip_finite = |v0: [f64; 2], v1: [f64; 2]| {
        let (in0, in1) = (clip_bounds.contains_f64(v0), clip_bounds.contains_f64(v1));
        if in0 == in1 {
            return Line::new(truncated(v0), truncated(v1));
        }
        let d = normalized([v1[0] - v0[0], v1[1] - v0[1]]);
        let vector = [d[0] * diagonal, d[1] * diagonal];
        let (mut a, mut b) = (v0, v1);
        if !in0 {
            a = [v1[0] - vector[0], v1[1] - vector[1]];
        } else {
            b = [v0[0] + vector[0], v0[1] + vector[1]];
        }
        Line::new(truncated(a), truncated(b))
    };

    let edge_count = vd.edges.len();
    let mut processed = vec![false; edge_count];
    let mark = |processed: &mut Vec<bool>, edge: usize| {
        processed[edge] = true;
        processed[diagram.twin(edge)] = true;
    };
    for edge in 0..edge_count {
        let twin = diagram.twin(edge);
        if diagram.source(edge) > diagram.source(twin) || processed[edge] {
            continue;
        }
        let (v0, v1) = diagram.ends(edge);
        let source = diagram.source(edge);
        match (v0, v1) {
            (None, None) => {}
            (Some(_), None) | (None, Some(_)) => {
                // An edge off without end from a vertex on the outline,
                // with no vertex where it meets the outline.
                let Some(samples) = diagram.clip_infinite_edge(edge, dim_max) else {
                    continue;
                };
                let edge_line = Line::new(truncated(samples[0]), truncated(samples[1]));
                let contour = lines[source].line;
                if let Some(crossing) = crossing_with_epsilon(&contour, &edge_line) {
                    let arc = graph.arcs[source];
                    let from = colour_of(v1.or(v0));
                    let to = if closer_to_start(&contour, crossing) {
                        arc.from
                    } else {
                        arc.to
                    };
                    if from != to && from < count && to < count {
                        graph.append(from, to);
                        mark(&mut processed, edge);
                    }
                }
            }
            (Some(v0), Some(v1)) => {
                let (c0, c1) = (colour[v0], colour[v1]);
                if (on_contour(v0) && on_contour(v1)) || c0 == c1 {
                    continue;
                }
                let edge_line = clip_finite(diagram.vertex(v0), diagram.vertex(v1));
                let coloured = lines[source];
                let contour = coloured.line;
                let prev = prev_line(source);
                let next = next_line(source);
                if c0 >= count || c1 >= count {
                    // One end beyond the outline's box: joined to the
                    // outline where the edge crosses it.
                    let mut to_outline = |vertex_colour: usize, graph: &mut Graph| {
                        let twin_line = lines[diagram.source(twin)].line;
                        if let Some(crossing) = crossing_with_epsilon(&twin_line, &edge_line) {
                            let arc = graph.arcs[diagram.source(twin)];
                            let to = if closer_to_start(&twin_line, crossing) {
                                arc.from
                            } else {
                                arc.to
                            };
                            graph.append(vertex_colour, to);
                        } else if let Some(crossing) = crossing_with_epsilon(&contour, &edge_line) {
                            let arc = graph.arcs[source];
                            let to = if closer_to_start(&contour, crossing) {
                                arc.from
                            } else {
                                arc.to
                            };
                            graph.append(vertex_colour, to);
                        }
                        mark(&mut processed, edge);
                    };
                    if c0 < count && !on_contour(v0) {
                        to_outline(c0, &mut graph);
                    }
                    if c1 < count && !on_contour(v1) {
                        to_outline(c1, &mut graph);
                    }
                } else if on_contour(v0) || on_contour(v1) {
                    mark(&mut processed, edge);
                    let (near, far) = if on_contour(v0) {
                        (edge_line.a, edge_line.b)
                    } else {
                        (edge_line.b, edge_line.a)
                    };
                    let ring = place[source].0;
                    let changes_or_forced =
                        |other: &ColouredLine| other.colour != coloured.colour || force[ring];
                    let add = if closer_to_start(&contour, near) {
                        changes_or_forced(&prev) && points_inside(&prev.line, &contour, far)
                    } else {
                        changes_or_forced(&next) && points_inside(&contour, &next.line, far)
                    };
                    if add {
                        graph.append(c0, c1);
                        force[ring] = false;
                    }
                } else if let Some(crossing) = crossing_with_epsilon(&contour, &edge_line) {
                    mark(&mut processed, edge);
                    // `Point(coord_t(x), coord_t(y))`: towards zero.
                    let real0 = truncated(graph.point(c0));
                    let real1 = truncated(graph.point(c1));
                    if closer_to_start(&contour, crossing) {
                        if prev.colour != coloured.colour {
                            let at = graph.arcs[source].from;
                            if points_inside(&prev.line, &contour, real0) {
                                graph.append(c0, at);
                            }
                            if points_inside(&prev.line, &contour, real1) {
                                graph.append(c1, at);
                            }
                        }
                    } else {
                        let at = graph.arcs[source].to;
                        if next.colour != coloured.colour {
                            if points_inside(&contour, &next.line, real0) {
                                graph.append(c0, at);
                            }
                            if points_inside(&contour, &next.line, real1) {
                                graph.append(c1, at);
                            }
                        }
                    }
                }
            }
        }
    }

    // Every other edge between two vertices inside the box.
    for edge in 0..edge_count {
        let twin = diagram.twin(edge);
        if diagram.source(edge) > diagram.source(twin) || processed[edge] {
            continue;
        }
        if let (Some(v0), Some(v1)) = diagram.ends(edge) {
            let (c0, c1) = (colour[v0], colour[v1]);
            if c0 < graph.nodes.len() && c1 < graph.nodes.len() && c0 != c1 {
                graph.append(c0, c1);
            }
        }
        mark(&mut processed, edge);
    }

    graph.remove_nodes_with_one_arc();
    Ok(graph)
}

fn scale_mm(mm: f64) -> i64 {
    super::scale(mm) as i64
}

/// `MMU_Graph::append_voronoi_vertices`: each vertex of the diagram
/// made a node, or joined to a corner of a ring it lies on or within
/// three `SCALED_EPSILON` of, or to a node already made at the same
/// place; one outside the rings' box (grown by `SCALED_EPSILON`) is
/// none. The node of each vertex.
fn append_voronoi_vertices(graph: &mut Graph, diagram: &Diagram, bounds: Bounds) -> Vec<usize> {
    let bounds = bounds.offset(SCALED_EPSILON);
    let vd = diagram.vd;
    let mut corners = Lookup::new(3 * SCALED_EPSILON);
    for node in 0..graph.border_points {
        let point = graph.nodes[node].point;
        corners.insert(rounded(point), point, node);
    }
    let mut vertices = Lookup::new(SCALED_EPSILON);
    let mut colour = vec![NONE; vd.vertices.len()];
    for (vertex, colour) in colour.iter_mut().enumerate() {
        let exact = diagram.vertex(vertex);
        let point = truncated(exact);
        let Some(edge) = vd.vertex_edges[vertex] else {
            continue;
        };
        let first = graph.point(diagram.source(edge));
        let second = graph.point(diagram.source(diagram.twin(edge)));
        if ulp_equal(exact[0], first[0]) && ulp_equal(exact[1], first[1]) {
            *colour = diagram.source(edge);
        } else if ulp_equal(exact[0], second[0]) && ulp_equal(exact[1], second[1]) {
            *colour = diagram.source(diagram.twin(edge));
        } else if bounds.contains(point) {
            if let Some((corner, _)) = corners.find(point) {
                *colour = corner.2;
            } else {
                let near = vertices.find(point);
                let merge = match near {
                    None => None,
                    Some((_, d2)) if d2 >= (SCALED_EPSILON as f64 / 10.0).powi(2) => None,
                    Some(_) => vertices
                        .all(point)
                        .find(|(entry, _)| {
                            let (dx, dy) = (exact[0] - entry.1[0], exact[1] - entry.1[1]);
                            dx * dx + dy * dy <= EPSILON * EPSILON
                        })
                        .map(|(entry, _)| entry.2),
                };
                *colour = match merge {
                    Some(node) => node,
                    None => {
                        let node = graph.nodes.len();
                        vertices.insert(rounded(exact), exact, node);
                        graph.nodes.push(Node {
                            point: exact,
                            arcs: Vec::new(),
                        });
                        node
                    }
                };
            }
        }
    }
    colour
}

/// How far the arcs run on from `arc` out of `start` while they go
/// nearly straight (within 15 degrees) through nodes of two arcs
/// (`compute_edge_length`).
fn compute_edge_length(graph: &Graph, start: usize, arc: usize) -> f64 {
    let mut used = FxHashMap::<usize, ()>::default();
    used.insert(arc, ());
    let mut arc = graph.arcs[arc];
    let mut idx = start;
    let distance = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let mut total = distance(graph.point(arc.to), graph.point(idx));
    while graph.nodes[arc.to].arcs.len() == 2 {
        let mut found = false;
        for &next in &graph.nodes[arc.to].arcs {
            let candidate = graph.arcs[next];
            if candidate.border || used.contains_key(&next) || candidate.to == idx {
                continue;
            }
            let (p, q, r) = (
                graph.point(idx),
                graph.point(arc.to),
                graph.point(candidate.to),
            );
            let first = normalized([p[0] - q[0], p[1] - q[1]]);
            let second = normalized([r[0] - q[0], r[1] - q[1]]);
            let mut angle = dot(first, second).clamp(-1.0, 1.0).acos();
            if cross2(first, second) < 0.0 {
                angle = 2.0 * std::f64::consts::PI - angle;
            }
            if (angle - std::f64::consts::PI).abs() >= std::f64::consts::PI / 12.0 {
                continue;
            }
            idx = arc.to;
            arc = candidate;
            total += distance(graph.point(arc.to), graph.point(idx));
            used.insert(next, ());
            found = true;
            break;
        }
        if !found {
            break;
        }
    }
    total
}

/// Where a run of one colour starts on a ring and more than one border
/// leaves the corner, only the one that runs furthest is kept
/// (`remove_multiple_edges_in_vertices`).
fn remove_multiple_edges_in_vertices(graph: &mut Graph, rings: &[Vec<ColouredLine>]) {
    for (ring, coloured) in rings.iter().enumerate() {
        for (first, _) in get_segments(coloured) {
            let node = graph.global(ring, first);
            if graph.nodes[node].arcs.len() < 3 {
                continue;
            }
            let mut check: Vec<(usize, f64)> = graph.nodes[node]
                .arcs
                .iter()
                .filter(|&&arc| !graph.arcs[arc].border)
                .map(|&arc| (graph.arcs[arc].to, compute_edge_length(graph, node, arc)))
                .collect();
            // Longest first, the order kept among equals.
            check.sort_by(|l, r| r.1.total_cmp(&l.1));
            while check.len() > 1 {
                let (to, _) = check.pop().expect("more than one");
                graph.remove_edge(node, to);
            }
        }
    }
}

/// The arc to go on by from `original`, which ran along `process`: the
/// one furthest to the left of those not yet walked that may follow it
/// in `colour`; `original` itself where there is none (`get_next_arc`).
fn next_arc(
    graph: &Graph,
    used: &[bool],
    process: [[f64; 2]; 2],
    original: usize,
    colour: i32,
) -> usize {
    let arc = graph.arcs[original];
    let mut sorted: Vec<(usize, f64)> = Vec::new();
    for &next in &graph.nodes[arc.to].arcs {
        let candidate = graph.arcs[next];
        if graph.point(candidate.to) == process[0] || used[next] {
            continue;
        }
        if arc.border && arc.colour != colour {
            continue;
        }
        if candidate.border && candidate.colour != colour {
            continue;
        }
        let back = normalized([process[0][0] - process[1][0], process[0][1] - process[1][1]]);
        let (from, to) = (graph.point(candidate.from), graph.point(candidate.to));
        let along = normalized([to[0] - from[0], to[1] - from[1]]);
        let mut angle = dot(along, back).clamp(-1.0, 1.0).acos();
        if cross2(along, back) < 0.0 {
            angle = 2.0 * std::f64::consts::PI - angle;
        }
        sorted.push((next, angle));
    }
    sorted.sort_by(|l, r| l.1.total_cmp(&r.1));
    sorted.first().map_or(original, |&(next, _)| next)
}

/// Whether a ring's edges cross one another, its neighbours apart
/// (`is_profile_self_interaction`).
fn crosses_itself(polygon: &Polygon) -> bool {
    let points = &polygon.points;
    let n = points.len();
    let line = |i: usize| Line::new(points[i], points[(i + 1) % n]);
    for i in 0..n {
        for j in i + 2..n.min(n + i - 1) {
            if intersection(&line(i), &line(j)).is_some() {
                return true;
            }
        }
    }
    false
}

fn is_valid_ccw(polygon: &Polygon) -> bool {
    // `Polygon::is_counter_clockwise` (Clipper's `Orientation`: an area
    // of naught counts) and `is_valid`.
    polygon.points.len() >= 3 && polygon.signed_area_mm2() >= 0.0
}

/// Each colour's faces, traced from each of its border arcs not yet
/// walked (`extract_colored_segments`). A trace that does not close
/// into a polygon wound counter-clockwise is cut back, an arc at a time,
/// and closed straight, until it is one that does not cross itself.
fn extract_colored_segments(graph: &Graph, colours: usize) -> Vec<Vec<Polygon>> {
    let mut used = vec![false; graph.arcs.len()];
    let mut faces = vec![Vec::new(); colours];
    for node in 0..graph.border_points {
        for &start in &graph.nodes[node].arcs {
            let arc = graph.arcs[start];
            if !arc.border || used[start] {
                continue;
            }
            if arc.colour < 0 || arc.colour as usize >= colours {
                continue;
            }
            let colour = arc.colour;
            let line_of = |arc: usize| {
                let arc = graph.arcs[arc];
                [graph.point(arc.from), graph.point(arc.to)]
            };
            used[start] = true;
            let mut face: Vec<(Option<usize>, [[f64; 2]; 2])> = vec![(Some(start), line_of(start))];
            let start_point = graph.point(arc.from);
            let mut process = line_of(start);
            let mut current = start;
            loop {
                let next = next_arc(graph, &used, process, current, colour);
                if used[next] {
                    break;
                }
                face.push((Some(next), line_of(next)));
                used[next] = true;
                process = line_of(next);
                current = next;
                let to = graph.arcs[current].to;
                if graph.point(to) == start_point
                    && graph.nodes[to].arcs.iter().all(|&arc| used[arc])
                {
                    break;
                }
            }
            let polygon = |face: &[(Option<usize>, [[f64; 2]; 2])]| Polygon {
                points: face.iter().map(|(_, line)| rounded(line[0])).collect(),
            };
            let whole = polygon(&face);
            if is_valid_ccw(&whole) {
                faces[colour as usize].push(whole);
                continue;
            }
            while face.len() > 1 {
                let (last, _) = face.pop().expect("more than one");
                if let Some(last) = last {
                    used[last] = false;
                }
                let closing = [face[face.len() - 1].1[1], face[0].1[0]];
                face.push((None, closing));
                let cut = polygon(&face);
                if !crosses_itself(&cut) && is_valid_ccw(&cut) {
                    faces[colour as usize].push(cut);
                    break;
                }
                face.pop();
            }
        }
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_in_the_last_place() {
        assert!(ulp_equal(1.0, 1.0 + 100.0 * f64::EPSILON));
        assert!(!ulp_equal(1.0, 1.0 + 1e-10));
        assert!(ulp_equal(-0.0, 0.0));
        assert!(!ulp_equal(-1.0, 1.0));
    }

    #[test]
    fn the_inside_of_a_corner_is_to_the_left() {
        let a = Point2 { x: 0, y: 0 };
        let b = Point2 { x: 100, y: 0 };
        let c = Point2 { x: 100, y: 100 };
        let (first, second) = (Line::new(a, b), Line::new(b, c));
        assert!(points_inside(&first, &second, Point2 { x: 50, y: 50 }));
        assert!(!points_inside(&first, &second, Point2 { x: 150, y: -50 }));
    }
}
