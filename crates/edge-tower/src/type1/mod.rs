//! The reference's Type 1 tower as rewritten at the pin (WipeTower.cpp,
//! Bambu Studio 2.9.1's block planner by way of Orca), for Bambu Lab
//! printers of one nozzle (A1, A1 mini, P1S, X1C) and of two nozzles of one
//! size, one to each extruder (the H2D): there a change onto the other
//! nozzle rams the outgoing filament on the tower first, and Bambu's tower
//! interface features act on its contact layers.
//!
//! What the tower reads ([`Settings`]), its planner ([`plan()`]): blocks,
//! depths, where it stands; and how it is laid ([`lay()`]), as the
//! reference's tower writes its G-code.
//!
//! Refused: rack hotends counted as extruders (H2C, H3C), more than two
//! nozzles or two of different sizes, the side scrape, sparse layers laid
//! as one, and Edge's own tower interface options.

mod gaps;
mod lay;
mod plan;
mod settings;
mod writer;

pub use lay::{NozzleVisit, Visit, lay};
pub use plan::{
    Block, Box2, Change, ChangeIn, Layer, LayerIn, LayerType, NozzleChange, Plan, PlanError,
    PurgeEstimate, Ribs, auto_brim, estimate_brim_real_width, estimate_rib_tower_bbox_side,
    estimate_tower_blocks_depth, limit_depth, plan, rib_footprint_side,
};
pub use settings::{
    Filament, PrimeVolumeMode, Settings, Wall, adhesiveness_category, interface_features_supported,
};
