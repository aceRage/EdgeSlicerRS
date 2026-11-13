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

use super::{Session, lock, printers};

/// The name a filament is listed by on the guide's page and in the
/// Filament manager: its preset's name up to the "@"
/// (22.js `GetFilamentShortname`; the alias of Preset.cpp).
pub(super) fn short_name(name: &str) -> &str {
    name.split('@').next().unwrap_or(name).trim()
}

/// A row of the Filament manager being gathered: its presets, its type
/// and the printers it is cut for.
type ManagerRow = (Vec<String>, String, BTreeSet<String>);

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

    /// The rows of the Filament manager: one for each filament of the
    /// vendors added (by vendor and name before the "@", all the printers'
    /// presets of it in one), then one for each of the person's
    /// (PresetManagerModel.cpp `build_preset_manager_model`).
    fn manager_rows(&self, args: &Value) -> Result<Vec<Value>, String> {
        let (vendors, enabled, in_use) = {
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
            )
        };
        // Every printer preset added, to list a row's printers by.
        let printers: Vec<(String, String)> = self
            .guide_machines(None)
            .into_iter()
            .map(|(vendor, machine, _, _)| (vendor, machine))
            .collect();
        let mut rows: BTreeMap<(String, String), ManagerRow> = BTreeMap::new();
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
            for name in shelf.all_of(Kind::Filament) {
                let maker = shelf.filament_vendor(&name).unwrap_or_default().to_owned();
                let key = (maker, short_name(&name).to_owned());
                let row = rows.entry(key).or_insert_with(|| {
                    (
                        Vec::new(),
                        shelf.filament_type(&name).unwrap_or_default().to_owned(),
                        BTreeSet::new(),
                    )
                });
                if let Some(printers) = for_printer.get(name.as_str()) {
                    row.2.extend(printers.iter().cloned());
                }
                if !row.0.contains(&name) {
                    row.0.push(name);
                }
            }
        }
        let mut answer = Vec::new();
        for ((maker, alias), (members, kind, printers)) in rows {
            let on = enabled
                .as_ref()
                .is_none_or(|on| members.iter().any(|name| on.contains(name)));
            let used = members.iter().any(|name| in_use.contains(name));
            answer.push(json!({
                "key": format!("s|{maker}|{alias}"),
                "name": alias,
                "members": members,
                "vendor": maker,
                "type": kind,
                "source": "System",
                "printers": printers,
                "enabled": on,
                "in_use": used,
                // A preset in use cannot be unticked (PresetManagerDialog.cpp
                // `apply_enabled`).
                "lock": if used && on { "in_use" } else { "" },
            }));
        }
        let users = self.users();
        let added: Vec<String> = printers
            .iter()
            .map(|(_, machine)| machine.clone())
            .collect();
        for preset in users.of(Kind::Filament).filter(|preset| preset.visible) {
            let (maker, kind, fits) = match vendors.first().and_then(|v| self.shelf(v).ok()) {
                Some(shelf) => {
                    let settings = self
                        .resolve_named(&shelf, Kind::Filament, &preset.name)
                        .ok();
                    let first = |key: &str| {
                        settings
                            .as_ref()
                            .and_then(|s| s.texts(key))
                            .and_then(|items| items.first().cloned())
                            .unwrap_or_default()
                    };
                    let fits: Vec<String> = added
                        .iter()
                        .filter(|machine| self.fits(&shelf, Kind::Filament, &preset.name, machine))
                        .cloned()
                        .collect();
                    (first("filament_vendor"), first("filament_type"), fits)
                }
                None => (String::new(), String::new(), Vec::new()),
            };
            answer.push(json!({
                "key": format!("u|{}", preset.name),
                "name": preset.name,
                "members": [preset.name],
                "vendor": maker,
                "type": kind,
                "source": "User",
                "printers": fits,
                "enabled": true,
                "in_use": in_use.contains(&preset.name),
                // User presets are always shown.
                "lock": "user",
            }));
        }
        Ok(answer)
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
