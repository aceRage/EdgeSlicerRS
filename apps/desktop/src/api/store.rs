//! What the app keeps from one run to the next, in a file of its own:
//! the folder of profiles chosen, the printers added, the printer last
//! used, and what was last used with each printer (process, filaments,
//! bed type). It is this app's counterpart of the reference's
//! `EdgeSlicer.conf` (AppConfig.cpp) and is laid out after it where the
//! two keep the same thing; it is never that file, which this app neither
//! reads nor writes.
//!
//! `settings.json` lives in
//! - macOS: `~/Library/Application Support/EdgeSlicerRS/`
//! - Windows: `%APPDATA%\EdgeSlicerRS\`
//! - elsewhere: `$XDG_CONFIG_HOME/EdgeSlicerRS/`, else `~/.config/EdgeSlicerRS/`
//!
//! or in the folder `EDGE_DESKTOP_DATA` names, when it is set.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

/// The folder the settings are kept under.
const FOLDER: &str = "EdgeSlicerRS";
const FILE: &str = "settings.json";

/// What was last used with one printer preset: the reference keeps the
/// same under `orca_presets` (PresetBundle.cpp `export_selections`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remembered {
    /// The vendor whose preset it is; none for a project's own printer.
    pub vendor: Option<String>,
    pub process: Option<String>,
    /// For filament 1, filament 2 and so on.
    pub filaments: Vec<String>,
    /// By name ("Textured PEI Plate"). The reference keeps the number of
    /// its `BedType`; a name survives a change to that list.
    pub bed_type: Option<String>,
}

/// Everything kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub profiles_dir: Option<String>,
    /// The language chosen, by its canonical name ("de_DE"), as the
    /// reference keeps `language` in its `app` section (GUI_App.cpp
    /// `load_language`, Preferences.cpp `create_item_language_combobox`).
    pub language: Option<String>,
    /// The printers added: vendor, then model, then the nozzles of it
    /// that are added (AppConfig.hpp `VendorMap`).
    pub models: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    /// The printer last selected: its vendor and its preset.
    pub last: Option<(String, String)>,
    /// What was last used with each printer preset, by its name.
    pub printers: BTreeMap<String, Remembered>,
    /// The projects last opened or saved, the latest first: the
    /// reference's `recent_projects` (AppConfig.cpp
    /// `set_recent_projects`), kept the same way, by number from "01".
    pub recent_projects: Vec<String>,
    /// The options beside the Arrange button, under the reference's own
    /// keys in its `arrange` section (GLCanvas3D.cpp
    /// `load_arrange_settings`, `_render_arrange_menu`):
    /// `min_object_distance_fff` and `_fff_seq_print`,
    /// `enable_rotation_fff` and `_fff_seq_print`,
    /// `allow_multi_materials_on_same_plate`, `avoid_extrusion_cali_region`.
    pub arrange: BTreeMap<String, String>,
    /// The page's own layout (`layout` in the file, [`super::layout`]):
    /// the sidebar's width as it was dragged, none for its usual width.
    pub sidebar_width: Option<u32>,
    /// The groups of the settings pages folded or unfolded by hand, by
    /// kind ("process", "filament", "printer", "item"), page and group,
    /// each by the reference's message id: true open, false folded. A
    /// group not here is as [`super::layout`] has it at first.
    pub groups: BTreeMap<(String, String, String), bool>,
    /// The Preferences (`prefs.rs`) set, by the reference's keys, kept in
    /// the `app` section beside the language as the reference keeps them.
    pub prefs: BTreeMap<String, String>,
    /// The filament presets enabled ("Filament Selection", the Filament
    /// manager), by name: the reference's `filaments` section
    /// (AppConfig.cpp `SECTION_FILAMENTS`), kept as it keeps it, each
    /// name with "true". None while there is no such section, when every
    /// filament is shown (Preset.cpp `set_visible_from_appconfig` leaves
    /// a preset visible without one).
    pub filaments: Option<BTreeSet<String>>,
    /// What the file held that this build does not know: written back
    /// as it was.
    other: Map<String, Value>,
}

/// The settings and where they are kept.
#[derive(Debug, Default)]
pub struct Store {
    /// None keeps nothing beyond this run.
    file: Option<PathBuf>,
    pub data: Data,
    /// Why the file could not be read, when it was there and could not.
    pub problem: Option<String>,
}

/// Where the settings of this user are kept; none when the system names
/// no such place.
pub fn default_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = var("EDGE_DESKTOP_DATA") {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(FOLDER)
        })
    } else if cfg!(windows) {
        var("APPDATA").map(|data| data.join(FOLDER))
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))
            .map(|config| config.join(FOLDER))
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl Data {
    fn from_json(mut root: Map<String, Value>) -> Self {
        let mut data = Data::default();
        if let Some(app) = root.get("app") {
            data.profiles_dir = text(app, "profiles_dir");
            data.language = text(app, "language");
            for (key, value) in app.as_object().into_iter().flatten() {
                if key == "profiles_dir" || key == "language" {
                    continue;
                }
                if let Some(value) = value.as_str() {
                    data.prefs.insert(key.clone(), value.to_owned());
                }
            }
        }
        // As the reference writes them: one entry for each model, its
        // nozzles in one text, "0.2;0.4".
        for entry in root
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(vendor), Some(model)) = (text(entry, "vendor"), text(entry, "model")) else {
                continue;
            };
            let variants = text(entry, "nozzle_diameter")
                .unwrap_or_default()
                .split(';')
                .map(str::trim)
                .filter(|variant| !variant.is_empty())
                .map(str::to_owned)
                .collect();
            data.models
                .entry(vendor)
                .or_default()
                .insert(model, variants);
        }
        if let Some(presets) = root.get("presets") {
            data.last = text(presets, "vendor").zip(text(presets, "machine"));
        }
        for entry in root
            .get("printers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(machine) = text(entry, "machine") else {
                continue;
            };
            let filaments = entry
                .get("filaments")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .filter_map(|name| name.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            data.printers.insert(
                machine,
                Remembered {
                    vendor: text(entry, "vendor"),
                    process: text(entry, "process"),
                    filaments,
                    bed_type: text(entry, "curr_bed_type"),
                },
            );
        }
        // By number, the latest first. The reference reads them in the
        // order of their keys as text, so "100" comes before "11"; here
        // they are read as numbers.
        if let Some(recent) = root.get("recent_projects").and_then(Value::as_object) {
            let mut numbered: Vec<(u64, String)> = recent
                .iter()
                .filter_map(|(at, path)| {
                    let path = path.as_str().filter(|path| !path.is_empty())?;
                    Some((at.parse().ok()?, path.to_owned()))
                })
                .collect();
            numbered.sort_by_key(|(at, _)| *at);
            data.recent_projects = numbered.into_iter().map(|(_, path)| path).collect();
        }
        if let Some(arrange) = root.get("arrange").and_then(Value::as_object) {
            data.arrange = arrange
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect();
        }
        if let Some(layout) = root.get("layout") {
            data.sidebar_width = layout
                .get("sidebar_width")
                .and_then(Value::as_u64)
                .and_then(|width| u32::try_from(width).ok());
            // { kind: { page: { group: true | false } } }
            for (kind, pages) in layout
                .get("groups")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                for (page, groups) in pages.as_object().into_iter().flatten() {
                    for (group, open) in groups.as_object().into_iter().flatten() {
                        if let Some(open) = open.as_bool() {
                            data.groups
                                .insert((kind.clone(), page.clone(), group.clone()), open);
                        }
                    }
                }
            }
        }
        // { name: "true" }: a name with an empty value is not enabled.
        if let Some(section) = root.get("filaments").and_then(Value::as_object) {
            data.filaments = Some(
                section
                    .iter()
                    .filter(|(_, on)| on.as_str().is_some_and(|on| !on.is_empty()))
                    .map(|(name, _)| name.clone())
                    .collect(),
            );
        }
        for known in [
            "filaments",
            "version",
            "app",
            "models",
            "presets",
            "printers",
            "recent_projects",
            "arrange",
            "layout",
        ] {
            root.remove(known);
        }
        data.other = root;
        data
    }

    fn to_json(&self) -> Value {
        let mut root = self.other.clone();
        root.insert("version".into(), json!(1));
        let mut app = json!({ "profiles_dir": self.profiles_dir });
        if let Some(language) = &self.language {
            app["language"] = json!(language);
        }
        for (key, value) in &self.prefs {
            app[key.as_str()] = json!(value);
        }
        root.insert("app".into(), app);
        let models: Vec<Value> = self
            .models
            .iter()
            .flat_map(|(vendor, models)| {
                models.iter().map(move |(model, variants)| {
                    json!({ "vendor": vendor, "model": model, "nozzle_diameter": variants.join(";") })
                })
            })
            .collect();
        root.insert("models".into(), json!(models));
        root.insert(
            "presets".into(),
            match &self.last {
                Some((vendor, machine)) => json!({ "vendor": vendor, "machine": machine }),
                None => json!({}),
            },
        );
        let printers: Vec<Value> = self
            .printers
            .iter()
            .map(|(machine, kept)| {
                json!({
                    "machine": machine,
                    "vendor": kept.vendor,
                    "process": kept.process,
                    "filaments": kept.filaments,
                    "curr_bed_type": kept.bed_type,
                })
            })
            .collect();
        root.insert("printers".into(), json!(printers));
        let recent: Map<String, Value> = self
            .recent_projects
            .iter()
            .enumerate()
            .map(|(at, path)| (format!("{:02}", at + 1), json!(path)))
            .collect();
        root.insert("recent_projects".into(), Value::Object(recent));
        if !self.arrange.is_empty() {
            root.insert("arrange".into(), json!(self.arrange));
        }
        if let Some(filaments) = &self.filaments {
            let section: Map<String, Value> = filaments
                .iter()
                .map(|name| (name.clone(), json!("true")))
                .collect();
            root.insert("filaments".into(), Value::Object(section));
        }
        let mut layout = Map::new();
        if let Some(width) = self.sidebar_width {
            layout.insert("sidebar_width".into(), json!(width));
        }
        if !self.groups.is_empty() {
            let mut kinds = Map::new();
            for ((kind, page, group), open) in &self.groups {
                let pages = kinds.entry(kind.clone()).or_insert_with(|| json!({}));
                let groups = pages
                    .as_object_mut()
                    .expect("an object")
                    .entry(page.clone())
                    .or_insert_with(|| json!({}));
                groups[group.as_str()] = json!(open);
            }
            layout.insert("groups".into(), Value::Object(kinds));
        }
        if !layout.is_empty() {
            root.insert("layout".into(), Value::Object(layout));
        }
        Value::Object(root)
    }
}

impl Store {
    /// The folder the settings are kept in; none when nothing is kept
    /// beyond the run.
    pub fn dir(&self) -> Option<&Path> {
        self.file.as_deref().and_then(Path::parent)
    }

    /// The settings kept in `dir`, read now; with no folder, settings
    /// that last as long as the run. A file that is not there yet is no
    /// settings; one that cannot be read is said (`problem`), left as it
    /// is, and set aside when the settings are next saved.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let Some(dir) = dir else {
            return Self::default();
        };
        let file = dir.join(FILE);
        let mut store = Self {
            file: Some(file.clone()),
            ..Self::default()
        };
        match std::fs::read_to_string(&file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => store.problem = Some(format!("{}: {error}", file.display())),
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(root)) => store.data = Data::from_json(root),
                Ok(_) => {
                    store.problem = Some(format!("{}: not a settings file", file.display()));
                }
                Err(error) => store.problem = Some(format!("{}: {error}", file.display())),
            },
        }
        store
    }

    /// The file the settings are kept in, when they are kept.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Changes the settings and writes them. If they cannot be written
    /// they stay as they were, and the reason comes back.
    pub fn change(&mut self, change: impl FnOnce(&mut Data)) -> Result<(), String> {
        let mut data = self.data.clone();
        change(&mut data);
        if data == self.data {
            return Ok(());
        }
        self.write(&data)?;
        self.data = data;
        Ok(())
    }

    /// Whole or not at all: written beside the file, then put in its
    /// place.
    fn write(&mut self, data: &Data) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let failed = |path: &Path, error: std::io::Error| {
            format!(
                "the settings could not be saved ({}: {error})",
                path.display()
            )
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|error| failed(dir, error))?;
        }
        // A file that could not be read is not written over: it is kept
        // beside the new one.
        if self.problem.is_some() && file.exists() {
            let aside = file.with_extension("unreadable.json");
            std::fs::rename(file, &aside).map_err(|error| failed(&aside, error))?;
            self.problem = None;
        }
        let text = serde_json::to_string_pretty(&data.to_json()).expect("plain JSON");
        let beside = file.with_extension("json.new");
        std::fs::write(&beside, text + "\n").map_err(|error| failed(&beside, error))?;
        std::fs::rename(&beside, file).map_err(|error| failed(file, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(data: &mut Data) {
        data.profiles_dir = Some("somewhere".into());
        data.models.entry("Snapmaker".into()).or_default().insert(
            "Snapmaker U1".into(),
            vec!["0.2".into(), "0.4".into(), "0.4+0.6".into()],
        );
        data.last = Some(("Snapmaker".into(), "Snapmaker U1 (0.4 nozzle)".into()));
        data.printers.insert(
            "Snapmaker U1 (0.4 nozzle)".into(),
            Remembered {
                vendor: Some("Snapmaker".into()),
                process: Some("0.20mm Standard @Snapmaker U1 (0.4 nozzle)".into()),
                filaments: vec!["Snapmaker PLA @U1".into(), "Snapmaker PETG @U1".into()],
                bed_type: Some("Cool Plate".into()),
            },
        );
        data.arrange
            .insert("min_object_distance_fff".into(), "12.5".into());
        data.arrange
            .insert("enable_rotation_fff".into(), "1".into());
        data.sidebar_width = Some(420);
        data.groups.insert(
            (
                "process".into(),
                "Strength".into(),
                "Top/bottom shells".into(),
            ),
            false,
        );
        data.groups.insert(
            ("printer".into(), "Extruder %d".into(), "Retraction".into()),
            true,
        );
        data.filaments = Some(
            ["Generic PLA @U1 0.4 nozzle", "Snapmaker PLA @U1"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );
    }

    #[test]
    fn settings_are_written_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        // The folder need not exist yet.
        let folder = dir.path().join("kept").join("here");
        let mut store = Store::open(Some(folder.clone()));
        assert_eq!(store.data, Data::default());
        assert_eq!(store.problem, None);
        assert_eq!(store.file(), Some(folder.join("settings.json").as_path()));
        assert!(
            !folder.exists(),
            "nothing is written until something changes"
        );

        store.change(sample).unwrap();
        let again = Store::open(Some(folder.clone()));
        assert_eq!(again.data, store.data);
        assert_eq!(again.problem, None);

        // The printers added are laid out as the reference lays them out.
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(
            written["models"],
            json!([{ "vendor": "Snapmaker", "model": "Snapmaker U1", "nozzle_diameter": "0.2;0.4;0.4+0.6" }])
        );
        assert_eq!(written["presets"]["machine"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(written["printers"][0]["curr_bed_type"], "Cool Plate");
        // The arrange options under the reference's keys.
        assert_eq!(written["arrange"]["min_object_distance_fff"], "12.5");
        // The filaments enabled, as the reference's `filaments` section.
        assert_eq!(
            written["filaments"],
            json!({ "Generic PLA @U1 0.4 nozzle": "true", "Snapmaker PLA @U1": "true" })
        );
        // The page's layout in a section of its own.
        assert_eq!(written["layout"]["sidebar_width"], 420);
        assert_eq!(
            written["layout"]["groups"]["process"]["Strength"]["Top/bottom shells"],
            false
        );
        assert_eq!(
            written["layout"]["groups"]["printer"]["Extruder %d"]["Retraction"],
            true
        );
        // Nothing is left beside it.
        let names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn recent_projects_are_kept_by_number_latest_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(dir.path().to_owned()));
        let paths: Vec<String> = (1..=101).map(|n| format!("p{n}.3mf")).collect();
        store
            .change(|data| data.recent_projects = paths.clone())
            .unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        // As the reference writes them: "01", "02", ...
        assert_eq!(written["recent_projects"]["01"], "p1.3mf");
        assert_eq!(written["recent_projects"]["100"], "p100.3mf");
        // Read back as numbers: "100" does not come between "10" and "11".
        let again = Store::open(Some(dir.path().to_owned()));
        assert_eq!(again.data.recent_projects, paths);
    }

    #[test]
    fn what_another_build_wrote_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{ "version": 1, "later": { "thing": [1, 2] }, "models": [ { "vendor": "A", "model": "B", "nozzle_diameter": "0.4" }, { "model": "no vendor" } ] }"#,
        )
        .unwrap();
        let mut store = Store::open(Some(dir.path().to_owned()));
        assert_eq!(store.data.models["A"]["B"], ["0.4"]);
        assert_eq!(store.data.models.len(), 1);
        store
            .change(|data| data.profiles_dir = Some("x".into()))
            .unwrap();
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["later"], json!({ "thing": [1, 2] }));
        assert_eq!(written["app"]["profiles_dir"], "x");
    }

    #[test]
    fn a_file_that_cannot_be_read_is_said_and_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        std::fs::write(&file, "{ not json").unwrap();
        let mut store = Store::open(Some(dir.path().to_owned()));
        assert!(
            store
                .problem
                .as_deref()
                .is_some_and(|said| said.contains("settings.json"))
        );
        assert_eq!(store.data, Data::default());
        // Until something is saved it is left alone.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ not json");
        store.change(sample).unwrap();
        assert_eq!(store.problem, None);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("settings.unreadable.json")).unwrap(),
            "{ not json"
        );
        assert_eq!(Store::open(Some(dir.path().to_owned())).data, store.data);

        // Not an object at all.
        std::fs::write(&file, "[1]").unwrap();
        assert!(Store::open(Some(dir.path().to_owned())).problem.is_some());
    }

    #[test]
    fn settings_that_cannot_be_saved_stay_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the folder should be: nothing can be written under it.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "a file").unwrap();
        let mut store = Store::open(Some(blocked.join("under")));
        let said = store.change(sample).unwrap_err();
        assert!(
            said.starts_with("the settings could not be saved"),
            "{said}"
        );
        assert_eq!(store.data, Data::default());
    }

    #[test]
    fn without_a_folder_settings_last_the_run() {
        let mut store = Store::open(None);
        assert_eq!(store.file(), None);
        store.change(sample).unwrap();
        assert_eq!(store.data.last.as_ref().unwrap().0, "Snapmaker");
    }
}
