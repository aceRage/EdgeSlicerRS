//! Where the number of lines changes by one: the middle of every such
//! change is placed on the skeleton, changes too near each other to be
//! worth making are dropped, and each one left is given a stretch of
//! skeleton to happen over, with a rib at either end
//! (`generateTransitioningRibs` and what it calls,
//! SkeletalTrapezoidation.cpp).

use edge_geom::{Coord, Point2};

use crate::graph::{EdgeId, TransitionEnd, TransitionMiddle, length};
use crate::skeleton::Skeleton;

/// A middle to be dropped: the edge it is on and which of that edge's.
type MidRef = (EdgeId, usize);

impl Skeleton<'_> {
    pub(crate) fn generate_transitioning_ribs(&mut self) {
        self.generate_transition_mids();
        self.filter_transition_mids();
        self.generate_all_transition_ends();
        self.apply_transitions();
    }

    fn edge_length(&self, edge: EdgeId) -> Coord {
        let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
        length(self.graph.node[to].p, self.graph.node[from].p)
    }

    fn radius_at(&self, node: usize) -> Coord {
        self.graph.node[node].distance_to_boundary
    }

    /// On every central edge that climbs from fewer lines to more, a
    /// middle where the part is as wide as each change in between asks.
    fn generate_transition_mids(&mut self) {
        for edge in self.graph.edges() {
            if !self.graph.edge[edge].is_central() {
                continue;
            }
            let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
            let (start_r, end_r) = (self.radius_at(from), self.radius_at(to));
            let start_bead_count = self.graph.node[from].bead_count as i32;
            let end_bead_count = self.graph.node[to].bead_count as i32;
            // Only the half that goes away from the outline, and only
            // where the count differs.
            if start_r >= end_r || start_bead_count == end_bead_count {
                continue;
            }
            let edge_size = self.edge_length(edge);
            for lower_bead_count in start_bead_count..end_bead_count {
                let mid_r = (self
                    .strategy
                    .get_transition_thickness(Coord::from(lower_bead_count))
                    / 2)
                .clamp(start_r, end_r);
                let mid_pos = edge_size * (mid_r - start_r) / (end_r - start_r);
                self.graph.edge[edge]
                    .transitions
                    .get_or_insert_with(Vec::new)
                    .push(TransitionMiddle {
                        pos: mid_pos,
                        lower_bead_count,
                        feature_radius: mid_r,
                    });
            }
        }
    }

    /// Drops a change that is undone again within a short way, and one
    /// so near the end of the central edges that there is no room for it.
    fn filter_transition_mids(&mut self) {
        for edge in self.graph.edges() {
            if !self.graph.edge[edge].has_transitions(false) {
                continue;
            }
            let Some(twin) = self.graph.edge[edge].twin else {
                continue;
            };
            let ab_size = self.edge_length(edge);
            let last = |skeleton: &Self| {
                *skeleton.graph.edge[edge]
                    .transitions
                    .as_ref()
                    .and_then(|list| list.last())
                    .expect("the list is not empty")
            };
            let first = |skeleton: &Self| {
                skeleton.graph.edge[edge]
                    .transitions
                    .as_ref()
                    .and_then(|list| list.first())
                    .copied()
                    .expect("the list is not empty")
            };

            let back = last(self);
            let to_dissolve = self.dissolve_nearby_transitions(
                edge,
                back,
                ab_size - back.pos,
                self.transition_filter_dist,
                true,
            );
            let mut should_dissolve_back = !to_dissolve.is_empty();
            for _ in &to_dissolve {
                self.dissolve_bead_count_region(
                    edge,
                    Coord::from(back.lower_bead_count) + 1,
                    Coord::from(back.lower_bead_count),
                );
            }
            self.erase_transitions(to_dissolve);
            {
                let back = last(self);
                let count = Coord::from(back.lower_bead_count);
                let upper_half_length = ((1.0
                    - f64::from(self.strategy.get_transition_anchor_pos(count) as f32))
                    * self.strategy.get_transitioning_length(count) as f64)
                    as Coord;
                should_dissolve_back |= self.filter_end_of_central_transition(
                    edge,
                    ab_size - back.pos,
                    upper_half_length,
                    count,
                );
            }
            let transitions = self.graph.edge[edge]
                .transitions
                .as_mut()
                .expect("it had transitions");
            if should_dissolve_back {
                transitions.pop();
            }
            if transitions.is_empty() {
                continue;
            }

            let front = first(self);
            let to_dissolve = self.dissolve_nearby_transitions(
                twin,
                front,
                front.pos,
                self.transition_filter_dist,
                false,
            );
            let mut should_dissolve_front = !to_dissolve.is_empty();
            for _ in &to_dissolve {
                self.dissolve_bead_count_region(
                    twin,
                    Coord::from(front.lower_bead_count),
                    Coord::from(front.lower_bead_count) + 1,
                );
            }
            self.erase_transitions(to_dissolve);
            {
                let front = first(self);
                let count = Coord::from(front.lower_bead_count);
                // In single precision, as the reference works it out.
                let lower_half_length = (self.strategy.get_transition_anchor_pos(count) as f32
                    * self.strategy.get_transitioning_length(count) as f32)
                    as Coord;
                should_dissolve_front |= self.filter_end_of_central_transition(
                    twin,
                    front.pos,
                    lower_half_length,
                    count + 1,
                );
            }
            if should_dissolve_front
                && let Some(transitions) = self.graph.edge[edge].transitions.as_mut()
            {
                transitions.remove(0);
            }
        }
    }

    /// Takes the middles named out of their edges' lists.
    fn erase_transitions(&mut self, mut refs: Vec<MidRef>) {
        // Last first, so that taking one out does not move another.
        refs.sort_unstable_by(|a, b| b.cmp(a));
        refs.dedup();
        for (edge, index) in refs {
            if let Some(list) = self.graph.edge[edge].transitions.as_mut()
                && index < list.len()
            {
                list.remove(index);
            }
        }
    }

    /// The changes beyond the end of `edge_to_start`, within `max_dist`
    /// along central edges, that undo `origin`: all of them, or none if
    /// some way on there is none near enough.
    fn dissolve_nearby_transitions(
        &self,
        edge_to_start: EdgeId,
        origin: TransitionMiddle,
        traveled_dist: Coord,
        max_dist: Coord,
        going_up: bool,
    ) -> Vec<MidRef> {
        let mut to_be_dissolved = Vec::new();
        if traveled_dist > max_dist {
            return to_be_dissolved;
        }
        let mut should_dissolve = true;
        for edge in self.onward(edge_to_start) {
            if !self.graph.edge[edge].is_central() {
                continue;
            }
            let ab_size = self.edge_length(edge);
            let is_aligned = self.graph.is_upward(edge);
            let aligned_edge = if is_aligned {
                edge
            } else {
                self.graph.edge[edge]
                    .twin
                    .expect("a central edge has a twin")
            };
            let mut seen_transition_on_this_edge = false;

            let radius_here = self.radius_at(self.graph.edge[edge].from);
            let dissolve_result_is_odd = (origin.lower_bead_count % 2 != 0) == going_up;
            // Twice, as it is lost on both sides of the edge.
            let width_deviation = (origin.feature_radius - radius_here).abs() * 2;
            // Shared between one line or two.
            let line_width_deviation = if dissolve_result_is_odd {
                width_deviation
            } else {
                width_deviation / 2
            };
            if line_width_deviation > self.allowed_filter_deviation {
                should_dissolve = false;
            }

            if should_dissolve && self.graph.edge[aligned_edge].has_transitions(false) {
                let transitions = self.graph.edge[aligned_edge]
                    .transitions
                    .as_ref()
                    .expect("it has transitions");
                for (index, transition) in transitions.iter().enumerate() {
                    let pos = if is_aligned {
                        transition.pos
                    } else {
                        ab_size - transition.pos
                    };
                    if traveled_dist + pos < max_dist
                        && transition.lower_bead_count == origin.lower_bead_count
                    {
                        to_be_dissolved.push((aligned_edge, index));
                        seen_transition_on_this_edge = true;
                    }
                }
            }
            if should_dissolve && !seen_transition_on_this_edge {
                let here = self.dissolve_nearby_transitions(
                    edge,
                    origin,
                    traveled_dist + ab_size,
                    max_dist,
                    going_up,
                );
                if here.is_empty() {
                    // Too long this way, so not to be dissolved at all.
                    return Vec::new();
                }
                to_be_dissolved.extend(here);
                should_dissolve = should_dissolve && !to_be_dissolved.is_empty();
            }
        }
        if !should_dissolve {
            to_be_dissolved.clear();
        }
        to_be_dissolved
    }

    /// Changes the count from one number to another at the end of
    /// `edge_to_start` and on along central edges as far as it is the
    /// first number.
    fn dissolve_bead_count_region(
        &mut self,
        edge_to_start: EdgeId,
        from_bead_count: Coord,
        to_bead_count: Coord,
    ) {
        let to = self.graph.edge[edge_to_start].to;
        if self.graph.node[to].bead_count != from_bead_count {
            return;
        }
        self.graph.node[to].bead_count = to_bead_count;
        for edge in self.onward(edge_to_start) {
            if self.graph.edge[edge].is_central() {
                self.dissolve_bead_count_region(edge, from_bead_count, to_bead_count);
            }
        }
    }

    /// Where the central edges end within `max_dist` beyond
    /// `edge_to_start`, the count on the way there becomes
    /// `replacing_bead_count`; says whether they do.
    fn filter_end_of_central_transition(
        &mut self,
        edge_to_start: EdgeId,
        traveled_dist: Coord,
        max_dist: Coord,
        replacing_bead_count: Coord,
    ) -> bool {
        if traveled_dist > max_dist {
            return false;
        }
        let mut is_end_of_central = true;
        let mut should_dissolve = false;
        for next_edge in self.onward(edge_to_start) {
            if self.graph.edge[next_edge].is_central() {
                let length = self.edge_length(next_edge);
                should_dissolve |= self.filter_end_of_central_transition(
                    next_edge,
                    traveled_dist + length,
                    max_dist,
                    replacing_bead_count,
                );
                is_end_of_central = false;
            }
        }
        if is_end_of_central && traveled_dist < max_dist {
            should_dissolve = true;
        }
        if should_dissolve {
            let to = self.graph.edge[edge_to_start].to;
            self.graph.node[to].bead_count = replacing_bead_count;
        }
        should_dissolve
    }
}

/// How far the lines may be from a whole number at the end of the
/// outline's reach: the reference's `snap_dist()`.
const SNAP_DIST: f64 = 0.02;

/// `p0` made `len` long (SkeletalTrapezoidation.cpp `normal`).
fn normal(p0: Point2, len: Coord) -> Point2 {
    let size = length(p0, Point2 { x: 0, y: 0 });
    if size < 1 {
        return Point2 { x: len, y: 0 };
    }
    Point2 {
        x: p0.x * len / size,
        y: p0.y * len / size,
    }
}

impl Skeleton<'_> {
    fn generate_all_transition_ends(&mut self) {
        for edge in self.graph.edges() {
            if !self.graph.edge[edge].has_transitions(false) {
                continue;
            }
            let middles = self.graph.edge[edge]
                .transitions
                .clone()
                .unwrap_or_default();
            for middle in middles {
                self.generate_transition_ends(
                    edge,
                    middle.pos,
                    Coord::from(middle.lower_bead_count),
                );
            }
        }
    }

    /// Either end of the change whose middle is `mid_pos` along `edge`:
    /// the end with fewer lines back towards the outline, the end with
    /// more further in.
    fn generate_transition_ends(&mut self, edge: EdgeId, mid_pos: Coord, lower_bead_count: Coord) {
        let ab_size = self.edge_length(edge);
        let transition_length = self.strategy.get_transitioning_length(lower_bead_count);
        let transition_mid_position =
            self.strategy.get_transition_anchor_pos(lower_bead_count) as f32;
        let mid_rest = f64::from(transition_mid_position);
        let end_rest = 1.0;
        if let Some(twin) = self.graph.edge[edge].twin {
            let start_pos = ab_size - mid_pos;
            let half_length = (transition_mid_position * transition_length as f32) as Coord;
            let end_pos = start_pos + half_length;
            self.generate_transition_end(
                twin,
                start_pos,
                end_pos,
                half_length,
                mid_rest,
                0.0,
                lower_bead_count,
            );
        }
        let half_length =
            ((1.0 - f64::from(transition_mid_position)) * transition_length as f64) as Coord;
        self.generate_transition_end(
            edge,
            mid_pos,
            mid_pos + half_length,
            half_length,
            mid_rest,
            end_rest,
            lower_bead_count,
        );
    }

    /// Marks where a change ends, `end_pos` along `edge` or, if that is
    /// beyond the edge, on every central edge that carries on from it.
    /// Says whether every way on only led down.
    #[allow(clippy::too_many_arguments)]
    fn generate_transition_end(
        &mut self,
        edge: EdgeId,
        start_pos: Coord,
        end_pos: Coord,
        transition_half_length: Coord,
        start_rest: f64,
        end_rest: f64,
        lower_bead_count: Coord,
    ) -> bool {
        let ab_size = self.edge_length(edge);
        let going_up = end_rest > start_rest;
        if !self.graph.edge[edge].is_central() {
            return false;
        }
        if end_pos > ab_size {
            let rest = (end_rest
                - (start_rest - end_rest) * (end_pos - ab_size) as f64
                    / (start_pos - end_pos) as f64) as f32;
            let outgoing: Vec<EdgeId> = self
                .onward(edge)
                .into_iter()
                .filter(|&out| self.graph.edge[out].is_central())
                .collect();
            let central_edge_count = outgoing.len();
            let mut is_only_going_down = true;
            let mut has_recursed = false;
            for out in outgoing {
                if central_edge_count > 1
                    && going_up
                    && self.is_going_down(
                        out,
                        0,
                        end_pos - ab_size + transition_half_length,
                        lower_bead_count,
                    )
                {
                    // Beyond a fork, and this way the count falls: no
                    // end of a rise belongs here.
                    continue;
                }
                let is_going_down = self.generate_transition_end(
                    out,
                    0,
                    end_pos - ab_size,
                    transition_half_length,
                    f64::from(rest),
                    end_rest,
                    lower_bead_count,
                );
                is_only_going_down &= is_going_down;
                has_recursed = true;
            }
            if !going_up || (has_recursed && !is_only_going_down) {
                let to = self.graph.edge[edge].to;
                self.graph.node[to].transition_ratio = rest;
                self.graph.node[to].bead_count = lower_bead_count;
            }
            return is_only_going_down;
        }

        let is_lower_end = end_rest == 0.0;
        let (upward_edge, pos) = if self.graph.is_upward(edge) {
            (edge, end_pos)
        } else {
            (
                self.graph.edge[edge]
                    .twin
                    .expect("a central edge has a twin"),
                ab_size - end_pos,
            )
        };
        let ends = self.graph.edge[upward_edge]
            .transition_ends
            .get_or_insert_with(Vec::new);
        let end = TransitionEnd {
            pos,
            lower_bead_count: lower_bead_count as i32,
            is_lower_end,
        };
        if ends.first().is_none_or(|first| pos < first.pos) {
            ends.insert(0, end);
        } else {
            ends.push(end);
        }
        false
    }

    /// Whether the count falls to `lower_bead_count` within `max_dist`
    /// along every central way on from `outgoing`.
    fn is_going_down(
        &self,
        outgoing: EdgeId,
        traveled_dist: Coord,
        max_dist: Coord,
        lower_bead_count: Coord,
    ) -> bool {
        let (from, to) = (self.graph.edge[outgoing].from, self.graph.edge[outgoing].to);
        if self.radius_at(to) == 0 {
            return true;
        }
        let is_upward = self.radius_at(to) >= self.radius_at(from);
        let upward_edge = if is_upward {
            outgoing
        } else {
            self.graph.edge[outgoing]
                .twin
                .expect("a central edge has a twin")
        };
        let there = &self.graph.node[to];
        if there.bead_count > lower_bead_count + 1 {
            return false;
        }
        let length = self.edge_length(outgoing);
        if self.graph.edge[upward_edge].has_transitions(false) {
            let middles = self.graph.edge[upward_edge]
                .transitions
                .as_ref()
                .expect("it has transitions");
            let mid = if is_upward {
                middles[0]
            } else {
                middles[middles.len() - 1]
            };
            if Coord::from(mid.lower_bead_count) == lower_bead_count
                && ((is_upward && mid.pos + traveled_dist < max_dist)
                    || (!is_upward && length - mid.pos + traveled_dist < max_dist))
            {
                return true;
            }
        }
        if traveled_dist + length > max_dist {
            return false;
        }
        if there.bead_count <= lower_bead_count
            && !(there.bead_count == lower_bead_count && there.transition_ratio > 0.0)
        {
            return true;
        }
        let mut is_only_going_down = true;
        let mut has_recursed = false;
        for next in self.onward(outgoing) {
            if !self.graph.edge[next].is_central() {
                continue;
            }
            is_only_going_down &=
                self.is_going_down(next, traveled_dist + length, max_dist, lower_bead_count);
            has_recursed = true;
        }
        has_recursed && is_only_going_down
    }

    /// Puts a node, with its ribs, wherever a change ends.
    fn apply_transitions(&mut self) {
        let snap_dist = edge_geom::scaled(SNAP_DIST);
        for edge in self.graph.edges() {
            let Some(twin) = self.graph.edge[edge].twin else {
                continue;
            };
            if !self.graph.edge[twin].has_transition_ends(false) {
                continue;
            }
            let length = self.edge_length(edge);
            let theirs = std::mem::take(
                self.graph.edge[twin]
                    .transition_ends
                    .as_mut()
                    .expect("it has transition ends"),
            );
            let ours = self.graph.edge[edge]
                .transition_ends
                .get_or_insert_with(Vec::new);
            ours.extend(theirs.into_iter().map(|end| TransitionEnd {
                pos: length - end.pos,
                ..end
            }));
        }

        for edge in self.graph.edges() {
            if !self.graph.edge[edge].has_transition_ends(false) {
                continue;
            }
            let ends = {
                let ends = self.graph.edge[edge]
                    .transition_ends
                    .as_mut()
                    .expect("it has transition ends");
                ends.sort_by_key(|end| end.pos);
                ends.clone()
            };
            let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
            let (a, b) = (self.graph.node[from].p, self.graph.node[to].p);
            let ab = Point2 {
                x: b.x - a.x,
                y: b.y - a.y,
            };
            let ab_size = length(b, a);
            let mut last_edge_replacing_input = edge;
            for end in ends {
                let new_node_bead_count =
                    Coord::from(end.lower_bead_count) + Coord::from(!end.is_lower_end);
                let end_pos = end.pos;
                let close_node = if end_pos < ab_size / 2 { from } else { to };
                if (end_pos < snap_dist || end_pos > ab_size - snap_dist)
                    && self.graph.node[close_node].bead_count == new_node_bead_count
                {
                    self.graph.node[close_node].transition_ratio = 0.0;
                    continue;
                }
                let step = normal(ab, end_pos);
                let mid = Point2 {
                    x: a.x + step.x,
                    y: a.y + step.y,
                };
                last_edge_replacing_input =
                    self.graph
                        .insert_node(last_edge_replacing_input, mid, new_node_bead_count);
            }
        }
    }
}
