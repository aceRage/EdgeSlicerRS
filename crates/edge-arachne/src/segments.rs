//! The lines themselves: what fits at each node is carried along the
//! skeleton to the nodes that were given nothing, every rib is marked
//! where each line crosses it, and the marks of one line are joined from
//! rib to rib (`generateExtraRibs`, `generateSegments` and what they
//! call, SkeletalTrapezoidation.cpp).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use edge_geom::{Coord, Point2, scaled};

use crate::beading::Beading;
use crate::graph::{BeadingPropagation, EdgeId, NodeId, length, shorter_than};
use crate::skeleton::Skeleton;
use crate::{ExtrusionLine, Junction, VariableWidthLines};

/// How far beading is looked for round a node that has none
/// (`SKELETAL_TRAPEZOIDATION_BEAD_SEARCH_MAX`).
const BEAD_SEARCH_MAX: usize = 1000;

/// A mix of two beadings, `ratio_left_to_whole` of the left one
/// (`interpolate`).
fn interpolate(left: &Beading, ratio_left_to_whole: f64, right: &Beading) -> Beading {
    let ratio_right_to_whole = f64::from((1.0 - ratio_left_to_whole) as f32);
    let mut ret = if left.total_thickness > right.total_thickness {
        left.clone()
    } else {
        right.clone()
    };
    for inset in 0..left.bead_widths.len().min(right.bead_widths.len()) {
        // A wall marked by a width of nothing stays so.
        ret.bead_widths[inset] = if left.bead_widths[inset] == 0 || right.bead_widths[inset] == 0 {
            0
        } else {
            (ratio_left_to_whole * left.bead_widths[inset] as f64
                + ratio_right_to_whole * right.bead_widths[inset] as f64) as Coord
        };
        ret.toolpath_locations[inset] = (ratio_left_to_whole
            * left.toolpath_locations[inset] as f64
            + ratio_right_to_whole * right.toolpath_locations[inset] as f64)
            as Coord;
    }
    ret
}

/// As [`interpolate`], but more of the left where the mix would put a
/// line beyond `switching_radius` that the left has within it.
fn interpolate_at(
    left: &Beading,
    ratio_left_to_whole: f64,
    right: &Beading,
    switching_radius: Coord,
) -> Beading {
    let ret = interpolate(left, ratio_left_to_whole, right);
    let Some(next_inset) = left
        .toolpath_locations
        .iter()
        .rposition(|&location| switching_radius > location)
    else {
        return ret;
    };
    if next_inset + 1 == left.toolpath_locations.len()
        || next_inset >= ret.toolpath_locations.len()
        || next_inset >= right.toolpath_locations.len()
    {
        return ret;
    }
    if ret.toolpath_locations[next_inset] > switching_radius {
        // One line was lost between the left and the mix.
        let new_ratio = (switching_radius - right.toolpath_locations[next_inset]) as f32
            / (left.toolpath_locations[next_inset] - right.toolpath_locations[next_inset]) as f32;
        let new_ratio = (f64::from(new_ratio) + 0.1).min(1.0) as f32;
        return interpolate(left, f64::from(new_ratio), right);
    }
    ret
}

impl Skeleton<'_> {
    fn span(&self, edge: EdgeId) -> Coord {
        let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
        length(self.graph.node[to].p, self.graph.node[from].p)
    }

    fn radius(&self, node: NodeId) -> Coord {
        self.graph.node[node].distance_to_boundary
    }

    /// Ribs where the strategy's widths do not change evenly with the
    /// part's. The strategies the reference builds name no such places,
    /// so this adds nothing; it is here for one that does.
    pub(crate) fn generate_extra_ribs(&mut self, discretization_step_size: Coord) {
        let snap_dist = scaled(0.02);
        let mut done = 0;
        loop {
            let edges = self.graph.edges();
            if done >= edges.len() {
                break;
            }
            for &edge in &edges[done..] {
                let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
                if !self.graph.edge[edge].is_central()
                    || shorter_than(
                        self.graph.node[to].p,
                        self.graph.node[from].p,
                        discretization_step_size,
                    )
                    || self.radius(from) >= self.radius(to)
                {
                    continue;
                }
                let rib_thicknesses = self
                    .strategy
                    .get_nonlinear_thicknesses(self.graph.node[from].bead_count);
                if rib_thicknesses.is_empty() {
                    continue;
                }
                let (a, b) = (self.graph.node[from].p, self.graph.node[to].p);
                let ab_size = length(b, a);
                let (a_r, b_r) = (self.radius(from), self.radius(to));
                let mut last_edge_replacing_input = edge;
                for rib_thickness in rib_thicknesses {
                    if rib_thickness / 2 <= a_r {
                        continue;
                    }
                    if rib_thickness / 2 >= b_r {
                        break;
                    }
                    let new_node_bead_count = self.graph.node[self.graph.edge[edge].from]
                        .bead_count
                        .min(self.graph.node[self.graph.edge[edge].to].bead_count);
                    let end_pos = ab_size * (rib_thickness / 2 - a_r) / (b_r - a_r);
                    let close_node = if end_pos < ab_size / 2 { from } else { to };
                    if (end_pos < snap_dist || end_pos > ab_size - snap_dist)
                        && self.graph.node[close_node].bead_count == new_node_bead_count
                    {
                        self.graph.node[close_node].transition_ratio = 0.0;
                        continue;
                    }
                    let size = length(b, a).max(1);
                    let mid = Point2 {
                        x: a.x + (b.x - a.x) * end_pos / size,
                        y: a.y + (b.y - a.y) * end_pos / size,
                    };
                    last_edge_replacing_input =
                        self.graph
                            .insert_node(last_edge_replacing_input, mid, new_node_bead_count);
                }
            }
            done = edges.len();
        }
    }

    pub(crate) fn generate_segments(&mut self) -> Vec<VariableWidthLines> {
        // The middle edges of the cells, each as the half that runs away
        // from the outline, furthest in first.
        let mut upward_quad_mids: Vec<EdgeId> = self
            .graph
            .edges()
            .into_iter()
            .filter(|&edge| {
                self.graph.edge[edge].prev.is_some()
                    && self.graph.edge[edge].next.is_some()
                    && self.graph.is_upward(edge)
            })
            .collect();
        let before = |a: EdgeId, b: EdgeId| -> bool {
            let ends = |edge: EdgeId| {
                (
                    self.radius(self.graph.edge[edge].from),
                    self.radius(self.graph.edge[edge].to),
                )
            };
            let ((a_from, a_to), (b_from, b_to)) = (ends(a), ends(b));
            if a_to == b_to {
                // Where one of two is level and joined to the other, the
                // order between them matters.
                let from_up = |edge: EdgeId| {
                    let twin = self.graph.edge[edge].twin;
                    let nearest = self.graph.dist_to_go_up(edge).unwrap_or(Coord::MAX).min(
                        twin.and_then(|twin| self.graph.dist_to_go_up(twin))
                            .unwrap_or(Coord::MAX),
                    );
                    nearest - self.span(edge)
                };
                if a_from == a_to && b_from == b_to {
                    return from_up(a) < from_up(b);
                } else if a_from == a_to {
                    return true;
                } else if b_from == b_to {
                    return false;
                }
            }
            a_to > b_to
        };
        // The reference sorts with `std::sort`, which may leave equals in
        // any order; here they keep the order they had.
        upward_quad_mids.sort_by(|&a, &b| {
            if before(a, b) {
                Ordering::Less
            } else if before(b, a) {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        });

        for node in self.graph.nodes() {
            let (bead_count, ratio, thickness) = {
                let node = &self.graph.node[node];
                (
                    node.bead_count,
                    node.transition_ratio,
                    node.distance_to_boundary * 2,
                )
            };
            if bead_count <= 0 {
                continue;
            }
            let beading = if ratio == 0.0 {
                self.strategy.compute(thickness, bead_count)
            } else {
                let low = self.strategy.compute(thickness, bead_count);
                let high = self.strategy.compute(thickness, bead_count + 1);
                interpolate(&low, 1.0 - f64::from(ratio), &high)
            };
            self.graph.node[node].beading = Some(BeadingPropagation::new(beading));
        }

        self.propagate_beadings_upward(&upward_quad_mids);
        self.propagate_beadings_downward(&upward_quad_mids);
        self.generate_junctions();
        let mut toolpaths = Vec::new();
        self.connect_junctions(&mut toolpaths);
        self.generate_local_maxima_single_beads(&mut toolpaths);
        toolpaths
    }

    fn propagate_beadings_upward(&mut self, upward_quad_mids: &[EdgeId]) {
        for &upward_edge in upward_quad_mids.iter().rev() {
            let (from, to) = (
                self.graph.edge[upward_edge].from,
                self.graph.edge[upward_edge].to,
            );
            // What a node was given is not overridden, and there must
            // be something to carry and somewhere to put it.
            if self.graph.node[to].bead_count >= 0 || self.graph.node[to].beading.is_some() {
                continue;
            }
            let Some(lower) = &self.graph.node[from].beading else {
                continue;
            };
            let mut upper = lower.clone();
            upper.dist_to_bottom_source += self.span(upward_edge);
            upper.is_upward_propagated_only = true;
            self.graph.node[to].beading = Some(upper);
        }
    }

    fn propagate_beadings_downward(&mut self, upward_quad_mids: &[EdgeId]) {
        for &mid in upward_quad_mids {
            if self.graph.edge[mid].is_central() {
                continue;
            }
            let (from, to) = (self.graph.edge[mid].from, self.graph.edge[mid].to);
            // On a level edge, from the node that has beading to the one
            // that has not.
            if self.radius(from) == self.radius(to)
                && self.graph.node[from].beading.is_some()
                && self.graph.node[to].beading.is_none()
                && let Some(twin) = self.graph.edge[mid].twin
            {
                self.propagate_beading_downward(twin);
            } else {
                self.propagate_beading_downward(mid);
            }
        }
    }

    fn propagate_beading_downward(&mut self, edge_to_peak: EdgeId) {
        let (from, to) = (
            self.graph.edge[edge_to_peak].from,
            self.graph.edge[edge_to_peak].to,
        );
        let length = self.span(edge_to_peak);
        let top = self.get_or_create_beading(to);
        let from_radius = self.radius(from);
        let transition_dist = self.beading_propagation_transition_dist;
        match &mut self.graph.node[from].beading {
            None => {
                let mut propagated = top;
                propagated.dist_from_top_source += length;
                self.graph.node[from].beading = Some(propagated);
            }
            Some(bottom) => {
                let total_dist = top.dist_from_top_source + length + bottom.dist_to_bottom_source;
                let ratio_of_top = f64::from(
                    bottom.dist_to_bottom_source as f32 / total_dist.min(transition_dist) as f32,
                )
                .max(0.0);
                if ratio_of_top >= 1.0 {
                    *bottom = top;
                    bottom.dist_from_top_source += length;
                } else {
                    let merged =
                        interpolate_at(&top.beading, ratio_of_top, &bottom.beading, from_radius);
                    *bottom = BeadingPropagation::new(merged);
                }
            }
        }
    }

    /// What fits at `node`: its own, or failing that what a node close
    /// by has, or what the strategy says of its width.
    fn get_or_create_beading(&mut self, node: NodeId) -> BeadingPropagation {
        if let Some(beading) = &self.graph.node[node].beading {
            return beading.clone();
        }
        if self.graph.node[node].bead_count == -1 {
            if let Some(nearest) = self.nearest_beading(node, scaled(0.1)) {
                return nearest;
            }
            let mut dist = Coord::MAX;
            let first = self.graph.node[node].incident_edge;
            let mut edge = first;
            while let Some(at) = edge {
                let to = self.graph.edge[at].to;
                dist = dist.min(self.radius(to) + self.span(at));
                edge = self.graph.edge[at]
                    .twin
                    .and_then(|twin| self.graph.edge[twin].next);
                if edge == first {
                    break;
                }
            }
            self.graph.node[node].bead_count =
                self.strategy.get_optimal_bead_count(dist.saturating_mul(2));
        }
        let beading = BeadingPropagation::new(
            self.strategy
                .compute(self.radius(node) * 2, self.graph.node[node].bead_count),
        );
        self.graph.node[node].beading = Some(beading.clone());
        beading
    }

    /// The beading of the nearest node along the graph that has one,
    /// within `max_dist`.
    fn nearest_beading(&self, node: NodeId, max_dist: Coord) -> Option<BeadingPropagation> {
        // Nearest first; of two as near, the one found first.
        let mut further: BinaryHeap<std::cmp::Reverse<(Coord, usize, EdgeId)>> = BinaryHeap::new();
        let mut order = 0;
        let mut push = |heap: &mut BinaryHeap<_>, dist: Coord, edge: EdgeId| {
            heap.push(std::cmp::Reverse((dist, order, edge)));
            order += 1;
        };
        let first = self.graph.node[node].incident_edge;
        let mut outgoing = first;
        while let Some(out) = outgoing {
            push(&mut further, self.span(out), out);
            outgoing = self.graph.edge[out]
                .twin
                .and_then(|twin| self.graph.edge[twin].next);
            if outgoing == first {
                break;
            }
        }
        for _ in 0..BEAD_SEARCH_MAX {
            let std::cmp::Reverse((dist, _, edge_to)) = further.pop()?;
            if dist > max_dist {
                return None;
            }
            if let Some(beading) = &self.graph.node[self.graph.edge[edge_to].to].beading {
                return Some(beading.clone());
            }
            for next in self.onward(edge_to) {
                push(&mut further, dist + self.span(next), next);
            }
        }
        None
    }

    /// On every edge that climbs to a node with a different count, where
    /// each line of the node's beading crosses it, furthest in first.
    fn generate_junctions(&mut self) {
        for edge in self.graph.edges() {
            let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
            if self.radius(from) > self.radius(to) {
                continue;
            }
            let start_r = self.radius(to);
            let end_r = self.radius(from);
            let (from_count, to_count) = (
                self.graph.node[from].bead_count,
                self.graph.node[to].bead_count,
            );
            if (from_count == to_count && from_count >= 0) || end_r >= start_r {
                continue;
            }
            let beading = self.get_or_create_beading(to).beading;
            let (a, b) = (self.graph.node[to].p, self.graph.node[from].p);
            let ab = Point2 {
                x: b.x - a.x,
                y: b.y - a.y,
            };
            let num_junctions = beading.toolpath_locations.len() as isize;
            let within = |index: isize| index >= 0 && index < num_junctions;
            // The first line this edge has: the outermost that is not
            // beyond its inner end, give or take a rounding.
            let mut index = (num_junctions.max(1) - 1) / 2;
            while within(index) {
                if beading.toolpath_locations[index as usize] <= start_r + 1 {
                    break;
                }
                index -= 1;
            }
            if within(index + 1)
                && beading.toolpath_locations[(index + 1) as usize] <= start_r + scaled(0.005)
                && beading.total_thickness < start_r + scaled(0.005)
            {
                index += 1;
            }
            let mut junctions = Vec::new();
            while within(index) {
                let bead_r = beading.toolpath_locations[index as usize];
                if bead_r < end_r {
                    // One on the node is the next edge's.
                    break;
                }
                let mut junction = Point2 {
                    x: a.x + ab.x * (bead_r - start_r) / (end_r - start_r),
                    y: a.y + ab.y * (bead_r - start_r) / (end_r - start_r),
                };
                if bead_r > start_r - scaled(0.005) {
                    // On the inner node if very near it, so that a
                    // meeting of three is seen for one later.
                    junction = a;
                }
                junctions.push(Junction {
                    p: junction,
                    w: beading.bead_widths[index as usize],
                    perimeter_index: index as usize,
                });
                index -= 1;
            }
            self.graph.edge[edge].junctions = Some(junctions);
        }
    }

    /// The edge of a cell that ends furthest from the outline
    /// (`getQuadMaxRedgeTo`).
    fn quad_max_r_edge_to(&self, quad_start: EdgeId) -> EdgeId {
        let mut max_r = -1;
        let mut ret = quad_start;
        let mut edge = Some(quad_start);
        while let Some(at) = edge {
            let r = self.radius(self.graph.edge[at].to);
            if r > max_r {
                max_r = r;
                ret = at;
            }
            edge = self.graph.edge[at].next;
        }
        if self.graph.edge[ret].next.is_none()
            && self.radius(self.graph.edge[ret].to) - scaled(0.005)
                < self.radius(self.graph.edge[ret].from)
            && let Some(prev) = self.graph.edge[ret].prev
        {
            ret = prev;
        }
        ret
    }

    /// Cell by cell round each part of the outline, joins each line's
    /// mark on the way into the cell to its mark on the way out.
    fn connect_junctions(&mut self, toolpaths: &mut Vec<VariableWidthLines>) {
        // The reference keeps these in a set that hands them back in
        // the order put in, the last moved into the place of one taken
        // out; the same is done here.
        let mut unprocessed: Vec<EdgeId> = self
            .graph
            .edges()
            .into_iter()
            .filter(|&edge| self.graph.edge[edge].prev.is_none())
            .collect();
        let mut place: HashMap<EdgeId, usize> = unprocessed
            .iter()
            .enumerate()
            .map(|(index, &edge)| (edge, index))
            .collect();
        let mut passed_odd_edges: HashSet<EdgeId> = HashSet::new();
        let junctions_of = |skeleton: &Self, edge: Option<EdgeId>| -> Vec<Junction> {
            edge.and_then(|edge| skeleton.graph.edge[edge].junctions.clone())
                .unwrap_or_default()
        };

        while let Some(&poly_domain_start) = unprocessed.first() {
            let mut quad_start = poly_domain_start;
            let mut new_domain_start = true;
            loop {
                let mut quad_end = quad_start;
                while let Some(next) = self.graph.edge[quad_end].next {
                    quad_end = next;
                }
                let edge_to_peak = self.quad_max_r_edge_to(quad_start);
                let edge_from_peak = self.graph.edge[edge_to_peak].next;

                if let Some(index) = place.remove(&quad_start) {
                    unprocessed.swap_remove(index);
                    if let Some(&moved) = unprocessed.get(index) {
                        place.insert(moved, index);
                    }
                }

                // The marks from the start of the cell up to its peak,
                // and from its end up to its peak.
                let mut from_junctions = junctions_of(self, Some(edge_to_peak));
                let from_peak_twin = edge_from_peak.and_then(|edge| self.graph.edge[edge].twin);
                let mut to_junctions = junctions_of(self, from_peak_twin);
                if let Some(prev) = self.graph.edge[edge_to_peak].prev {
                    let from_prev = junctions_of(self, Some(prev));
                    while let (Some(last), Some(first)) = (from_junctions.last(), from_prev.first())
                    {
                        if last.perimeter_index > first.perimeter_index {
                            break;
                        }
                        from_junctions.pop();
                    }
                    from_junctions.extend(from_prev);
                }
                if let Some(next) = edge_from_peak.and_then(|edge| self.graph.edge[edge].next) {
                    let to_next = junctions_of(self, self.graph.edge[next].twin);
                    while let (Some(last), Some(first)) = (to_junctions.last(), to_next.first()) {
                        if last.perimeter_index > first.perimeter_index {
                            break;
                        }
                        to_junctions.pop();
                    }
                    to_junctions.extend(to_next);
                }

                let start_to = self.graph.edge[quad_start].to;
                let end_from = self.graph.edge[quad_end].from;
                let start_next = self.graph.edge[quad_start].next;
                let segment_count = from_junctions.len().min(to_junctions.len());
                for rev in 0..segment_count {
                    let from = from_junctions[from_junctions.len() - 1 - rev];
                    let to = to_junctions[to_junctions.len() - 1 - rev];
                    // The single line down the middle, at either end.
                    let odd_at = |node: NodeId, p: Point2| {
                        let node = &self.graph.node[node];
                        node.bead_count > 0
                            && node.bead_count % 2 == 1
                            && node.transition_ratio == 0.0
                            && rev == segment_count - 1
                            && shorter_than(p, node.p, scaled(0.005))
                    };
                    let from_is_odd = odd_at(start_to, from.p);
                    let to_is_odd = odd_at(end_from, to.p);
                    let is_odd_segment = from_is_odd && to_is_odd;
                    // Once only, though two cells share it.
                    if is_odd_segment
                        && start_next
                            .and_then(|next| self.graph.edge[next].twin)
                            .is_some_and(|twin| passed_odd_edges.contains(&twin))
                    {
                        continue;
                    }
                    let from_is_3way = from_is_odd && self.graph.is_multi_intersection(start_to);
                    let to_is_3way = to_is_odd && self.graph.is_multi_intersection(end_from);
                    if let Some(next) = start_next {
                        passed_odd_edges.insert(next);
                    }
                    add_toolpath_segment(
                        toolpaths,
                        from,
                        to,
                        is_odd_segment,
                        new_domain_start,
                        from_is_3way,
                        to_is_3way,
                    );
                }
                new_domain_start = false;
                match self.graph.next_unconnected(quad_start) {
                    Some(next) if next != poly_domain_start => quad_start = next,
                    _ => break,
                }
            }
        }
    }

    /// A dot where the part is widest and nothing runs through: a tiny
    /// ring, as much plastic as a blob one line wide.
    fn generate_local_maxima_single_beads(&self, toolpaths: &mut Vec<VariableWidthLines>) {
        for node in self.graph.nodes() {
            let Some(beading) = &self.graph.node[node].beading else {
                continue;
            };
            let beading = &beading.beading;
            if beading.bead_widths.len() % 2 != 1
                || !self.graph.is_local_maximum(node, true)
                || self.graph.is_central(node)
            {
                continue;
            }
            let inset_index = beading.bead_widths.len() / 2;
            if inset_index >= toolpaths.len() {
                toolpaths.resize(inset_index + 1, Vec::new());
            }
            let width = beading.bead_widths[inset_index];
            // A ring of radius w/8 is as long as a blob of width w needs.
            let r = (width / 8) as f32;
            let p = self.graph.node[node].p;
            let junctions = (0..6)
                .map(|segment| {
                    let a = (2.0 * std::f64::consts::PI / 6.0 * f64::from(segment)) as f32;
                    Junction {
                        p: Point2 {
                            x: p.x + (r * a.cos()) as Coord,
                            y: p.y + (r * a.sin()) as Coord,
                        },
                        w: width,
                        perimeter_index: inset_index,
                    }
                })
                .collect();
            toolpaths[inset_index].push(ExtrusionLine {
                inset_idx: inset_index,
                is_odd: true,
                is_closed: false,
                junctions,
            });
        }
    }
}

/// Adds the piece from `from` to `to` to the last line of its depth
/// where it carries on from it, or as a new line (`addToolpathSegment`).
fn add_toolpath_segment(
    toolpaths: &mut Vec<VariableWidthLines>,
    from: Junction,
    to: Junction,
    is_odd: bool,
    mut force_new_path: bool,
    from_is_3way: bool,
    to_is_3way: bool,
) {
    if from == to {
        return;
    }
    let inset_idx = from.perimeter_index;
    if inset_idx >= toolpaths.len() {
        toolpaths.resize(inset_idx + 1, Vec::new());
    }
    let lines = &mut toolpaths[inset_idx];
    let last_junction = lines
        .last()
        .filter(|line| line.is_odd == is_odd)
        .and_then(|line| line.junctions.last().copied())
        .filter(|junction| junction.perimeter_index == inset_idx);
    let Some(last_junction) = last_junction else {
        force_new_path = true;
        lines.push(ExtrusionLine {
            inset_idx,
            is_odd,
            is_closed: false,
            junctions: vec![from, to],
        });
        debug_assert!(force_new_path);
        return;
    };
    let near = |junction: Junction| {
        shorter_than(last_junction.p, junction.p, scaled(0.010))
            && (last_junction.w - junction.w).abs() < scaled(0.010)
    };
    let line = lines.last_mut().expect("there is a last line");
    if !force_new_path && near(from) && !from_is_3way {
        line.junctions.push(to);
    } else if !force_new_path && near(to) && !to_is_3way {
        line.junctions.push(from);
    } else {
        lines.push(ExtrusionLine {
            inset_idx,
            is_odd,
            is_closed: false,
            junctions: vec![from, to],
        });
    }
}
