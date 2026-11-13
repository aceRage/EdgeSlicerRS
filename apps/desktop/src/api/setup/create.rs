//! "Create Printer/Nozzle" and "Create Filament" (CreatePresetsDialog.cpp
//! `CreatePrinterPresetDialog`, `CreateFilamentPresetDialog`): new printer
//! presets, with the filament and process presets made for them, and new
//! filaments, each made for the printers chosen. What they write is what
//! the reference's writer writes (Preset.cpp `clone_presets`,
//! `clone_presets_for_printer`, `clone_presets_for_filament`,
//! `save_current_preset` with `detach`, `Preset::save`): presets that
//! stand alone (`inherits` empty, kept in `base/`), written whole.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use edge_config::{
    Kind, Settings, Vendor,
    user::{self, ToSave},
};
use md5::{Digest, Md5};
use serde_json::{Value, json};

use super::super::{
    Session,
    i18n::{fill, tr},
    lock, needed, printers, text,
};
use super::lists::{FILAMENT_VENDORS, NOZZLES, PRINTER_MODELS, PRINTER_VENDORS};
use super::natural;

/// The folder the reference keeps its printer templates in, beside its
/// profiles (Preset.hpp `PRESET_PROFILES_TEMOLATE_DIR`), and the vendor
/// in it (`PRESET_TEMPLATE_DIR`).
const TEMPLATE_DIR: &str = "profiles_template";
const TEMPLATE_VENDOR: &str = "Template";

/// The largest file "Hot Bed STL" and "Hot Bed SVG" take, in MB
/// (Utils/FileHelp.hpp `STL_SVG_MAX_FILE_SIZE_MB`).
const BED_FILE_MAX_MB: u64 = 3;

/// The characters taken out of a typed vendor, model or serial
/// (`remove_special_key`).
fn without_special(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\n' | '\t' | '\r' | '\u{b}' | '@' | ';'))
        .collect()
}

/// The nozzle of "0.4 mm" ("0.4").
fn nozzle_of(choice: &str) -> &str {
    choice.split(" mm").next().unwrap_or(choice).trim()
}

/// A nozzle the dialog offers, as a number (`nozzle_diameter_map`).
fn nozzle_number(nozzle: &str) -> Option<f64> {
    NOZZLES
        .contains(&nozzle)
        .then(|| nozzle.parse().ok())
        .flatten()
}

/// A preset's name up to " @" (`clone_presets_for_printer`'s prefix).
fn prefix_of(name: &str) -> &str {
    name.split(" @").next().unwrap_or(name)
}

/// The name a filament goes by in `get_filament_id`: up to the first "@",
/// less the character before it (`substr(0, index_at - 1)`, which takes
/// the space of " @" off). None for a name without "@".
fn filament_name_of(name: &str) -> Option<&str> {
    let at = name.find('@')?;
    Some(&name[..at.saturating_sub(1)])
}

/// A preset's settings as the reference's clone writes them: every
/// setting it knows, the name, the kind's id and what says where it comes
/// from left to the writer (`clone_presets`: the source preset's whole
/// config, its link to a parent cleared).
fn whole(settings: &Settings, kind: Kind) -> Settings {
    let mut own = Settings::default();
    for (key, value) in settings.iter() {
        if matches!(
            key,
            "name" | "from" | "version" | "is_custom_defined" | "inherits" | "instantiation"
        ) || key == user::id_key(kind)
            || !edge_config::reference_knows(key)
        {
            continue;
        }
        own.insert(key, value.clone());
    }
    own
}

/// The settings of the process templates for a nozzle
/// (`generate_process_presets_data`): layer heights half the nozzle, line
/// widths the nozzle, walls and shells at least as thick as for a 0.4
/// nozzle.
/// The reference keeps the nozzle as a `float` there: its sums are
/// worked out in that precision ("3 * 0.4 / 0.4f" is just under 3, so a
/// 0.4 nozzle keeps three bottom layers), and so are they here.
fn for_nozzle(settings: &mut Settings, nozzle: f64) {
    let nozzle = f64::from(nozzle as f32);
    let number = |value: f64| {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    for key in ["layer_height", "initial_layer_print_height"] {
        settings.insert(key, edge_config::Value::One(number(nozzle / 2.0)));
    }
    for key in [
        "line_width",
        "initial_layer_line_width",
        "outer_wall_line_width",
        "inner_wall_line_width",
        "top_surface_line_width",
        "sparse_infill_line_width",
        "internal_solid_infill_line_width",
        "support_line_width",
    ] {
        settings.insert(key, edge_config::Value::One(number(nozzle)));
    }
    let at_least =
        |least: i64, layers: f64| least.max((layers * 0.4 / nozzle).ceil() as i64).to_string();
    settings.insert("wall_loops", edge_config::Value::One(at_least(2, 2.0)));
    settings.insert(
        "top_shell_layers",
        edge_config::Value::One(at_least(5, 5.0)),
    );
    settings.insert(
        "bottom_shell_layers",
        edge_config::Value::One(at_least(3, 3.0)),
    );
}

/// What "Create Printer/Nozzle" was given.
struct PrinterForm {
    /// "Create Nozzle for Existing Printer".
    nozzle_only: bool,
    /// "Can't find my printer model": the vendor and model typed.
    custom: bool,
    vendor: String,
    model: String,
    /// The printer of one's own a nozzle is made for.
    existing: String,
    nozzle: String,
    bed: [f64; 2],
    origin: [f64; 2],
    height: f64,
    bed_model: Option<PathBuf>,
    bed_texture: Option<PathBuf>,
}

fn number_of(args: &Value, key: &str) -> f64 {
    match args.get(key) {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(0.0),
        // As `ToDouble` reads a field: nothing it can read is 0.
        Some(Value::String(text)) => text.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

impl PrinterForm {
    fn read(args: &Value) -> Self {
        let pair = |key: &str| -> [f64; 2] {
            let list = args.get(key).and_then(Value::as_array);
            let at = |i: usize| {
                list.and_then(|list| list.get(i))
                    .map(|value| number_of(&json!({ "v": value }), "v"))
                    .unwrap_or(0.0)
            };
            [at(0), at(1)]
        };
        let path = |key: &str| {
            text(args, key)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
        };
        Self {
            nozzle_only: text(args, "type") == Some("nozzle"),
            custom: args.get("custom").and_then(Value::as_bool) == Some(true),
            vendor: text(args, "vendor").unwrap_or_default().to_owned(),
            model: text(args, "model").unwrap_or_default().to_owned(),
            existing: text(args, "printer").unwrap_or_default().to_owned(),
            nozzle: nozzle_of(text(args, "nozzle").unwrap_or("0.4 mm")).to_owned(),
            bed: pair("bed"),
            origin: pair("origin"),
            height: number_of(args, "height"),
            bed_model: path("bed_model"),
            bed_texture: path("bed_texture"),
        }
    }

    /// The bed as given is one the reference takes (`check_printable_area`).
    fn area_ok(&self) -> bool {
        let ([x, y], [dx, dy]) = (self.bed, self.origin);
        x != 0.0 && y != 0.0 && dx < x && dy < y
    }

    /// The first page's checks (`validate_input_valid`), in its order.
    fn check(&self) -> Result<(), String> {
        if self.nozzle_only {
            if self.existing.is_empty() {
                return Err(tr(
                    "You have not yet selected the printer to replace the nozzle, please choose.",
                ));
            }
            return Ok(());
        }
        if self.vendor.is_empty() || self.model.is_empty() {
            return Err(tr(
                "You have not selected the vendor and model or entered the custom vendor and model.",
            ));
        }
        let (vendor, model) = (without_special(&self.vendor), without_special(&self.model));
        if vendor.is_empty() || model.is_empty() {
            return Err(tr(
                "There may be escape characters in the custom printer vendor or model. Please delete and re-enter.",
            ));
        }
        if vendor.trim().is_empty() || model.trim().is_empty() {
            return Err(tr(
                "All inputs in the custom printer vendor or model are spaces. Please re-enter.",
            ));
        }
        if !self.area_ok() {
            return Err(tr("Please check bed printable shape and origin input."));
        }
        Ok(())
    }
}

/// The models of a vendor for a nozzle as "Printer Preset" offers them,
/// the nearest nozzle first ("<model> @ <nozzle> nozzle",
/// `printer_preset_sort_with_nozzle_diameter`).
fn by_nearest_nozzle(models: &[edge_engine::PrinterModel], nozzle: f64) -> Vec<String> {
    let mut sorted: Vec<(f64, String)> = Vec::new();
    for model in models {
        for variant in &model.variants {
            if let Ok(size) = variant.parse::<f64>() {
                sorted.push((size, format!("{} @ {variant} nozzle", model.name)));
            }
        }
    }
    // A stable sort by size, as `std::sort` leaves equals in no set
    // order; here in the vendor's.
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut nearest: isize = -1;
    let mut best = 1.0;
    for (at, (size, _)) in sorted.iter().enumerate() {
        let off = (nozzle - size).abs();
        if off < best {
            nearest = at as isize;
            best = off;
            if off == 0.0 {
                break;
            }
        }
    }
    let mut out = Vec::new();
    let mut left = nearest;
    let mut right = (nearest + 1) as usize;
    while left >= 0 || right < sorted.len() {
        if left >= 0 && right < sorted.len() {
            let l = (nozzle - sorted[left as usize].0).abs();
            let r = (nozzle - sorted[right].0).abs();
            if l < r {
                out.push(sorted[left as usize].1.clone());
                left -= 1;
            } else {
                out.push(sorted[right].1.clone());
                right += 1;
            }
        } else if left >= 0 {
            out.push(sorted[left as usize].1.clone());
            left -= 1;
        } else {
            out.push(sorted[right].1.clone());
            right += 1;
        }
    }
    out
}

/// A model and nozzle of "Printer Preset" ("<model> @ 0.4 nozzle").
fn model_and_variant(choice: &str) -> Option<(&str, &str)> {
    let (model, rest) = choice.split_once(" @ ")?;
    let variant = rest.strip_suffix(" nozzle")?;
    Some((model, variant.trim()))
}

/// One preset to write: its kind, name and settings, whole.
struct Made {
    kind: Kind,
    name: String,
    own: Settings,
    version: String,
}

impl Session {
    /// The folder of the reference's printer templates, beside the
    /// profiles in use.
    fn template_vendor(&self) -> Result<Vendor, String> {
        let dir = lock(&self.state)
            .library
            .as_ref()
            .map(|library| library.dir().to_owned())
            .ok_or("no profiles folder has been chosen")?;
        let templates = dir
            .parent()
            .map(|parent| parent.join(TEMPLATE_DIR))
            .filter(|templates| templates.join(format!("{TEMPLATE_VENDOR}.json")).is_file())
            .ok_or_else(|| tr("Preset path was not found, please reselect vendor."))?;
        Vendor::open(&templates, TEMPLATE_VENDOR).map_err(|e| e.to_string())
    }

    /// The person's printers that stand alone: those "Create Nozzle for
    /// Existing Printer" offers (`set_current_visible_printer`).
    pub(in crate::api) fn standalone_printers(&self) -> Vec<String> {
        self.users()
            .of(Kind::Machine)
            .filter(|preset| preset.visible && preset.inherits.is_none())
            .map(|preset| preset.name.clone())
            .collect()
    }

    /// `create_printer_choices`: what the dialog's lists offer.
    pub(in crate::api) fn create_printer_choices(&self) -> Result<Value, String> {
        let library = lock(&self.state)
            .library
            .clone()
            .ok_or("no profiles folder has been chosen")?;
        // The vendors whose models a printer can be made from: every vendor
        // of the profiles with models (`get_exist_vendor_choices`).
        let mut template_vendors = Vec::new();
        for vendor in library.vendors().map_err(|e| e.to_string())? {
            if self
                .models_of(&vendor)
                .is_ok_and(|models| !models.is_empty())
            {
                template_vendors.push(vendor);
            }
        }
        Ok(json!({
            "vendors": PRINTER_VENDORS
                .iter()
                .map(|vendor| {
                    let models = PRINTER_MODELS
                        .iter()
                        .find(|(known, _)| known == vendor)
                        .map(|(_, models)| models.to_vec())
                        .unwrap_or_default();
                    json!({ "vendor": vendor, "models": models })
                })
                .collect::<Vec<_>>(),
            "nozzles": NOZZLES.iter().map(|nozzle| format!("{nozzle} mm")).collect::<Vec<_>>(),
            "existing": self.standalone_printers(),
            "template_vendors": template_vendors,
        }))
    }

    /// `create_printer_models`: a vendor's models for "Printer Preset",
    /// the nearest to the nozzle chosen first.
    pub(in crate::api) fn create_printer_models(&self, args: &Value) -> Result<Value, String> {
        let vendor = needed(args, "vendor")?;
        let models = self
            .models_of(vendor)
            .map_err(|_| tr("Vendor was not found, please reselect."))?;
        let nozzle =
            nozzle_number(nozzle_of(text(args, "nozzle").unwrap_or("0.4 mm"))).unwrap_or(0.4);
        let listed = by_nearest_nozzle(&models, nozzle);
        if listed.is_empty() {
            return Err(tr("Current vendor has no models, please reselect."));
        }
        Ok(json!({ "models": listed }))
    }

    /// The system printer preset of a model and nozzle of a vendor, by
    /// the dialog's "<model> @ <nozzle> nozzle"
    /// (`find_system_preset_by_model_and_variant`).
    fn template_printer(&self, vendor: &str, choice: &str) -> Result<String, String> {
        let (name, variant) = model_and_variant(choice)
            .ok_or_else(|| tr("The nozzle diameter was not found, please reselect."))?;
        let models = self
            .models_of(vendor)
            .map_err(|_| tr("Preset path was not found, please reselect vendor."))?;
        models
            .iter()
            .filter(|model| model.name == name)
            .flat_map(|model| model.machines.iter())
            .find(|preset| preset.variant == variant)
            .map(|preset| preset.name.clone())
            .ok_or_else(|| tr("The printer preset was not found, please reselect."))
    }

    /// `create_printer_presets`: the filament and process presets the
    /// second page offers to make for the new printer: the templates
    /// ("Create from Template": the reference's `profiles_template`), or
    /// those that go with the printer chosen ("Create Based on Current
    /// Printer": the vendor's made for it and the person's that go with
    /// it, `update_presets_list`).
    pub(in crate::api) fn create_printer_presets(&self, args: &Value) -> Result<Value, String> {
        let vendor = needed(args, "vendor")?;
        let machine = self.template_printer(vendor, needed(args, "model")?)?;
        if args.get("template").and_then(Value::as_bool) == Some(true) {
            let templates = self.template_vendor()?;
            let list = |kind: Kind| -> Vec<String> {
                template_names(&templates, kind)
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect()
            };
            return Ok(json!({
                "printer": machine,
                "filaments": list(Kind::Filament),
                "processes": list(Kind::Process),
            }));
        }
        let shelf = self.shelf(vendor)?;
        let offered = self.offered_all(&shelf, &machine);
        Ok(json!({
            "printer": machine,
            "filaments": offered.0,
            "processes": offered.1,
        }))
    }

    /// Every filament and process preset that goes with a printer, the
    /// vendor's whether enabled or not, the person's after.
    fn offered_all(&self, shelf: &edge_engine::Shelf, machine: &str) -> (Vec<String>, Vec<String>) {
        let users = self.users();
        let mine = |kind: Kind| -> Vec<String> {
            users
                .of(kind)
                .filter(|preset| preset.visible && !shelf.knows(kind, &preset.name))
                .filter(|preset| self.fits(shelf, kind, &preset.name, machine))
                .map(|preset| preset.name.clone())
                .collect()
        };
        let mut filaments: Vec<String> = shelf
            .filaments_for(machine)
            .into_iter()
            .map(str::to_owned)
            .collect();
        filaments.extend(mine(Kind::Filament));
        let mut processes: Vec<String> = shelf
            .processes_for(machine)
            .into_iter()
            .map(str::to_owned)
            .collect();
        processes.extend(mine(Kind::Process));
        (filaments, processes)
    }

    /// `create_printer_check`: the first page's "OK" (`validate_input_valid`).
    pub(in crate::api) fn create_printer_check(&self, args: &Value) -> Result<Value, String> {
        PrinterForm::read(args).check()?;
        Ok(json!({ "ok": true }))
    }

    /// `create_printer_bed_file`: a file for "Hot Bed STL" (`kind`
    /// "model") or "Hot Bed SVG" ("texture"), checked as the reference
    /// checks it (`load_model_stl`, `load_texture`).
    pub(in crate::api) fn create_printer_bed_file(&self, args: &Value) -> Result<Value, String> {
        let path = PathBuf::from(needed(args, "path")?);
        let kinds: &[&str] = match text(args, "kind") {
            Some("model") => &["stl"],
            _ => &["png", "svg"],
        };
        bed_file_ok(&path, kinds)?;
        Ok(json!({
            "path": super::super::shown(&path),
            "name": path.file_name().map(|name| name.to_string_lossy().into_owned()),
        }))
    }

    /// The `filament_id` a new filament is given (`get_filament_id`): that
    /// of a preset already there of the same filament name, else "P" and
    /// the first seven of the name's MD5, made again with the time while
    /// another filament has it.
    pub(in crate::api) fn new_filament_id(&self, wanted: &str) -> String {
        // Every filament preset known, the person's and the vendors' of
        // the printers added with the shared library, in order of name as
        // the reference's collection holds them (it reads every vendor's).
        let mut known: Vec<(String, String)> = Vec::new();
        let users = self.users();
        for preset in users.of(Kind::Filament) {
            let id = preset.own.text("filament_id").unwrap_or_default();
            known.push((preset.name.clone(), id.to_owned()));
        }
        let vendors: Vec<String> = lock(&self.state)
            .store
            .data
            .models
            .keys()
            .cloned()
            .collect();
        for vendor in vendors {
            let Ok(shelf) = self.shelf(&vendor) else {
                continue;
            };
            for name in shelf.all_of(Kind::Filament) {
                let id = shelf
                    .resolve(Kind::Filament, &name)
                    .ok()
                    .and_then(|settings| settings.text("filament_id").map(str::to_owned))
                    .unwrap_or_default();
                known.push((name, id));
            }
        }
        known.sort();
        known.dedup();
        let mut by_id: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (name, id) in &known {
            let Some(filament) = filament_name_of(name) else {
                continue;
            };
            if filament == wanted && id != "null" {
                return id.clone();
            }
            by_id
                .entry(id.clone())
                .or_default()
                .insert(filament.to_owned());
        }
        let md5 = |text: &str| -> String {
            Md5::digest(text.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        let mut id = format!("P{}", &md5(wanted)[..7]);
        let mut salt = 0u32;
        while let Some(names) = by_id.get(&id) {
            if names.contains(wanted) {
                break;
            }
            // The reference adds the time of day; a count is as good and
            // repeatable.
            salt += 1;
            id = format!("P{}", &md5(&format!("{wanted}{salt}"))[..7]);
        }
        id
    }

    /// The settings of a preset named on the second page: a template, or
    /// the vendor's or the person's.
    fn source_preset(
        &self,
        templates: Option<&Vendor>,
        shelf: &edge_engine::Shelf,
        kind: Kind,
        name: &str,
    ) -> Result<(Settings, String), String> {
        match templates {
            Some(templates) => {
                let listed = template_names(templates, kind)
                    .into_iter()
                    .find(|(shown, _)| shown == name)
                    .map(|(_, listed)| listed)
                    .ok_or_else(|| format!("there is no template {name}"))?;
                Ok((
                    templates
                        .resolve(kind, &listed)
                        .map_err(|e| e.to_string())?,
                    templates.preset_version(),
                ))
            }
            None => {
                let users = self.users();
                let version = users
                    .get(kind, name)
                    .and_then(|preset| preset.version.clone())
                    .unwrap_or_else(|| shelf.preset_version());
                Ok((self.resolve_named(shelf, kind, name)?, version))
            }
        }
    }

    /// `create_printer`: "Create" on the second page. Checks in the
    /// reference's order (`create_page2_dialog_buttons`), asks before a
    /// preset of the name is written over (`rewrite`, then
    /// `rewrite_presets`), then writes the filament and process presets
    /// for the new printer and the printer itself, every one standing
    /// alone, and puts the project shown on the new printer.
    pub(in crate::api) fn create_printer(&self, args: &Value) -> Result<Value, String> {
        let form = PrinterForm::read(args);
        form.check()?;
        let info = |message: String| -> Result<Value, String> { Err(message) };
        let template_vendor = text(args, "preset_vendor").unwrap_or_default();
        let template_model = text(args, "preset_model").unwrap_or_default();
        if template_vendor.is_empty() || template_model.is_empty() {
            return info(tr(
                "You have not yet chosen which printer preset to create based on. Please choose the vendor and model of the printer",
            ));
        }
        let template_machine = self.template_printer(template_vendor, template_model)?;
        let shelf = self.shelf(template_vendor)?;
        if !form.nozzle_only && !form.area_ok() {
            return info(tr(
                "You have entered an illegal input in the printable area section on the first page. Please check before creating it.",
            ));
        }
        let nozzle_name = format!("{} nozzle", form.nozzle);
        let (printer_name, printer_model) = if form.nozzle_only {
            let existing = self
                .users()
                .get(Kind::Machine, &form.existing)
                .cloned()
                .ok_or_else(|| tr("You have not yet selected the printer to replace the nozzle, please choose."))?;
            let model = existing
                .own
                .text("printer_model")
                .unwrap_or_default()
                .to_owned();
            (format!("{model} {nozzle_name}"), model)
        } else {
            let vendor = without_special(&form.vendor).trim().to_owned();
            let model = without_special(&form.model).trim().to_owned();
            if form.custom && (vendor.is_empty() || model.is_empty()) {
                return info(tr(
                    "The custom printer or model is not entered, please enter it.",
                ));
            }
            (
                format!("{vendor} {model} {nozzle_name}"),
                format!("{vendor} {model}"),
            )
        };
        let rewrite = args.get("rewrite").and_then(Value::as_bool) == Some(true);
        let users = self.users();
        if !rewrite
            && (users.get(Kind::Machine, &printer_name).is_some()
                || shelf.knows(Kind::Machine, &printer_name))
        {
            return Ok(json!({
                "created": false,
                "step": "printer",
                "title": tr("Info"),
                "ask": tr(
                    "The printer preset you created already has a preset with the same name. Do you want to overwrite it?\n\tYes: Overwrite the printer preset with the same name, and filament and process presets with the same preset name will be recreated \nand filament and process presets without the same preset name will be reserve.\n\tCancel: Do not create a preset, return to the creation interface."
                ),
            }));
        }
        if shelf.knows(Kind::Machine, &printer_name) {
            return info(tr("Overwriting a system profile is not allowed."));
        }
        let wanted = |key: &str| -> Vec<String> {
            args.get(key)
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|name| name.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (filaments, processes) = (wanted("filaments"), wanted("processes"));
        let named_for =
            |name: &str| format!("{} @{printer_name}", prefix_of(name).replace('/', "-"));
        // "You need to select at least one …" unless one made for the
        // printer is there already. The reference looks for " @ " (with a
        // space after "@") where the presets it makes have " @": the names
        // made are looked for here.
        if filaments.is_empty()
            && !users
                .of(Kind::Filament)
                .any(|preset| preset.name.ends_with(&format!(" @{printer_name}")))
        {
            return info(tr("You need to select at least one filament preset."));
        }
        if processes.is_empty()
            && !users
                .of(Kind::Process)
                .any(|preset| preset.name.ends_with(&format!(" @{printer_name}")))
        {
            return info(tr("You need to select at least one process preset."));
        }
        let from_template = args.get("template").and_then(Value::as_bool) == Some(true);
        let templates = if from_template {
            Some(self.template_vendor()?)
        } else {
            None
        };
        // What would be written over: asked once for both kinds, the
        // filaments' first (the reference asks for each kind in turn).
        let rewrite_presets =
            rewrite || args.get("rewrite_presets").and_then(Value::as_bool) == Some(true);
        if !rewrite_presets {
            for (kind, names, heading) in [
                (
                    Kind::Filament,
                    &filaments,
                    "Create filament presets failed. As follows:\n",
                ),
                (
                    Kind::Process,
                    &processes,
                    "Create process presets failed. As follows:\n",
                ),
            ] {
                let taken: Vec<String> = names
                    .iter()
                    .map(|name| named_for(name))
                    .filter(|made| users.get(kind, made).is_some() || shelf.knows(kind, made))
                    .collect();
                if !taken.is_empty() {
                    let mut message = tr(heading);
                    for name in &taken {
                        message.push('\t');
                        message.push_str(name);
                        message.push('\n');
                    }
                    message.push_str(&tr("\nDo you want to rewrite it?"));
                    return Ok(json!({
                        "created": false,
                        "step": if kind == Kind::Filament { "filaments" } else { "processes" },
                        "title": tr("Info"),
                        "ask": message,
                    }));
                }
            }
        }
        let nozzle = nozzle_number(&form.nozzle).unwrap_or(0.4);
        let mut made: Vec<Made> = Vec::new();
        for name in &filaments {
            let (settings, version) =
                self.source_preset(templates.as_ref(), &shelf, Kind::Filament, name)?;
            let mut own = whole(&settings, Kind::Filament);
            own.insert(
                "compatible_printers",
                edge_config::Value::Many(vec![printer_name.clone()]),
            );
            let prefix = prefix_of(name).replace('/', "-");
            own.insert(
                "filament_id",
                edge_config::Value::One(self.new_filament_id(&prefix)),
            );
            made.push(Made {
                kind: Kind::Filament,
                name: named_for(name),
                own,
                version,
            });
        }
        for name in &processes {
            let (settings, version) =
                self.source_preset(templates.as_ref(), &shelf, Kind::Process, name)?;
            let mut own = whole(&settings, Kind::Process);
            if from_template {
                for_nozzle(&mut own, nozzle);
            }
            own.insert(
                "compatible_printers",
                edge_config::Value::Many(vec![printer_name.clone()]),
            );
            made.push(Made {
                kind: Kind::Process,
                name: named_for(name),
                own,
                version,
            });
        }
        // The printer: the template printer's whole, its bed, height and
        // bed files from the first page (`save_printable_area_config`),
        // or the existing printer's for another nozzle, and its model,
        // nozzle and variant.
        let template = shelf
            .resolve(Kind::Machine, &template_machine)
            .map_err(|e| e.to_string())?;
        let mut own = whole(&template, Kind::Machine);
        let number = |value: f64| {
            let text = format!("{value:.6}");
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        };
        let data_dir = lock(&self.state).store.dir().map(Path::to_owned);
        if form.nozzle_only {
            let existing = users
                .resolve(Kind::Machine, &form.existing, &|parent| {
                    shelf.resolve(Kind::Machine, parent)
                })
                .map_err(|e| e.to_string())?;
            for key in [
                "printable_area",
                "printable_height",
                "bed_custom_model",
                "bed_custom_texture",
            ] {
                match existing.get(key) {
                    Some(value) => {
                        own.insert(key, value.clone());
                    }
                    None => {
                        own.remove(key);
                    }
                }
            }
        } else {
            let ([x, y], [dx, dy]) = (form.bed, form.origin);
            let corners = [(0.0, 0.0), (x, 0.0), (x, y), (0.0, y)]
                .iter()
                .map(|(cx, cy)| format!("{}x{}", number(cx - dx), number(cy - dy)))
                .collect();
            own.insert("printable_area", edge_config::Value::Many(corners));
            own.insert(
                "printable_height",
                edge_config::Value::One(number(form.height)),
            );
            // The files are kept in this app's data folder, not where they
            // were picked: the reference keeps the picked file's path.
            let keep =
                |path: &Option<PathBuf>, kinds: &[&str], what: &str| -> Result<String, String> {
                    let Some(path) = path else {
                        return Ok(String::new());
                    };
                    bed_file_ok(path, kinds)?;
                    let dir = data_dir
                        .as_ref()
                        .ok_or("there is no data folder to keep the bed's files in")?
                        .join("bed_files");
                    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                    let extension = path
                        .extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase())
                        .unwrap_or_default();
                    let kept = dir.join(format!("{printer_name} {what}.{extension}"));
                    let bytes =
                        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
                    user::write_whole(&kept, &bytes).map_err(|e| e.to_string())?;
                    Ok(super::super::shown(&kept))
                };
            own.insert(
                "bed_custom_model",
                edge_config::Value::One(keep(&form.bed_model, &["stl"], "bed")?),
            );
            own.insert(
                "bed_custom_texture",
                edge_config::Value::One(keep(&form.bed_texture, &["png", "svg"], "texture")?),
            );
        }
        own.insert("printer_model", edge_config::Value::One(printer_model));
        own.insert(
            "printer_variant",
            edge_config::Value::One(form.nozzle.clone()),
        );
        own.insert(
            "nozzle_diameter",
            edge_config::Value::Many(vec![number(nozzle)]),
        );
        let printer_version = shelf.preset_version();
        // Written: filaments, processes, then the printer, as the reference
        // does; each whole or not at all.
        {
            let mut state = lock(&self.state);
            if state.users.dir.is_none() {
                return Err("there is no data folder to keep presets in".into());
            }
            let presets = Arc::make_mut(&mut state.users.presets);
            for one in made.iter().chain(std::iter::once(&Made {
                kind: Kind::Machine,
                name: printer_name.clone(),
                own: own.clone(),
                version: printer_version.clone(),
            })) {
                presets
                    .save(
                        one.kind,
                        &one.name,
                        ToSave {
                            own: one.own.clone(),
                            inherits: None,
                            version: one.version.clone(),
                            base_id: None,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        // Its vendor, for the lists, is that of the printer it was made from.
        {
            let (machine, vendor) = (printer_name.clone(), template_vendor.to_owned());
            let mut state = lock(&self.state);
            printers::keep(&mut state, |data| {
                data.printers.entry(machine).or_default().vendor = Some(vendor);
            });
        }
        let mut select = json!({ "vendor": template_vendor, "machine": printer_name });
        if let Some(project) = args.get("project") {
            select["project"] = project.clone();
        }
        let mut answer = self.printer_select(&select)?;
        if let Some(object) = answer.as_object_mut() {
            object.insert("created".into(), json!(true));
            object.insert("printer".into(), json!(printer_name));
            object.insert(
                "presets".into(),
                json!(made.iter().map(|one| one.name.clone()).collect::<Vec<_>>()),
            );
        }
        Ok(answer)
    }
}

/// A filament preset "Create Filament" can make from: its name, type, the
/// printers it names, whether it is laid over another, and its id.
struct Source {
    name: String,
    kind: String,
    printers: Vec<String>,
    stands_alone: bool,
    filament_id: String,
}

/// The nozzle a printer preset's name gives ("… 0.4 nozzle" or "…(0.4
/// nozzle)"), as `get_printer_nozzle_diameter` reads it; empty without.
fn printer_nozzle(name: &str) -> String {
    let (head, open) = match name.find(" nozzle)") {
        Some(at) => (&name[..at], '('),
        None => match name.find(" nozzle") {
            Some(at) => (&name[..at], ' '),
            None => return String::new(),
        },
    };
    head.rsplit(open).next().unwrap_or_default().to_owned()
}

/// A printer preset's name without its nozzle, and the nozzle
/// (`split_printer_name_nozzle`).
fn printer_base(name: &str) -> (String, String) {
    let nozzle = printer_nozzle(name);
    if nozzle.is_empty() {
        return (name.to_owned(), nozzle);
    }
    let bracket = format!("({nozzle} nozzle)");
    let plain = format!("{nozzle} nozzle");
    let base = if let Some(at) = name.find(&bracket) {
        format!("{}{}", &name[..at], &name[at + bracket.len()..])
    } else if let Some(at) = name.find(&plain) {
        format!("{}{}", &name[..at], &name[at + plain.len()..])
    } else {
        name.to_owned()
    };
    let collapsed = base
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let base = if collapsed.is_empty() {
        name.to_owned()
    } else {
        collapsed
    };
    (base, nozzle)
}

impl Session {
    /// The printer presets "Create Filament" offers: every one shown, the
    /// vendors' of the printers added and the person's
    /// (`get_all_visible_printer_name`).
    fn visible_printers(&self) -> Vec<String> {
        let mut printers: Vec<String> = self
            .guide_machines(None)
            .into_iter()
            .map(|(_, machine, _, _)| machine)
            .collect();
        printers.extend(
            self.users()
                .of(Kind::Machine)
                .filter(|preset| preset.visible)
                .map(|preset| preset.name.clone()),
        );
        printers.sort();
        printers.dedup();
        printers
    }

    /// Every filament preset "Create Filament" can make from: the person's
    /// and the vendors' shown, each with a `filament_id`
    /// (`get_all_filament_presets`), and the types of all the vendors'
    /// (`m_system_filament_types_set`).
    fn filament_sources(&self) -> (Vec<Source>, BTreeSet<String>) {
        let vendors: Vec<String> = lock(&self.state)
            .store
            .data
            .models
            .keys()
            .cloned()
            .collect();
        let shown = self.shown_filaments();
        let mut sources = Vec::new();
        let mut types = BTreeSet::new();
        let source_of = |name: &str, settings: &Settings| -> Source {
            let first = |key: &str| {
                settings
                    .texts(key)
                    .and_then(|items| items.first().cloned())
                    .or_else(|| settings.text(key).map(str::to_owned))
                    .unwrap_or_default()
            };
            Source {
                name: name.to_owned(),
                kind: first("filament_type"),
                printers: settings
                    .texts("compatible_printers")
                    .map(<[String]>::to_vec)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|printer| !printer.is_empty())
                    .collect(),
                stands_alone: settings.text("inherits").is_none_or(str::is_empty),
                filament_id: settings.text("filament_id").unwrap_or_default().to_owned(),
            }
        };
        let mut first_shelf = None;
        for vendor in &vendors {
            let Ok(shelf) = self.shelf(vendor) else {
                continue;
            };
            for name in shelf.all_of(Kind::Filament) {
                let Ok(settings) = shelf.resolve(Kind::Filament, &name) else {
                    continue;
                };
                let source = source_of(&name, &settings);
                if source.filament_id.is_empty() || source.filament_id == "null" {
                    continue;
                }
                types.insert(source.kind.clone());
                if shown.as_ref().is_none_or(|shown| shown.contains(&name)) {
                    sources.push(source);
                }
            }
            first_shelf.get_or_insert(shelf);
        }
        if let Some(shelf) = first_shelf {
            let users = self.users();
            for preset in users.of(Kind::Filament) {
                let Ok(settings) = self.resolve_named(&shelf, Kind::Filament, &preset.name) else {
                    continue;
                };
                let mut source = source_of(&preset.name, &settings);
                source.stands_alone = preset.inherits.is_none();
                if source.filament_id.is_empty() || source.filament_id == "null" {
                    continue;
                }
                sources.retain(|known| known.name != source.name);
                sources.push(source);
            }
        }
        (sources, types)
    }

    /// `create_filament_choices`: the lists of "Create Filament": its
    /// vendors (Snapmaker first, then in natural order), the types of the
    /// filaments known, and the printers shown.
    pub(in crate::api) fn create_filament_choices(&self) -> Result<Value, String> {
        let mut vendors: Vec<&str> = FILAMENT_VENDORS
            .iter()
            .copied()
            .filter(|v| *v != "Snapmaker")
            .collect();
        vendors.sort_by(|a, b| natural(a, b));
        if FILAMENT_VENDORS.contains(&"Snapmaker") {
            vendors.insert(0, "Snapmaker");
        }
        let (_, types) = self.filament_sources();
        let mut types: Vec<String> = types.into_iter().filter(|kind| !kind.is_empty()).collect();
        types.sort_by(|a, b| natural(a, b));
        Ok(json!({ "vendors": vendors, "types": types }))
    }

    /// `create_filament_presets`: for a type, what "Create Based on
    /// Current Filament" offers (each filament that stands alone, by its
    /// name up to " @", with the printers its presets are for:
    /// `get_filament_preset_choices`) and what "Copy Current Filament
    /// Preset " offers (each printer shown, by its name without the
    /// nozzle, its nozzles, and for each the presets of the type for it:
    /// `get_filament_presets_by_machine`, `build_printer_nozzle_map`),
    /// with the first row the printer and filament the project is on.
    pub(in crate::api) fn create_filament_presets(&self, args: &Value) -> Result<Value, String> {
        let wanted = needed(args, "type")?;
        let (sources, _) = self.filament_sources();
        let printers = self.visible_printers();
        // For a printer, a preset's printers: those it names that are shown,
        // or every one shown with a nozzle in its name when it names none.
        let printers_of = |source: &Source| -> Vec<String> {
            if source.printers.is_empty() {
                printers
                    .iter()
                    .filter(|printer| nozzle_number(&printer_nozzle(printer)).is_some())
                    .cloned()
                    .collect()
            } else {
                source
                    .printers
                    .iter()
                    .filter(|printer| printers.contains(printer))
                    .filter(|printer| nozzle_number(&printer_nozzle(printer)).is_some())
                    .cloned()
                    .collect()
            }
        };
        // "Create Based on Current Filament".
        let mut by_id: BTreeMap<String, Vec<&Source>> = BTreeMap::new();
        for source in sources
            .iter()
            .filter(|s| s.stands_alone && s.kind == wanted)
        {
            by_id
                .entry(source.filament_id.clone())
                .or_default()
                .push(source);
        }
        let mut choices: Vec<(String, Vec<(String, String)>)> = Vec::new();
        let mut taken: BTreeSet<String> = BTreeSet::new();
        let mut suffix = 0;
        for presets in by_id.values() {
            let names: BTreeSet<&str> = presets.iter().map(|s| prefix_of(&s.name)).collect();
            for public in names {
                let mut label = public.to_owned();
                if taken.contains(&label) {
                    suffix += 1;
                    label = format!("{public}_{suffix}");
                }
                taken.insert(label.clone());
                let mut pairs: Vec<(String, String)> = presets
                    .iter()
                    .flat_map(|source| {
                        printers_of(source)
                            .into_iter()
                            .map(|printer| (printer, source.name.clone()))
                    })
                    .collect();
                pairs.sort_by(|a, b| natural(&a.0, &b.0));
                choices.push((label, pairs));
            }
        }
        let (snapmaker, others): (Vec<_>, Vec<_>) = choices
            .into_iter()
            .partition(|(label, _)| label.contains("Snapmaker"));
        let mut snapmaker = snapmaker;
        let mut others = others;
        snapmaker.sort_by(|a, b| natural(&a.0, &b.0));
        others.sort_by(|a, b| natural(&a.0, &b.0));
        let base: Vec<Value> = snapmaker
            .into_iter()
            .chain(others)
            .map(|(label, pairs)| {
                json!({
                    "name": label,
                    "printers": pairs.iter().map(|(printer, preset)| json!({ "printer": printer, "preset": preset })).collect::<Vec<_>>(),
                })
            })
            .collect();
        // "Copy Current Filament Preset ".
        let mut for_printer: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for source in sources.iter().filter(|s| s.kind == wanted) {
            for printer in printers_of(source) {
                for_printer
                    .entry(printer)
                    .or_default()
                    .push(source.name.clone());
            }
        }
        let mut bases: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for printer in &printers {
            let (base, nozzle) = printer_base(printer);
            let nozzles = bases.entry(base.clone()).or_default();
            if nozzles.iter().any(|(known, _)| *known == nozzle) {
                bases
                    .entry(printer.clone())
                    .or_default()
                    .push((nozzle, printer.clone()));
            } else {
                nozzles.push((nozzle, printer.clone()));
            }
        }
        let mut table: Vec<(String, Vec<(String, String)>)> = bases.into_iter().collect();
        table.sort_by(|a, b| natural(&a.0, &b.0));
        let table: Vec<Value> = table
            .into_iter()
            .map(|(base, mut nozzles)| {
                nozzles.sort_by(|a, b| natural(&a.0, &b.0));
                json!({
                    "printer": base,
                    "nozzles": nozzles.iter().map(|(nozzle, printer)| {
                        let mut presets = for_printer.get(printer).cloned().unwrap_or_default();
                        presets.sort_by(|a, b| natural(a, b));
                        presets.dedup();
                        json!({ "nozzle": if nozzle.is_empty() { "-" } else { nozzle.as_str() }, "machine": printer, "presets": presets })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        // The first row: what the project is on.
        let (machine, filament) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.choices.machine.clone(),
                project.choices.filaments.first().cloned(),
            )
        };
        Ok(
            json!({ "base": base, "table": table, "current": { "machine": machine, "filament": filament } }),
        )
    }

    /// `create_filament`: "Create" (CreateFilamentPresetDialog's
    /// `create_dialog_buttons`): its checks in its order, then one preset
    /// for each printer and preset chosen (`presets: [{ printer, preset }]`),
    /// each a copy of the preset standing alone, named "<vendor> <type>
    /// <serial> @<printer>", its vendor, type and printer set, all with one
    /// new `filament_id` (`clone_presets_for_filament`). Asked first: a
    /// name already a filament's (`continue_name` answers), a preset of a
    /// name taken (`rewrite`).
    pub(in crate::api) fn create_filament(&self, args: &Value) -> Result<Value, String> {
        let custom = args.get("custom").and_then(Value::as_bool) == Some(true);
        let mut vendor = if custom {
            let typed = text(args, "custom_vendor").unwrap_or_default();
            if typed.is_empty() {
                return Err(tr(
                    "Custom vendor is not input, please input custom vendor.",
                ));
            }
            if typed == "Bambu" || typed == "Generic" {
                return Err(tr(
                    "\"Bambu\" or \"Generic\" cannot be used as a Vendor for custom filaments.",
                ));
            }
            typed.to_owned()
        } else {
            match text(args, "vendor").filter(|vendor| !vendor.is_empty()) {
                Some(vendor) => vendor.to_owned(),
                None => return Err(tr("Vendor is not selected, please reselect vendor.")),
            }
        };
        let kind = text(args, "type")
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| tr("Filament type is not selected, please reselect type."))?
            .to_owned();
        let mut serial = text(args, "serial").unwrap_or_default().to_owned();
        if serial.is_empty() {
            return Err(tr("Filament serial is not entered, please enter serial."));
        }
        vendor = without_special(&vendor);
        serial = without_special(&serial);
        if vendor.is_empty() || serial.is_empty() {
            return Err(tr(
                "There may be escape characters in the vendor or serial input of filament. Please delete and re-enter.",
            ));
        }
        vendor = vendor.trim().to_owned();
        serial = serial.trim().to_owned();
        if vendor.is_empty() || serial.is_empty() {
            return Err(tr(
                "All inputs in the custom vendor or serial are spaces. Please re-enter.",
            ));
        }
        if custom && vendor.chars().all(|c| c.is_ascii_digit()) {
            return Err(tr("The vendor cannot be a number. Please re-enter."));
        }
        let mut chosen: Vec<(String, String)> = Vec::new();
        for pair in args
            .get("presets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(printer), Some(preset)) = (
                pair.get("printer").and_then(Value::as_str),
                pair.get("preset").and_then(Value::as_str),
            ) else {
                continue;
            };
            let pair = (printer.to_owned(), preset.to_owned());
            if !chosen.contains(&pair) {
                chosen.push(pair);
            }
        }
        if chosen.is_empty() {
            return Err(tr(
                "You have not selected a printer or preset yet. Please select at least one.",
            ));
        }
        let name = format!(
            "{vendor} {} {serial}",
            if kind == "PLA-AERO" {
                "PLA Aero"
            } else {
                kind.as_str()
            }
        );
        // `is_alias_exist`: a filament of that name already.
        let (sources, _) = self.filament_sources();
        let users = self.users();
        let exists = sources
            .iter()
            .any(|source| super::short_name(&source.name) == name)
            || users
                .of(Kind::Filament)
                .any(|preset| super::short_name(&preset.name) == name);
        if exists && args.get("continue_name").and_then(Value::as_bool) != Some(true) {
            return Ok(json!({
                "created": false,
                "step": "name",
                "title": tr("Info"),
                "ask": fill(&tr("The Filament name %s you created already exists.\nIf you continue creating, the preset created will be displayed with its full name. Do you want to continue?"), &[&name]),
            }));
        }
        let made_names: Vec<String> = chosen
            .iter()
            .map(|(printer, _)| format!("{name} @{printer}"))
            .collect();
        let taken: Vec<&String> = made_names
            .iter()
            .filter(|made| users.get(Kind::Filament, made).is_some())
            .collect();
        if !taken.is_empty() && args.get("rewrite").and_then(Value::as_bool) != Some(true) {
            let mut message = tr("Some existing presets have failed to be created, as follows:\n");
            for made in &taken {
                message.push_str(made);
                message.push('\n');
            }
            message.push_str(&tr("\nDo you want to rewrite it?"));
            return Ok(
                json!({ "created": false, "step": "rewrite", "title": tr("Info"), "ask": message }),
            );
        }
        let filament_id = self.new_filament_id(&name);
        let vendors: Vec<String> = lock(&self.state)
            .store
            .data
            .models
            .keys()
            .cloned()
            .collect();
        let shelf = self.shelf(vendors.first().ok_or("no printer is added")?)?;
        let mut made = Vec::new();
        for ((printer, preset), made_name) in chosen.iter().zip(&made_names) {
            // The preset as its own vendor has it, else the person's.
            let settings = vendors
                .iter()
                .filter_map(|vendor| self.shelf(vendor).ok())
                .find_map(|shelf| self.resolve_named(&shelf, Kind::Filament, preset).ok())
                .ok_or_else(|| format!("there is no filament preset {preset}"))?;
            let version = users
                .get(Kind::Filament, preset)
                .and_then(|mine| mine.version.clone())
                .unwrap_or_else(|| shelf.preset_version());
            let mut own = whole(&settings, Kind::Filament);
            own.insert(
                "filament_vendor",
                edge_config::Value::Many(vec![vendor.clone()]),
            );
            own.insert(
                "compatible_printers",
                edge_config::Value::Many(vec![printer.clone()]),
            );
            own.insert(
                "filament_type",
                edge_config::Value::Many(vec![kind.clone()]),
            );
            own.insert("filament_id", edge_config::Value::One(filament_id.clone()));
            made.push((made_name.clone(), own, version));
        }
        {
            let mut state = lock(&self.state);
            if state.users.dir.is_none() {
                return Err("there is no data folder to keep presets in".into());
            }
            let presets = Arc::make_mut(&mut state.users.presets);
            for (made_name, own, version) in &made {
                presets
                    .save(
                        Kind::Filament,
                        made_name,
                        ToSave {
                            own: own.clone(),
                            inherits: None,
                            version: version.clone(),
                            base_id: None,
                        },
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(
            json!({ "created": true, "name": name, "filament_id": filament_id, "presets": made_names }),
        )
    }
}

/// The templates of a kind, each by the name its file gives it (as the
/// reference names a vendor's preset, PresetBundle.cpp
/// `load_vendor_configs_from_json` l.5716) and by the name the vendor's
/// index lists it under.
fn template_names(templates: &Vendor, kind: Kind) -> Vec<(String, String)> {
    templates
        .names(kind)
        .filter_map(|listed| {
            let settings = templates.resolve(kind, listed).ok()?;
            let shown = settings.text("name").unwrap_or(listed).to_owned();
            Some((shown, listed.to_owned()))
        })
        .collect()
}

/// A bed file the reference takes: of its kinds, and no larger than its
/// limit.
fn bed_file_ok(path: &Path, kinds: &[&str]) -> Result<(), String> {
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !kinds.contains(&extension.as_str()) {
        return Err(tr("Invalid file format."));
    }
    let size = std::fs::metadata(path)
        .map_err(|_| tr("Exception in obtaining file size, please import again."))?
        .len();
    if size > BED_FILE_MAX_MB * 1024 * 1024 {
        return Err(fill(
            &tr("The file exceeds %d MB, please import again."),
            &[&BED_FILE_MAX_MB],
        ));
    }
    Ok(())
}
