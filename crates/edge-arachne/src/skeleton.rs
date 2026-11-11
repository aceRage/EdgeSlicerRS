//! How many lines fit where, along a part's skeleton, and the lines
//! themselves: a port of `SkeletalTrapezoidation`
//! (SkeletalTrapezoidation.cpp). The work is in three files beside this
//! one: `central` marks the edges lines run along and counts the lines
//! at each node, `transitions` places the stretches over which that
//! count changes by one, and `segments` works out each line's place and
//! width at every rib and joins those into walls.

use edge_geom::{Coord, Polygon};
use edge_medial::MedialError;

use crate::VariableWidthLines;
use crate::beading::BeadingStrategy;
use crate::graph::{EdgeId, Graph};

pub(crate) struct Skeleton<'a> {
    pub graph: Graph,
    pub strategy: &'a dyn BeadingStrategy,
    /// About how long the pieces a curved edge was cut into are.
    pub discretization_step_size: Coord,
    /// Changes in the number of lines nearer each other than this are
    /// looked at for leaving out.
    pub transition_filter_dist: Coord,
    /// How much a line's width may be changed by leaving one out.
    pub allowed_filter_deviation: Coord,
    /// The distance over which lines worked out from below give way to
    /// those worked out from above.
    pub beading_propagation_transition_dist: Coord,
}

impl<'a> Skeleton<'a> {
    pub fn new(
        polys: &[Polygon],
        strategy: &'a dyn BeadingStrategy,
        transitioning_angle: f64,
        discretization_step_size: Coord,
        transition_filter_dist: Coord,
        allowed_filter_deviation: Coord,
        beading_propagation_transition_dist: Coord,
    ) -> Result<Self, MedialError> {
        Ok(Self {
            graph: crate::construct::from_polygons(
                polys,
                transitioning_angle,
                discretization_step_size,
            )?,
            strategy,
            discretization_step_size,
            transition_filter_dist,
            allowed_filter_deviation,
            beading_propagation_transition_dist,
        })
    }

    /// The walls, by depth (`generateToolpaths`).
    pub fn generate_toolpaths(
        &mut self,
        filter_outermost_central_edges: bool,
    ) -> Vec<VariableWidthLines> {
        self.update_is_central();
        // The reference's `filterCentral` asks for an edge whose end is
        // and is not a local maximum, so it never does anything.
        if filter_outermost_central_edges {
            self.filter_outer_central();
        }
        self.update_bead_count();
        self.filter_noncentral_regions();
        self.generate_transitioning_ribs();
        self.generate_extra_ribs(self.discretization_step_size);
        self.generate_segments()
    }

    /// The edges leaving the end of `edge`, but for its own twin, in
    /// turn: `for (e = edge->next; e && e != edge->twin; e = e->twin->next)`.
    pub fn onward(&self, edge: EdgeId) -> Vec<EdgeId> {
        let twin = self.graph.edge[edge].twin;
        let mut found = Vec::new();
        let mut next = self.graph.edge[edge].next;
        while let Some(at) = next {
            if Some(at) == twin {
                break;
            }
            found.push(at);
            next = self.graph.edge[at]
                .twin
                .and_then(|twin| self.graph.edge[twin].next);
        }
        found
    }
}
