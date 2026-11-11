//! Fuzzy skin: walls roughened on purpose, by moving their points in and
//! out or by changing how wide they are.
//!
//! A port of the reference's Feature/FuzzySkin/FuzzySkin.cpp, with the
//! noise of libnoise it draws on (`noise.rs`) and the line splitting of
//! Algorithm/LineSplit.cpp (`split.rs`).
//!
//! Where this differs from the reference on purpose, the code says so.
//! In short: the random numbers are seeded from each call's input, so a
//! slice is the same on every run and thread (`rng.rs`); a wall is split
//! along a region's edge by plain geometry, not through the clipping
//! library (`split.rs`); and libnoise's value noise is taken as written,
//! not as an optimising compiler happens to build it (`noise.rs`).

mod apply;
mod config;
mod fuzz;
mod noise;
mod rng;
mod split;
#[cfg(test)]
mod testing;
mod vectortable;

pub use apply::{
    Context, Region, apply_to_line, apply_to_polygon, skip_overhangs_wanted, supported_area,
};
pub use config::{FuzzyConfig, FuzzyMode, FuzzyType, NoiseType, should_fuzzify};
pub use fuzz::{fuzzy_extrusion_line, fuzzy_polyline};
pub use split::{SplitLineJunction, split_line};

use edge_geom::Coord;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A boolean or an offset of the fuzzy regions failed.
    #[error(transparent)]
    Clip(#[from] edge_clip::ClipError),
    #[error("the fuzzy skin point distance must be more than zero, not {0} units")]
    PointDistance(Coord),
    #[error("the fuzzy skin noise takes 1 to 30 octaves, not {0}")]
    Octaves(i32),
    #[error("the fuzzy skin noise scale must be a number other than zero, not {0}")]
    Scale(f64),
}
