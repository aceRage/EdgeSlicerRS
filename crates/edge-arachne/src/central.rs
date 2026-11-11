//! Which edges of the skeleton lines run along, and how many lines fit
//! at each node: `updateIsCentral`, `filterOuterCentral`,
//! `updateBeadCount` and `filterNoncentralRegions`
//! (SkeletalTrapezoidation.cpp).

use edge_geom::{Coord, scaled};

use crate::graph::{EdgeId, EdgeType, length, shorter_than};
use crate::skeleton::Skeleton;

impl Skeleton<'_> {
    /// An edge is central where the part's two sides run nearly
    /// alongside each other: where the distance to the outline changes
    /// slowly along it.
    pub(crate) fn update_is_central(&mut self) {
        let outer_edge_filter_length = self.strategy.get_transition_thickness(0) / 2;
        let cap = (self.strategy.get_transitioning_angle() * 0.5).sin() as f32;
        for edge in self.graph.edges() {
            let Some(twin) = self.graph.edge[edge].twin else {
                continue;
            };
            let (from, to) = (self.graph.edge[edge].from, self.graph.edge[edge].to);
            let (from_r, to_r) = (
                self.graph.node[from].distance_to_boundary,
                self.graph.node[to].distance_to_boundary,
            );
            let central = if let Some(central) = self.graph.edge[twin].central {
                central
            } else if self.graph.edge[edge].kind == EdgeType::ExtraVd
                || from_r.max(to_r) < outer_edge_filter_length
            {
                false
            } else {
                let d_r = (to_r - from_r).abs();
                let d_d = length(self.graph.node[to].p, self.graph.node[from].p);
                (d_r as f32) < d_d as f32 * cap
            };
            self.graph.edge[edge].central = Some(central);
        }
    }

    /// No lines along the first edge of any cell.
    pub(crate) fn filter_outer_central(&mut self) {
        for edge in self.graph.edges() {
            if self.graph.edge[edge].prev.is_none() {
                self.graph.edge[edge].central = Some(false);
                if let Some(twin) = self.graph.edge[edge].twin {
                    self.graph.edge[twin].central = Some(false);
                }
            }
        }
    }

    /// The number of lines that fit best at the end of every central
    /// edge, and at every node no neighbour is further in than.
    pub(crate) fn update_bead_count(&mut self) {
        for edge in self.graph.edges() {
            if self.graph.edge[edge].is_central() {
                let to = self.graph.edge[edge].to;
                self.graph.node[to].bead_count = self
                    .strategy
                    .get_optimal_bead_count(self.graph.node[to].distance_to_boundary * 2);
            }
        }
        for node in self.graph.nodes() {
            if !self.graph.is_local_maximum(node, false) {
                continue;
            }
            if self.graph.node[node].distance_to_boundary < 0 {
                let mut nearest = Coord::MAX;
                let first = self.graph.node[node].incident_edge;
                let mut edge = first;
                while let Some(at) = edge {
                    let (from, to) = (self.graph.edge[at].from, self.graph.edge[at].to);
                    nearest = nearest.min(
                        self.graph.node[to].distance_to_boundary
                            + length(self.graph.node[from].p, self.graph.node[to].p),
                    );
                    edge = self.graph.edge[at]
                        .twin
                        .and_then(|twin| self.graph.edge[twin].next);
                    if edge == first {
                        break;
                    }
                }
                self.graph.node[node].distance_to_boundary = nearest;
            }
            self.graph.node[node].bead_count = self
                .strategy
                .get_optimal_bead_count(self.graph.node[node].distance_to_boundary * 2);
        }
    }

    /// Whether the central edges stop at the end of `edge`
    /// (`isEndOfCentral`).
    pub(crate) fn is_end_of_central(&self, edge: EdgeId) -> bool {
        if !self.graph.edge[edge].is_central() {
            return false;
        }
        if self.graph.edge[edge].next.is_none() {
            return true;
        }
        !self
            .onward(edge)
            .into_iter()
            .any(|next| self.graph.edge[next].is_central())
    }

    /// Two central stretches with only a short way between them, and
    /// the same number of lines or one more, are made one.
    pub(crate) fn filter_noncentral_regions(&mut self) {
        for edge in self.graph.edges() {
            if !self.is_end_of_central(edge) {
                continue;
            }
            let to = self.graph.edge[edge].to;
            let max_dist = scaled(0.4);
            self.filter_noncentral_region(edge, self.graph.node[to].bead_count, 0, max_dist);
        }
    }

    fn filter_noncentral_region(
        &mut self,
        to_edge: EdgeId,
        bead_count: Coord,
        traveled_dist: Coord,
        max_dist: Coord,
    ) -> bool {
        let r = self.graph.node[self.graph.edge[to_edge].to].distance_to_boundary;
        // Only upwards.
        let Some(next_edge) = self.onward(to_edge).into_iter().find(|&next| {
            let (from, to) = (self.graph.edge[next].from, self.graph.edge[next].to);
            self.graph.node[to].distance_to_boundary >= r
                || shorter_than(self.graph.node[to].p, self.graph.node[from].p, scaled(0.01))
        }) else {
            return false;
        };
        let (from, to) = (
            self.graph.edge[next_edge].from,
            self.graph.edge[next_edge].to,
        );
        let length = length(self.graph.node[to].p, self.graph.node[from].p);
        let there = self.graph.node[to].bead_count;
        let dissolve = if there == bead_count {
            true
        } else if there < 0 {
            self.filter_noncentral_region(next_edge, bead_count, traveled_dist + length, max_dist)
        } else {
            traveled_dist + length < max_dist && (there - bead_count).abs() == 1
        };
        if dissolve {
            self.graph.edge[next_edge].central = Some(true);
            if let Some(twin) = self.graph.edge[next_edge].twin {
                self.graph.edge[twin].central = Some(true);
            }
            self.graph.node[to].bead_count = self
                .strategy
                .get_optimal_bead_count(self.graph.node[to].distance_to_boundary * 2);
            self.graph.node[to].transition_ratio = 0.0;
        }
        dissolve
    }
}
