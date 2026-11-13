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

use super::{Session, lock, needed, printers};

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

    /// `filament_manager`: the Filament manager's table.
    pub(super) fn filament_manager(&self, args: &Value) -> Result<Value, String> {
        Ok(json!({ "rows": self.manager_rows(args)? }))
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
