//! Preferences (Preferences.cpp `PreferencesDialog`): the settings of the
//! reference's dialog that mean something in this app, by its keys and
//! in its words, kept in the app's own settings (`settings.json`, the
//! `app` section, as the reference keeps them in `EdgeSlicer.conf`'s),
//! with the reference's defaults (AppConfig.cpp `set_defaults`).
//!
//! The dialog is described here once, as the menu bar is (`menu.rs`):
//! its pages, their groups and each setting's label, tooltip, kind and
//! value. The page draws it (`ui/prefs.js`) and does what each one says;
//! the session does what is its (the recent projects, the printer's
//! remembered presets, the backups). Settings of the reference's dialog
//! that belong to what is not built are not shown (UI_PARITY.md,
//! section 18).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Session, i18n::tr, lock};

/// The calls answered here.
pub(super) const JSON: [&str; 7] = [
    "preferences",
    "preference_set",
    "backup_write",
    "backup_list",
    "backup_restore",
    "backup_clear",
    "autosave_due",
];

/// What a setting takes.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// On or off, kept as "true" / "false" (AppConfig `set_bool`).
    Flag,
    /// One of these: the value kept, its label (a message id) and what is
    /// written after the label as it is (" (mm, g)").
    Choice(&'static [(&'static str, &'static str, &'static str)]),
    /// A number from `least` to `most`; `whole` for a whole one; `unit`
    /// a message id written after the field.
    Number {
        least: f64,
        most: f64,
        whole: bool,
        unit: &'static str,
    },
}

/// One setting.
#[derive(Clone, Copy, Debug)]
pub struct Pref {
    pub key: &'static str,
    pub label: &'static str,
    pub tip: &'static str,
    pub kind: Kind,
    pub default: &'static str,
    /// Shown under the setting it goes with, set in by a step ("every").
    pub under: Option<&'static str>,
}

const fn flag(key: &'static str, label: &'static str, tip: &'static str, on: bool) -> Pref {
    Pref {
        key,
        label,
        tip,
        kind: Kind::Flag,
        default: if on { "true" } else { "false" },
        under: None,
    }
}

/// The view types Preview has, by the reference's names for them in
/// `preview_default_view_type` (GCodeViewer.cpp `view_type_config_map`),
/// with the page's own names for them, which are message ids.
pub const VIEW_TYPES: [(&str, &str); 8] = [
    ("feature_type", "Line Type"),
    ("color_print", "Filament"),
    ("speed", "Speed"),
    ("height", "Layer Height"),
    ("width", "Line Width"),
    ("volumetric_flow_rate", "Flow"),
    ("fan_speed", "Fan Speed"),
    ("temperature", "Temperature"),
];

const VIEW_CHOICES: [(&str, &str, &str); 10] = [
    ("auto", "Automatic", ""),
    ("last", "Last used", ""),
    ("feature_type", "Line Type", ""),
    ("color_print", "Filament", ""),
    ("speed", "Speed", ""),
    ("height", "Layer Height", ""),
    ("width", "Line Width", ""),
    ("volumetric_flow_rate", "Flow", ""),
    ("fan_speed", "Fan Speed", ""),
    ("temperature", "Temperature", ""),
];

/// "General": the reference's General Settings, Presets, Project, Dark
/// Mode and Develop mode, as far as they are built.
pub static GENERAL: [(&str, &[Pref]); 5] = [
    (
        "General Settings",
        &[
            Pref {
                key: "use_inches",
                label: "Units",
                tip: "Units",
                kind: Kind::Choice(&[("0", "Metric", " (mm, g)"), ("1", "Imperial", " (in, oz)")]),
                default: "0",
                under: None,
            },
            Pref {
                key: "camera_navigation_style",
                label: "Camera style",
                tip: "Select camera navigation style.\nDefault: LMB+move for rotation, RMB/MMB+move for panning.\nTouchpad: Alt+move for rotation, Shift+move for panning.",
                kind: Kind::Choice(&[("0", "Default", ""), ("1", "Touchpad", "")]),
                default: "0",
                under: None,
            },
            flag(
                "zoom_to_mouse",
                "Zoom to mouse position",
                "Zoom in towards the mouse pointer's position in the 3D view, rather than the 2D window center.",
                false,
            ),
            flag(
                "use_free_camera",
                "Use free camera",
                "If enabled, use free camera. If not enabled, use constrained camera.",
                false,
            ),
            flag(
                "swap_mouse_buttons",
                "Swap pan and rotate mouse buttons",
                "If enabled, swaps the left and right mouse buttons pan and rotate functions.",
                false,
            ),
            flag(
                "reverse_mouse_wheel_zoom",
                "Reverse mouse zoom",
                "If enabled, reverses the direction of zoom with mouse wheel.",
                false,
            ),
            Pref {
                key: "camera_orbit_mult",
                label: "Orbit speed multiplier",
                tip: "Multiplies the orbit speed for finer or coarser camera movement.",
                kind: Kind::Number {
                    least: 0.05,
                    most: 2.0,
                    whole: false,
                    unit: "",
                },
                default: "1.0",
                under: None,
            },
            Pref {
                key: "preview_default_view_type",
                label: "Default preview view type",
                tip: "The color scheme the sliced preview opens with.\nAutomatic: Filament for multi material prints, Line Type for single material ones.\nLast used: the view type you selected last.\nAny other value always opens that view type.\nYou can still switch the view type in the preview afterwards.",
                kind: Kind::Choice(&VIEW_CHOICES),
                default: "auto",
                under: None,
            },
        ],
    ),
    (
        "Presets",
        &[
            flag(
                "remember_printer_config",
                "Remember printer configuration",
                "If enabled, Orca will remember and switch filament/process configuration for each printer automatically.",
                true,
            ),
            flag(
                "remember_print_action",
                "Remember last print action",
                "If enabled, EdgeSlicer will remember the last selected option in the print button's dropdown (for example Print, Print plate, Export plate sliced file, or Export G-code file) and restore it on the next startup and when the printer or preset changes, when the current printer still offers it.",
                false,
            ),
        ],
    ),
    (
        "Project",
        &[
            Pref {
                key: "max_recent_count",
                label: "Maximum recent files",
                tip: "Maximum count of recent files",
                kind: Kind::Number {
                    least: 0.0,
                    most: 10000.0,
                    whole: true,
                    unit: "",
                },
                default: "18",
                under: None,
            },
            flag(
                "recent_models",
                "Add model files (stl/step) to recent file list.",
                "Add model files (stl/step) to recent file list.",
                false,
            ),
            flag(
                "backup_switch",
                "Auto-Backup",
                "Backup your project periodically for restoring from the occasional crash.",
                true,
            ),
            Pref {
                key: "backup_interval",
                label: "every",
                tip: "The period of backup in seconds.",
                kind: Kind::Number {
                    least: 1.0,
                    most: 86_400.0,
                    whole: true,
                    unit: "Second",
                },
                default: "10",
                under: Some("backup_switch"),
            },
        ],
    ),
    (
        "Dark Mode",
        &[Pref {
            key: "dark_color_mode",
            label: "Enable Dark mode",
            tip: "Enable dark mode",
            // The reference's own default is light ("0"); here none is
            // kept until it is set, and the page follows the system's.
            kind: Kind::Flag,
            default: "",
            under: None,
        }],
    ),
    // "Develop mode" (Preferences.cpp l.2337, `developer_mode`): on, the
    // settings' mode is "develop" (`user_mode`), which shows the
    // developer options and the save dialog's "Detach from parent".
    (
        "Develop mode",
        &[Pref {
            key: "developer_mode",
            label: "Develop mode",
            tip: "Develop mode",
            kind: Kind::Flag,
            default: "false",
            under: None,
        }],
    ),
];

/// "Extras": the reference's Project group, as far as it is built.
pub static EXTRAS: [(&str, &[Pref]); 1] = [(
    "Project",
    &[
        flag(
            "autosave_switch",
            "Auto-Save project",
            "Periodically save the project to its own file. Only projects that have already been saved once are auto-saved.",
            false,
        ),
        Pref {
            key: "autosave_interval",
            label: "every",
            tip: "The period of auto-save in seconds.",
            kind: Kind::Number {
                least: 1.0,
                most: 86_400.0,
                whole: true,
                unit: "Second",
            },
            default: "300",
            under: Some("autosave_switch"),
        },
    ],
)];

/// Kept by the page as it goes, not shown in the dialog: the view type
/// last picked (`preview_last_view_type`) and the print button's last
/// action (the reference's `print_select_type`).
const KEPT_QUIETLY: [&str; 2] = ["preview_last_view_type", "print_action_last"];

fn every() -> impl Iterator<Item = &'static Pref> {
    GENERAL
        .iter()
        .chain(EXTRAS.iter())
        .flat_map(|(_, prefs)| prefs.iter())
}

/// The setting kept under `key`, if this app has it.
pub fn find(key: &str) -> Option<&'static Pref> {
    every().find(|pref| pref.key == key)
}

/// `value` as the setting keeps it, or why it cannot be.
pub fn checked(pref: &Pref, value: &Value) -> Result<String, String> {
    let text = match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Bool(on) => on.to_string(),
        Value::Number(number) => number.to_string(),
        _ => return Err(format!("{} cannot be {value}", pref.key)),
    };
    match pref.kind {
        Kind::Flag => match text.as_str() {
            "true" | "1" => Ok("true".into()),
            "false" | "0" => Ok("false".into()),
            _ => Err(format!("{} cannot be {text}", pref.key)),
        },
        Kind::Choice(choices) => choices
            .iter()
            .find(|(kept, ..)| *kept == text)
            .map(|(kept, ..)| (*kept).to_owned())
            .ok_or_else(|| format!("{} cannot be {text}", pref.key)),
        Kind::Number {
            least, most, whole, ..
        } => {
            let number: f64 = text
                .parse()
                .ok()
                .filter(|number: &f64| number.is_finite())
                .ok_or_else(|| tr("Invalid numeric."))?;
            if whole && number.fract() != 0.0 {
                return Err(tr("Invalid numeric."));
            }
            // Put within its range, as the reference's fields put it
            // (MainFrame.cpp `set_max_recent_count`, Preferences.cpp).
            let number = number.clamp(least, most);
            Ok(if whole {
                format!("{}", number as i64)
            } else {
                format!("{number}")
            })
        }
    }
}

/// The settings as they stand: each one's kept value, else its default.
pub fn values(kept: &std::collections::BTreeMap<String, String>) -> Map<String, Value> {
    let mut out = Map::new();
    for pref in every() {
        let value = kept
            .get(pref.key)
            .cloned()
            .unwrap_or_else(|| pref.default.to_owned());
        out.insert(pref.key.into(), json!(value));
    }
    for key in KEPT_QUIETLY {
        if let Some(value) = kept.get(key) {
            out.insert(key.into(), json!(value));
        }
    }
    out
}

/// Whether a flag is on, from what is kept and its default.
pub fn on(kept: &std::collections::BTreeMap<String, String>, key: &str) -> bool {
    let value = kept
        .get(key)
        .map(String::as_str)
        .or_else(|| find(key).map(|pref| pref.default));
    matches!(value, Some("true" | "1"))
}

/// A whole number setting, from what is kept and its default.
pub fn number(kept: &std::collections::BTreeMap<String, String>, key: &str) -> Option<u64> {
    kept.get(key)
        .map(String::as_str)
        .or_else(|| find(key).map(|pref| pref.default))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u64)
}

fn described(groups: &[(&str, &[Pref])]) -> Value {
    json!(
        groups
            .iter()
            .map(|(title, prefs)| json!({
                "title": title,
                "items": prefs
                    .iter()
                    .map(|pref| {
                        let mut item = json!({
                            "key": pref.key,
                            "label": pref.label,
                            "tip": pref.tip,
                            "under": pref.under,
                        });
                        match pref.kind {
                            Kind::Flag => item["kind"] = json!("flag"),
                            Kind::Choice(choices) => {
                                item["kind"] = json!("choice");
                                item["options"] = json!(
                                    choices
                                        .iter()
                                        .map(|(value, label, after)| json!({ "value": value, "label": label, "after": after }))
                                        .collect::<Vec<_>>()
                                );
                            }
                            Kind::Number { least, most, whole, unit } => {
                                item["kind"] = json!("number");
                                item["least"] = json!(least);
                                item["most"] = json!(most);
                                item["whole"] = json!(whole);
                                item["unit"] = json!(unit);
                            }
                        }
                        item
                    })
                    .collect::<Vec<_>>(),
            }))
            .collect::<Vec<_>>()
    )
}

/// Whether a run before this one left backups behind.
pub(super) fn left_behind(store: &super::store::Store) -> bool {
    store
        .dir()
        .is_some_and(|dir| !list(&dir.join("backup")).is_empty())
}

/// A project as it is backed up: its id, name, the project file it was
/// opened from, its model, what was chosen for it, and whether it has
/// unsaved changes.
type Backed = (
    u32,
    String,
    Option<String>,
    Option<std::sync::Arc<super::File>>,
    super::Choices,
    bool,
);

/// A backup of a project: the project file, and beside it what it was.
fn backup_paths(dir: &Path, id: u32) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("{id}.3mf")),
        dir.join(format!("{id}.json")),
    )
}

impl Session {
    /// Where backups are kept: `backup` beside the settings; none when
    /// nothing is kept beyond the run.
    fn backup_dir(&self) -> Option<PathBuf> {
        lock(&self.state).store.dir().map(|dir| dir.join("backup"))
    }

    /// A project saved: its backup, if any, goes.
    pub(super) fn drop_backup(&self, id: u32) {
        if let Some(dir) = self.backup_dir() {
            let (model, about) = backup_paths(&dir, id);
            let _ = std::fs::remove_file(model);
            let _ = std::fs::remove_file(about);
        }
    }

    pub(super) fn prefs_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            // The dialog: its pages, and every setting's value.
            "preferences" => {
                let state = lock(&self.state);
                Ok(json!({
                    "pages": [
                        { "title": "General", "groups": described(&GENERAL) },
                        { "title": "Extras", "groups": described(&EXTRAS) },
                    ],
                    "values": values(&state.store.data.prefs),
                    "view_types": VIEW_TYPES.iter().map(|(key, name)| json!({ "key": key, "name": name })).collect::<Vec<_>>(),
                }))
            }
            // One setting changed: `{ key, value }`; answers every value.
            "preference_set" => {
                let key = super::needed(args, "key")?;
                let value = args.get("value").ok_or("value was not given")?;
                let kept = match find(key) {
                    Some(pref) => checked(pref, value)?,
                    None if KEPT_QUIETLY.contains(&key) => value
                        .as_str()
                        .ok_or_else(|| format!("{key} cannot be {value}"))?
                        .to_owned(),
                    None => return Err(format!("there is no preference {key}")),
                };
                let mut state = lock(&self.state);
                let key = key.to_owned();
                state.store.change(|data| {
                    data.prefs.insert(key.clone(), kept.clone());
                    // Fewer recent files kept: the list is cut at once
                    // (MainFrame.cpp `set_max_recent_count`).
                    if key == "max_recent_count"
                        && let Ok(most) = kept.parse::<usize>()
                    {
                        data.recent_projects.truncate(most);
                    }
                })?;
                Ok(json!({ "values": values(&state.store.data.prefs) }))
            }
            "backup_write" => self.backup_write(),
            "backup_list" => self.backup_list(),
            "backup_restore" => self.backup_restore(),
            "backup_clear" => {
                if let Some(dir) = self.backup_dir() {
                    let _ = std::fs::remove_dir_all(&dir);
                }
                lock(&self.state).backups_left = false;
                Ok(json!({ "cleared": true }))
            }
            "autosave_due" => self.autosave_due(),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// "Auto-Backup" (`backup_switch`, every `backup_interval` seconds):
    /// each open project with unsaved changes is written to the backup
    /// folder as its project file, with what it was named and opened
    /// from; the backup of one that has none is taken away. Answers the
    /// projects backed up.
    fn backup_write(&self) -> Result<Value, String> {
        let Some(dir) = self.backup_dir() else {
            return Ok(json!({ "written": [] }));
        };
        let projects: Vec<Backed> = {
            let state = lock(&self.state);
            if !on(&state.store.data.prefs, "backup_switch") || state.backups_left {
                return Ok(json!({ "written": [] }));
            }
            state
                .projects
                .iter()
                .map(|project| {
                    (
                        project.id,
                        project.name.clone(),
                        project
                            .file
                            .as_ref()
                            .and_then(|file| file.path.as_deref())
                            .filter(|path| {
                                path.extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
                            })
                            .map(super::shown),
                        project.file.clone(),
                        project.choices.clone(),
                        project.dirty(),
                    )
                })
                .collect()
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut written = Vec::new();
        let ids: Vec<u32> = projects.iter().map(|(id, ..)| *id).collect();
        for (id, name, origin, file, choices, dirty) in projects {
            let (model_path, about) = backup_paths(&dir, id);
            let Some(file) = file.filter(|file| dirty && !file.model.objects().is_empty()) else {
                let _ = std::fs::remove_file(&model_path);
                let _ = std::fs::remove_file(&about);
                continue;
            };
            let saving = self.saving(&choices, Some(&file))?;
            let mut model = file.model.clone();
            model.set_inside(choices.inside.clone());
            model
                .save(&model_path, &saving, None)
                .map_err(|error| error.to_string())?;
            let note = json!({ "name": name, "origin": origin });
            super::menu::write_whole(&about, note.to_string().as_bytes())?;
            written.push(id);
        }
        // Projects closed since: their backups go.
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.parse::<u32>().ok());
            if id.is_some_and(|id| !ids.contains(&id)) {
                let _ = std::fs::remove_file(&path);
            }
        }
        Ok(json!({ "written": written }))
    }

    /// The backups left from a run that did not end as it should, for the
    /// question at the start ("Previous unsaved project detected, do you
    /// want to restore it?"): `{ backups: [{ name, origin }] }`.
    fn backup_list(&self) -> Result<Value, String> {
        let Some(dir) = self.backup_dir() else {
            return Ok(json!({ "backups": [] }));
        };
        // This run's own are not offered back to it.
        if !lock(&self.state).backups_left {
            return Ok(json!({ "backups": [] }));
        }
        Ok(json!({ "backups": list(&dir).into_iter().map(|(_, about)| about).collect::<Vec<_>>() }))
    }

    /// Opens every backup as a project of its own, named as it was and
    /// with its unsaved changes; one opened from a project file is saved
    /// to that file again. The backups then go.
    fn backup_restore(&self) -> Result<Value, String> {
        let Some(dir) = self.backup_dir() else {
            return Ok(json!({ "restored": [] }));
        };
        let mut restored = Vec::new();
        for (path, about) in list(&dir) {
            let origin = about["origin"].as_str().map(PathBuf::from);
            let name = about["name"].as_str().unwrap_or_default().to_owned();
            let id = self.open_backup(&path, origin, name)?;
            restored.push(id);
        }
        let _ = std::fs::remove_dir_all(&dir);
        let mut state = lock(&self.state);
        state.backups_left = false;
        Ok(state.listing_with(json!({ "restored": restored })))
    }

    /// "Auto-Save project" (`autosave_switch`, every `autosave_interval`
    /// seconds): the projects due to be saved, those already saved once
    /// to a project file that have unsaved changes and are not slicing.
    /// The page saves each as "Save Project" does.
    fn autosave_due(&self) -> Result<Value, String> {
        let state = lock(&self.state);
        if !on(&state.store.data.prefs, "autosave_switch") {
            return Ok(json!({ "due": [] }));
        }
        let due: Vec<u32> = state
            .projects
            .iter()
            .filter(|project| project.dirty() && !project.slicing())
            .filter(|project| {
                project
                    .file
                    .as_ref()
                    .and_then(|file| file.path.as_deref())
                    .is_some_and(|path| {
                        path.extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
                    })
            })
            .map(|project| project.id)
            .collect();
        Ok(json!({ "due": due }))
    }
}

/// The backups in `dir`: each project file with what it was.
fn list(dir: &Path) -> Vec<(PathBuf, Value)> {
    let mut out: Vec<(u32, PathBuf, Value)> = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "json") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u32>().ok())
        else {
            continue;
        };
        let model = path.with_extension("3mf");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(about) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        if model.is_file() {
            out.push((id, model, about));
        }
    }
    out.sort_by_key(|(id, ..)| *id);
    out.into_iter()
        .map(|(_, path, about)| (path, about))
        .collect()
}

#[cfg(test)]
mod tests;
