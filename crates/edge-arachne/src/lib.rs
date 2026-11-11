//! Walls of changing width: as many lines as fit across a part wherever
//! it is measured, each as wide as its share, so that a thin part is one
//! line and nothing is left to gap fill.
//!
//! A port of the reference's Arachne (src/libslic3r/Arachne, itself from
//! CuraEngine): the part's outline is cut into the trapezoids of its
//! skeleton, the number of lines that fit is decided along the skeleton,
//! and the lines are laid across each trapezoid.

pub mod beading;
mod central;
mod construct;
mod graph;
pub mod line;
mod prepare;
mod segments;
mod skeleton;
pub mod stitch;
mod toolpaths;
mod transitions;

pub use toolpaths::{Error, Params, Walls, generate};

use edge_geom::{Coord, Point2};

/// One point of a wall, with how wide the wall is there
/// (utils/ExtrusionJunction.hpp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    pub p: Point2,
    /// The line's width here, in scaled units.
    pub w: Coord,
    /// Which wall this is, counted from the outside in, from 0.
    pub perimeter_index: usize,
}

/// One wall, or a piece of one: a line whose width changes along it
/// (utils/ExtrusionLine.hpp).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtrusionLine {
    /// Which wall this is, counted from the outside in, from 0.
    pub inset_idx: usize,
    /// Set for the single line down the middle where an odd number fit:
    /// it has no outline of the part on either side of it.
    pub is_odd: bool,
    /// Whether the line comes round to where it began; the first junction
    /// is then repeated at the end, as the reference keeps it.
    pub is_closed: bool,
    pub junctions: Vec<Junction>,
}

/// The walls of one depth.
pub type VariableWidthLines = Vec<ExtrusionLine>;
