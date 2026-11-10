use std::path::{Path, PathBuf};

use edge_config::{Kind, Settings, Vendor};

use crate::{Chosen, Failure};

/// A directory of vendor profiles, laid out as the reference's
/// `resources/profiles` is.
#[derive(Clone, Debug)]
pub struct Library {
    dir: PathBuf,
}

/// One vendor's presets: the printers that can be picked, and the
/// process and filament presets that go with each.
pub struct Shelf {
    vendor: Vendor,
    machines: Vec<String>,
    /// (name, the printers it is for)
    processes: Vec<(String, Vec<String>)>,
    filaments: Vec<(String, Vec<String>)>,
}

impl Library {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The vendors with profiles here, in order of name.
    pub fn vendors(&self) -> Result<Vec<String>, Failure> {
        Ok(Vendor::list(&self.dir)?)
    }

    /// Reads a vendor's presets. A preset that is only there for others
    /// to inherit from (`instantiation` not true) is left out, as is one
    /// that cannot be read.
    pub fn shelf(&self, vendor: &str) -> Result<Shelf, Failure> {
        let vendor = Vendor::open(&self.dir, vendor)?;
        let usable = |kind: Kind| -> Vec<(String, Settings)> {
            vendor
                .names(kind)
                .filter_map(|name| Some((name.to_owned(), vendor.resolve(kind, name).ok()?)))
                .filter(|(_, settings)| settings.text("instantiation") == Some("true"))
                .collect()
        };
        let with_printers = |kind: Kind| -> Vec<(String, Vec<String>)> {
            usable(kind)
                .into_iter()
                .map(|(name, settings)| {
                    let printers = settings
                        .texts("compatible_printers")
                        .map(<[String]>::to_vec)
                        .unwrap_or_default();
                    (name, printers)
                })
                .collect()
        };
        Ok(Shelf {
            machines: usable(Kind::Machine)
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
            processes: with_printers(Kind::Process),
            filaments: with_printers(Kind::Filament),
            vendor,
        })
    }
}

fn for_machine<'a>(presets: &'a [(String, Vec<String>)], machine: &str) -> Vec<&'a str> {
    presets
        .iter()
        .filter(|(_, printers)| printers.iter().any(|printer| printer == machine))
        .map(|(name, _)| name.as_str())
        .collect()
}

impl Shelf {
    pub fn vendor(&self) -> &str {
        self.vendor.name()
    }

    /// The printers that can be picked, in the vendor's own order.
    pub fn machines(&self) -> &[String] {
        &self.machines
    }

    /// The process presets made for a printer.
    pub fn processes_for(&self, machine: &str) -> Vec<&str> {
        for_machine(&self.processes, machine)
    }

    /// The filament presets made for a printer.
    pub fn filaments_for(&self, machine: &str) -> Vec<&str> {
        for_machine(&self.filaments, machine)
    }

    /// The presets named, each with everything it inherits.
    pub fn choose(
        &self,
        machine: Option<&str>,
        process: Option<&str>,
        filament: Option<&str>,
    ) -> Result<Chosen, Failure> {
        let resolve = |kind, name: Option<&str>| -> Result<Option<(String, Settings)>, Failure> {
            match name {
                Some(name) => Ok(Some((name.to_owned(), self.vendor.resolve(kind, name)?))),
                None => Ok(None),
            }
        };
        Ok(Chosen {
            machine: resolve(Kind::Machine, machine)?,
            process: resolve(Kind::Process, process)?,
            filament: resolve(Kind::Filament, filament)?,
        })
    }
}
