//! The skeleton as a graph of half edges: every edge is two, one each
//! way, and the half edges round a cell of the part are chained by
//! `next` and `prev` from the outline, in along the skeleton, and back
//! out to the outline.
//!
//! A port of SkeletalTrapezoidationGraph.cpp with what it holds
//! (SkeletalTrapezoidationEdge.hpp, SkeletalTrapezoidationJoint.hpp and
//! the half-edge headers under utils/). The reference keeps nodes and
//! edges in linked lists and points at them; here they are numbered, and
//! one removed is marked so and passed over. The order the reference
//! walks its lists in decides which of several equal choices is made
//! later, so it is kept: see [`Graph::edges`] and [`Graph::nodes`].

use edge_geom::{Coord, Point2};

use crate::Junction;
use crate::beading::Beading;

pub(crate) type EdgeId = usize;
pub(crate) type NodeId = usize;

/// The lines that fit at a node, and how far that was carried from where
/// it was worked out (`BeadingPropagation`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BeadingPropagation {
    pub beading: Beading,
    pub dist_to_bottom_source: Coord,
    pub dist_from_top_source: Coord,
    pub is_upward_propagated_only: bool,
}

impl BeadingPropagation {
    pub fn new(beading: Beading) -> Self {
        Self {
            beading,
            dist_to_bottom_source: 0,
            dist_from_top_source: 0,
            is_upward_propagated_only: false,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub p: Point2,
    pub incident_edge: Option<EdgeId>,
    /// How far the outline is; -1 until known.
    pub distance_to_boundary: Coord,
    /// How many lines fit here; -1 until known.
    pub bead_count: Coord,
    /// How far through a change in the number of lines this node is.
    pub transition_ratio: f32,
    pub beading: Option<BeadingPropagation>,
}

impl Node {
    fn new(p: Point2) -> Self {
        Self {
            p,
            incident_edge: None,
            distance_to_boundary: -1,
            bead_count: -1,
            transition_ratio: 0.0,
            beading: None,
        }
    }
}

/// The middle of a change in the number of lines, along an edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransitionMiddle {
    /// Along the edge from its `from`.
    pub pos: Coord,
    pub lower_bead_count: i32,
    /// How far from the outline the change is placed.
    pub feature_radius: Coord,
}

/// Either end of such a change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransitionEnd {
    /// Along the edge from its `from`, the edge being the half that
    /// runs away from the outline.
    pub pos: Coord,
    pub lower_bead_count: i32,
    pub is_lower_end: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeType {
    /// From the Voronoi diagram.
    Normal,
    /// A rib, from the skeleton out to the outline.
    ExtraVd,
    /// A rib put in where a change in the number of lines ends.
    TransitionEnd,
}

#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub twin: Option<EdgeId>,
    pub next: Option<EdgeId>,
    pub prev: Option<EdgeId>,
    pub kind: EdgeType,
    /// Whether lines are laid along this edge; unknown at first.
    pub central: Option<bool>,
    pub transitions: Option<Vec<TransitionMiddle>>,
    pub transition_ends: Option<Vec<TransitionEnd>>,
    pub junctions: Option<Vec<Junction>>,
}

impl Edge {
    fn new(kind: EdgeType) -> Self {
        Self {
            // Set by whoever makes the edge, at once.
            from: usize::MAX,
            to: usize::MAX,
            twin: None,
            next: None,
            prev: None,
            kind,
            central: None,
            transitions: None,
            transition_ends: None,
            junctions: None,
        }
    }

    /// Unknown counts as not, as in the reference built for release;
    /// it stays unknown only for an edge that has no twin, which a
    /// sound diagram does not make.
    pub fn is_central(&self) -> bool {
        self.central == Some(true)
    }

    pub fn has_transitions(&self, ignore_empty: bool) -> bool {
        self.transitions
            .as_ref()
            .is_some_and(|list| ignore_empty || !list.is_empty())
    }

    pub fn has_transition_ends(&self, ignore_empty: bool) -> bool {
        self.transition_ends
            .as_ref()
            .is_some_and(|list| ignore_empty || !list.is_empty())
    }
}

/// The length of the vector from `b` to `a`, cut to a whole number as
/// the reference's integer `norm()` is.
pub(crate) fn length(a: Point2, b: Point2) -> Coord {
    let (x, y) = ((a.x - b.x) as i128, (a.y - b.y) as i128);
    ((x * x + y * y) as f64).sqrt() as Coord
}

/// Point.hpp `shorter_then`.
pub(crate) fn shorter_than(a: Point2, b: Point2, len: Coord) -> bool {
    let (x, y) = (a.x - b.x, a.y - b.y);
    if x > len || x < -len || y > len || y < -len {
        return false;
    }
    x * x + y * y <= len * len
}

/// The point of the line through `a` and `b` nearest `point`, cut to
/// whole units (Line.hpp `distance_to_infinite_squared`).
pub(crate) fn nearest_on_line(a: Point2, b: Point2, point: Point2) -> Point2 {
    let v = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let va = ((point.x - a.x) as f64, (point.y - a.y) as f64);
    let l2 = v.0 * v.0 + v.1 * v.1;
    if l2 == 0.0 {
        return a;
    }
    let t = (va.0 * v.0 + va.1 * v.1) / l2;
    Point2 {
        x: (a.x as f64 + t * v.0) as Coord,
        y: (a.y as f64 + t * v.1) as Coord,
    }
}

/// The point of the segment from `a` to `b` nearest `point` (Line.hpp
/// `distance_to_squared`).
pub(crate) fn nearest_on_segment(a: Point2, b: Point2, point: Point2) -> Point2 {
    let v = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let va = ((point.x - a.x) as f64, (point.y - a.y) as f64);
    let l2 = v.0 * v.0 + v.1 * v.1;
    if l2 == 0.0 {
        return a;
    }
    let t = (va.0 * v.0 + va.1 * v.1) / l2;
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        Point2 {
            x: (a.x as f64 + t * v.0) as Coord,
            y: (a.y as f64 + t * v.1) as Coord,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Graph {
    pub edge: Vec<Edge>,
    pub node: Vec<Node>,
    edge_removed: Vec<bool>,
    node_removed: Vec<bool>,
    /// Edges put at the front of the reference's list, in the order made
    /// (so walked last made first), and those put at its back.
    edges_front: Vec<EdgeId>,
    edges_back: Vec<EdgeId>,
    nodes_front: Vec<NodeId>,
    nodes_back: Vec<NodeId>,
}

impl Graph {
    /// Every edge, in the order the reference's list holds them.
    pub fn edges(&self) -> Vec<EdgeId> {
        self.edges_front
            .iter()
            .rev()
            .chain(&self.edges_back)
            .copied()
            .filter(|&edge| !self.edge_removed[edge])
            .collect()
    }

    /// Every node, in the order the reference's list holds them.
    pub fn nodes(&self) -> Vec<NodeId> {
        self.nodes_front
            .iter()
            .rev()
            .chain(&self.nodes_back)
            .copied()
            .filter(|&node| !self.node_removed[node])
            .collect()
    }

    pub fn new_node_front(&mut self, p: Point2) -> NodeId {
        self.node.push(Node::new(p));
        self.node_removed.push(false);
        self.nodes_front.push(self.node.len() - 1);
        self.node.len() - 1
    }

    pub fn new_node_back(&mut self, p: Point2) -> NodeId {
        self.node.push(Node::new(p));
        self.node_removed.push(false);
        self.nodes_back.push(self.node.len() - 1);
        self.node.len() - 1
    }

    /// A copy of `node` at the back of the list.
    pub fn copy_node_back(&mut self, node: NodeId) -> NodeId {
        let copy = self.node[node].clone();
        self.node.push(copy);
        self.node_removed.push(false);
        self.nodes_back.push(self.node.len() - 1);
        self.node.len() - 1
    }

    pub fn new_edge_front(&mut self, kind: EdgeType) -> EdgeId {
        self.edge.push(Edge::new(kind));
        self.edge_removed.push(false);
        self.edges_front.push(self.edge.len() - 1);
        self.edge.len() - 1
    }

    pub fn new_edge_back(&mut self, kind: EdgeType) -> EdgeId {
        self.edge.push(Edge::new(kind));
        self.edge_removed.push(false);
        self.edges_back.push(self.edge.len() - 1);
        self.edge.len() - 1
    }

    fn radius(&self, node: NodeId) -> Coord {
        self.node[node].distance_to_boundary
    }

    /// Whether following `edge` leads further from the outline, at once
    /// or, unless `strict`, after going level for a while.
    pub fn can_go_up(&self, edge: EdgeId, strict: bool) -> bool {
        let (from, to) = (self.edge[edge].from, self.edge[edge].to);
        if self.radius(to) > self.radius(from) {
            return true;
        }
        if self.radius(to) < self.radius(from) || strict {
            return false;
        }
        // Between two nodes as far in as each other: look further.
        let twin = self.edge[edge].twin;
        let mut outgoing = self.edge[edge].next;
        while outgoing != twin {
            let Some(out) = outgoing else { return false };
            if self.can_go_up(out, false) {
                return true;
            }
            let Some(out_twin) = self.edge[out].twin else {
                return false;
            };
            if self.edge[out_twin].next.is_none() {
                // On the outline, which should not happen.
                return true;
            }
            outgoing = self.edge[out_twin].next;
        }
        false
    }

    /// How far along level edges before `edge` leads further from the
    /// outline; none if it never does.
    pub fn dist_to_go_up(&self, edge: EdgeId) -> Option<Coord> {
        let (from, to) = (self.edge[edge].from, self.edge[edge].to);
        if self.radius(to) > self.radius(from) {
            return Some(0);
        }
        if self.radius(to) < self.radius(from) {
            return None;
        }
        let mut found: Option<Coord> = None;
        let twin = self.edge[edge].twin;
        let mut outgoing = self.edge[edge].next;
        while outgoing != twin {
            let Some(out) = outgoing else { break };
            if let Some(up) = self.dist_to_go_up(out) {
                found = Some(found.map_or(up, |so_far| so_far.min(up)));
            }
            let out_twin = self.edge[out].twin?;
            if self.edge[out_twin].next.is_none() {
                return Some(0);
            }
            outgoing = self.edge[out_twin].next;
        }
        found.map(|found| found + length(self.node[to].p, self.node[from].p))
    }

    /// Whether `edge` runs away from the outline, its twin then running
    /// towards it: where the two ends are as far in as each other, the
    /// half that reaches higher ground sooner.
    pub fn is_upward(&self, edge: EdgeId) -> bool {
        let (from, to) = (self.edge[edge].from, self.edge[edge].to);
        if self.radius(to) > self.radius(from) {
            return true;
        }
        if self.radius(to) < self.radius(from) {
            return false;
        }
        let forward = self.dist_to_go_up(edge);
        let backward = self.edge[edge]
            .twin
            .and_then(|twin| self.dist_to_go_up(twin));
        match (forward, backward) {
            (Some(forward), Some(backward)) => forward < backward,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            // Either way would do, as long as the twin says the opposite.
            (None, None) => {
                let (to, from) = (self.node[to].p, self.node[from].p);
                (to.x, to.y) < (from.x, from.y)
            }
        }
    }

    /// The edge leaving the outline on the far side of the cell `edge`
    /// is in, seen from the next cell; none where the cell is a ring.
    pub fn next_unconnected(&self, edge: EdgeId) -> Option<EdgeId> {
        let mut result = edge;
        while let Some(next) = self.edge[result].next {
            result = next;
            if result == edge {
                return None;
            }
        }
        self.edge[result].twin
    }

    /// Whether more than two edges with lines along them meet at `node`.
    pub fn is_multi_intersection(&self, node: NodeId) -> bool {
        let mut odd_path_count = 0;
        let first = self.node[node].incident_edge;
        let mut outgoing = first;
        loop {
            let Some(out) = outgoing else { return false };
            if self.edge[out].is_central() {
                odd_path_count += 1;
            }
            outgoing = self.edge[out].twin.and_then(|twin| self.edge[twin].next);
            if outgoing == first {
                break;
            }
        }
        odd_path_count > 2
    }

    /// Whether any edge at `node` has lines along it.
    pub fn is_central(&self, node: NodeId) -> bool {
        let Some(first) = self.node[node].incident_edge else {
            return false;
        };
        let mut edge = first;
        loop {
            if self.edge[edge].is_central() {
                return true;
            }
            let Some(twin) = self.edge[edge].twin else {
                return false;
            };
            match self.edge[twin].next {
                Some(next) if next != first => edge = next,
                _ => return false,
            }
        }
    }

    /// Whether nowhere next to `node` is further from the outline.
    pub fn is_local_maximum(&self, node: NodeId, strict: bool) -> bool {
        if self.node[node].distance_to_boundary == 0 {
            return false;
        }
        let Some(first) = self.node[node].incident_edge else {
            return false;
        };
        let mut edge = first;
        loop {
            if self.can_go_up(edge, strict) {
                return false;
            }
            let Some(twin) = self.edge[edge].twin else {
                return false;
            };
            let Some(next) = self.edge[twin].next else {
                // On the outline.
                return false;
            };
            edge = next;
            if edge == first {
                return true;
            }
        }
    }

    /// Takes out edges no longer than `snap_dist`: a cell's top where its
    /// two sides nearly meet, and a whole cell whose sides lie on each
    /// other.
    pub fn collapse_small_edges(&mut self, snap_dist: Coord) {
        for quad_start in self.edges() {
            if self.edge_removed[quad_start] || self.edge[quad_start].prev.is_some() {
                continue;
            }
            let mut quad_end = quad_start;
            while let Some(next) = self.edge[quad_end].next {
                quad_end = next;
            }
            let quad_mid = self.edge[quad_start].next.filter(|&next| next != quad_end);

            if let Some(mid) = quad_mid
                && shorter_than(
                    self.node[self.edge[mid].from].p,
                    self.node[self.edge[mid].to].p,
                    snap_dist,
                )
            {
                let Some(mid_twin) = self.edge[mid].twin else {
                    continue;
                };
                let mid_from = self.edge[mid].from;
                let mut count = 0;
                let mut edge_from_3 = Some(quad_end);
                while let Some(edge) = edge_from_3.filter(|&edge| edge != mid_twin) {
                    self.edge[edge].from = mid_from;
                    let Some(twin) = self.edge[edge].twin else {
                        break;
                    };
                    self.edge[twin].to = mid_from;
                    count += 1;
                    if count > 1000 {
                        break;
                    }
                    edge_from_3 = self.edge[twin].next;
                }

                if self.node[mid_from].incident_edge == Some(mid) {
                    self.node[mid_from].incident_edge = match self.edge[mid_twin].next {
                        Some(next) => Some(next),
                        None => self.edge[mid].prev.and_then(|prev| self.edge[prev].twin),
                    };
                }
                let mid_to = self.edge[mid].to;
                self.node_removed[mid_to] = true;

                let (prev, next) = (self.edge[mid].prev, self.edge[mid].next);
                if let Some(prev) = prev {
                    self.edge[prev].next = next;
                }
                if let Some(next) = next {
                    self.edge[next].prev = prev;
                }
                let (twin_prev, twin_next) = (self.edge[mid_twin].prev, self.edge[mid_twin].next);
                if let Some(twin_next) = twin_next {
                    self.edge[twin_next].prev = twin_prev;
                }
                if let Some(twin_prev) = twin_prev {
                    self.edge[twin_prev].next = twin_next;
                }
                self.edge_removed[mid_twin] = true;
                self.edge_removed[mid] = true;
            }

            let (start_from, start_to) = (self.edge[quad_start].from, self.edge[quad_start].to);
            let (end_from, end_to) = (self.edge[quad_end].from, self.edge[quad_end].to);
            if shorter_than(self.node[start_from].p, self.node[end_to].p, snap_dist)
                && shorter_than(self.node[start_to].p, self.node[end_from].p, snap_dist)
            {
                // The two sides become one edge and the cell is gone.
                let (Some(start_twin), Some(end_twin)) =
                    (self.edge[quad_start].twin, self.edge[quad_end].twin)
                else {
                    continue;
                };
                self.edge[start_twin].to = end_to;
                self.node[end_to].incident_edge = Some(end_twin);
                if self.node[end_from].incident_edge == Some(quad_end) {
                    self.node[end_from].incident_edge = match self.edge[end_twin].next {
                        Some(next) => Some(next),
                        None => self.edge[quad_end]
                            .prev
                            .and_then(|prev| self.edge[prev].twin),
                    };
                }
                self.node_removed[start_from] = true;
                self.edge[start_twin].twin = Some(end_twin);
                self.edge[end_twin].twin = Some(start_twin);
                self.edge_removed[quad_start] = true;
                self.edge_removed[quad_end] = true;
            }
        }
    }

    /// From the end of `prev_edge` out to the outline and back: the rib
    /// that closes one trapezoid and opens the next. `prev_edge` becomes
    /// the half coming back.
    pub fn make_rib(&mut self, prev_edge: &mut EdgeId, start_source: Point2, end_source: Point2) {
        let at = self.edge[*prev_edge].to;
        let p = nearest_on_line(start_source, end_source, self.node[at].p);
        let dist = length(self.node[at].p, p);
        self.node[at].distance_to_boundary = dist;

        let node = self.new_node_front(p);
        self.node[node].distance_to_boundary = 0;
        let forth = self.new_edge_front(EdgeType::ExtraVd);
        let back = self.new_edge_front(EdgeType::ExtraVd);

        self.edge[*prev_edge].next = Some(forth);
        self.edge[forth].prev = Some(*prev_edge);
        self.edge[forth].from = at;
        self.edge[forth].to = node;
        self.edge[forth].twin = Some(back);
        self.edge[back].twin = Some(forth);
        self.edge[back].from = node;
        self.edge[back].to = at;
        self.node[node].incident_edge = Some(back);

        *prev_edge = back;
    }

    /// Cuts `edge` at `mid_node` and runs a rib from there to the
    /// outline. Gives back the two halves of what was `edge`.
    fn insert_rib(&mut self, edge: EdgeId, mid_node: NodeId) -> (EdgeId, EdgeId) {
        let edge_before = self.edge[edge].prev;
        let edge_after = self.edge[edge].next;
        let node_before = self.edge[edge].from;
        let node_after = self.edge[edge].to;

        let p = self.node[mid_node].p;
        let (source_a, source_b) = self.source(edge);
        let px = nearest_on_segment(source_a, source_b, p);
        let dist = length(p, px);
        debug_assert!(dist > 0);
        self.node[mid_node].distance_to_boundary = dist;
        // At either end of a change a whole number of lines fits.
        self.node[mid_node].transition_ratio = 0.0;

        let source_node = self.new_node_back(px);
        self.node[source_node].distance_to_boundary = 0;

        let first = edge;
        let second = self.new_edge_back(EdgeType::Normal);
        let outward = self.new_edge_back(EdgeType::TransitionEnd);
        let inward = self.new_edge_back(EdgeType::TransitionEnd);

        if let Some(before) = edge_before {
            self.edge[before].next = Some(first);
        }
        self.edge[first].next = Some(outward);
        self.edge[outward].next = None;
        self.edge[inward].next = Some(second);
        self.edge[second].next = edge_after;

        if let Some(after) = edge_after {
            self.edge[after].prev = Some(second);
        }
        self.edge[second].prev = Some(inward);
        self.edge[inward].prev = None;
        self.edge[outward].prev = Some(first);
        self.edge[first].prev = edge_before;

        self.edge[first].to = mid_node;
        self.edge[outward].to = source_node;
        self.edge[inward].to = mid_node;
        self.edge[second].to = node_after;

        self.edge[first].from = node_before;
        self.edge[outward].from = mid_node;
        self.edge[inward].from = source_node;
        self.edge[second].from = mid_node;

        self.node[node_before].incident_edge = Some(first);
        self.node[mid_node].incident_edge = Some(outward);
        self.node[source_node].incident_edge = Some(inward);
        if let Some(after) = edge_after {
            self.node[node_after].incident_edge = Some(after);
        }

        self.edge[first].central = Some(true);
        self.edge[outward].central = Some(false);
        self.edge[inward].central = Some(false);
        self.edge[second].central = Some(true);

        self.edge[outward].twin = Some(inward);
        self.edge[inward].twin = Some(outward);
        // Not known yet.
        self.edge[first].twin = None;
        self.edge[second].twin = None;

        (first, second)
    }

    /// Puts a node on `edge` at `mid`, with a rib to the outline on
    /// either side. Gives back the half of `edge` beyond the new node.
    pub fn insert_node(&mut self, edge: EdgeId, mid: Point2, mid_node_bead_count: Coord) -> EdgeId {
        let mid_node = self.new_node_back(mid);
        let twin = self.edge[edge]
            .twin
            .expect("an edge on the skeleton has a twin");
        self.edge[edge].twin = None;
        self.edge[twin].twin = None;
        let (first_of_input, last_of_input) = self.insert_rib(edge, mid_node);
        let (first_of_twin, last_of_twin) = self.insert_rib(twin, mid_node);

        self.edge[first_of_input].twin = Some(last_of_twin);
        self.edge[last_of_twin].twin = Some(first_of_input);
        self.edge[last_of_input].twin = Some(first_of_twin);
        self.edge[first_of_twin].twin = Some(last_of_input);

        self.node[mid_node].bead_count = mid_node_bead_count;
        last_of_input
    }

    /// The piece of the outline the cell of `edge` lies against.
    pub fn source(&self, edge: EdgeId) -> (Point2, Point2) {
        let mut from_edge = edge;
        while let Some(prev) = self.edge[from_edge].prev {
            from_edge = prev;
        }
        let mut to_edge = edge;
        while let Some(next) = self.edge[to_edge].next {
            to_edge = next;
        }
        (
            self.node[self.edge[from_edge].from].p,
            self.node[self.edge[to_edge].to].p,
        )
    }
}
