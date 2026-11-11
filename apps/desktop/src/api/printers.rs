//! The printers of the sidebar: which are added, which one a project is
//! on, and what a change of printer does to the rest of what the project
//! is sliced with. The rules are the reference's, each named where it is
//! ported:
//!
//! - the printer list shows the printers added, one row for each model
//!   (PresetComboBoxes.cpp `PlaterPresetComboBox::update`), and a model's
//!   nozzles pick among its presets (Plater.cpp `update_nozzle_settings`,
//!   PresetBundle.cpp `get_similar_printer_preset`);
//! - "Select/Remove printers" lists every model of every vendor and
//!   keeps what is ticked (WebGuideDialog.cpp, `resources/web/guide/24`);
//! - another printer keeps the process and the filaments where they fit
//!   it and finds the nearest where they do not (PresetBundle.cpp
//!   `update_compatible`, `update_selections`), and brings the bed type
//!   last used with it (Plater.cpp `Sidebar::update_all_preset_comboboxes`).
//!
//! What is added and what was last used is kept by [`super::store`].

use std::{collections::BTreeMap, sync::Arc};

use edge_config::Settings;
use edge_engine::{ModelPreset, PrinterModel, Shelf};
use serde_json::{Value, json};

use super::{Choices, Session, State, lock, plates_for, shown, text};

/// The vendor the reference lists first in the printer selection
/// (printer_table.js `VENDOR_PRIORITY`) and looks at first for the
/// printer to select after one (WebGuideDialog.cpp `apply_config`).
const FIRST_VENDOR: &str = "Snapmaker";
/// The printer the reference adds when the first run's selection is
/// closed with nothing added (PresetBundle.cpp `SM_DEFAULT_PRINTER_MODEL`).
const DEFAULT_MODEL: &str = "Snapmaker U1";
/// The nozzle a model is given when nothing says which
/// (PresetBundle.cpp `SM_DEFAULT_PRINTER_VARIANT`).
const DEFAULT_VARIANT: &str = "0.4";

/// The bed types by their number in the reference (PrintConfig.hpp
/// `BedType`, 1 to 7), each under the name profiles and projects use.
const BED_TYPES: [&str; 7] = [
    "Cool Plate",
    "Engineering Plate",
    "High Temp Plate",
    "Textured PEI Plate",
    "Textured Cool Plate",
    "Graphic Effect Plate",
    "Supertack Plate",
];

/// What a vendor is shown as (printer_table.js `BRAND_NAMES`).
fn brand(vendor: &str) -> &str {
    match vendor {
        "BBL" => "Bambu Lab",
        "Custom" => "Custom Printer",
        "Other" => "Orca colosseum",
        other => other,
    }
}

/// What a preset is called without the printer it is cut for: its name
/// up to the "@" (PresetBundle.cpp, "Derive the profile logical name aka
/// alias from the preset name").
fn alias(name: &str) -> &str {
    let short = name.split('@').next().unwrap_or(name).trim_end();
    if short.is_empty() { name } else { short }
}

/// The bed type a printer starts on (Preset.cpp `get_default_bed_type`):
/// the one its profile names, by name or by number; failing that, by the
/// model it is.
///
/// The reference looks for `default_bed_type` in the printer preset
/// only. The profiles that give one (Bambu's) give it in the model's
/// file, which the reference never reads for this, so its Bambu printers
/// start on the High Temp Plate whatever their profile says. The key is
/// plainly there to be the default, so it is read from the model's file
/// too, after the preset.
fn default_bed_type(printer: &Settings, model: Option<&PrinterModel>) -> &'static str {
    let model_id = model.and_then(|model| model.model_id.as_deref());
    let named = printer
        .text("default_bed_type")
        .or(model.and_then(|model| model.default_bed_type.as_deref()))
        .map(str::trim)
        .filter(|given| !given.is_empty());
    if let Some(given) = named {
        if let Some(known) = BED_TYPES.iter().find(|name| **name == given) {
            return known;
        }
        return match given.parse::<usize>() {
            Ok(number @ 1..=7) => BED_TYPES[number - 1],
            // A profile that names one there is not.
            _ => "High Temp Plate",
        };
    }
    match model_id {
        Some("BL-P001" | "BL-P002" | "C13") => "Cool Plate",
        Some("SM_U1") => "Textured PEI Plate",
        _ => "High Temp Plate",
    }
}

/// The bed type a project is put on when it goes to a printer: the one
/// last used with that printer, else the printer's own. A printer with
/// no choice of plates is always on its own (Plater.cpp
/// `Sidebar::update_all_preset_comboboxes`).
fn bed_type_for(
    printer: &Settings,
    vendor: &str,
    model: Option<&PrinterModel>,
    kept: Option<&str>,
) -> String {
    let plates = plates_for(printer, Some(vendor));
    let default = default_bed_type(printer, model);
    if !plates.enabled {
        return default.to_owned();
    }
    let offered = |name: &str| plates.offered.iter().any(|(value, _)| *value == name);
    let wanted = kept.unwrap_or(default);
    if offered(wanted) {
        return wanted.to_owned();
    }
    if plates.u1 && !plates.several {
        // A U1 has four plates of its own; what was kept from another
        // list is brought to the nearest of them, as the reference does.
        let nearest = if wanted == "Supertack Plate" {
            "Cool Plate"
        } else {
            "Textured PEI Plate"
        };
        return nearest.to_owned();
    }
    // The reference would show the first row and slice with a plate the
    // list does not have; the printer's own, or the first offered, is
    // what the list is plainly for.
    if offered(default) {
        default.to_owned()
    } else {
        plates.offered[0].0.to_owned()
    }
}

/// The preset of a model for a nozzle. With `wanted`, that nozzle or
/// none; without, the nozzle the project is on now, else the usual 0.4,
/// else the first by name (PresetBundle.cpp `get_similar_printer_preset`,
/// whose first try, the present preset's name with the model swapped in,
/// finds the same preset as its second for the vendors' presets).
fn similar<'a>(
    model: &'a PrinterModel,
    wanted: Option<&str>,
    current: Option<&str>,
) -> Option<&'a ModelPreset> {
    let mut presets: Vec<&ModelPreset> = model.machines.iter().collect();
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    let of = |variant: &str| presets.iter().copied().find(|p| p.variant == variant);
    match wanted {
        Some(wanted) => of(wanted),
        None => current
            .and_then(of)
            .or_else(|| of(DEFAULT_VARIANT))
            .or(presets.first().copied()),
    }
}

/// The process a project is put on when it goes to a printer: the one
/// last used with that printer; else the one it is on, if that fits;
/// else one of the same name cut for this printer; else the printer's
/// usual one; else the first (PresetBundle.cpp `update_selections`, then
/// `PreferedPrintProfileMatch`).
fn pick_process(
    shelf: &Shelf,
    machine: &str,
    current: Option<&str>,
    kept: Option<&str>,
) -> Option<String> {
    let offered = shelf.processes_for(machine);
    let fits = |name: &&str| offered.contains(name);
    let same_name = |current: &str| {
        offered
            .iter()
            .copied()
            .find(|name| alias(name) == alias(current))
    };
    kept.filter(fits)
        .or(current.filter(fits))
        .or_else(|| current.and_then(same_name))
        .map(str::to_owned)
        .or_else(|| shelf.usual_for(machine).0)
        .or_else(|| offered.first().map(|name| (*name).to_owned()))
}

/// The filament a slot is put on when its project goes to a printer. One
/// that fits stays. One that does not is replaced by the one of the same
/// name cut for this printer; else by one of the same material, the
/// printer's usual one before others (PresetBundle.cpp
/// `PreferedFilamentProfileMatch`). A slot with nothing in it yet takes
/// what was last used in it with this printer, else the printer's usual.
fn pick_filament(
    shelf: &Shelf,
    machine: &str,
    current: Option<(&str, Option<&str>)>,
    kept: Option<&str>,
) -> Option<String> {
    let offered = shelf.filaments_for(machine);
    let usual = shelf.usual_for(machine).1;
    let Some((current, material)) = current else {
        return kept
            .filter(|name| offered.contains(name))
            .map(str::to_owned)
            .or(usual)
            .or_else(|| offered.first().map(|name| (*name).to_owned()));
    };
    if offered.contains(&current) {
        return Some(current.to_owned());
    }
    if let Some(same) = offered.iter().find(|name| alias(name) == alias(current)) {
        return Some((*same).to_owned());
    }
    // The reference's score; among equals the first listed wins.
    let material = material.unwrap_or("PLA");
    let score = |name: &str| {
        let usual = if usual.as_deref() == Some(name) { 2 } else { 1 };
        let alike = if shelf.filament_type(name) == Some(material) {
            10
        } else {
            1
        };
        usual * alike
    };
    let mut best: Option<(&str, i32)> = None;
    for name in offered {
        let score = score(name);
        if best.is_none_or(|(_, most)| score > most) {
            best = Some((name, score));
        }
    }
    best.map(|(name, _)| name.to_owned())
}

/// Bytes as base64, for a picture handed to the page as a `data:` URL.
fn base64(bytes: &[u8]) -> String {
    const LETTERS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let word = chunk.iter().enumerate().fold(0u32, |word, (at, byte)| {
            word | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..4 {
            if at <= chunk.len() {
                out.push(LETTERS[(word >> (18 - 6 * at)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Changes what is kept between runs. If it cannot be written, the run
/// goes on with what it has and the reason is noted: what was chosen
/// still holds until the app is closed.
fn keep(state: &mut State, change: impl FnOnce(&mut super::store::Data)) {
    if let Err(error) = state.store.change(change) {
        eprintln!("warning: {error}");
    }
}

/// After `project_set`: what was chosen for the printer the project is
/// on is what was last used with it (PresetBundle.cpp
/// `export_selections`). A project sliced with its own settings has the
/// printer it was saved with; only its bed type is chosen here.
pub(super) fn remember(state: &mut State, at: usize, args: &Value) {
    if !["bed_type", "process", "filaments"]
        .iter()
        .any(|key| args.get(key).is_some())
    {
        return;
    }
    let project = &state.projects[at];
    let choices = project.choices.clone();
    let machine = if choices.from_presets {
        choices.machine.clone()
    } else {
        project
            .file
            .as_ref()
            .and_then(|file| file.settings.as_ref())
            .and_then(|settings| settings.text("printer_settings_id"))
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
    };
    let Some(machine) = machine else {
        return;
    };
    keep(state, |data| {
        let kept = data.printers.entry(machine).or_default();
        // A bed type let go of (back to the project's own) is not one
        // that was used with the printer.
        if args.get("bed_type").is_some() && choices.bed_type.is_some() {
            kept.bed_type = choices.bed_type.clone();
        }
        if choices.from_presets {
            kept.vendor = choices.vendor.clone();
            if args.get("process").is_some() {
                kept.process = choices.process.clone();
            }
            if args.get("filaments").is_some() {
                kept.filaments = choices.filaments.clone();
            }
        }
    });
}

impl Session {
    /// A vendor's printer models, read once and kept, as its presets are.
    fn models_of(&self, vendor: &str) -> Result<Arc<Vec<PrinterModel>>, String> {
        let library = {
            let state = lock(&self.state);
            if let Some(models) = state.catalogue.get(vendor) {
                return Ok(models.clone());
            }
            state
                .library
                .clone()
                .ok_or("no profiles folder has been chosen")?
        };
        let models = Arc::new(library.models(vendor).map_err(|e| e.to_string())?);
        let mut state = lock(&self.state);
        if state.library.as_ref().map(edge_engine::Library::dir) == Some(library.dir()) {
            return Ok(state
                .catalogue
                .entry(vendor.to_owned())
                .or_insert(models)
                .clone());
        }
        Ok(models)
    }

    /// The printers added that the profiles in use have, by model name
    /// and then by vendor: the order of the reference's list, which is
    /// that of the names. One added from other profiles is not listed
    /// and stays added.
    fn installed(&self) -> Vec<PrinterModel> {
        let added = lock(&self.state).store.data.models.clone();
        let mut installed: Vec<PrinterModel> = added
            .iter()
            .filter_map(|(vendor, models)| Some((self.models_of(vendor).ok()?, models)))
            .flat_map(|(known, models)| {
                known
                    .iter()
                    .filter(|model| models.contains_key(&model.model))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect();
        installed.sort_by(|a, b| (&a.model, &a.vendor).cmp(&(&b.model, &b.vendor)));
        installed
    }

    /// The model a printer preset is of, and the nozzle it is for.
    fn located(&self, vendor: &str, machine: &str) -> Option<(PrinterModel, String)> {
        let models = self.models_of(vendor).ok()?;
        models.iter().find_map(|model| {
            let preset = model.machines.iter().find(|p| p.name == machine)?;
            Some((model.clone(), preset.variant.clone()))
        })
    }

    /// The printer a project with none is given: the one last selected,
    /// if it is still added; else the first of those added, by the name
    /// of its preset (PresetBundle.cpp `load_selections`, Preset.cpp
    /// `select_preset_by_name`).
    ///
    /// The first preset by name is the one for a model's smallest nozzle
    /// ("... 0.2 nozzle"), which the reference takes there; where it picks
    /// a preset of a model elsewhere (`get_similar_printer_preset`) it
    /// says that is not wanted and takes the 0.4 one. The model found
    /// first is kept, on its usual nozzle.
    fn usual_printer(&self, installed: &[PrinterModel]) -> Option<(String, String)> {
        let last = lock(&self.state).store.data.last.clone();
        let added = |vendor: &str, machine: &str| {
            installed.iter().any(|model| {
                model.vendor == vendor && model.machines.iter().any(|p| p.name == machine)
            })
        };
        last.filter(|(vendor, machine)| added(vendor, machine))
            .or_else(|| {
                let (_, first) = installed
                    .iter()
                    .flat_map(|model| model.machines.iter().map(move |p| (&p.name, model)))
                    .min_by_key(|(name, model)| (*name, &model.vendor))?;
                let preset = similar(first, None, None)?;
                Some((first.vendor.clone(), preset.name.clone()))
            })
    }

    /// Puts a project on a printer preset, and on the process, the
    /// filaments and the bed type that go with it. `initial` is the
    /// printer a project starts with, which is not a change to it;
    /// `selected` is a printer that was picked, which is the one the next
    /// project starts with.
    fn put_on(
        &self,
        id: u32,
        vendor: &str,
        machine: &str,
        initial: bool,
        selected: bool,
    ) -> Result<(), String> {
        let shelf = self.shelf(vendor)?;
        let (_, printer) = shelf
            .choose(Some(machine), None, &[])
            .map_err(|e| e.to_string())?
            .machine
            .ok_or_else(|| format!("{vendor} has no printer preset {machine}"))?;
        let model = self.located(vendor, machine).map(|(model, _)| model);
        let (current, saved, count, kept) = {
            let state = lock(&self.state);
            let at = state
                .index_of(id)
                .ok_or_else(|| format!("there is no project {id}"))?;
            let project = &state.projects[at];
            let saved = project
                .file
                .as_ref()
                .and_then(|file| file.settings.clone())
                .filter(|_| !project.choices.from_presets);
            (
                project.choices.clone(),
                saved,
                project.filament_count(),
                state.store.data.printers.get(machine).cloned(),
            )
        };
        // What the project is on now: its presets, or for one sliced
        // with its own settings the presets it was saved with, by name.
        let (process_now, filaments_now): (Option<String>, Vec<(String, Option<String>)>) =
            match &saved {
                Some(settings) => {
                    let materials = settings.texts("filament_type").unwrap_or_default();
                    (
                        settings.text("print_settings_id").map(str::to_owned),
                        settings
                            .texts("filament_settings_id")
                            .unwrap_or_default()
                            .iter()
                            .enumerate()
                            .map(|(at, name)| (name.clone(), materials.get(at).cloned()))
                            .collect(),
                    )
                }
                None => {
                    // The material of a filament of another vendor is
                    // known to that vendor's presets.
                    let theirs = match &current.vendor {
                        Some(other) if other != vendor => self.shelf(other).ok(),
                        _ => Some(shelf.clone()),
                    };
                    let material = |name: &str| {
                        theirs
                            .as_ref()
                            .and_then(|shelf| shelf.filament_type(name))
                            .map(str::to_owned)
                    };
                    (
                        current.process.clone(),
                        current
                            .filaments
                            .iter()
                            .map(|name| (name.clone(), material(name)))
                            .collect(),
                    )
                }
            };
        let kept = kept.unwrap_or_default();
        let process = pick_process(
            &shelf,
            machine,
            process_now.as_deref(),
            kept.process.as_deref(),
        );
        let filaments: Vec<String> = (0..count)
            .filter_map(|slot| {
                // A filament past the last chosen prints with the last.
                let now = filaments_now.get(slot).or(filaments_now.last());
                pick_filament(
                    &shelf,
                    machine,
                    now.map(|(name, material)| (name.as_str(), material.as_deref())),
                    kept.filaments.get(slot).map(String::as_str),
                )
            })
            .collect();
        let bed_type = bed_type_for(&printer, vendor, model.as_ref(), kept.bed_type.as_deref());

        let mut state = lock(&self.state);
        let at = state
            .index_of(id)
            .ok_or_else(|| format!("there is no project {id}"))?;
        let project = &mut state.projects[at];
        let choices = Choices {
            from_presets: true,
            vendor: Some(vendor.to_owned()),
            machine: Some(machine.to_owned()),
            process,
            filaments,
            bed_type: Some(bed_type),
            ..project.choices.clone()
        };
        if !choices.settle_as(&project.choices) {
            project.forget();
        }
        project.choices = choices.clone();
        if initial {
            project.baseline = choices.clone();
        }
        if !selected {
            return Ok(());
        }
        // What the reference writes whenever its selection changes
        // (`export_selections`): the printer, and what it is used with.
        let (vendor, machine) = (vendor.to_owned(), machine.to_owned());
        keep(&mut state, |data| {
            data.printers.insert(
                machine.clone(),
                super::store::Remembered {
                    vendor: Some(vendor.clone()),
                    process: choices.process,
                    filaments: choices.filaments,
                    bed_type: choices.bed_type,
                },
            );
            data.last = Some((vendor, machine));
        });
        Ok(())
    }

    /// The sidebar's printer list: the printers added, the printer the
    /// project is on, and the project's own when it carries one.
    pub(super) fn printers(&self, args: &Value) -> Result<Value, String> {
        let (choices, saved, has_library, file, problem, last) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.choices.clone(),
                project.file.as_ref().and_then(|file| file.settings.clone()),
                state.library.is_some(),
                state.store.file().map(shown),
                state.store.problem.clone(),
                state.store.data.last.clone(),
            )
        };
        let installed = self.installed();
        let is_added = |vendor: &str, model: &str| {
            installed
                .iter()
                .any(|known| known.vendor == vendor && known.model == model)
        };
        let named = |key: &str| {
            saved
                .as_ref()
                .and_then(|settings| settings.text(key))
                .filter(|text| !text.is_empty())
        };
        let selected = match (choices.from_presets, &choices.vendor, &choices.machine) {
            (true, Some(vendor), Some(machine)) => Some(match self.located(vendor, machine) {
                Some((model, variant)) => json!({
                    "source": "preset",
                    "vendor": vendor,
                    "machine": machine,
                    "model": model.model,
                    "variant": variant,
                    "added": is_added(vendor, &model.model),
                }),
                // A name the vendor does not have: said when slicing.
                None => json!({
                    "source": "preset",
                    "vendor": vendor,
                    "machine": machine,
                    "model": null,
                    "variant": null,
                    "added": false,
                }),
            }),
            // The printer a project was saved with. Its vendor is known
            // where a printer of that model is among those added.
            (false, _, _) if saved.is_some() => Some(json!({
                "source": "project",
                "vendor": installed
                    .iter()
                    .find(|known| Some(known.model.as_str()) == named("printer_model"))
                    .map(|known| known.vendor.as_str()),
                "machine": named("printer_settings_id"),
                "model": named("printer_model"),
                "variant": named("printer_variant"),
                "added": false,
            })),
            _ => None,
        };
        let installed: Vec<Value> = installed
            .iter()
            .map(|model| {
                // The nozzles to pick from, the smallest first.
                let mut variants: Vec<&ModelPreset> = model.machines.iter().collect();
                variants.sort_by(|a, b| (&a.variant, &a.name).cmp(&(&b.variant, &b.name)));
                variants.dedup_by(|b, a| a.variant == b.variant);
                json!({
                    "vendor": model.vendor,
                    "brand": brand(&model.vendor),
                    "model": model.model,
                    "name": model.name,
                    "cover": model.cover.is_some(),
                    "variants": variants
                        .iter()
                        .map(|preset| json!({ "variant": preset.variant, "machine": preset.name }))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({
            // Nothing is added yet: the reference opens its setup then.
            "first_run": has_library && installed.is_empty(),
            "installed": installed,
            "selected": selected,
            // The printer a project was saved with: its own row.
            "project_printer": saved.as_ref().map(|_| named("printer_settings_id")),
            "last": last.map(|(vendor, machine)| json!({ "vendor": vendor, "machine": machine })),
            "settings": { "file": file, "problem": problem },
        }))
    }

    /// Every printer model of every vendor, for "Select/Remove
    /// printers": the first vendor's, then the others' by vendor name,
    /// each vendor's in its own order. The reference lists the others in
    /// the order the folder is read in, which is no order; by name is the
    /// same list, the same on every machine.
    pub(super) fn printer_catalogue(&self) -> Result<Value, String> {
        let (library, added) = {
            let state = lock(&self.state);
            (
                state
                    .library
                    .clone()
                    .ok_or("no profiles folder has been chosen")?,
                state.store.data.models.clone(),
            )
        };
        let mut vendors = library.vendors().map_err(|e| e.to_string())?;
        vendors.sort_by_key(|vendor| vendor != FIRST_VENDOR);
        let mut rows = Vec::new();
        let mut unread = Vec::new();
        for vendor in vendors {
            let models = match self.models_of(&vendor) {
                Ok(models) => models,
                Err(error) => {
                    unread.push(error);
                    continue;
                }
            };
            let ticked = added.get(&vendor);
            rows.extend(models.iter().map(|model| {
                json!({
                    "vendor": model.vendor,
                    "brand": brand(&model.vendor),
                    "model": model.model,
                    "name": model.name,
                    "nozzle_diameter": model.variants.join(";"),
                    "x": model.bed.map(|bed| bed[0]),
                    "y": model.bed.map(|bed| bed[1]),
                    "z": model.height,
                    "toolheads": model.nozzles,
                    "cover": model.cover.is_some(),
                    "enabled": ticked.is_some_and(|models| models.contains_key(&model.model)),
                })
            }));
        }
        Ok(json!({ "models": rows, "unread": unread }))
    }

    /// A printer model's picture, as a `data:` URL the page can show
    /// without asking anything else for it; null for a model without one.
    pub(super) fn printer_cover(&self, args: &Value) -> Result<Value, String> {
        let (vendor, wanted) = (
            super::needed(args, "vendor")?,
            super::needed(args, "model")?,
        );
        let models = self.models_of(vendor)?;
        let model = models
            .iter()
            .find(|model| model.model == wanted)
            .ok_or_else(|| format!("{vendor} has no printer {wanted}"))?;
        let Some(cover) = &model.cover else {
            return Ok(json!({ "data": null }));
        };
        let bytes = std::fs::read(cover).map_err(|e| format!("{}: {e}", cover.display()))?;
        Ok(json!({ "data": format!("data:image/png;base64,{}", base64(&bytes)) }))
    }

    /// "Confirm" of the printer selection: the models ticked are the
    /// printers added, every nozzle of each, as the reference's page
    /// sends them. At least one must be.
    pub(super) fn printers_save(&self, args: &Value) -> Result<Value, String> {
        let wanted: Vec<(String, String)> = args
            .get("models")
            .and_then(Value::as_array)
            .ok_or("models is not a list")?
            .iter()
            .map(|entry| {
                Some((
                    text(entry, "vendor")?.to_owned(),
                    text(entry, "model")?.to_owned(),
                ))
            })
            .collect::<Option<_>>()
            .ok_or("models is not a list of a vendor and a model each")?;
        if wanted.is_empty() {
            return Err("At least one printer must be selected.".into());
        }
        self.install(&wanted, args)
    }

    /// The first run's selection closed with nothing added: the
    /// reference then adds its usual printer, so there is one to slice
    /// for (WebGuideDialog.cpp `GuideFrame::run`). Where a printer is
    /// added already, or the profiles have no such printer, nothing
    /// changes.
    pub(super) fn printers_default(&self, args: &Value) -> Result<Value, String> {
        let there = self
            .models_of(FIRST_VENDOR)
            .is_ok_and(|models| models.iter().any(|model| model.model == DEFAULT_MODEL));
        if !there || !self.installed().is_empty() {
            return self.changed(args, &[], &[], &[]);
        }
        self.install(&[(FIRST_VENDOR.to_owned(), DEFAULT_MODEL.to_owned())], args)
    }

    fn install(&self, wanted: &[(String, String)], args: &Value) -> Result<Value, String> {
        let (target, library, before) = {
            let state = lock(&self.state);
            (
                state.projects[state.target(args)?].id,
                state
                    .library
                    .clone()
                    .ok_or("no profiles folder has been chosen")?,
                state.store.data.models.clone(),
            )
        };
        let offered = library.vendors().map_err(|e| e.to_string())?;
        // The reference builds the list anew from what its page sends.
        // A printer added from other profiles was not on this page to be
        // unticked, so it stays added.
        let mut after: BTreeMap<String, BTreeMap<String, Vec<String>>> = before
            .iter()
            .filter(|(vendor, _)| !offered.contains(vendor))
            .map(|(vendor, models)| (vendor.clone(), models.clone()))
            .collect();
        for (vendor, wanted) in wanted {
            let models = self.models_of(vendor)?;
            let model = models
                .iter()
                .find(|model| model.model == *wanted)
                .ok_or_else(|| format!("{vendor} has no printer {wanted}"))?;
            after
                .entry(vendor.clone())
                .or_default()
                .insert(model.model.clone(), model.variants.clone());
        }
        let pairs =
            |models: &BTreeMap<String, BTreeMap<String, Vec<String>>>| -> Vec<(String, String)> {
                models
                    .iter()
                    .flat_map(|(vendor, models)| {
                        models
                            .keys()
                            .map(move |model| (vendor.clone(), model.clone()))
                    })
                    .collect()
            };
        // New, or with a nozzle it did not have: the first vendor's
        // before the others', then by name (`apply_config`).
        let mut added: Vec<(String, String)> = pairs(&after)
            .into_iter()
            .filter(|(vendor, model)| {
                let had = before.get(vendor).and_then(|models| models.get(model));
                let has = &after[vendor][model];
                had.is_none_or(|had| has.iter().any(|variant| !had.contains(variant)))
            })
            .collect();
        added.sort_by_key(|(vendor, _)| vendor != FIRST_VENDOR);
        let removed: Vec<(String, String)> = pairs(&before)
            .into_iter()
            .filter(|(vendor, model)| {
                !after
                    .get(vendor)
                    .is_some_and(|models| models.contains_key(model))
            })
            .collect();

        // Kept first: if it cannot be written, nothing has changed.
        lock(&self.state)
            .store
            .change(|data| data.models = after.clone())?;

        // The printer the reference selects after its setup: the first
        // one newly added, on its usual nozzle (0.4, else the smallest).
        let preferred = added.iter().find_map(|(vendor, model)| {
            let models = self.models_of(vendor).ok()?;
            let model = models.iter().find(|known| known.model == *model)?;
            let mut variants = model.variants.clone();
            variants.sort();
            let variant = variants
                .iter()
                .find(|variant| *variant == DEFAULT_VARIANT)
                .or(variants.first());
            let preset = variant
                .and_then(|variant| similar(model, Some(variant), None))
                .or_else(|| similar(model, None, None))?;
            Some((vendor.clone(), preset.name.clone()))
        });
        let installed = self.installed();
        let fallback = self.usual_printer(&installed);
        let on: Vec<(u32, String, Choices)> = lock(&self.state)
            .projects
            .iter()
            .map(|project| (project.id, project.name.clone(), project.choices.clone()))
            .collect();
        let mut switched = Vec::new();
        for (id, name, choices) in on {
            // A project sliced with its own settings is not on a printer
            // of this list: it stays as it is.
            if !choices.from_presets {
                continue;
            }
            if id == target
                && let Some((vendor, machine)) = &preferred
            {
                self.put_on(id, vendor, machine, choices.machine.is_none(), true)?;
                continue;
            }
            // On a printer that is no longer added: the reference goes to
            // the first printer that is, by name, and so does every open
            // project that was on it.
            let (Some(vendor), Some(machine)) = (&choices.vendor, &choices.machine) else {
                continue;
            };
            let Some((model, _)) = self.located(vendor, machine) else {
                continue;
            };
            let gone = !installed
                .iter()
                .any(|known| known.vendor == model.vendor && known.model == model.model);
            if gone && let Some((to_vendor, to)) = &fallback {
                self.put_on(id, to_vendor, to, false, false)?;
                switched.push(json!({ "project": id, "name": name, "from": machine, "to": to }));
            }
        }
        self.changed(args, &added, &removed, &switched)
    }

    /// The printer list, and what a change to the printers added did.
    fn changed(
        &self,
        args: &Value,
        added: &[(String, String)],
        removed: &[(String, String)],
        switched: &[Value],
    ) -> Result<Value, String> {
        let listed = |pairs: &[(String, String)]| -> Vec<Value> {
            pairs
                .iter()
                .map(|(vendor, model)| json!({ "vendor": vendor, "model": model }))
                .collect()
        };
        let mut answer = self.printers(args)?;
        if let Some(answer) = answer.as_object_mut() {
            answer.insert("added".into(), json!(listed(added)));
            answer.insert("removed".into(), json!(listed(removed)));
            answer.insert("switched".into(), json!(switched));
        }
        Ok(answer)
    }

    /// Puts a project on a printer: a model of those added (`vendor`,
    /// `model`), on the nozzle named (`variant`) or the one nearest what
    /// the project is on; another nozzle of the model it is on
    /// (`variant` alone); or, with none of these, the printer a project
    /// starts with. `initial` marks that start.
    pub(super) fn printer_select(&self, args: &Value) -> Result<Value, String> {
        let initial = args.get("initial").and_then(Value::as_bool) == Some(true);
        let (id, choices) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.id, project.choices.clone())
        };
        let on = match (choices.from_presets, &choices.vendor, &choices.machine) {
            (true, Some(vendor), Some(machine)) => self.located(vendor, machine),
            _ => None,
        };
        let variant = text(args, "variant");
        let installed = self.installed();
        let target = match (text(args, "vendor"), text(args, "model")) {
            (Some(vendor), Some(wanted)) => {
                let model = installed
                    .iter()
                    .find(|model| model.vendor == vendor && model.model == wanted)
                    .ok_or_else(|| format!("{wanted} is not among the printers added"))?;
                let now = on.as_ref().map(|(_, variant)| variant.as_str());
                let preset = similar(model, variant, now).ok_or_else(|| match variant {
                    Some(variant) => format!("{wanted} has no preset for a {variant} nozzle"),
                    None => format!("{wanted} has no printer preset"),
                })?;
                Some((model.vendor.clone(), preset.name.clone()))
            }
            (None, None) => match variant {
                Some(variant) => {
                    let (model, _) = on.as_ref().ok_or("the project is on no printer yet")?;
                    let preset = similar(model, Some(variant), None).ok_or_else(|| {
                        format!("{} has no preset for a {variant} nozzle", model.model)
                    })?;
                    Some((model.vendor.clone(), preset.name.clone()))
                }
                None => self.usual_printer(&installed),
            },
            _ => return Err("vendor and model are given together".into()),
        };
        if let Some((vendor, machine)) = target {
            self.put_on(id, &vendor, &machine, initial, !initial)?;
        }
        let state = lock(&self.state);
        let at = state
            .index_of(id)
            .ok_or_else(|| format!("there is no project {id}"))?;
        Ok(state.projects[at].detail())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn reference(path: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer")
            .join(path)
            .to_string_lossy()
            .into_owned()
    }

    /// A session that keeps its settings in `dir`, on the reference's
    /// profiles whatever the environment says.
    fn session(dir: &Path) -> Arc<Session> {
        let session = Arc::new(Session::with_settings(None, Some(dir.to_owned())));
        ask(
            &session,
            "set_profiles",
            json!({ "dir": reference("resources/profiles") }),
        );
        session
    }

    fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
        session
            .call(name, &args)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
    }

    fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
        session
            .call(name, &args)
            .expect_err("the call should be refused")
    }

    fn models(pairs: &[(&str, &str)]) -> Value {
        json!({
            "models": pairs
                .iter()
                .map(|(vendor, model)| json!({ "vendor": vendor, "model": model }))
                .collect::<Vec<_>>()
        })
    }

    fn names(listing: &Value) -> Vec<&str> {
        listing["installed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["model"].as_str().unwrap())
            .collect()
    }

    const U1: (&str, &str) = ("Snapmaker", "Snapmaker U1");
    const A1: (&str, &str) = ("BBL", "Bambu Lab A1 mini");
    const CC: (&str, &str) = ("Elegoo", "Elegoo Centauri Carbon");
    const PRUSA: (&str, &str) = ("Prusa", "Prusa CORE One");

    #[test]
    fn a_first_run_has_no_printers_and_every_model_to_choose_from() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        let listing = ask(&session, "printers", json!({}));
        assert_eq!(listing["first_run"], true);
        assert_eq!(listing["installed"], json!([]));
        assert_eq!(listing["selected"], Value::Null);
        assert_eq!(listing["project_printer"], Value::Null);
        assert_eq!(listing["settings"]["problem"], Value::Null);
        assert_eq!(
            PathBuf::from(listing["settings"]["file"].as_str().unwrap()),
            dir.path().join("settings.json")
        );
        // A project with no printer and none to give it stays so.
        let project = ask(&session, "printer_select", json!({ "initial": true }));
        assert_eq!(project["choices"]["machine"], Value::Null);
        assert_eq!(project["dirty"], false);

        let catalogue = ask(&session, "printer_catalogue", json!({}));
        assert_eq!(catalogue["unread"], json!([]));
        let rows = catalogue["models"].as_array().unwrap();
        assert!(rows.len() > 300, "only {} models", rows.len());
        // The first vendor's first; then the others by name.
        let u1 = &rows[0];
        assert_eq!(u1["vendor"], "Snapmaker");
        assert_eq!(u1["brand"], "Snapmaker");
        assert_eq!(u1["model"], "Snapmaker U1");
        assert_eq!(u1["nozzle_diameter"], "0.2;0.4;0.6;0.8;0.4+0.6");
        assert_eq!(u1["toolheads"], 4);
        assert_eq!(u1["cover"], true);
        assert!(u1["x"].as_f64().unwrap() > 200.0 && u1["z"].as_f64().unwrap() > 200.0);
        let vendors: Vec<&str> = rows
            .iter()
            .map(|row| row["vendor"].as_str().unwrap())
            .collect();
        let mut others: Vec<&str> = vendors
            .iter()
            .copied()
            .filter(|vendor| *vendor != "Snapmaker")
            .collect();
        let listed = others.clone();
        others.sort();
        assert_eq!(listed, others, "the other vendors are in order of name");
        assert!(rows.iter().all(|row| row["enabled"] == false));
        let bambu = rows.iter().find(|row| row["vendor"] == "BBL").unwrap();
        assert_eq!(bambu["brand"], "Bambu Lab");
    }

    #[test]
    fn printers_are_added_listed_by_model_and_kept_for_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        // Nothing ticked, a printer there is not, and what is no list.
        assert_eq!(
            refused(&session, "printers_save", models(&[])),
            "At least one printer must be selected."
        );
        assert_eq!(
            refused(
                &session,
                "printers_save",
                models(&[("Snapmaker", "Snapmaker U9")])
            ),
            "Snapmaker has no printer Snapmaker U9"
        );
        assert!(
            refused(&session, "printers_save", models(&[("Nobody", "Thing")])).contains("Nobody")
        );
        assert_eq!(
            refused(&session, "printers_save", json!({})),
            "models is not a list"
        );
        assert_eq!(
            refused(
                &session,
                "printers_save",
                json!({ "models": [{ "vendor": "Snapmaker" }] })
            ),
            "models is not a list of a vendor and a model each"
        );
        assert_eq!(
            ask(&session, "printers", json!({}))["first_run"],
            true,
            "nothing was added"
        );

        let saved = ask(&session, "printers_save", models(&[CC, U1]));
        assert_eq!(saved["first_run"], false);
        // By model name, whatever the vendor.
        assert_eq!(names(&saved), ["Elegoo Centauri Carbon", "Snapmaker U1"]);
        assert_eq!(saved["added"].as_array().unwrap().len(), 2);
        assert_eq!(saved["removed"], json!([]));
        assert_eq!(saved["switched"], json!([]));
        let u1 = &saved["installed"][1];
        assert_eq!(u1["cover"], true);
        // Every nozzle of a model is added with it, the smallest first.
        let variants: Vec<&str> = u1["variants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|variant| variant["variant"].as_str().unwrap())
            .collect();
        assert_eq!(variants, ["0.2", "0.4", "0.4+0.6", "0.6", "0.8"]);
        assert_eq!(u1["variants"][1]["machine"], "Snapmaker U1 (0.4 nozzle)");

        // The project shown, which had no printer, starts on the first
        // one added: the first vendor's, on its 0.4 nozzle, with the
        // process and filament that printer usually prints with.
        assert_eq!(saved["selected"]["source"], "preset");
        assert_eq!(saved["selected"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(saved["selected"]["model"], "Snapmaker U1");
        assert_eq!(saved["selected"]["variant"], "0.4");
        assert_eq!(saved["selected"]["added"], true);
        let project = ask(&session, "project", json!({}));
        assert_eq!(
            project["dirty"], false,
            "the printer a project starts with is no change"
        );
        let usual = ask(
            &session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": "Snapmaker U1 (0.4 nozzle)" }),
        );
        assert_eq!(project["choices"]["process"], usual["usual_process"]);
        assert_eq!(
            project["choices"]["filaments"],
            json!([usual["usual_filament"]])
        );
        assert_eq!(project["choices"]["bed_type"], "Textured PEI Plate");
        let catalogue = ask(&session, "printer_catalogue", json!({}));
        let enabled: Vec<&str> = catalogue["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["enabled"] == true)
            .map(|row| row["model"].as_str().unwrap())
            .collect();
        assert_eq!(enabled, ["Snapmaker U1", "Elegoo Centauri Carbon"]);

        // The file, laid out as the reference lays out its own.
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            written["models"][1],
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1", "nozzle_diameter": "0.2;0.4;0.6;0.8;0.4+0.6" })
        );

        // The next run finds them, and the profiles folder with them.
        let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        if std::env::var_os("EDGE_PROFILES").is_none() {
            let library = ask(&again, "library", json!({}));
            assert_eq!(
                PathBuf::from(library["dir"].as_str().unwrap()),
                PathBuf::from(reference("resources/profiles"))
            );
        } else {
            ask(
                &again,
                "set_profiles",
                json!({ "dir": reference("resources/profiles") }),
            );
        }
        let listing = ask(&again, "printers", json!({}));
        assert_eq!(names(&listing), ["Elegoo Centauri Carbon", "Snapmaker U1"]);
        assert_eq!(listing["first_run"], false);
        assert_eq!(
            listing["selected"],
            Value::Null,
            "a new window's project has no printer yet"
        );
        // Its project starts on the printer the last run ended on.
        assert_eq!(listing["last"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        let project = ask(&again, "printer_select", json!({ "initial": true }));
        assert_eq!(project["choices"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(project["dirty"], false);
        // Once that printer is removed and none is added: on the first of
        // those that are, by the name of its preset, on its usual nozzle.
        let saved = ask(&again, "printers_save", models(&[CC]));
        assert_eq!(
            saved["switched"][0]["to"],
            "Elegoo Centauri Carbon 0.4 nozzle"
        );
        assert_eq!(saved["removed"].as_array().unwrap().len(), 1);
        let third = session_again(dir.path());
        let project = ask(&third, "printer_select", json!({ "initial": true }));
        assert_eq!(
            project["choices"]["machine"],
            "Elegoo Centauri Carbon 0.4 nozzle"
        );
    }

    #[test]
    fn a_printer_is_selected_by_model_and_by_nozzle() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        ask(&session, "printers_save", models(&[U1, CC]));
        // Not added, half named, a nozzle the model does not have.
        assert_eq!(
            refused(
                &session,
                "printer_select",
                json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" })
            ),
            "Bambu Lab A1 mini is not among the printers added"
        );
        assert_eq!(
            refused(&session, "printer_select", json!({ "vendor": "Snapmaker" })),
            "vendor and model are given together"
        );
        assert_eq!(
            refused(&session, "printer_select", json!({ "variant": "1.2" })),
            "Snapmaker U1 has no preset for a 1.2 nozzle"
        );
        assert_eq!(
            refused(
                &session,
                "printer_select",
                json!({ "vendor": "Elegoo", "model": "Elegoo Centauri Carbon", "variant": "1.2" })
            ),
            "Elegoo Centauri Carbon has no preset for a 1.2 nozzle"
        );
        assert!(refused(&session, "printer_select", json!({ "project": 99 })).contains("99"));

        // Another nozzle of the model the project is on.
        let project = ask(&session, "printer_select", json!({ "variant": "0.6" }));
        assert_eq!(project["choices"]["machine"], "Snapmaker U1 (0.6 nozzle)");
        assert_eq!(
            project["dirty"], true,
            "a printer picked by hand is a change"
        );
        // The process and the filament are ones made for that preset.
        let fits = ask(
            &session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": "Snapmaker U1 (0.6 nozzle)" }),
        );
        let among = |list: &Value, name: &Value| list.as_array().unwrap().contains(name);
        assert!(among(&fits["processes"], &project["choices"]["process"]));
        assert!(among(
            &fits["filaments"],
            &project["choices"]["filaments"][0]
        ));

        // Another model keeps the nozzle the project is on, where it has it.
        let project = ask(
            &session,
            "printer_select",
            json!({ "vendor": "Elegoo", "model": "Elegoo Centauri Carbon" }),
        );
        assert_eq!(project["choices"]["vendor"], "Elegoo");
        assert_eq!(
            project["choices"]["machine"],
            "Elegoo Centauri Carbon 0.6 nozzle"
        );
        let listing = ask(&session, "printers", json!({}));
        assert_eq!(listing["selected"]["variant"], "0.6");
        assert_eq!(
            listing["last"]["machine"],
            "Elegoo Centauri Carbon 0.6 nozzle"
        );
        // A filament of the material it was on, cut for the new printer.
        let fits = ask(
            &session,
            "presets",
            json!({ "vendor": "Elegoo", "machine": "Elegoo Centauri Carbon 0.6 nozzle" }),
        );
        assert!(among(&fits["processes"], &project["choices"]["process"]));
        assert!(among(
            &fits["filaments"],
            &project["choices"]["filaments"][0]
        ));
        assert!(
            project["choices"]["filaments"][0]
                .as_str()
                .unwrap()
                .contains("PLA"),
            "{}",
            project["choices"]["filaments"][0]
        );

        // A nozzle the other model does not have: its usual 0.4.
        ask(
            &session,
            "printer_select",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1", "variant": "0.4+0.6" }),
        );
        let project = ask(
            &session,
            "printer_select",
            json!({ "vendor": "Elegoo", "model": "Elegoo Centauri Carbon" }),
        );
        assert_eq!(
            project["choices"]["machine"],
            "Elegoo Centauri Carbon 0.4 nozzle"
        );

        // The next window's project starts on the printer last selected.
        let again = session_again(dir.path());
        let project = ask(&again, "printer_select", json!({ "initial": true }));
        assert_eq!(
            project["choices"]["machine"],
            "Elegoo Centauri Carbon 0.4 nozzle"
        );
        assert_eq!(project["dirty"], false);
    }

    fn session_again(dir: &Path) -> Arc<Session> {
        session(dir)
    }

    #[test]
    fn what_was_last_used_with_a_printer_comes_back_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        ask(&session, "printers_save", models(&[U1, A1, PRUSA]));
        let project = ask(&session, "project", json!({}));
        assert_eq!(project["choices"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(project["choices"]["bed_type"], "Textured PEI Plate");

        // A bed type, a process and a filament chosen on the U1.
        let fits = ask(
            &session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": "Snapmaker U1 (0.4 nozzle)" }),
        );
        let other = |list: &Value, not: &Value| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|name| *name != not)
                .cloned()
                .unwrap()
        };
        let process = other(&fits["processes"], &project["choices"]["process"]);
        let filament = other(&fits["filaments"], &project["choices"]["filaments"][0]);
        ask(
            &session,
            "project_set",
            json!({ "bed_type": "Cool Plate", "process": process, "filaments": [filament] }),
        );

        // A Bambu printer has the reference's six plates and its own
        // default; the U1's choice does not follow there.
        let project = ask(
            &session,
            "printer_select",
            json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" }),
        );
        assert_eq!(
            project["choices"]["machine"],
            "Bambu Lab A1 mini 0.4 nozzle"
        );
        let settled = ask(&session, "settled", json!({}));
        let types = &settled["printer"]["bed_types"];
        assert_eq!(types["enabled"], true);
        assert_eq!(types["choices"].as_array().unwrap().len(), 6);
        assert_eq!(types["in_force"], project["choices"]["bed_type"]);
        assert_eq!(
            project["choices"]["bed_type"], "Textured PEI Plate",
            "its profile's default_bed_type"
        );
        ask(
            &session,
            "project_set",
            json!({ "bed_type": "Supertack Plate" }),
        );

        // A printer that takes one plate is on its own, and the list is off.
        let project = ask(
            &session,
            "printer_select",
            json!({ "vendor": "Prusa", "model": "Prusa CORE One" }),
        );
        let settled = ask(&session, "settled", json!({}));
        assert_eq!(settled["printer"]["bed_types"]["enabled"], false);
        assert_eq!(
            settled["printer"]["bed_types"]["in_force"],
            project["choices"]["bed_type"]
        );

        // Back on the U1: its bed type, process and what its slot held.
        let project = ask(
            &session,
            "printer_select",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        assert_eq!(project["choices"]["bed_type"], "Cool Plate");
        assert_eq!(project["choices"]["process"], process);
        // The filament went to the other printers with the project and
        // came back as what it had become there; it is chosen again.
        ask(&session, "project_set", json!({ "filaments": [filament] }));

        // And after the app was closed and opened: the Bambu printer's
        // plate with the Bambu printer, the U1's with the U1.
        let again = session_again(dir.path());
        let project = ask(&again, "printer_select", json!({ "initial": true }));
        assert_eq!(project["choices"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(project["choices"]["bed_type"], "Cool Plate");
        assert_eq!(project["choices"]["process"], process);
        assert_eq!(project["choices"]["filaments"], json!([filament]));
        let project = ask(
            &again,
            "printer_select",
            json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" }),
        );
        assert_eq!(project["choices"]["bed_type"], "Supertack Plate");
        // A plate kept from the long list is brought to the U1's own
        // when it is not one of them.
        ask(
            &again,
            "printer_select",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        ask(
            &again,
            "project_set",
            json!({ "bed_type": "Supertack Plate" }),
        );
        ask(
            &again,
            "printer_select",
            json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" }),
        );
        let project = ask(
            &again,
            "printer_select",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        assert_eq!(project["choices"]["bed_type"], "Cool Plate");

        // A project started from this one is on its printer, bed type
        // and all.
        let fresh = ask(&again, "project_new", json!({}))["id"]
            .as_u64()
            .unwrap();
        let fresh = ask(&again, "project", json!({ "project": fresh }));
        assert_eq!(fresh["choices"]["machine"], project["choices"]["machine"]);
        assert_eq!(fresh["choices"]["bed_type"], "Cool Plate");
        assert_eq!(fresh["dirty"], false);
    }

    #[test]
    fn removing_a_printer_moves_the_projects_that_were_on_it() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        ask(&session, "printers_save", models(&[U1, A1, CC]));
        // Three projects: one on the Bambu printer, one on the U1, and one
        // sliced with the settings it was saved with.
        let cube = reference("resources/handy_models/OrcaToleranceTest.stl");
        let bare = ask(&session, "project_open", json!({ "path": cube }))["id"]
            .as_u64()
            .unwrap();
        ask(
            &session,
            "printer_select",
            json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" }),
        );
        let on_u1 = ask(&session, "project_new", json!({}))["id"]
            .as_u64()
            .unwrap();
        assert_ne!(on_u1, bare);
        ask(
            &session,
            "printer_select",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        let own = ask(
            &session,
            "project_open",
            json!({ "path": reference("tests/mcbrim_test.3mf") }),
        )["id"]
            .as_u64()
            .unwrap();
        // It was opened in a new project, started from one on the U1:
        // that printer's bed type is not chosen for it, its own holds.
        let opened = ask(&session, "project", json!({ "project": own }));
        assert_eq!(opened["choices"]["bed_type"], Value::Null);
        assert_eq!(opened["dirty"], false);
        let own_bed =
            ask(&session, "settled", json!({ "project": own }))["printer"]["bed_types"]["in_force"]
                .clone();
        assert!(own_bed.is_string());
        let listing = ask(&session, "printers", json!({}));
        assert_eq!(listing["selected"]["source"], "project");
        // The picture of its printer can be shown: a U1 is among those added.
        assert_eq!(listing["selected"]["vendor"], "Snapmaker");
        assert_eq!(listing["selected"]["model"], "Snapmaker U1");
        assert_eq!(listing["project_printer"], listing["selected"]["machine"]);
        assert!(listing["project_printer"].as_str().unwrap().contains("U1"));

        // The Bambu printer is removed, and a printer is added, while
        // the project on its own settings is the one shown.
        let saved = ask(
            &session,
            "printers_save",
            models(&[U1, CC, ("Elegoo", "Elegoo Neptune 4")]),
        );
        assert_eq!(
            saved["removed"],
            json!([{ "vendor": "BBL", "model": "Bambu Lab A1 mini" }])
        );
        assert_eq!(
            saved["added"],
            json!([{ "vendor": "Elegoo", "model": "Elegoo Neptune 4" }])
        );
        assert_eq!(
            names(&saved),
            ["Elegoo Centauri Carbon", "Elegoo Neptune 4", "Snapmaker U1"]
        );
        // Its own settings stay what it is sliced with.
        assert_eq!(saved["selected"]["source"], "project");
        assert_eq!(
            ask(&session, "project", json!({ "project": own }))["dirty"],
            false
        );
        // The project that was on the removed printer is on the first
        // of those added, by preset name; the one on the U1 stays.
        assert_eq!(
            saved["switched"],
            json!([{
                "project": bare,
                "name": "OrcaToleranceTest",
                "from": "Bambu Lab A1 mini 0.4 nozzle",
                "to": "Snapmaker U1 (0.4 nozzle)",
            }])
        );
        let moved = ask(&session, "project", json!({ "project": bare }));
        assert_eq!(moved["choices"]["vendor"], "Snapmaker");
        assert_eq!(moved["choices"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        let fits = ask(
            &session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": "Snapmaker U1 (0.4 nozzle)" }),
        );
        assert!(
            fits["processes"]
                .as_array()
                .unwrap()
                .contains(&moved["choices"]["process"])
        );
        assert!(
            fits["filaments"]
                .as_array()
                .unwrap()
                .contains(&moved["choices"]["filaments"][0])
        );
        assert_eq!(
            ask(&session, "project", json!({ "project": on_u1 }))["choices"]["machine"],
            "Snapmaker U1 (0.4 nozzle)"
        );
        assert_eq!(
            refused(
                &session,
                "printer_select",
                json!({ "vendor": "BBL", "model": "Bambu Lab A1 mini" })
            ),
            "Bambu Lab A1 mini is not among the printers added"
        );

        // A printer added while a project on presets is shown: that
        // project goes to it, as the reference's selection does.
        ask(&session, "project_select", json!({ "id": on_u1 }));
        let saved = ask(&session, "printers_save", models(&[U1, CC, A1]));
        assert_eq!(saved["selected"]["machine"], "Bambu Lab A1 mini 0.4 nozzle");
        assert_eq!(saved["switched"], json!([]));

        // The project on its own settings goes to a printer of the list
        // when one is picked for it: on the process of the same name.
        let project = ask(
            &session,
            "printer_select",
            json!({ "project": own, "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        assert_eq!(project["choices"]["source"], "presets");
        assert!(project["choices"]["bed_type"].is_string());
        let back = ask(
            &session,
            "project_set",
            json!({ "project": own, "source": "project", "bed_type": null }),
        );
        assert_eq!(back["dirty"], false);
        assert_eq!(
            ask(&session, "settled", json!({ "project": own }))["printer"]["bed_types"]["in_force"],
            own_bed
        );
        let project = ask(
            &session,
            "printer_select",
            json!({ "project": own, "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        // Each filament is the one of the name it was saved under, as
        // that printer's presets have it now.
        let short = |names: &Value| -> Vec<String> {
            names
                .as_array()
                .unwrap()
                .iter()
                .map(|name| alias(name.as_str().unwrap()).to_owned())
                .collect()
        };
        assert_eq!(
            short(&project["choices"]["filaments"]),
            short(&project["saved"]["filaments"])
        );
        let fits = ask(
            &session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": "Snapmaker U1 (0.4 nozzle)" }),
        );
        assert!(
            fits["processes"]
                .as_array()
                .unwrap()
                .contains(&project["choices"]["process"])
        );
        assert_eq!(
            project["choices"]["filaments"].as_array().unwrap().len() as u64,
            project["filament_count"].as_u64().unwrap()
        );
    }

    #[test]
    fn closing_the_first_selection_adds_the_usual_printer() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        let listing = ask(&session, "printers_default", json!({}));
        assert_eq!(names(&listing), ["Snapmaker U1"]);
        assert_eq!(listing["first_run"], false);
        assert_eq!(
            listing["added"],
            json!([{ "vendor": "Snapmaker", "model": "Snapmaker U1" }])
        );
        assert_eq!(listing["selected"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(ask(&session, "project", json!({}))["dirty"], false);
        // With a printer added it changes nothing.
        ask(&session, "printers_save", models(&[CC]));
        let listing = ask(&session, "printers_default", json!({}));
        assert_eq!(names(&listing), ["Elegoo Centauri Carbon"]);
        assert_eq!(listing["added"], json!([]));

        // Profiles without that printer: nothing is added, nothing fails.
        let profiles = tempfile::tempdir().unwrap();
        std::fs::write(profiles.path().join("Maker.json"), r#"{ "name": "Maker" }"#).unwrap();
        let other = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::with_settings(None, Some(other.path().to_owned())));
        ask(
            &session,
            "set_profiles",
            json!({ "dir": profiles.path().to_string_lossy() }),
        );
        let listing = ask(&session, "printers_default", json!({}));
        assert_eq!(listing["installed"], json!([]));
        assert_eq!(listing["first_run"], true);
        assert_eq!(
            ask(&session, "printer_catalogue", json!({}))["models"],
            json!([])
        );
    }

    #[test]
    fn printers_added_from_other_profiles_stay_added() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        ask(&session, "printers_save", models(&[U1]));
        // Other profiles, with a vendor of their own.
        let profiles = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(profiles.path().join("Maker").join("machine")).unwrap();
        std::fs::write(
            profiles.path().join("Maker.json"),
            r#"{ "name": "Maker",
                 "machine_model_list": [{ "name": "Maker One", "sub_path": "machine/model.json" }],
                 "machine_list": [{ "name": "Maker One 0.4 nozzle", "sub_path": "machine/04.json" }] }"#,
        )
        .unwrap();
        std::fs::write(
            profiles
                .path()
                .join("Maker")
                .join("machine")
                .join("model.json"),
            r#"{ "type": "machine_model", "name": "Maker One", "nozzle_diameter": "0.4" }"#,
        )
        .unwrap();
        std::fs::write(
            profiles.path().join("Maker").join("machine").join("04.json"),
            r#"{ "type": "machine", "name": "Maker One 0.4 nozzle", "instantiation": "true",
                 "printer_model": "Maker One", "printer_variant": "0.4", "nozzle_diameter": ["0.4"],
                 "printable_area": ["0x0", "100x0", "100x100", "0x100"], "printable_height": "100" }"#,
        )
        .unwrap();
        ask(
            &session,
            "set_profiles",
            json!({ "dir": profiles.path().to_string_lossy() }),
        );
        // The U1 is not among these profiles: not listed, not offered.
        let listing = ask(&session, "printers", json!({}));
        assert_eq!(listing["installed"], json!([]));
        assert_eq!(listing["first_run"], true);
        let saved = ask(&session, "printers_save", models(&[("Maker", "Maker One")]));
        assert_eq!(names(&saved), ["Maker One"]);
        assert_eq!(saved["removed"], json!([]));
        assert_eq!(saved["installed"][0]["cover"], false);
        assert_eq!(
            ask(
                &session,
                "printer_cover",
                json!({ "vendor": "Maker", "model": "Maker One" })
            )["data"],
            Value::Null
        );
        // A printer with no default and no list of plates: the
        // reference's High Temp Plate, and no choice.
        assert_eq!(
            ask(&session, "project", json!({}))["choices"]["bed_type"],
            "High Temp Plate"
        );
        // Back on the reference's profiles the U1 is still added.
        ask(
            &session,
            "set_profiles",
            json!({ "dir": reference("resources/profiles") }),
        );
        assert_eq!(
            names(&ask(&session, "printers", json!({}))),
            ["Snapmaker U1"]
        );
    }

    #[test]
    fn a_printers_picture_is_handed_over_whole() {
        let dir = tempfile::tempdir().unwrap();
        let session = session(dir.path());
        let cover = ask(
            &session,
            "printer_cover",
            json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
        );
        let data = cover["data"].as_str().unwrap();
        // A PNG begins 89 50 4E 47 0D 0A 1A 0A.
        assert!(
            data.starts_with("data:image/png;base64,iVBORw0KGgo"),
            "{}",
            &data[..40]
        );
        let size = std::fs::metadata(reference(
            "resources/profiles/Snapmaker/Snapmaker U1_cover.png",
        ))
        .unwrap()
        .len() as usize;
        assert_eq!(
            data.len(),
            "data:image/png;base64,".len() + size.div_ceil(3) * 4
        );
        assert_eq!(
            refused(
                &session,
                "printer_cover",
                json!({ "vendor": "Snapmaker", "model": "../Snapmaker U1" })
            ),
            "Snapmaker has no printer ../Snapmaker U1"
        );
        assert_eq!(
            refused(&session, "printer_cover", json!({ "vendor": "Snapmaker" })),
            "model was not given"
        );
        assert!(
            refused(
                &session,
                "printer_cover",
                json!({ "vendor": "Nobody", "model": "x" })
            )
            .contains("Nobody")
        );
    }

    #[test]
    fn the_rules_a_printer_change_follows() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(&[0xfb, 0xff, 0xfe, 0x00]), "+//+AA==");

        assert_eq!(
            alias("0.20mm Standard @Snapmaker U1 (0.4 nozzle)"),
            "0.20mm Standard"
        );
        assert_eq!(alias("Generic PLA"), "Generic PLA");
        assert_eq!(alias("@odd"), "@odd");

        let printer = |default: Option<&str>| {
            let mut settings = Settings::from_json("{}").unwrap();
            if let Some(default) = default {
                settings.insert(
                    "default_bed_type",
                    edge_config::Value::One(default.to_owned()),
                );
            }
            settings
        };
        let model = |id: &str, default: Option<&str>| PrinterModel {
            vendor: String::new(),
            model: String::new(),
            name: String::new(),
            model_id: Some(id.to_owned()),
            variants: Vec::new(),
            default_materials: Vec::new(),
            bed_model: None,
            bed_texture: None,
            default_bed_type: default.map(str::to_owned),
            cover: None,
            machines: Vec::new(),
            bed: None,
            height: None,
            nozzles: None,
        };
        let u1 = model("SM_U1", None);
        let on = |default: Option<&str>, model: Option<&PrinterModel>| {
            default_bed_type(&printer(default), model)
        };
        assert_eq!(on(Some("Textured PEI Plate"), None), "Textured PEI Plate");
        assert_eq!(on(Some("7"), None), "Supertack Plate");
        assert_eq!(on(Some("1"), Some(&u1)), "Cool Plate");
        assert_eq!(on(Some("Glass"), Some(&u1)), "High Temp Plate");
        assert_eq!(on(Some(""), Some(&u1)), "Textured PEI Plate");
        assert_eq!(on(None, Some(&model("BL-P001", None))), "Cool Plate");
        assert_eq!(on(None, Some(&model("C11", None))), "High Temp Plate");
        assert_eq!(on(None, None), "High Temp Plate");
        // The model's own file, when the preset names none.
        let a1 = model("N1", Some("Textured PEI Plate"));
        assert_eq!(on(None, Some(&a1)), "Textured PEI Plate");
        assert_eq!(on(Some("2"), Some(&a1)), "Engineering Plate");
    }
}
