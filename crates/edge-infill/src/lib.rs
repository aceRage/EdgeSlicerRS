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
//! monotonic ordering, gap fill inside solid fill, and every sparse
//! pattern but lines and grid.

mod fill;
pub mod lines;
mod settings;
mod surfaces;

pub use fill::{FillParams, fill};
pub use settings::{InfillSettings, Pattern};
pub use surfaces::{
    Anchoring, Beneath, Shells, Surface, SurfaceKind, WallsAround, anchor, bridge_edges,
    bridge_over_infill, carry_bridge_edges, classify, inside_walls,
};
