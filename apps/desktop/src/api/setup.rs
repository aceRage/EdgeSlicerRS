//! Setting up printers and filaments, as the reference does it beyond
//! Printer Selection: "Filament Selection" (the guide's filament page,
//! resources/web/guide/22 with WebGuideDialog.cpp `SaveProfile` and
//! `apply_config`), the Filament manager that "Add/Remove filaments"
//! opens (PresetManagerDialog.cpp with libslic3r/PresetManagerModel.cpp),
//! and which filament presets are then shown (Preset.cpp
//! `set_visible_from_appconfig`). Creating printers and filaments is in
//! [`create`].
//!
//! What is enabled is kept as the reference keeps it, the `filaments`
//! section of the app's settings (`store::Data::filaments`): the names of
//! the vendors' filament presets shown. Without the section every one is
//! shown. The person's own presets are always shown.

pub(super) mod create;
mod lists;

use std::collections::{BTreeMap, BTreeSet};

use edge_config::Kind;
use serde_json::{Value, json};

use std::sync::Arc;

use super::{
    Session,
    i18n::{fill, tr},
    lock, needed, printers, text,
};

/// The name a filament is listed by on the guide's page and in the
/// Filament manager: its preset's name up to the "@"
/// (22.js `GetFilamentShortname`; the alias of Preset.cpp).
pub(super) fn short_name(name: &str) -> &str {
    name.split('@').next().unwrap_or(name).trim()
}

/// Where the reference keeps the favourite filaments: a key of its
/// config's `app` section (FavoritePresets.hpp `config_key`), the keys
/// joined as `escape_strings_cstyle` joins a list.
pub(super) const FAVORITES_KEY: &str = "favorite_filaments";

/// The key a favourite filament is kept by (FavoriteFilaments.hpp
/// `key_for`): a vendor's preset's alias (its name up to the "@"), one of
/// the person's by its full name.
pub(super) fn favorite_key(name: &str, system: bool) -> String {
    if system {
        name.split('@').next().unwrap_or(name).trim_end().to_owned()
    } else {
        name.to_owned()
    }
}

/// A list as Config.cpp `escape_strings_cstyle` writes it: joined by ";",
/// an item with a space, a tab, a backslash, a quote or a new line in
/// quotes with its backslashes, quotes and new lines escaped.
pub(super) fn escape_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| {
            let quote = (items.len() == 1 && item.is_empty())
                || item
                    .chars()
                    .any(|c| matches!(c, ' ' | '\t' | '\\' | '"' | '\r' | '\n'));
            if !quote {
                return item.clone();
            }
            let mut out = String::from('"');
            for c in item.chars() {
                match c {
                    '\\' | '"' => {
                        out.push('\\');
                        out.push(c);
                    }
                    '\r' => out.push_str("\\r"),
                    '\n' => out.push_str("\\n"),
                    c => out.push(c),
                }
            }
            out.push('"');
            out
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The reverse of [`escape_list`] (Config.cpp `unescape_strings_cstyle`).
pub(super) fn unescape_list(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut chars = text.chars().peekable();
    if text.is_empty() {
        return items;
    }
    loop {
        let mut item = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some('n') => item.push('\n'),
                        Some('r') => item.push('\r'),
                        Some(other) => item.push(other),
                        None => {}
                    },
                    c => item.push(c),
                }
            }
            // Up to the next ";".
            for c in chars.by_ref() {
                if c == ';' {
                    break;
                }
            }
            items.push(item);
            if chars.peek().is_none() && !text.ends_with(';') {
                break;
            }
        } else {
            let mut ended = true;
            for c in chars.by_ref() {
                if c == ';' {
                    ended = false;
                    break;
                }
                item.push(c);
            }
            items.push(item);
            if ended {
                break;
            }
        }
        if chars.peek().is_none() {
            break;
        }
    }
    items
}

/// The reference's natural order without regard to case, digits by value
/// (`natural_less_nocase`), near enough for its lists: by the text in
/// lower case with each run of digits compared as a number.
pub(super) fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |text: &str| -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in text.to_lowercase().chars() {
            let digit = c.is_ascii_digit();
            match out.last_mut() {
                Some((was, run)) if *was == digit => run.push(c),
                _ => out.push((digit, c.to_string())),
            }
        }
        out
    };
    let (x, y) = (parts(a), parts(b));
    for ((dx, px), (dy, py)) in x.iter().zip(&y) {
        let order = if *dx && *dy {
            let (nx, ny) = (px.trim_start_matches('0'), py.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
        } else {
            px.cmp(py)
        };
        if order != std::cmp::Ordering::Equal {
            return order;
        }
    }
    x.len().cmp(&y.len())
}

/// The renames a name typed makes (from, to), or the name that cannot be
/// had and why.
type RenamePlan = Result<Vec<(String, String)>, (String, String)>;

/// The reference's order of names in the managers (PresetManagerModel.cpp
/// `pm_less_name`): without regard to case, a shorter name first, then
/// exactly.
pub(super) fn less_name(a: &str, b: &str) -> std::cmp::Ordering {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    for (ca, cb) in x.iter().zip(y) {
        let (la, lb) = (ca.to_ascii_lowercase(), cb.to_ascii_lowercase());
        if la != lb {
            return la.cmp(&lb);
        }
    }
    x.len().cmp(&y.len()).then_with(|| a.cmp(b))
}

/// A name and its " @printer" part (PresetManagerModel.cpp
/// `split_preset_suffix`).
pub(super) fn split_suffix(name: &str) -> (String, String) {
    match name.find(" @") {
        Some(at) => (name[..at].to_owned(), name[at..].to_owned()),
        None => (name.to_owned(), String::new()),
    }
}

/// The filament setting the bed temperature of a plate is in
/// (PrintConfig.hpp `get_bed_temp_key`), by the plate's name; the
/// project's plate, or the High Temp Plate when it has none
/// (PresetManagerDialog.cpp `build_model`: `btPEI` for `btDefault`).
pub(super) fn bed_temperature_key(plate: Option<&str>) -> &'static str {
    match plate {
        Some("Supertack Plate") => "supertack_plate_temp",
        Some("Cool Plate") => "cool_plate_temp",
        Some("Textured Cool Plate") => "textured_cool_plate_temp",
        Some("Engineering Plate") => "eng_plate_temp",
        Some("Textured PEI Plate") => "textured_plate_temp",
        Some("Graphic Effect Plate") => "graphic_effect_plate_temp",
        _ => "hot_plate_temp",
    }
}

/// The filament types put first on the guide's page, in this order
/// (22.js `FilamentPriority`); the rest follow as they are met.
const TYPE_PRIORITY: [&str; 5] = ["pla", "abs", "pet", "tpu", "pc"];

/// One filament of the guide's page: the presets of one vendor, type and
/// name, whatever printer each is cut for (22.js `SortUI`).
#[derive(Default)]
struct Group {
    vendor: String,
    kind: String,
    name: String,
    members: Vec<String>,
    /// "[model++nozzle]" for each printer added it is cut for.
    models: BTreeSet<String>,
}

impl Session {
    /// The vendors' filament presets shown in the lists: none while every
    /// one is (no `filaments` section), else those enabled and those an
    /// open project is on (Preset.cpp `load_external_preset` makes the
    /// preset a project names visible for the run, without enabling it).
    pub(super) fn shown_filaments(&self) -> Option<BTreeSet<String>> {
        let state = lock(&self.state);
        let mut shown = state.store.data.filaments.clone()?;
        for project in &state.projects {
            for choices in [&project.choices, &project.baseline] {
                shown.extend(choices.filaments.iter().cloned());
            }
        }
        Some(shown)
    }

    /// The printers a filament can be cut for, as the guide lists them:
    /// each printer preset of a model added, for a nozzle added, with its
    /// "[model++nozzle]" (WebGuideDialog.cpp `LoadProfileFamily`). `wanted`
    /// is the models ticked on the printer page; none: those added.
    fn guide_machines(
        &self,
        wanted: Option<Vec<(String, String, Vec<String>)>>,
    ) -> Vec<(String, String, String, String)> {
        let added: Vec<(String, String, Vec<String>)> = wanted.unwrap_or_else(|| {
            lock(&self.state)
                .store
                .data
                .models
                .iter()
                .flat_map(|(vendor, models)| {
                    models.iter().map(move |(model, nozzles)| {
                        (vendor.clone(), model.clone(), nozzles.clone())
                    })
                })
                .collect()
        });
        let mut machines = Vec::new();
        for (vendor, model, nozzles) in added {
            let Ok(known) = self.models_of(&vendor) else {
                continue;
            };
            let Some(entry) = known.iter().find(|entry| entry.model == model) else {
                continue;
            };
            for preset in &entry.machines {
                if nozzles.is_empty() || nozzles.contains(&preset.variant) {
                    machines.push((
                        vendor.clone(),
                        preset.name.clone(),
                        model.clone(),
                        preset.variant.clone(),
                    ));
                }
            }
        }
        machines
    }

    /// The groups of the guide's filament page for the printers given.
    fn guide_groups(
        &self,
        machines: &[(String, String, String, String)],
    ) -> Result<Vec<Group>, String> {
        // By preset name first, the first vendor's met kept, as the
        // reference keys its list by name.
        let mut by_name: BTreeMap<String, Group> = BTreeMap::new();
        for (vendor, machine, model, variant) in machines {
            let shelf = self.shelf(vendor)?;
            for name in shelf.filaments_for(machine) {
                let entry = by_name.entry(name.to_owned()).or_insert_with(|| Group {
                    vendor: shelf.filament_vendor(name).unwrap_or_default().to_owned(),
                    kind: shelf.filament_type(name).unwrap_or_default().to_owned(),
                    name: short_name(name).to_owned(),
                    members: vec![name.to_owned()],
                    models: BTreeSet::new(),
                });
                entry.models.insert(format!("[{model}++{variant}]"));
            }
        }
        // Then one entry for each vendor, type and short name.
        let mut groups: BTreeMap<(String, String, String), Group> = BTreeMap::new();
        for (_, group) in by_name {
            let key = (group.vendor.clone(), group.kind.clone(), group.name.clone());
            match groups.get_mut(&key) {
                Some(known) => {
                    known.members.extend(group.members);
                    known.models.extend(group.models);
                }
                None => {
                    groups.insert(key, group);
                }
            }
        }
        let mut groups: Vec<Group> = groups.into_values().collect();
        // Generic first, then the rest (22.js), each by name.
        groups.sort_by(|a, b| {
            let generic = |group: &Group| !group.vendor.eq_ignore_ascii_case("generic");
            (generic(a), &a.name, &a.vendor).cmp(&(generic(b), &b.name, &b.vendor))
        });
        Ok(groups)
    }

    /// `filament_selection`: the guide's filament page for the printers
    /// ticked (`models`, `[{ vendor, model, nozzles? }]`; none: those
    /// added). Each filament with the printers it is for, its type and
    /// vendor, whether it is enabled and whether it is among the models'
    /// usual ones (`default_materials`); the printers, types and vendors
    /// to tick by, in the page's order.
    pub(super) fn filament_selection(&self, args: &Value) -> Result<Value, String> {
        let wanted = args.get("models").and_then(Value::as_array).map(|models| {
            models
                .iter()
                .filter_map(|entry| {
                    let vendor = entry.get("vendor")?.as_str()?.to_owned();
                    let model = entry.get("model")?.as_str()?.to_owned();
                    let nozzles = entry
                        .get("nozzles")
                        .and_then(Value::as_array)
                        .map(|nozzles| {
                            nozzles
                                .iter()
                                .filter_map(|n| n.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some((vendor, model, nozzles))
                })
                .collect::<Vec<_>>()
        });
        let machines = self.guide_machines(wanted);
        let groups = self.guide_groups(&machines)?;
        let enabled = lock(&self.state).store.data.filaments.clone();
        // The usual filaments of the models ticked.
        let mut usual: BTreeSet<String> = BTreeSet::new();
        let mut printers: Vec<(String, BTreeSet<String>)> = Vec::new();
        for (vendor, _, model, variant) in &machines {
            if let Some(known) = self
                .models_of(vendor)
                .ok()
                .and_then(|known| known.iter().find(|entry| entry.model == *model).cloned())
            {
                usual.extend(known.default_materials.iter().cloned());
            }
            match printers.iter_mut().find(|(name, _)| name == model) {
                Some((_, nozzles)) => {
                    nozzles.insert(variant.clone());
                }
                None => printers.push((model.clone(), BTreeSet::from([variant.clone()]))),
            }
        }
        let mut types: Vec<String> = Vec::new();
        let mut vendors: Vec<String> = Vec::new();
        for group in &groups {
            if !types
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&group.kind))
            {
                types.push(group.kind.clone());
            }
            if !vendors
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&group.vendor))
            {
                vendors.push(group.vendor.clone());
            }
        }
        let rank = |kind: &String| {
            TYPE_PRIORITY
                .iter()
                .position(|first| kind.eq_ignore_ascii_case(first))
                .unwrap_or(TYPE_PRIORITY.len())
        };
        types.sort_by_key(rank);
        vendors.sort_by_key(|vendor| !vendor.eq_ignore_ascii_case("generic"));
        let rows: Vec<Value> = groups
            .iter()
            .map(|group| {
                let selected = enabled
                    .as_ref()
                    .is_some_and(|on| group.members.iter().any(|name| on.contains(name)));
                // The models' usual ones: the reference's page compares
                // the short name with the full names listed, so it ticks
                // none of a list of full names (see UI_PARITY section 8,
                // errors); here a member named, or the short name, counts.
                let default = usual.contains(&group.name)
                    || group.members.iter().any(|name| usual.contains(name));
                json!({
                    "key": format!("{}\u{1f}{}\u{1f}{}", group.vendor, group.kind, group.name),
                    "name": group.name,
                    "vendor": group.vendor,
                    "type": group.kind,
                    "members": group.members,
                    "models": group.models,
                    "selected": selected,
                    "default": default,
                })
            })
            .collect();
        Ok(json!({
            "printers": printers
                .iter()
                .map(|(model, nozzles)| json!({ "model": model, "nozzles": nozzles }))
                .collect::<Vec<_>>(),
            "types": types,
            "vendors": vendors,
            "filaments": rows,
        }))
    }

    /// `filament_selection_save`: the filament presets ticked (`names`)
    /// are those enabled, the `filaments` section written anew
    /// (WebGuideDialog.cpp `SaveProfile` l.703). None: "At least one
    /// filament must be selected.", the page's own notice, nothing kept.
    pub(super) fn filament_selection_save(&self, args: &Value) -> Result<Value, String> {
        let names: BTreeSet<String> = args
            .get("names")
            .and_then(Value::as_array)
            .ok_or("names were not given")?
            .iter()
            .filter_map(|name| name.as_str().map(str::to_owned))
            .collect();
        if names.is_empty() {
            return Err("At least one filament must be selected.".to_owned());
        }
        let mut state = lock(&self.state);
        state.store.change(|data| data.filaments = Some(names))?;
        Ok(json!({ "enabled": state.store.data.filaments }))
    }

    /// The rows of the Filament manager (PresetManagerModel.cpp
    /// `collect_preset_manager_items`, `build_preset_manager_model`): one
    /// for each filament of a vendor's bundle, by the bundle and the alias
    /// (the name up to the "@"), its members in order of name and its
    /// values the first member's that has them; then one for each of the
    /// person's. The vendors' by maker and name, the person's by name.
    pub(super) fn manager_rows(&self, args: &Value) -> Result<Vec<Value>, String> {
        let (vendors, enabled, in_use, bed_key) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            let mut in_use: BTreeSet<String> = BTreeSet::new();
            if project.choices.from_presets {
                let count = project.filament_count().max(1);
                for slot in 0..count {
                    let names = &project.choices.filaments;
                    if let Some(name) = names.get(slot).or(names.last()) {
                        in_use.insert(name.clone());
                    }
                }
            }
            (
                state.store.data.models.keys().cloned().collect::<Vec<_>>(),
                state.store.data.filaments.clone(),
                in_use,
                bed_temperature_key(project.choices.bed_type.as_deref()),
            )
        };
        // Every printer preset added, to list a row's printers by.
        let printers: Vec<(String, String)> = self
            .guide_machines(None)
            .into_iter()
            .map(|(vendor, machine, _, _)| (vendor, machine))
            .collect();
        let added: Vec<String> = printers
            .iter()
            .map(|(_, machine)| machine.clone())
            .collect();
        let favs = self.favorites();
        let temperatures = |settings: Option<&edge_config::Settings>| -> [Option<f64>; 4] {
            let first = |key: &str| -> Option<f64> {
                let settings = settings?;
                let text = settings
                    .texts(key)
                    .and_then(|items| items.first().cloned())
                    .or_else(|| settings.text(key).map(str::to_owned))?;
                // "nil" and what is not a number are not known.
                text.trim().parse::<f64>().ok()
            };
            [
                first("nozzle_temperature"),
                first("nozzle_temperature_range_low"),
                first("nozzle_temperature_range_high"),
                first(bed_key),
            ]
        };
        struct Group {
            vendor: String,
            name: String,
            key: String,
            members: Vec<String>,
            kind: String,
            filament_id: String,
            printers: BTreeSet<String>,
            values: [Option<f64>; 4],
            version: String,
        }
        let mut groups: Vec<Group> = Vec::new();
        for vendor in &vendors {
            let Ok(shelf) = self.shelf(vendor) else {
                continue;
            };
            let mut for_printer: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
            for (of, machine) in &printers {
                if of != vendor {
                    continue;
                }
                for name in shelf.filaments_for(machine) {
                    for_printer.entry(name).or_default().insert(machine.clone());
                }
            }
            let mut names = shelf.all_of(Kind::Filament);
            names.sort_by(|a, b| less_name(a, b));
            for name in names {
                let alias = favorite_key(&name, true);
                let key = format!("s|{vendor}|{alias}");
                let settings = shelf.resolve(Kind::Filament, &name).ok();
                let values = temperatures(settings.as_ref());
                let at = match groups.iter().position(|group| group.key == key) {
                    Some(at) => at,
                    None => {
                        groups.push(Group {
                            vendor: String::new(),
                            name: alias,
                            key,
                            members: Vec::new(),
                            kind: String::new(),
                            filament_id: String::new(),
                            printers: BTreeSet::new(),
                            values: [None; 4],
                            version: shelf.preset_version(),
                        });
                        groups.len() - 1
                    }
                };
                let group = &mut groups[at];
                if !group.members.contains(&name) {
                    group.members.push(name.clone());
                }
                if let Some(printers) = for_printer.get(name.as_str()) {
                    group.printers.extend(printers.iter().cloned());
                }
                if group.vendor.is_empty() {
                    group.vendor = shelf.filament_vendor(&name).unwrap_or_default().to_owned();
                }
                if group.kind.is_empty() {
                    group.kind = shelf.filament_type(&name).unwrap_or_default().to_owned();
                }
                if group.filament_id.is_empty() {
                    group.filament_id = settings
                        .as_ref()
                        .and_then(|s| s.text("filament_id"))
                        .unwrap_or_default()
                        .to_owned();
                }
                for (known, value) in group.values.iter_mut().zip(values) {
                    if known.is_none() {
                        *known = value;
                    }
                }
            }
        }
        groups.sort_by(|a, b| {
            less_name(&a.vendor, &b.vendor)
                .then_with(|| less_name(&a.name, &b.name))
                .then_with(|| less_name(&a.key, &b.key))
        });
        let all_of = |printers: &[String]| added.len() > 1 && printers.len() == added.len();
        let mut answer = Vec::new();
        for group in groups {
            let on = enabled
                .as_ref()
                .is_none_or(|on| group.members.iter().any(|name| on.contains(name)));
            let used = group.members.iter().any(|name| in_use.contains(name));
            let mut printers: Vec<String> = group.printers.into_iter().collect();
            printers.sort_by(|a, b| less_name(a, b));
            let [nozzle_t, nozzle_lo, nozzle_hi, bed_t] = group.values;
            answer.push(json!({
                "fav": group.name,
                "star": favs.contains(&group.name),
                "key": group.key,
                "name": group.name,
                "members": group.members,
                "vendor": group.vendor,
                "type": group.kind,
                "filament_id": group.filament_id,
                "source": "System",
                "version": group.version,
                "all": all_of(&printers),
                "printers": printers,
                "nozzle_t": nozzle_t,
                "nozzle_lo": nozzle_lo,
                "nozzle_hi": nozzle_hi,
                "bed_t": bed_t,
                "mtime": null,
                "enabled": on,
                "in_use": used,
                // A preset in use cannot be unticked (PresetManagerDialog.cpp
                // `apply_enabled`).
                "lock": if used && on { "in_use" } else { "" },
                "can_rename": false,
                "can_delete": false,
            }));
        }
        let users = self.users();
        let shelf = vendors.first().and_then(|v| self.shelf(v).ok());
        let mut mine: Vec<Value> = Vec::new();
        for preset in users.of(Kind::Filament).filter(|preset| preset.visible) {
            let settings = shelf
                .as_ref()
                .and_then(|shelf| self.resolve_named(shelf, Kind::Filament, &preset.name).ok());
            let first = |key: &str| {
                settings
                    .as_ref()
                    .and_then(|s| s.texts(key))
                    .and_then(|items| items.first().cloned())
                    .unwrap_or_default()
            };
            let mut fits: Vec<String> = match &shelf {
                Some(shelf) => added
                    .iter()
                    .filter(|machine| self.fits(shelf, Kind::Filament, &preset.name, machine))
                    .cloned()
                    .collect(),
                None => Vec::new(),
            };
            fits.sort_by(|a, b| less_name(a, b));
            let [nozzle_t, nozzle_lo, nozzle_hi, bed_t] = temperatures(settings.as_ref());
            let mtime = std::fs::metadata(&preset.file)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs());
            let (base, suffix) = split_suffix(&preset.name);
            mine.push(json!({
                "fav": preset.name,
                "star": favs.contains(&preset.name),
                "key": format!("u|{}", preset.name),
                "name": preset.name,
                "members": [preset.name],
                "vendor": first("filament_vendor"),
                "type": first("filament_type"),
                "filament_id": settings.as_ref().and_then(|s| s.text("filament_id")).unwrap_or_default(),
                "source": "User",
                "version": "",
                "all": all_of(&fits),
                "printers": fits,
                "nozzle_t": nozzle_t,
                "nozzle_lo": nozzle_lo,
                "nozzle_hi": nozzle_hi,
                "bed_t": bed_t,
                "mtime": mtime,
                "enabled": true,
                "in_use": in_use.contains(&preset.name),
                // User presets are always shown.
                "lock": "user",
                "can_rename": true,
                "can_delete": true,
                "base": base,
                "suffix": suffix,
            }));
        }
        // A user filament's family: those of its id and its name before " @".
        let family_of = |row: &Value| -> Vec<String> {
            let mut family: Vec<String> = mine
                .iter()
                .filter(|other| {
                    !row["filament_id"].as_str().unwrap_or_default().is_empty()
                        && other["filament_id"] == row["filament_id"]
                        && other["base"] == row["base"]
                })
                .filter_map(|other| other["name"].as_str().map(str::to_owned))
                .collect();
            if family.is_empty() {
                family.push(row["name"].as_str().unwrap_or_default().to_owned());
            }
            family.sort_by(|a, b| less_name(a, b));
            family
        };
        let families: Vec<Vec<String>> = mine.iter().map(family_of).collect();
        for (row, family) in mine.iter_mut().zip(families) {
            row["family"] = json!(family);
        }
        mine.sort_by(|a, b| {
            less_name(
                a["name"].as_str().unwrap_or_default(),
                b["name"].as_str().unwrap_or_default(),
            )
        });
        answer.extend(mine);
        Ok(answer)
    }

    /// The favourite filaments' keys, as kept (FavoriteFilamentsUI.cpp
    /// `fav_read_config`: an empty key is passed over).
    pub(super) fn favorites(&self) -> BTreeSet<String> {
        lock(&self.state)
            .store
            .data
            .prefs
            .get(FAVORITES_KEY)
            .map(|kept| unescape_list(kept))
            .unwrap_or_default()
            .into_iter()
            .filter(|key| !key.is_empty())
            .collect()
    }

    fn keep_favorites(&self, keys: &BTreeSet<String>) {
        let text = escape_list(&keys.iter().cloned().collect::<Vec<_>>());
        let mut state = lock(&self.state);
        printers::keep(&mut state, |data| {
            data.prefs.insert(FAVORITES_KEY.to_owned(), text);
        });
    }

    /// `favorite_toggle`: a filament made a favourite, or no longer one
    /// (`FavoriteFilamentsUI::toggle`), by its key; kept at once.
    pub(super) fn favorite_toggle(&self, args: &Value) -> Result<Value, String> {
        let key = needed(args, "key")?.to_owned();
        let mut keys = self.favorites();
        let on = !keys.remove(&key);
        if on {
            keys.insert(key);
        }
        self.keep_favorites(&keys);
        Ok(json!({ "on": on, "favs": keys }))
    }

    /// A preset of one's own renamed or deleted: its star follows it, or
    /// goes (`rename_key`, `remove_key`).
    pub(super) fn favorite_renamed(&self, old: &str, new: Option<&str>) {
        let mut keys = self.favorites();
        if !keys.remove(old) {
            return;
        }
        if let Some(new) = new {
            keys.insert(new.to_owned());
        }
        self.keep_favorites(&keys);
    }

    /// The favourites among the filaments offered, in order of name: the
    /// "Favorite filaments" group at the head of a filament list
    /// (PresetComboBoxes.cpp `append_favorite_group`).
    pub(super) fn favorite_filaments(&self, system: &[String], mine: &[String]) -> Vec<String> {
        let keys = self.favorites();
        let mut names: Vec<String> = system
            .iter()
            .filter(|name| keys.contains(&favorite_key(name, true)))
            .chain(mine.iter().filter(|name| keys.contains(*name)))
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The rows the Filament manager shows, in its order: the search (every
    /// word in the name, vendor, type, printers or members), the filters
    /// of its columns, the "Printer" list, then the sort by a column
    /// (data_table.js `passes`, `compare`; preset_manager.js `cols`).
    /// `filters` is by column: `{ exclude: [values] }` for a list of values
    /// (a row stays while one of its values is not excluded), a text for
    /// "Name" (every word in it), `{ min, max }` for a temperature (a row
    /// without the value goes while a limit is set); `sort` is `{ key, dir }`
    /// (1 up, -1 down), unknown values last either way, equals by name,
    /// then as given; no sort: as given.
    fn manager_view(rows: &[Value], args: &Value, current: Option<&str>) -> (Vec<String>, Value) {
        let set_keys = |column: &str, row: &Value| -> Vec<String> {
            let text = |key: &str| row[key].as_str().unwrap_or_default().to_owned();
            let keys = match column {
                "checked" => vec![if row["enabled"] == true { "yes" } else { "no" }.to_owned()],
                "star" => vec![if row["star"] == true { "yes" } else { "no" }.to_owned()],
                "printers" => row["printers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect(),
                key => vec![text(key)],
            };
            if keys.is_empty() {
                vec![String::new()]
            } else {
                keys
            }
        };
        const SETS: [&str; 6] = ["checked", "star", "vendor", "type", "source", "printers"];
        let filters = args.get("filters").cloned().unwrap_or(Value::Null);
        let words = |text: &str| -> Vec<String> {
            text.to_lowercase()
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        };
        let search = words(text(args, "search").unwrap_or_default());
        let passes = |row: &Value| -> bool {
            let hay = {
                let list = |key: &str| -> Vec<String> {
                    row[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                };
                [
                    row["name"].as_str().unwrap_or_default().to_owned(),
                    row["vendor"].as_str().unwrap_or_default().to_owned(),
                    row["type"].as_str().unwrap_or_default().to_owned(),
                    list("printers").join("\u{0}"),
                    list("members").join("\u{0}"),
                ]
                .join("\u{0}")
                .to_lowercase()
            };
            if !search.iter().all(|word| hay.contains(word.as_str())) {
                return false;
            }
            for column in SETS {
                let Some(excluded) = filters[column]["exclude"].as_array() else {
                    continue;
                };
                let excluded: Vec<&str> = excluded.iter().filter_map(Value::as_str).collect();
                if !excluded.is_empty()
                    && set_keys(column, row)
                        .iter()
                        .all(|key| excluded.contains(&key.as_str()))
                {
                    return false;
                }
            }
            if let Some(text) = filters["name"].as_str() {
                let name = row["name"].as_str().unwrap_or_default().to_lowercase();
                if !words(text).iter().all(|word| name.contains(word.as_str())) {
                    return false;
                }
            }
            for column in ["nozzle_t", "bed_t"] {
                let (min, max) = (
                    filters[column]["min"].as_f64(),
                    filters[column]["max"].as_f64(),
                );
                if min.is_none() && max.is_none() {
                    continue;
                }
                let Some(value) = row[column].as_f64() else {
                    return false;
                };
                if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
                    return false;
                }
            }
            true
        };
        let mut view: Vec<(usize, &Value)> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| passes(row))
            .collect();
        let key = args["sort"]["key"].as_str().unwrap_or_default();
        let dir = if args["sort"]["dir"].as_i64() == Some(-1) {
            -1
        } else {
            1
        };
        enum Sort {
            Text(String),
            Number(f64),
        }
        let value_of = |row: &Value| -> Option<Sort> {
            let text = |value: &Value| {
                value
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(|text| Sort::Text(text.to_owned()))
            };
            match key {
                "checked" => Some(Sort::Number(if row["enabled"] == true { 0.0 } else { 1.0 })),
                "star" => Some(Sort::Number(if row["star"] == true { 0.0 } else { 1.0 })),
                "printers" => {
                    if row["all"] == true {
                        Some(Sort::Text(String::new()))
                    } else {
                        text(&row["printers"][0])
                    }
                }
                "nozzle_t" | "bed_t" | "mtime" => row[key].as_f64().map(Sort::Number),
                "name" | "vendor" | "type" | "source" => text(&row[key]),
                _ => None,
            }
        };
        if !key.is_empty() {
            view.sort_by(|(at_a, a), (at_b, b)| {
                use std::cmp::Ordering;
                match (value_of(a), value_of(b)) {
                    (None, None) => at_a.cmp(at_b),
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (Some(x), Some(y)) => {
                        let order = match (x, y) {
                            (Sort::Number(x), Sort::Number(y)) => x.total_cmp(&y),
                            (Sort::Text(x), Sort::Text(y)) => natural(&x, &y),
                            (Sort::Number(_), Sort::Text(_)) => Ordering::Less,
                            (Sort::Text(_), Sort::Number(_)) => Ordering::Greater,
                        };
                        let order = if dir < 0 { order.reverse() } else { order };
                        order
                            .then_with(|| {
                                if key == "name" {
                                    Ordering::Equal
                                } else {
                                    natural(
                                        a["name"].as_str().unwrap_or_default(),
                                        b["name"].as_str().unwrap_or_default(),
                                    )
                                }
                            })
                            .then_with(|| at_a.cmp(at_b))
                    }
                }
            });
        }
        // Each list filter's values, in their order (`sortKeys`, `setOrder`).
        let mut universe = serde_json::Map::new();
        for column in SETS {
            let mut values: Vec<String> = Vec::new();
            for row in rows {
                for value in set_keys(column, row) {
                    if !values.contains(&value) {
                        values.push(value);
                    }
                }
            }
            values.sort_by(|a, b| match column {
                "checked" | "star" => (a != "yes").cmp(&(b != "yes")),
                "printers" => {
                    let rank = |v: &str| {
                        if Some(v) == current {
                            0
                        } else if v.is_empty() {
                            2
                        } else {
                            1
                        }
                    };
                    rank(a).cmp(&rank(b)).then_with(|| natural(a, b))
                }
                _ => natural(a, b),
            });
            universe.insert(column.to_owned(), json!(values));
        }
        (
            view.into_iter()
                .map(|(_, row)| row["key"].as_str().unwrap_or_default().to_owned())
                .collect(),
            Value::Object(universe),
        )
    }

    /// A row of one's own of the Filament manager by its key, and the
    /// shelf to check names against (the project's vendor's, else the
    /// first added).
    fn user_row(
        &self,
        args: &Value,
    ) -> Result<(Value, Arc<edge_engine::Shelf>, Option<String>), String> {
        let key = needed(args, "key")?;
        let rows = self.manager_rows(args)?;
        let row = rows
            .into_iter()
            .find(|row| row["key"] == key)
            .ok_or_else(|| format!("the Filament manager has no row {key}"))?;
        let (vendor, machine) = {
            let state = lock(&self.state);
            let choices = &state.projects[state.target(args)?].choices;
            let vendor = choices
                .vendor
                .clone()
                .or_else(|| state.store.data.models.keys().next().cloned())
                .ok_or("no printer is added")?;
            (vendor, choices.machine.clone())
        };
        Ok((row, self.shelf(&vendor)?, machine))
    }

    /// The renames a name typed for a row makes (PresetManagerModel.cpp
    /// `plan_preset_rename`): a name without " @" whole, by the save
    /// dialog's rules; a filament's " @printer" variant by the part before
    /// it, the printer part kept, its family (`scope` "family", the default:
    /// the person's presets of its id and that part) renamed with it. The
    /// first name that cannot be had stops it, with why.
    fn plan_rename(&self, args: &Value) -> Result<RenamePlan, String> {
        let (row, shelf, machine) = self.user_row(args)?;
        if row["source"] != "User" {
            return Err(tr("System presets cannot be renamed."));
        }
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let typed = text(args, "name").unwrap_or_default().to_owned();
        let (_, suffix) = split_suffix(&name);
        let book = super::settings::Book::Filament;
        let check = |to: &str, from: &str| -> Option<String> {
            let (level, message) =
                self.check_name(&shelf, book, to, Some(from), true, machine.as_deref());
            (level == super::presets::Level::Invalid).then(|| message.unwrap_or_default())
        };
        if suffix.is_empty() {
            if let Some(why) = check(&typed, &name) {
                return Ok(Err((typed, why)));
            }
            return Ok(Ok(if typed == name {
                Vec::new()
            } else {
                vec![(name, typed)]
            }));
        }
        let invalid = |why: String| Ok(Err((format!("{typed}{suffix}"), why)));
        if typed.is_empty() {
            return invalid(tr("The name is not allowed to be empty."));
        }
        if typed.contains('@') {
            return invalid(tr(
                "The printer part of the name (after \" @\") is kept; type the name without \"@\".",
            ));
        }
        if typed.starts_with(' ') {
            return invalid(tr("The name is not allowed to start with space character."));
        }
        if typed.ends_with(' ') {
            return invalid(tr("The name is not allowed to end with space character."));
        }
        let mut members = vec![name.clone()];
        if text(args, "scope") != Some("one") {
            for other in row["family"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                if other != name {
                    members.push(other.to_owned());
                }
            }
        }
        let mut renames = Vec::new();
        for member in members {
            let (_, member_suffix) = split_suffix(&member);
            let to = format!("{typed}{member_suffix}");
            if to == member {
                continue;
            }
            if let Some(why) = check(&to, &member) {
                return Ok(Err((to, why)));
            }
            renames.push((member, to));
        }
        Ok(Ok(renames))
    }

    /// `filament_manager_check_name`: what the rename field says of a name
    /// as it is typed (`check_name`): `{ level, message, count }`.
    pub(super) fn filament_manager_check_name(&self, args: &Value) -> Result<Value, String> {
        Ok(match self.plan_rename(args) {
            Err(why) => json!({ "level": "invalid", "message": why, "count": 0 }),
            Ok(Err((_, why))) => json!({ "level": "invalid", "message": why, "count": 0 }),
            Ok(Ok(renames)) => json!({ "level": "ok", "message": "", "count": renames.len() }),
        })
    }

    /// `filament_manager_rename`: the renames made (PresetManagerDialog.cpp
    /// `rename`), each as "Rename" in a list's cog menu makes it, then the
    /// table again: `{ ok, message, new_key, ... }`.
    pub(super) fn filament_manager_rename(&self, args: &Value) -> Result<Value, String> {
        let key = needed(args, "key")?.to_owned();
        let renames = match self.plan_rename(args)? {
            Err((_, why)) => return Ok(json!({ "ok": false, "message": why })),
            Ok(renames) => renames,
        };
        let mut new_key = key.clone();
        let row_name = key.strip_prefix("u|").unwrap_or_default().to_owned();
        let mut error = None;
        for (from, to) in &renames {
            if let Err(why) = self.apply_rename(super::settings::Book::Filament, from, to) {
                error = Some(fill(&tr("Could not rename \"%s\": %s"), &[from, &why]));
                break;
            }
            if *from == row_name {
                new_key = format!("u|{to}");
            }
        }
        let mut answer = self.filament_manager(args)?;
        answer["ok"] = json!(error.is_none());
        answer["message"] = json!(error.unwrap_or_default());
        answer["new_key"] = json!(new_key);
        Ok(answer)
    }

    /// `filament_manager_delete`: a row of one's own deleted
    /// (PresetManagerDialog.cpp `delete_row`). Refused for a vendor's
    /// ("System presets cannot be deleted.") and for one others are laid
    /// over (the reference's message, "Delete Preset"); without `confirm`
    /// the question: "Delete the preset \"%s\"?", what uses it and where
    /// it goes, the note for a filament that stands alone; with it, gone as
    /// the cog menu's "Delete this preset" deletes it (slots to the next,
    /// the star away), and the table again.
    pub(super) fn filament_manager_delete(&self, args: &Value) -> Result<Value, String> {
        let (row, _, _) = self.user_row(args)?;
        if row["source"] != "User" {
            return Err(tr("System presets cannot be deleted."));
        }
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let users = self.users();
        let preset = users
            .get(Kind::Filament, &name)
            .cloned()
            .ok_or_else(|| tr("This preset cannot be deleted."))?;
        let children = users.children(Kind::Filament, &name);
        if !children.is_empty() {
            let mut message = format!(
                "{}\n{}",
                tr("Presets inherited by other presets cannot be deleted!"),
                super::i18n::trn(
                    "The following presets inherit this preset.",
                    "The following preset inherits this preset.",
                    // The count picks the sentence that fits (see presets.rs).
                    if children.len() == 1 { 2 } else { 1 },
                )
            );
            for child in &children {
                message.push_str("\n - ");
                message.push_str(child);
            }
            return Ok(
                json!({ "deleted": false, "refused": true, "title": tr("Delete Preset"), "message": message }),
            );
        }
        if args.get("confirm").and_then(Value::as_bool) != Some(true) {
            let mut message = fill(&tr("Delete the preset \"%s\"?"), &[&name]);
            let used = self.used_by(super::settings::Book::Filament, &name, args)?;
            if !used.is_empty() {
                message.push_str("\n\n");
                message.push_str(&tr("Used by:"));
                for line in used {
                    message.push_str("\n - ");
                    message.push_str(&line);
                }
                message.push_str("\n\n");
                message.push_str(&tr("They switch to the next visible filament preset."));
            }
            if preset.inherits.is_none() {
                message.push_str("\n\n");
                message.push_str(&tr("If the preset corresponds to a filament currently in use on your printer, please reset the filament information for that slot."));
            }
            return Ok(json!({ "deleted": false, "title": tr("Delete Preset"), "ask": message }));
        }
        let mut one = json!({ "kind": "filament", "preset": name, "confirm": true });
        if let Some(project) = args.get("project") {
            one["project"] = project.clone();
        }
        self.preset_delete(&one)?;
        let mut answer = self.filament_manager(args)?;
        answer["deleted"] = json!(true);
        Ok(answer)
    }

    /// `filament_manager`: the Filament manager's table.
    pub(super) fn filament_manager(&self, args: &Value) -> Result<Value, String> {
        let rows = self.manager_rows(args)?;
        let current = {
            let state = lock(&self.state);
            state.projects[state.target(args)?].choices.machine.clone()
        };
        let (view, universe) = Self::manager_view(&rows, args, current.as_deref());
        Ok(json!({ "rows": rows, "view": view, "values": universe, "current_printer": current }))
    }

    /// `filament_enable`: rows of the Filament manager ticked or unticked
    /// (`changes: [{ key, on }]`), kept at once as the reference keeps
    /// them (PresetManagerDialog.cpp `apply_enabled`,
    /// `filament_section_with`): every preset of a row enabled, or every
    /// one taken out. A row of one's own presets, and a row in use being
    /// unticked, are passed over. Answered with the rows as they then are.
    ///
    /// Without a `filaments` section (every filament shown) the reference
    /// takes nothing out of the empty section it starts from, so unticking
    /// does nothing, and ticking one row makes it the only one shown. Here
    /// the section starts from every filament of the vendors added, as
    /// the table shows them all ticked.
    pub(super) fn filament_enable(&self, args: &Value) -> Result<Value, String> {
        let changes = args
            .get("changes")
            .and_then(Value::as_array)
            .ok_or("changes were not given")?;
        let rows = self.manager_rows(args)?;
        let mut section: BTreeSet<String> = match lock(&self.state).store.data.filaments.clone() {
            Some(section) => section,
            None => rows
                .iter()
                .filter(|row| row["source"] == "System")
                .flat_map(|row| row["members"].as_array().cloned().unwrap_or_default())
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
        };
        let before = section.clone();
        for change in changes {
            let (Some(key), on) = (
                change.get("key").and_then(Value::as_str),
                change.get("on").and_then(Value::as_bool).unwrap_or(true),
            ) else {
                continue;
            };
            let Some(row) = rows.iter().find(|row| row["key"] == key) else {
                continue;
            };
            if row["lock"] == "user" || (row["lock"] == "in_use" && !on) {
                continue;
            }
            for name in row["members"].as_array().into_iter().flatten() {
                let Some(name) = name.as_str() else {
                    continue;
                };
                if on {
                    section.insert(name.to_owned());
                } else {
                    section.remove(name);
                }
            }
        }
        if section != before || lock(&self.state).store.data.filaments.is_none() {
            let mut state = lock(&self.state);
            printers::keep(&mut state, |data| data.filaments = Some(section));
        }
        self.filament_manager(args)
    }
}

#[cfg(test)]
mod tests;
