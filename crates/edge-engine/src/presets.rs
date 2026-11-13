use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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
    /// What each filament preset is made of (`filament_type`), by name.
    filament_types: HashMap<String, String>,
    /// Who makes each filament preset's filament (`filament_vendor`), by name.
    filament_vendors: HashMap<String, String>,
}

/// A printer model as a vendor's index lists it (`machine_model_list`):
/// what the reference's printer selection shows a row for
/// (WebGuideDialog.cpp `LoadProfileFamily`).
#[derive(Clone, Debug, PartialEq)]
pub struct PrinterModel {
    pub vendor: String,
    /// The name in the index: what a printer preset names as its
    /// `printer_model`.
    pub model: String,
    /// The model file's own name, which is the one shown.
    pub name: String,
    pub model_id: Option<String>,
    /// The nozzles it is sold with, as its file lists them
    /// (`nozzle_diameter`, "0.4;0.2;0.6"), in that order.
    pub variants: Vec<String>,
    /// The filament presets it usually prints with (`default_materials`).
    pub default_materials: Vec<String>,
    pub bed_model: Option<String>,
    pub bed_texture: Option<String>,
    /// The bed type the model's own file names as its usual one
    /// (`default_bed_type`), where it names one.
    pub default_bed_type: Option<String>,
    /// Its picture, `<model>_cover.png` in the vendor's folder, when the
    /// file is there.
    pub cover: Option<PathBuf>,
    /// The printer presets made for it that can be picked, each with the
    /// nozzle it is for (`printer_variant`), in the vendor's own order.
    pub machines: Vec<ModelPreset>,
    /// The bed's width and depth in mm, the height that can be printed
    /// and the number of nozzles, from the preset for its usual nozzle
    /// (0.4 where it has one). None where the preset does not say.
    pub bed: Option<[f64; 2]>,
    pub height: Option<f64>,
    pub nozzles: Option<usize>,
}

/// A printer preset of a model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPreset {
    pub name: String,
    /// The nozzle it is for, as the preset names it ("0.4").
    pub variant: String,
    /// The first nozzle's diameter in mm.
    nozzle: Option<f64>,
    bed: Option<[f64; 2]>,
    height: Option<f64>,
    nozzles: Option<usize>,
}

/// The width and depth of a bed given as the corners of `printable_area`,
/// each "XxY" (PrinterModelSpecs.cpp `printable_area_size`).
fn area_size(corners: &[String]) -> Option<[f64; 2]> {
    if corners.len() < 2 {
        return None;
    }
    let mut extent: Option<[f64; 4]> = None;
    for corner in corners {
        let (x, y) = corner.split_once('x')?;
        let (x, y): (f64, f64) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
        let [min_x, min_y, max_x, max_y] = extent.unwrap_or([x, y, x, y]);
        extent = Some([min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]);
    }
    let [min_x, min_y, max_x, max_y] = extent?;
    (max_x > min_x && max_y > min_y).then_some([max_x - min_x, max_y - min_y])
}

/// A list a profile writes as one text, "a;b;c".
fn listed(text: Option<&str>) -> Vec<String> {
    text.unwrap_or_default()
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
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
        let mut filament_types = HashMap::new();
        let mut filament_vendors = HashMap::new();
        let filaments = usable(Kind::Filament)
            .into_iter()
            .map(|(name, settings)| {
                if let Some(kind) = settings
                    .texts("filament_type")
                    .and_then(|kinds| kinds.first())
                {
                    filament_types.insert(name.clone(), kind.clone());
                }
                if let Some(maker) = settings
                    .texts("filament_vendor")
                    .and_then(|makers| makers.first())
                {
                    filament_vendors.insert(name.clone(), maker.clone());
                }
                let printers = settings
                    .texts("compatible_printers")
                    .map(<[String]>::to_vec)
                    .unwrap_or_default();
                (name, printers)
            })
            .collect();
        Ok(Shelf {
            machines: usable(Kind::Machine)
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
            processes: with_printers(Kind::Process),
            filaments,
            filament_types,
            filament_vendors,
            vendor,
        })
    }

    /// A vendor's printer models, in the order of its index, each with
    /// the printer presets made for it. A model whose file cannot be read
    /// is left out, as the reference leaves it out; so is a preset that
    /// cannot be read or is only there to inherit from.
    pub fn models(&self, vendor: &str) -> Result<Vec<PrinterModel>, Failure> {
        let opened = Vendor::open(&self.dir, vendor)?;
        let mut by_model: HashMap<String, Vec<ModelPreset>> = HashMap::new();
        for name in opened.names(Kind::Machine) {
            let Ok(settings) = opened.resolve(Kind::Machine, name) else {
                continue;
            };
            if settings.text("instantiation") != Some("true") {
                continue;
            }
            let Some(model) = settings.text("printer_model").filter(|m| !m.is_empty()) else {
                continue;
            };
            let diameters = settings.texts("nozzle_diameter");
            by_model
                .entry(model.to_owned())
                .or_default()
                .push(ModelPreset {
                    name: name.to_owned(),
                    variant: settings
                        .text("printer_variant")
                        .unwrap_or_default()
                        .to_owned(),
                    nozzle: diameters
                        .and_then(|sizes| sizes.first())
                        .and_then(|size| size.trim().parse().ok()),
                    bed: settings.texts("printable_area").and_then(area_size),
                    height: settings
                        .float("printable_height")
                        .ok()
                        .flatten()
                        .filter(|height| *height > 0.0),
                    nozzles: diameters.map(<[String]>::len).filter(|count| *count > 0),
                });
        }
        let mut models = Vec::new();
        for model in opened.names(Kind::MachineModel) {
            let Ok(file) = opened.resolve(Kind::MachineModel, model) else {
                continue;
            };
            let text = |key: &str| {
                file.text(key)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            };
            let variants = listed(file.text("nozzle_diameter"));
            let machines = by_model.remove(model).unwrap_or_default();
            // The preset the sizes are read from: the one for a 0.4
            // nozzle, else for the first nozzle the model lists that has
            // one, else the first; among several, the shortest name, then
            // by name (PrinterModelSpecs.cpp `specs_for_model`).
            let mut candidates: Vec<&ModelPreset> = machines.iter().collect();
            candidates.sort_by(|a, b| (a.name.len(), &a.name).cmp(&(b.name.len(), &b.name)));
            let wanted = std::iter::once(0.4).chain(
                variants
                    .iter()
                    .filter_map(|variant| variant.parse::<f64>().ok()),
            );
            let usual = wanted
                .filter_map(|size| {
                    candidates.iter().find(|preset| {
                        preset
                            .nozzle
                            .is_some_and(|nozzle| (nozzle - size).abs() < 1e-6)
                    })
                })
                .next()
                .or(candidates.first())
                .copied();
            let cover = self.dir.join(vendor).join(format!("{model}_cover.png"));
            models.push(PrinterModel {
                vendor: vendor.to_owned(),
                model: model.to_owned(),
                name: text("name").unwrap_or_else(|| model.to_owned()),
                model_id: text("model_id"),
                variants,
                default_materials: listed(file.text("default_materials")),
                bed_model: text("bed_model"),
                bed_texture: text("bed_texture"),
                default_bed_type: text("default_bed_type"),
                cover: cover.is_file().then_some(cover),
                bed: usual.and_then(|preset| preset.bed),
                height: usual.and_then(|preset| preset.height),
                nozzles: usual.and_then(|preset| preset.nozzles),
                machines,
            });
        }
        Ok(models)
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

    /// A preset of this vendor (or its filament library) with everything
    /// it inherits, by name.
    pub fn resolve(&self, kind: Kind, name: &str) -> Result<Settings, edge_config::ConfigError> {
        self.vendor.resolve(kind, name)
    }

    /// Whether the vendor has a preset of this name, one only there to
    /// inherit from included.
    pub fn knows(&self, kind: Kind, name: &str) -> bool {
        self.vendor.has(kind, name)
    }

    /// The version a preset made from these is stamped with ("2.3.3.14").
    pub fn preset_version(&self) -> String {
        self.vendor.preset_version()
    }

    /// Every process or filament preset that can be picked, whatever
    /// printer it is made for, in the vendor's own order.
    pub fn all_of(&self, kind: Kind) -> Vec<String> {
        let presets = match kind {
            Kind::Process => &self.processes,
            Kind::Filament => &self.filaments,
            _ => return self.machines.clone(),
        };
        presets.iter().map(|(name, _)| name.clone()).collect()
    }

    /// The process presets made for a printer.
    pub fn processes_for(&self, machine: &str) -> Vec<&str> {
        for_machine(&self.processes, machine)
    }

    /// The filament presets made for a printer.
    pub fn filaments_for(&self, machine: &str) -> Vec<&str> {
        for_machine(&self.filaments, machine)
    }

    /// What a filament preset is made of ("PLA"), when it says.
    pub fn filament_type(&self, filament: &str) -> Option<&str> {
        self.filament_types.get(filament).map(String::as_str)
    }

    /// Who makes a filament preset's filament ("Generic", "Polymaker"),
    /// when it says (`filament_vendor`).
    pub fn filament_vendor(&self, filament: &str) -> Option<&str> {
        self.filament_vendors.get(filament).map(String::as_str)
    }

    /// The process and the filament preset a printer's own profile names
    /// as its usual ones, where they are among those made for it.
    pub fn usual_for(&self, machine: &str) -> (Option<String>, Option<String>) {
        let Ok(profile) = self.vendor.resolve(Kind::Machine, machine) else {
            return (None, None);
        };
        let among = |named: Option<&str>, offered: Vec<&str>| {
            named
                .filter(|name| offered.contains(name))
                .map(str::to_owned)
        };
        (
            among(
                profile.text("default_print_profile"),
                self.processes_for(machine),
            ),
            among(
                profile
                    .texts("default_filament_profile")
                    .and_then(|names| names.first())
                    .map(String::as_str),
                self.filaments_for(machine),
            ),
        )
    }

    /// The presets named, each with everything it inherits. `filaments`
    /// are for filament 1, filament 2 and so on.
    pub fn choose(
        &self,
        machine: Option<&str>,
        process: Option<&str>,
        filaments: &[&str],
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
            filaments: filaments
                .iter()
                .map(|name| {
                    Ok((
                        (*name).to_owned(),
                        self.vendor.resolve(Kind::Filament, name)?,
                    ))
                })
                .collect::<Result<_, Failure>>()?,
        })
    }
}
