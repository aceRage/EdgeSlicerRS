//! "Save Project" [Ctrl+S] and "Save Project as..." [Ctrl+Shift+S]
//! (MainFrame.cpp `init_menubar_as_editor`, Plater.cpp `save_project`),
//! and what is asked before a project or the window closes with
//! unsaved changes (ProjectStrip.cpp `close_project`, `confirm_close_all`).
//!
//! A project is saved as the reference's project file (`edge_3mf::write`):
//! its model as it stands and the settings it is sliced with: its file's
//! own as they were when nothing is set over them; its own with what was
//! set over them; or the presets chosen with what was set over those,
//! complete, naming the presets and what was changed over each.

use std::path::{Path, PathBuf};

use edge_engine::{Changed, Model, Saving};
use serde_json::{Value, json};

use super::{Choices, File, Session, bed_of, lock, name_from_path, settings, shown};

/// Where a project is offered to be saved when the person is asked: as
/// a project beside the file it was opened from, else under its name in
/// the strip (Plater.cpp `get_export_file`, "Untitled.3mf").
fn suggested(file: Option<&File>, name: &str) -> String {
    let path = file.and_then(|file| file.path.as_deref());
    let stem = path
        .and_then(Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| name.to_owned());
    let named = format!("{stem}.3mf");
    match path
        .and_then(Path::parent)
        .filter(|dir| !dir.as_os_str().is_empty())
    {
        Some(dir) => shown(&dir.join(named)),
        None => named,
    }
}

/// Whether a path names a project file.
fn is_project(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
}

/// What was set by hand over each preset, by the reference's names.
fn changed(choices: &Choices) -> Changed {
    Changed {
        process: choices.over.keys().cloned().collect(),
        filaments: choices
            .filament_over
            .iter()
            .map(|own| own.keys().cloned().collect())
            .collect(),
        printer: choices
            .printer_over
            .keys()
            .map(|key| {
                key.split_once('#')
                    .map_or(key.as_str(), |(key, _)| key)
                    .to_owned()
            })
            .collect(),
    }
}

impl Session {
    /// The settings a project is saved with: what it is sliced with.
    fn saving(&self, choices: &Choices, file: Option<&File>) -> Result<Saving, String> {
        let own = file.and_then(|file| file.settings.clone());
        let set_by_hand = !choices.over.is_empty()
            || choices.filament_over.iter().any(|own| !own.is_empty())
            || !choices.printer_over.is_empty()
            || choices.bed_type.is_some();
        if !choices.from_presets
            && let Some(own) = &own
        {
            if !set_by_hand {
                return Ok(Saving::Own);
            }
            let basis = self.basis(choices, file)?;
            let overrides = settings::overrides(choices, &basis);
            let settled = edge_engine::settled(&basis.chosen, &overrides, Some(own))
                .map_err(|e| e.to_string())?;
            let mut settings = edge_engine::own_settings(own, &settled, &changed(choices));
            if let Some(bed) = &choices.bed_type {
                settings.insert("curr_bed_type", edge_config::Value::One(bed.clone()));
            }
            return Ok(Saving::Settings {
                settings,
                presets: true,
            });
        }
        if choices.vendor.is_none() || choices.machine.is_none() {
            // No printer chosen yet: saved without settings.
            return Ok(Saving::None);
        }
        let basis = self.basis(choices, file)?;
        let overrides = settings::overrides(choices, &basis);
        let settled = edge_engine::settled(&basis.chosen, &overrides, basis.project.as_deref())
            .map_err(|e| e.to_string())?;
        Ok(Saving::Settings {
            settings: edge_engine::project_settings(&settled, &changed(choices)),
            presets: false,
        })
    }

    /// Saves a project (`project?`) to `path`, else to the project file
    /// it was opened from; with `suggest: true` only answers `suggested`. Answers the strip and `saved: true`, `path`;
    /// or `saved: false` with `ask: "path"` and `suggested` (a name to
    /// offer in the save dialog: the project has no project file of its
    /// own yet), or `ask: "losses"` with `losses`, what its file held that
    /// would not be kept (saved all the same with `accept: true`).
    pub(super) fn project_save(&self, args: &Value) -> Result<Value, String> {
        let accept = args.get("accept").and_then(Value::as_bool).unwrap_or(false);
        let (id, name, file, choices) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.name.clone(),
                project.file.clone(),
                project.choices.clone(),
            )
        };
        // "Save Project as...": only where to offer it, nothing written.
        if args
            .get("suggest")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Ok(json!({ "suggested": suggested(file.as_deref(), &name) }));
        }
        let asked = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        let path = match asked {
            Some(path) => path,
            None => match file
                .as_ref()
                .and_then(|file| file.path.clone())
                .filter(|path| is_project(path))
            {
                Some(path) => path,
                None => {
                    let state = lock(&self.state);
                    return Ok(state.listing_with(json!({
                        "saved": false,
                        "ask": "path",
                        "name": name,
                        "suggested": suggested(file.as_deref(), &name),
                    })));
                }
            },
        };
        // A project is a 3MF, whatever the name typed.
        let path = if is_project(&path) {
            path
        } else {
            let mut named = path.into_os_string();
            named.push(".3mf");
            PathBuf::from(named)
        };
        let saving = self.saving(&choices, file.as_deref())?;
        let model = file
            .as_ref()
            .map_or_else(Model::new, |file| file.model.clone());
        let losses = model.save_losses(&saving);
        if !losses.is_empty() && !accept {
            let state = lock(&self.state);
            return Ok(state.listing_with(json!({
                "saved": false,
                "ask": "losses",
                "name": name,
                "losses": losses,
            })));
        }
        let bed = self
            .printer_of(&choices, file.as_deref())?
            .and_then(|(_, printer)| bed_of(&printer))
            .map(edge_engine::Bed::of);
        model
            .save(&path, &saving, bed.as_ref())
            .map_err(|e| e.to_string())?;

        // The project is now that file, and nothing of it is unsaved.
        let mut state = lock(&self.state);
        let at = state
            .index_of(id)
            .ok_or_else(|| format!("there is no project {id}"))?;
        let project = &mut state.projects[at];
        let unchanged = match (&project.file, &file) {
            (Some(now), Some(saved)) => std::sync::Arc::ptr_eq(now, saved),
            (None, None) => true,
            _ => false,
        };
        if let Some(saved) = &file {
            let mut model = saved.model.clone();
            model.saved_as(&path);
            let renamed = std::sync::Arc::new(File {
                path: Some(path.clone()),
                ..File::of(model)
            });
            // An edit made while it was being written is not in the file.
            if unchanged {
                project.file = Some(renamed.clone());
                project.opened = Some(renamed);
                if project.choices == choices {
                    project.baseline = choices;
                }
            }
        } else if unchanged && project.choices == choices {
            project.baseline = choices;
        }
        project.name = name_from_path(&shown(&path));
        Ok(state.listing_with(json!({
            "saved": true,
            "id": id,
            "path": shown(&path),
        })))
    }

    /// The projects that have unsaved changes, for the question asked
    /// before the window closes: `{ unsaved: [{ id, name, path }] }`.
    pub(super) fn unsaved(&self) -> Value {
        let state = lock(&self.state);
        let unsaved: Vec<Value> = state
            .projects
            .iter()
            .filter(|project| project.dirty())
            .map(|project| {
                json!({
                    "id": project.id,
                    "name": project.name,
                    "path": project
                        .file
                        .as_ref()
                        .and_then(|file| file.path.as_deref())
                        .map(shown),
                })
            })
            .collect();
        json!({ "unsaved": unsaved, "projects": state.projects.len() })
    }
}

#[cfg(test)]
mod tests;
