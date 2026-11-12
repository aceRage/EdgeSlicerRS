//! Arranging objects on the bed as the reference slicer does: libnest2d's
//! no-fit-polygon placer and first-fit selection driven by libslic3r's
//! `Arrange.cpp` (the objective, the parameters and how each item is
//! grown), each object taken as its convex outline on the bed.
//!
//! No UI here: the engine gathers the outlines and settings
//! (`edge_engine::Model::arrange`) and applies what comes back.

mod arranger;
mod edges;
mod geom;
mod item;
mod outline;
mod params;
mod placer;
mod rotation;
mod subplex;

pub use arranger::{LARGE_COST_TO_REJECT, arrange};
pub use geom::Bbox;
pub use outline::{
    HIGH_LOW_COMPATIBLE, HIGH_TEMP, LOW_TEMP, SupportKind, UNDEFINED_TEMP,
    compatible_filament_type, filament_temp_type, may_get_support, outline, support_allowance,
};
pub use params::{
    BED_SHRINK_SEQ_PRINT, MAX_NUM_PLATES, MAX_OUTER_NOZZLE_DIAMETER, Params, Piece,
    seq_print_min_obj_distance, shrunk_bed, update_arrange_params,
    update_selected_items_axis_align, update_selected_items_inflation,
    update_unselected_items_inflation,
};
pub use rotation::{fit_into_box_rotation, min_area_rotation};
pub use subplex::{Optimum, minimize};
