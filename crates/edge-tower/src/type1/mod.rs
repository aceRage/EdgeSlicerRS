//! The reference's Type 1 tower as rewritten at the pin (WipeTower.cpp,
//! Bambu Studio 2.9.1's block planner by way of Orca), for Bambu Lab
//! printers of one nozzle (A1, A1 mini, P1S, X1C), being ported here a
//! stage at a time beside the older port in `bambu.rs`, which stays the
//! tower that is laid until this one is complete.
//!
//! Done so far: what the tower reads ([`Settings`]).
//!
//! Nothing here reaches the G-code yet: the settings it reads stay on the
//! unfollowed list (edge-engine `unfollowed.rs`, `TOWER`) until it does.

mod settings;

pub use settings::{
    Filament, PrimeVolumeMode, Settings, Wall, adhesiveness_category, interface_features_supported,
};
