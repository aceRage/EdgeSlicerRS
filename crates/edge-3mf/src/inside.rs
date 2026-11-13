//! Presets kept inside a project ("Preset Inside Project" in the
//! reference's save dialog, listed under "Project-inside presets"): each
//! an entry of the package, `Metadata/process_settings_<n>.config`,
//! `filament_settings_<n>.config` or `machine_settings_<n>.config`, the
//! preset's JSON as the reference writes a preset, with `from` "project"
//! (bbs_3mf.cpp `_add_project_embedded_presets_to_archive` l.9272,
//! `_extract_project_embedded_presets_from_archive` l.3619; Config.cpp
//! `save_to_json` l.1991).
//!
//! What a preset holds is what the reference writes of it
//! (`get_preset_differed_for_save`): the settings that differ from the
//! preset it inherits, with its name, `inherits` and the kind's id; one
//! that inherits nothing, every setting.

use std::collections::BTreeMap;

use edge_config::{Kind, Settings, Value, user};

use crate::kept::{Carry, Kept};

/// One preset kept inside a project.
#[derive(Clone, Debug, PartialEq)]
pub struct InsidePreset {
    pub kind: Kind,
    pub name: String,
    /// The preset it is laid over; none for one that stands alone.
    pub inherits: Option<String>,
    /// The version it was written with, as written.
    pub version: Option<String>,
    /// What it sets, without its name, `from`, `version` and `inherits`
    /// (the kind's id is kept as it was read and written back named so).
    pub own: Settings,
}

/// An entry of a project the reference would read as a preset inside it
/// but this one could not, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsideUnread {
    pub entry: String,
    pub reason: String,
}

/// The entries' names, by the reference's prefixes (bbs_3mf.cpp
/// l.203–206), and the kind each holds.
const PREFIXES: [(&str, Kind); 3] = [
    ("metadata/process_settings_", Kind::Process),
    ("metadata/filament_settings_", Kind::Filament),
    ("metadata/machine_settings_", Kind::Machine),
];

/// The old INI form of a process preset (`print_setting_`), which the
/// reference still reads (`use_json` false).
const OLD_PROCESS: &str = "metadata/print_setting_";

impl InsidePreset {
    /// The preset's JSON, as `save_to_json(file, name, "project", version)`
    /// writes it: the header and every value, the keys in order.
    pub fn text(&self) -> String {
        let mut values: BTreeMap<String, Value> = self
            .own
            .iter()
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        values.insert("name".into(), Value::One(self.name.clone()));
        values.insert("from".into(), Value::One("project".into()));
        if let Some(version) = &self.version {
            values.insert("version".into(), Value::One(version.clone()));
        }
        values.insert(
            "inherits".into(),
            Value::One(self.inherits.clone().unwrap_or_default()),
        );
        let id = match self.kind {
            Kind::Filament => Value::Many(vec![self.name.clone()]),
            _ => Value::One(self.name.clone()),
        };
        values.insert(user::id_key(self.kind).into(), id);
        user::to_json(&values)
    }

    /// Reads one entry: its kind from its name, its preset from the JSON
    /// (the name from the kind's id, as the reference takes it).
    pub fn read(entry: &str, bytes: &[u8]) -> Result<Option<Self>, String> {
        let lower = entry.to_ascii_lowercase();
        if lower.starts_with(OLD_PROCESS) {
            return Err(
                "a process preset in the old INI form, which this app does not read".into(),
            );
        }
        let Some(kind) = PREFIXES
            .iter()
            .find(|(prefix, _)| lower.starts_with(prefix) && lower.ends_with(".config"))
            .map(|(_, kind)| *kind)
        else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
        let mut own = Settings::from_json(&text)?;
        // The name is the kind's id (`print_settings_id`, the first of
        // `filament_settings_id`, `printer_settings_id`); without it the
        // reference passes the entry over.
        let name = match own.get(user::id_key(kind)) {
            Some(Value::One(name)) => name.clone(),
            Some(Value::Many(names)) => names.first().cloned().unwrap_or_default(),
            None => String::new(),
        };
        if name.is_empty() {
            return Err(format!("it has no {}", user::id_key(kind)));
        }
        let inherits = own
            .text("inherits")
            .filter(|parent| !parent.is_empty())
            .map(str::to_owned);
        let version = own.text("version").map(str::to_owned);
        for key in ["name", "from", "version", "inherits"] {
            own.remove(key);
        }
        Ok(Some(Self {
            kind,
            name,
            inherits,
            version,
            own,
        }))
    }
}

impl Kept {
    /// The presets the file keeps inside it, in the order of its entries,
    /// and those it could not read. A second preset of a kind and name is
    /// passed over, as the reference loads only the first.
    pub fn inside_presets(&self) -> (Vec<InsidePreset>, Vec<InsideUnread>) {
        let mut presets: Vec<InsidePreset> = Vec::new();
        let mut unread = Vec::new();
        for file in self.files.iter().filter(|file| file.carry == Carry::Preset) {
            match InsidePreset::read(&file.name, &file.bytes) {
                Ok(Some(preset)) => {
                    if presets
                        .iter()
                        .any(|other| other.kind == preset.kind && other.name == preset.name)
                    {
                        unread.push(InsideUnread {
                            entry: file.name.clone(),
                            reason: format!(
                                "another entry holds a preset named \"{}\"",
                                preset.name
                            ),
                        });
                    } else {
                        presets.push(preset);
                    }
                }
                Ok(None) => {}
                Err(reason) => unread.push(InsideUnread {
                    entry: file.name.clone(),
                    reason,
                }),
            }
        }
        (presets, unread)
    }
}

/// The entries a project's presets are written as: the process presets,
/// then the filaments, then the printers (`get_current_project_embedded_presets`
/// takes the collections in that order), each kind in order of name as
/// the reference keeps a collection, numbered from 1.
pub fn inside_entries(presets: &[InsidePreset]) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    for (kind, format) in [
        (Kind::Process, "Metadata/process_settings_"),
        (Kind::Filament, "Metadata/filament_settings_"),
        (Kind::Machine, "Metadata/machine_settings_"),
    ] {
        let mut of_kind: Vec<&InsidePreset> = presets.iter().filter(|p| p.kind == kind).collect();
        of_kind.sort_by(|a, b| a.name.cmp(&b.name));
        for (at, preset) in of_kind.into_iter().enumerate() {
            entries.push((
                format!("{format}{}.config", at + 1),
                preset.text().into_bytes(),
            ));
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process preset as the reference writes one inside a project: the
    /// keys in order, four spaces, `from` "project", a new line at the end.
    const PROCESS: &str = "{\n    \"from\": \"project\",\n    \"inherits\": \"0.20mm Standard @Snapmaker U1 (0.4 nozzle)\",\n    \"name\": \"Project process\",\n    \"print_settings_id\": \"Project process\",\n    \"version\": \"2.3.3.14\",\n    \"wall_loops\": \"5\"\n}\n";

    const FILAMENT: &str = "{\n    \"filament_settings_id\": [\n        \"Project PLA\"\n    ],\n    \"from\": \"project\",\n    \"inherits\": \"Snapmaker PLA @U1\",\n    \"name\": \"Project PLA\",\n    \"nozzle_temperature\": [\n        \"231\"\n    ],\n    \"version\": \"2.3.3.14\"\n}\n";

    #[test]
    fn an_entry_is_read_and_written_back_byte_for_byte() {
        let process = InsidePreset::read("Metadata/process_settings_1.config", PROCESS.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(process.kind, Kind::Process);
        assert_eq!(process.name, "Project process");
        assert_eq!(
            process.inherits.as_deref(),
            Some("0.20mm Standard @Snapmaker U1 (0.4 nozzle)")
        );
        assert_eq!(process.own.text("wall_loops"), Some("5"));
        assert_eq!(process.text(), PROCESS);
        let filament =
            InsidePreset::read("Metadata/filament_settings_1.config", FILAMENT.as_bytes())
                .unwrap()
                .unwrap();
        assert_eq!(filament.kind, Kind::Filament);
        assert_eq!(filament.name, "Project PLA");
        assert_eq!(filament.text(), FILAMENT);
    }

    #[test]
    fn entries_are_named_by_kind_and_numbered_in_order_of_name() {
        let read =
            |entry: &str, text: &str| InsidePreset::read(entry, text.as_bytes()).unwrap().unwrap();
        let mut second = read("Metadata/process_settings_1.config", PROCESS);
        second.name = "A process".into();
        let presets = vec![
            read("Metadata/filament_settings_1.config", FILAMENT),
            read("Metadata/process_settings_1.config", PROCESS),
            second,
        ];
        let names: Vec<String> = inside_entries(&presets)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            names,
            [
                "Metadata/process_settings_1.config",
                "Metadata/process_settings_2.config",
                "Metadata/filament_settings_1.config",
            ]
        );
        // The renamed one is written under its new name, its id with it.
        let (_, first) = &inside_entries(&presets)[0];
        let text = String::from_utf8(first.clone()).unwrap();
        assert!(text.contains("\"name\": \"A process\""));
        assert!(text.contains("\"print_settings_id\": \"A process\""));
    }

    #[test]
    fn what_is_not_a_preset_is_said_and_passed_over() {
        assert!(InsidePreset::read("Metadata/print_setting_1.config", b"wall_loops = 3").is_err());
        assert!(
            InsidePreset::read("Metadata/process_settings_1.config", b"{\"name\": \"x\"}").is_err()
        );
        assert_eq!(
            InsidePreset::read("Metadata/model_settings.config", b"<x/>"),
            Ok(None)
        );
    }
}
