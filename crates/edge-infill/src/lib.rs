//! What goes inside the walls: which parts of each layer are top, bottom,
//! bridge, solid backing or sparse, and the lines that fill them.
//!
//! This is a first working version rather than a port. Classification
//! follows the reference's `PrintObject::detect_surfaces_type` and the core
//! of `discover_horizontal_shells`; fill lines are plain parallel lines.
//! Not done: bridge direction, growing external surfaces to anchor them,
//! the lengths the reference goes to in running internal bridges out to
//! the lines that carry them, vertical shell thickness, shell thickness in mm, joining
//! lines into continuous paths, monotonic ordering, fitting solid lines to
//! the region's width, and every sparse pattern but lines and grid.

mod fill;
pub mod lines;
mod settings;
mod surfaces;

pub use fill::{FillParams, fill};
pub use settings::{InfillSettings, Pattern};
pub use surfaces::{Shells, Surface, SurfaceKind, bridge_over_infill, classify};
