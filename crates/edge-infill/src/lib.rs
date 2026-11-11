//! What goes inside the walls: which parts of each layer are top, bottom,
//! bridge, solid backing or sparse, and the lines that fill them.
//!
//! Classification follows the reference's
//! `PrintObject::detect_surfaces_type` and the core of
//! `discover_horizontal_shells`. Fill lines are parallel lines, placed,
//! joined and anchored as the reference's `FillRectilinear` and
//! `Fill::connect_infill` do it.
//! Not done: the lengths the reference goes to in running internal bridges out to
//! the lines that carry them, vertical shell thickness, shell thickness in mm,
//! gap fill inside solid fill, and every sparse
//! pattern but lines and grid.

mod bridges;
mod curves;
mod fill;
mod ironing;
pub mod lines;
mod settings;
mod surfaces;

pub use fill::{FillParams, Frame, GapFillTarget, fill, fill_from, fill_within};
pub use ironing::{
    IronedLayer, Ironing, IroningPattern, IroningSettings, IroningType, exposed, iron,
    ironing_areas,
};
pub use settings::{InfillSettings, Pattern};
pub use surfaces::{
    Anchoring, Beneath, Shells, Surface, SurfaceKind, WallsAround, anchor, bridge_edges,
    bridge_over_infill, carry_bridge_edges, classify, inside_walls, internal,
};
