//! Which nozzle each filament prints from on a printer of two nozzles of
//! different kinds (a Bambu Lab H2D), as the reference decides it: the
//! filament grouping (FilamentGroup.cpp, GCode/ToolOrderUtils.cpp,
//! MultiNozzleUtils.cpp and ToolOrdering.cpp `get_recommended_filament_maps`,
//! ported from libslic3r).
//!
//! Only what the reference runs for a two-nozzle printer without a nozzle
//! rack, sliced without a printer to ask, is here:
//!
//! - "Manual": the project's `filament_map` as it stands;
//! - every automatic mode: every way of putting the plate's filaments on
//!   the nozzles is tried in turn (`calc_group_by_enum`) and scored by the
//!   flush the layers would cost on each nozzle and the time the filament
//!   changes take (`full_evaluate`).
//!
//! Left out, and refused by the caller where they would be reached:
//!
//! - "Auto For Match", which matches the printer's loaded spools; without a
//!   printer (the command line) the reference finds none and groups for
//!   flush, which is what is done here for that mode too;
//! - fourteen or more filaments, which the reference groups by a k-medoids
//!   search bounded by a three-second clock (`calc_group_by_kmedoids`), so
//!   that its answer depends on the speed of the machine;
//! - nozzle racks (H2C, H3C: `extruder_max_nozzle_count` above one,
//!   `handle_hotend_as_extruder`);
//! - the pool of near-best groupings kept for matching the printer's spools
//!   (`update_memoryed_groups`, `select_best_group_for_ams`): with no spools
//!   to match the reference takes the best grouping, as here.

mod group;
mod nozzle;
mod order;
mod preload;
mod time;

pub use group::{
    ENUM_LIMIT, FlushMatrix, Grouping, Input, Machine, Mode, Usage, group_filaments,
    prepare_flush_matrices,
};
pub use nozzle::{GroupResult, NozzleInfo, VolumeType, nozzle_list};
pub use order::{
    LayerSequence, extruders_order, other_layers_sequences, reorder_for_nozzles, solve_order,
};
pub use preload::preload_first_layer_tool_order;
pub use time::{ChangeTimes, simulate_change_time};
