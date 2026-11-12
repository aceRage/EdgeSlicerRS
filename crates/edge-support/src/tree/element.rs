//! A branch on one layer (TreeSupport3D.hpp `SupportElementState` and
//! `SupportElement`): the area its centre may be in, what is known of it,
//! and which branches of the layer above it carries.

use edge_geom::{Coord, Point2};

use super::config::TreeConfig;
use super::geometry::Polys;
use super::volumes::Avoidance;

/// How an influence area was grown to the layer below (`AreaIncreaseSettings`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AreaIncrease {
    pub kind: Avoidance,
    pub speed: Coord,
    pub increase_radius: bool,
    pub no_error: bool,
    pub use_min_distance: bool,
    pub moves: bool,
}

impl Default for AreaIncrease {
    fn default() -> Self {
        Self {
            kind: Avoidance::Fast,
            speed: 0,
            increase_radius: false,
            no_error: false,
            use_min_distance: false,
            moves: false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    /// The layer and place of the tip this branch reaches up to.
    pub target_height: usize,
    pub target_position: Point2,
    /// Where the branch would like to be, as a hint inside its area.
    pub next_position: Point2,
    pub layer: usize,
    /// Layers under the tip as far as its radius goes, and in fact.
    pub effective_radius_height: usize,
    pub distance_to_top: usize,
    /// The branch's centre, once placed.
    pub result: Option<Point2>,
    pub increased_to_model_radius: Coord,
    pub elephant_foot_increases: f64,
    pub dont_move_until: usize,
    pub last_area_increase: AreaIncrease,
    pub to_buildplate: bool,
    pub to_model_gracious: bool,
    pub use_min_xy_dist: bool,
    pub supports_roof: bool,
    pub can_use_safe_radius: bool,
    pub skip_ovalisation: bool,
    /// Kept only by moving faster than allowed, or lost altogether.
    pub lost: bool,
    pub verylost: bool,
    pub deleted: bool,
    pub marked: bool,
}

impl State {
    /// The state one layer down (`propagate_down`).
    pub fn propagate_down(&self) -> Self {
        Self {
            distance_to_top: self.distance_to_top + 1,
            layer: self.layer - 1,
            result: None,
            skip_ovalisation: false,
            ..self.clone()
        }
    }

    /// `getEffectiveDTT`.
    fn effective_dtt(&self, config: &TreeConfig) -> usize {
        if self.effective_radius_height < config.increase_radius_until_layer {
            self.distance_to_top.min(config.increase_radius_until_layer)
        } else {
            self.effective_radius_height
        }
    }

    /// The radius the branch is drawn with (`support_element_radius`).
    pub fn radius(&self, config: &TreeConfig) -> Coord {
        config.radius(self.effective_dtt(config), self.elephant_foot_increases)
    }

    /// The radius it keeps clear of the part with, which may lag the one
    /// it is drawn with (`support_element_collision_radius`).
    pub fn collision_radius(&self, config: &TreeConfig) -> Coord {
        config.radius(self.effective_radius_height, self.elephant_foot_increases)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Element {
    pub state: State,
    /// Branches of the layer above this one carries, by index.
    pub parents: Vec<u32>,
    /// Where the centre may be.
    pub area: Polys,
}
