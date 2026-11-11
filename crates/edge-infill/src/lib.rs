//! What goes inside the walls: which parts of each layer are top, bottom,
//! bridge, solid backing or sparse, and the lines that fill them.
//!
//! Classification follows the reference's
//! `PrintObject::detect_surfaces_type` and the core of
//! `discover_horizontal_shells`. Fill lines are parallel lines, placed,
//! joined and anchored as the reference's `FillRectilinear` and
//! `Fill::connect_infill` do it.
//! Bridges over infill are run out to the sparse lines that carry them and
//! turned to cross them, as `PrintObject::bridge_over_infill` does.

mod anchored;
mod bridges;
mod counterbore;
mod curves;
mod fill;
mod internal_bridges;
mod ironing;
pub mod lines;
mod narrow;
mod settings;
mod surfaces;

pub use counterbore::bridge_counterbores;
pub use fill::{FillError, FillParams, Frame, GapFillTarget, fill, fill_from, fill_within};
pub use internal_bridges::{
    BridgeDirection, InternalBridgeFilter, OverInfill, bridge_over_infill_by,
    second_internal_bridges,
};
pub use ironing::{
    IronedLayer, Ironing, IroningPattern, IroningSettings, IroningType, exposed, iron,
    ironing_areas,
};
pub use narrow::NarrowFill;
pub use settings::{ExtraBridgeLayer, InfillSettings, Pattern};
pub use surfaces::{
    Anchoring, Beneath, Shells, Surface, SurfaceKind, WallsAround, anchor, classify, inside_walls,
    internal, shell_depths,
};
