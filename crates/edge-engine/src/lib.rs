//! The slicing pipeline behind one call, for whatever drives it: the
//! command line, the desktop app, a test.
//!
//! [`open`] reads a model, [`prepare`] settles what it is sliced with,
//! [`slice`] does the work and [`Sliced::gcode`] writes the result. A
//! [`Model`] is a file held in memory to be changed first: what stands on
//! its plates moved, added, taken away. For
//! looking at, [`Opened::scene`] gives the file's objects as meshes and
//! [`read_toolpath`] the moves of a finished G-code file. [`followed`]
//! says, for a set of settings, which of them the engine reads. Nothing
//! here prints or touches a window; a [`Watcher`] is told how far the
//! work has got and may stop it.

mod clock;
mod followed;
mod holes;
mod job;
pub mod lay_on_face;
mod layer_heights;
mod load;
mod model;
mod order;
mod paint;
mod pipeline;
mod presets;
mod preview;
mod printable;
mod scene;
mod toolpath;
mod unfollowed;

use std::{error::Error, fmt};

pub use clock::timestamp;
pub use edge_mesh::Transform;
pub use followed::{Followed, followed, followed_in, settled};
pub use job::{BedType, Chosen, Job, Overrides, Sourced, prepare, prepare_noted};
pub use load::{Loaded, Opened, PlateInfo, open};
pub use model::{
    Bed, MAX_PLATES, Model, ModelCopy, ModelObject, ModelPlate, PlateStamp, Reach, Shape,
};
pub use pipeline::{Gcode, Sliced, Timings, slice};
pub use presets::{Library, ModelPreset, PrinterModel, Shelf};
pub use preview::Stroke;
pub use scene::{Mark, Placing, Scene, SceneObject, ScenePlate, plate_position};
pub use toolpath::{Feature, Move, MoveKind, Toolpath, ToolpathLayer, read_toolpath};

/// Anything that stops the work, said in words fit to show a person.
pub type Failure = Box<dyn Error + Send + Sync>;

/// The stages of a slice, in the order they run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Cutting the model into layers.
    Layers,
    Walls,
    Supports,
    /// Telling surfaces apart and filling them.
    Fill,
    /// Tool order, prime tower, skirt and brim.
    Around,
}

impl Stage {
    pub const ALL: [Stage; 5] = [
        Stage::Layers,
        Stage::Walls,
        Stage::Supports,
        Stage::Fill,
        Stage::Around,
    ];

    /// What is being done, for a progress line.
    pub fn label(self) -> &'static str {
        match self {
            Stage::Layers => "Cutting layers",
            Stage::Walls => "Walls",
            Stage::Supports => "Supports",
            Stage::Fill => "Fill",
            Stage::Around => "Tool order, skirt and brim",
        }
    }
}

/// Follows a slice as it runs, and may call it off.
pub trait Watcher: Sync {
    /// A stage is starting.
    fn stage(&self, _stage: Stage) {}
    /// Whether to stop; asked before each stage.
    fn cancelled(&self) -> bool {
        false
    }
}

/// A watcher that neither listens nor stops anything.
pub struct Unwatched;

impl Watcher for Unwatched {}

/// The error a slice ends with when its watcher called it off.
#[derive(Debug)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("slicing was cancelled")
    }
}

impl Error for Cancelled {}
