//! The project's list of filaments, as the sidebar's "Filament
//! Management" edits it (Plater.cpp `Sidebar`): "Add one filament",
//! "Remove last filament", a row's "Delete" and "Merge with", its colour
//! ("Click to select filament color"), and "Apply All".
//!
//! What each filament is lives in two places: the model's own settings
//! (how many there are, their colours, what is flushed between them, and
//! for a project on its own settings every filament's values), which go
//! with the model through the undo history and into a save
//! (`edge_engine::Model::add_filament` and the rest); and the project's
//! choices (the preset each has, what is set over it), which a step of
//! the history here carries with it.
//!
//! The reference takes no snapshot for any of these (Plater.cpp
//! `on_filaments_change`, `on_filaments_delete`, `Sidebar::delete_filament`):
//! its filament list is not in the undo stack, while the renumbering of
//! the model a deletion makes is in the model that the stack holds, so an
//! undo after a deletion gives the objects back their old numbers against
//! the shorter list. Here each edit is one step, list and model together.

use std::collections::BTreeMap;
use std::sync::Arc;

use edge_engine::{FlushBasis, Model, NEXT_COLOURS};
use serde_json::{Value, json};

use super::{Choices, File, Project, Session, lock, needed, settings, whole};

/// The calls answered here.
pub(super) const JSON: [&str; 1] = ["filament_edit"];

/// The choices a step of the history carries for the filaments: the
/// preset each has and what is set over it, as they were.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FilamentChoices {
    pub(super) presets: Vec<String>,
    pub(super) over: Vec<BTreeMap<String, String>>,
}

impl FilamentChoices {
    pub(super) fn of(project: &Project) -> Self {
        Self {
            presets: project.choices.filaments.clone(),
            over: project.choices.filament_over.clone(),
        }
    }

    /// Puts these in the project's choices and answers what was there.
    pub(super) fn swap_into(self, project: &mut Project) -> Self {
        let was = Self::of(project);
        project.choices.filaments = self.presets;
        project.choices.filament_over = self.over;
        was
    }
}

/// Each filament's preset made one entry for each of `count` filaments:
/// a filament past the last named has the last, as a slice reads it.
fn spread(presets: &[String], count: usize) -> Vec<String> {
    match presets.last() {
        None => Vec::new(),
        Some(last) => (0..count)
            .map(|at| presets.get(at).unwrap_or(last).clone())
            .collect(),
    }
}

/// What is set over each filament's preset with empty ones at the end
/// left out, as the session keeps it.
fn tidied(mut over: Vec<BTreeMap<String, String>>) -> Vec<BTreeMap<String, String>> {
    while over.last().is_some_and(BTreeMap::is_empty) {
        over.pop();
    }
    over
}

impl Session {
    pub(super) fn filaments_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "filament_edit" => self.filament_edit(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// What working out flushing volumes needs of each filament, from the
    /// settings a slice would read now; a filament past their end reads
    /// the last, as one added does.
    fn flush_basis(&self, choices: &Choices, file: Option<&File>, count: usize) -> FlushBasis {
        let settled = self.basis(choices, file).ok().and_then(|basis| {
            let overrides = settings::overrides(choices, &basis);
            edge_engine::settled(&basis.chosen, &overrides, basis.project.as_deref()).ok()
        });
        let Some(settled) = settled else {
            return FlushBasis {
                minimums: vec![0; count],
                support: vec![false; count],
            };
        };
        let minimums = edge_engine::flush::min_flush_volumes(&settled);
        let support: Vec<bool> = settled
            .texts("filament_is_support")
            .map(|items| items.iter().map(|item| item.trim() == "1").collect())
            .unwrap_or_default();
        fn stretch<T: Copy>(list: Vec<T>, count: usize, otherwise: T) -> Vec<T> {
            let last = list.last().copied().unwrap_or(otherwise);
            (0..count)
                .map(|at| list.get(at).copied().unwrap_or(last))
                .collect()
        }
        FlushBasis {
            minimums: stretch(minimums, count, 0),
            support: stretch(support, count, false),
        }
    }

    /// One edit of the filament list: `op` and what it takes.
    ///
    /// - `add` (`colour?`): "Add one filament": after the last, its preset
    ///   and settings the last one's, in the next colour of the
    ///   reference's palette.
    /// - `remove_last`: "Remove last filament".
    /// - `delete` (`filament`): the row menu's "Delete".
    /// - `merge` (`filament`, `into`): "Merge with": the filament goes and
    ///   what printed in it prints in `into`.
    /// - `colour` (`filament`, `colour` "#RRGGBB").
    /// - `apply_all` (`preset`): "Apply All", that preset for every one.
    ///
    /// Answers `{ project, scene, done }`, as `scene_edit` does.
    fn filament_edit(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let bed = stage.bed();
        let op = needed(args, "op")?;
        let filament = || -> Result<usize, String> { Ok(whole(args, "filament")? as usize) };
        let (mut model, count, mut wanted, choices, next) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            if project.id != stage.id {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            let model = project
                .file
                .as_ref()
                .map_or_else(Model::new, |file| file.model.clone());
            let count = model.filament_count();
            (
                model,
                count,
                FilamentChoices::of(project),
                project.choices.clone(),
                state.next_colour,
            )
        };
        // Worked out with the session free: it may read presets.
        let basis = match op {
            "add" => Some(self.flush_basis(&choices, stage.file.as_deref(), count + 1)),
            "colour" => Some(self.flush_basis(&choices, stage.file.as_deref(), count)),
            _ => None,
        };
        // The presets each filament has, one entry for each, when they are
        // chosen here.
        let named = !wanted.presets.is_empty();
        wanted.presets = spread(&wanted.presets, count);
        let mut used_colour = false;
        let done = match op {
            "add" => {
                let colour = match args.get("colour").and_then(Value::as_str) {
                    Some(colour) => colour.to_owned(),
                    None => {
                        used_colour = true;
                        NEXT_COLOURS[next % NEXT_COLOURS.len()].to_owned()
                    }
                };
                model
                    .add_filament(bed.as_ref(), &colour, basis.as_ref().expect("worked out"))
                    .map_err(|e| e.to_string())?;
                if let Some(last) = wanted.presets.last().cloned() {
                    wanted.presets.push(last);
                }
                if let Some(last) = wanted.over.get(count - 1).cloned() {
                    wanted.over.resize_with(count, BTreeMap::new);
                    wanted.over.push(last);
                }
                "Add one filament"
            }
            "remove_last" | "delete" | "merge" => {
                let gone = if op == "remove_last" {
                    count
                } else {
                    filament()?
                };
                let into = match op {
                    "merge" => Some(whole(args, "into")? as usize),
                    _ => None,
                };
                model
                    .remove_filament(bed.as_ref(), gone, into)
                    .map_err(|e| e.to_string())?;
                if gone <= wanted.presets.len() {
                    wanted.presets.remove(gone - 1);
                }
                if gone <= wanted.over.len() {
                    wanted.over.remove(gone - 1);
                }
                match op {
                    "remove_last" => "Remove last filament",
                    "delete" => "Delete this filament",
                    _ => "Merge with",
                }
            }
            "colour" => {
                let colour = needed(args, "colour")?;
                model
                    .set_filament_colour(bed.as_ref(), filament()?, colour, basis.as_ref())
                    .map_err(|e| e.to_string())?;
                "Change Filament Color"
            }
            "apply_all" => {
                let preset = needed(args, "preset")?;
                if !named {
                    return Err(
                        "the project is sliced with its own settings: choose presets for it first"
                            .into(),
                    );
                }
                if wanted.presets.iter().all(|held| held == preset) {
                    return self.edited(stage.id, json!({ "done": null }));
                }
                wanted.presets = vec![preset.to_owned(); count];
                "Apply All"
            }
            other => return Err(format!("there is no filament edit named {other}")),
        };
        wanted.over = tidied(wanted.over);
        if !named {
            wanted.presets.clear();
        }
        // A changed model is a new state of it; Apply All leaves the
        // model as it is.
        let model_changed = op != "apply_all";
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(stage.id)
                .ok_or_else(|| format!("there is no project {}", stage.id))?;
            if used_colour {
                state.next_colour += 1;
            }
            let project = &mut state.projects[at];
            let still = match (&project.file, &stage.file) {
                (Some(now), Some(then)) => Arc::ptr_eq(now, then),
                (None, None) => true,
                _ => false,
            };
            if !still {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            let before_choices = wanted.clone().swap_into(project);
            let before = if model_changed {
                let next = Arc::new(File::of(model));
                project.file.replace(next)
            } else {
                project.file.clone()
            };
            super::edit::push_step(project, done, before, Some(before_choices));
            // What every plate is sliced with has changed: the filaments
            // listed with each print, and their colours.
            project.forget();
            // The presets are what was last used with the printer
            // (PresetBundle.cpp `export_selections`, after each of these).
            if named {
                let presets = json!({ "filaments": project.choices.filaments });
                super::printers::remember(&mut state, at, &presets);
            }
        }
        self.edited(stage.id, json!({ "done": done }))
    }
}

#[cfg(test)]
mod tests;
