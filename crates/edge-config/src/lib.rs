//! Printer, process and filament presets in the reference slicer's on-disk
//! format, resolved through inheritance into flat settings.

mod defaults;
mod settings;
pub mod variants;
mod vendor;

use std::{io, path::PathBuf};

pub use defaults::{reference_defaults, reference_knows, reference_producer, serialized};
pub use settings::{FloatOrPercent, Reads, Settings, Value};
pub use vendor::{Kind, Vendor};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{path} is not valid preset JSON: {reason}")]
    Json { path: PathBuf, reason: String },
    #[error("vendor {vendor} has no {kind} preset named \"{name}\"")]
    UnknownPreset {
        vendor: String,
        kind: Kind,
        name: String,
    },
    #[error("preset \"{name}\" inherits from itself: {chain}")]
    InheritanceCycle { name: String, chain: String },
    #[error("{path} includes \"{include}\", which was not found")]
    MissingInclude { path: PathBuf, include: String },
    #[error("setting {key} is {found:?}, expected {expected}")]
    BadValue {
        key: String,
        found: String,
        expected: &'static str,
    },
}
