//! The reference's Type 1 tower as rewritten at the pin (WipeTower.cpp,
//! Bambu Studio 2.9.1's block planner by way of Orca), for Bambu Lab
//! printers of one nozzle (A1, A1 mini, P1S, X1C), being ported here a
//! stage at a time beside the older port in `bambu.rs`, which stays the
//! tower that is laid until this one is complete.
//!
//! Done so far: what the tower reads ([`Settings`]), and its planner
//! ([`plan()`]): blocks, depths, where it stands; and how it is laid
//! ([`lay()`]), as the reference's tower writes its G-code.
//!
//! Nothing here reaches the G-code yet: the settings it reads stay on the
//! unfollowed list (edge-engine `unfollowed.rs`, `TOWER`) until it does.

mod gaps;
mod lay;
mod plan;
mod settings;
mod writer;

pub use lay::{LayOptions, Visit, lay};
pub use plan::{
    Block, Box2, Change, ChangeIn, Layer, LayerIn, LayerType, Plan, PlanError, PurgeEstimate, Ribs,
    auto_brim, estimate_brim_real_width, estimate_rib_tower_bbox_side, estimate_tower_blocks_depth,
    limit_depth, plan, rib_footprint_side,
};
pub use settings::{
    Filament, PrimeVolumeMode, Settings, Wall, adhesiveness_category, interface_features_supported,
};
