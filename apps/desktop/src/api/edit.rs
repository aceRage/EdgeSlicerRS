//! Edits to what stands on a project's plates, and taking them back.
//!
//! A project's model is held as it stands (`File`); an edit makes the next
//! one from a copy of it, which is cheap because the meshes are shared,
//! and the one before goes onto the project's undo history, as the
//! reference takes a snapshot before each change (Utils/UndoRedo.cpp,
//! `Plater::take_snapshot`). Taking an edit back puts that model in place
//! again. Either way the slices of the plates that would now slice
//! otherwise are let go, and a slice that is running is called off, as the
//! reference invalidates a plate's result when what is on it changes
//! (PartPlate.cpp `update_slice_result_valid_state`).
//!
//! The page asks for all of it with one call, `scene_edit`, which names
//! what to do (`op`), and with `undo` and `redo`. Each answers the project
//! and its scene as they then are, so the page asks for nothing more.

use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

use edge_engine::{Bed, Model, Transform};
use serde_json::{Value, json};

use super::{File, Project, Session, lock, needed, scene::Stage, text, whole};

/// The calls answered here.
pub(super) const JSON: [&str; 4] = ["scene_edit", "undo", "redo", "scene_faces"];

/// The most edits a project keeps to take back. The reference keeps as
/// many as fit in a tenth of the machine's memory (UndoRedo.cpp
/// `StackImpl`); a step here is a list of where things stand, so a count
/// serves.
const DEPTH: usize = 200;

/// What is said of a plate that cannot be sliced for an object reaching
/// over its edge: the reference's own words (Plater.cpp
/// `validate_current_plate`).
pub(super) const OVER_THE_EDGE: &str = "An object is laid over the boundary of plate or exceeds the height limit.\nPlease solve the problem by moving it totally on or off the plate, and confirming that the height is within the build volume.";

/// Whether something that is to be printed on a plate reaches over the
/// plate's edge.
pub(super) fn over_the_edge(file: &File, bed: Option<&Bed>, plate: Option<u32>) -> bool {
    let Some(plate) = plate else { return false };
    let model = &file.model;
    let on = model
        .plates()
        .iter()
        .find(|held| held.id() == plate)
        .map(|held| held.copies())
        .unwrap_or_default();
    model
        .objects()
        .iter()
        .flat_map(|object| object.copies())
        .any(|copy| copy.printable() && on.contains(&copy.id()) && model.overhangs(bed, copy.id()))
}

/// One step of the history: what was done, and the model on its far side.
struct Step {
    name: String,
    file: Option<Arc<File>>,
}

/// The edits made to a project's model, to take back, and those taken
/// back, to make again.
#[derive(Default)]
pub(super) struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Counts every change to the model, so the page can tell one state
    /// from the next.
    revision: u64,
}

impl History {
    /// How many changes the model has seen.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    /// What the page shows of it: whether there is something to take back
    /// or to make again, and what.
    pub(super) fn marks(&self) -> Value {
        json!({
            "revision": self.revision,
            "undo": self.undo.last().map(|step| &step.name),
            "redo": self.redo.last().map(|step| &step.name),
        })
    }
}

/// Lets go of what a change to a project's model has made stale: the
/// slices and failures of the plates that would now slice otherwise, a
/// slice that is running, the views from above, and a plate shown that is
/// no longer there.
fn settle(project: &mut Project, before: Option<&File>) {
    let after = project.file.as_deref();
    // A plate's slot, and what its slice depends on, in a model.
    let stamps = |file: Option<&File>| -> Vec<(u32, edge_engine::PlateStamp)> {
        let Some(file) = file else { return Vec::new() };
        match file.plates.as_slice() {
            [] => vec![(0, file.model.stamp(None))],
            plates => plates
                .iter()
                .map(|plate| (plate.id, file.model.stamp(Some(plate.id))))
                .collect(),
        }
    };
    let (was, now) = (stamps(before), stamps(after));
    let same = |slot: &u32| {
        let of = |stamps: &[(u32, edge_engine::PlateStamp)]| {
            stamps
                .iter()
                .find(|(held, _)| held == slot)
                .map(|(_, stamp)| stamp.clone())
        };
        matches!((of(&was), of(&now)), (Some(a), Some(b)) if a == b)
    };
    project.results.retain(|slot, _| same(slot));
    project.failures.retain(|slot, _| same(slot));
    if was != now
        && let Some(work) = &project.work
    {
        work.cancel.store(true, Ordering::Relaxed);
    }
    project.footprints.clear();
    // The plate shown: the one it was on while the model still has it,
    // else the first; none for a model without plates.
    let ids: Vec<u32> = after
        .map(|file| file.plates.iter().map(|plate| plate.id).collect())
        .unwrap_or_default();
    if !project.plate.is_some_and(|plate| ids.contains(&plate)) {
        project.plate = ids.first().copied();
    }
}

/// A matrix as the page has it (sixteen numbers, column by column) as
/// the transform it stands for.
fn transform_of(value: &Value) -> Result<Transform, String> {
    let numbers: Vec<f64> = value
        .as_array()
        .map(|list| list.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    if numbers.len() != 16 || value.as_array().is_some_and(|list| list.len() != 16) {
        return Err("matrix is not sixteen numbers".into());
    }
    Ok(Transform {
        rows: std::array::from_fn(|row| std::array::from_fn(|axis| numbers[row * 4 + axis])),
    })
}

/// The copies or objects a call names, by number.
fn ids(args: &Value, key: &str) -> Result<Vec<u64>, String> {
    args.get(key)
        .and_then(Value::as_array)
        .and_then(|list| list.iter().map(Value::as_u64).collect::<Option<Vec<u64>>>())
        .filter(|list| !list.is_empty())
        .ok_or_else(|| format!("{key} was not given"))
}

/// Three numbers a call gives under `key`.
fn numbers(args: &Value, key: &str) -> Result<[f64; 3], String> {
    let list: Vec<f64> = args
        .get(key)
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    <[f64; 3]>::try_from(list).map_err(|_| format!("{key} is not three numbers"))
}

/// What an edit by the gizmos or the number fields may be called in the
/// history: the reference's own names for them.
const TRANSFORMS: [&str; 9] = [
    "Move Object",
    "Tool Move",
    "Gizmo-Move",
    "Gizmo-Rotate",
    "Gizmo-Scale",
    "Gizmo-Place on Face",
    "Set Position",
    "Set Orientation",
    "Set Scale",
];

/// What an edit came to, beyond the model as it then is.
#[derive(Default)]
struct Outcome {
    /// What the history calls it.
    name: String,
    /// The copies it made, for the page to select.
    made: Vec<u64>,
    /// The plate to show afterwards, when the edit says.
    show: Option<u32>,
    /// Something to tell the person.
    note: Option<String>,
}

impl Session {
    /// Makes the edit `args` names to a model. `shown` is the plate the
    /// project is on.
    fn apply(
        model: &mut Model,
        bed: Option<&Bed>,
        shown: Option<u32>,
        args: &Value,
        added: Option<&Model>,
    ) -> Result<Outcome, String> {
        let failed = |error: edge_engine::Failure| error.to_string();
        let plate = |args: &Value| -> Option<u32> {
            args.get("plate")
                .and_then(Value::as_u64)
                .and_then(|plate| u32::try_from(plate).ok())
        };
        let mut outcome = Outcome::default();
        match needed(args, "op")? {
            "transform" => {
                let name = text(args, "name").unwrap_or("Move Object");
                if !TRANSFORMS.contains(&name) {
                    return Err(format!("name cannot be {name}"));
                }
                let to = args
                    .get("copies")
                    .and_then(Value::as_array)
                    .filter(|list| !list.is_empty())
                    .ok_or("copies was not given")?
                    .iter()
                    .map(|item| {
                        let id = item
                            .get("id")
                            .and_then(Value::as_u64)
                            .ok_or("a copy's id was not given")?;
                        Ok((
                            id,
                            transform_of(item.get("matrix").unwrap_or(&Value::Null))?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                model.set_transforms(bed, &to).map_err(failed)?;
                outcome.name = name.to_owned();
            }
            "delete" => {
                model.remove(bed, &ids(args, "copies")?).map_err(failed)?;
                outcome.name = "Delete selected".into();
            }
            "rename" => {
                model
                    .rename(bed, whole(args, "object")?, needed(args, "name")?)
                    .map_err(failed)?;
                outcome.name = "Rename Object".into();
            }
            "printable" => {
                let value = args
                    .get("value")
                    .and_then(Value::as_bool)
                    .ok_or("value was not given")?;
                model
                    .set_printable(bed, &ids(args, "copies")?, value)
                    .map_err(failed)?;
                outcome.name = if value {
                    "Set Printable"
                } else {
                    "Set Unprintable"
                }
                .into();
            }
            "filament" => {
                let filament = whole(args, "filament")? as usize;
                model
                    .set_filament(bed, whole(args, "object")?, filament)
                    .map_err(failed)?;
                outcome.name = "Change Filament".into();
            }
            "add" => {
                let added = added.ok_or("path was not given")?;
                outcome.made = model
                    .import(bed, added, plate(args).or(shown))
                    .map_err(failed)?;
                outcome.name = "Import Object".into();
                // What the plate had no room for stands beside the plates.
                let aside = outcome
                    .made
                    .iter()
                    .filter(|copy| model.plate_of(**copy).is_none())
                    .count();
                if aside > 0 {
                    outcome.note = Some(format!(
                        "{aside} of the objects added did not fit on the plate and stand beside the plates."
                    ));
                }
            }
            "clone" => {
                outcome.made = model
                    .clone_copies(bed, &ids(args, "copies")?, plate(args))
                    .map_err(failed)?;
                outcome.name = text(args, "name")
                    .filter(|name| *name == "Paste From Clipboard")
                    .unwrap_or("Clone")
                    .to_owned();
            }
            "add_plate" => {
                outcome.show = Some(model.add_plate(bed).map_err(failed)?);
                outcome.name = "add partplate".into();
            }
            "delete_plate" => {
                // Which plate is shown afterwards (PartPlate.cpp
                // `delete_plate`): the one before the plate shown when the
                // plate taken away is it or stands before it.
                let gone = plate(args).or(shown).ok_or("plate was not given")?;
                let ids: Vec<u32> = model.plates().iter().map(|plate| plate.id()).collect();
                let (at, on) = (
                    ids.iter().position(|id| *id == gone),
                    shown.and_then(|shown| ids.iter().position(|id| *id == shown)),
                );
                model.remove_plate(bed, gone).map_err(failed)?;
                outcome.show = match (at, on) {
                    (Some(at), Some(on)) if on >= at && on > 0 => Some(on as u32),
                    (Some(_), Some(on)) => Some(on as u32 + 1),
                    _ => None,
                };
                outcome.name = "delete partplate".into();
            }
            "arrange" => {
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let on = if all { None } else { plate(args).or(shown) };
                let left = model.arrange(bed, on).map_err(failed)?;
                if left > 0 {
                    outcome.note = Some(format!(
                        "{left} of the objects did not fit on the plate and stand beside the plates."
                    ));
                }
                outcome.name = "Arrange".into();
            }
            "mirror" => {
                let axis = match needed(args, "axis")? {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    other => return Err(format!("axis cannot be {other}")),
                };
                model
                    .mirror(bed, &ids(args, "copies")?, axis)
                    .map_err(failed)?;
                outcome.name = "Mirror Object".into();
            }
            "split_objects" => {
                let (made, dropped) = model
                    .split_to_objects(bed, whole(args, "object")?)
                    .map_err(failed)?;
                outcome.made = made;
                // The reference leaves its modifiers and negative parts
                // out without a word (the notice is commented out in
                // Plater.cpp `split_object`); this says so, in the words
                // that notice had.
                if dropped {
                    outcome.note = Some("All non-solid parts (modifiers) were deleted".into());
                }
                outcome.name = "Split to Objects".into();
            }
            "split_parts" => {
                model
                    .split_to_parts(bed, whole(args, "object")?)
                    .map_err(failed)?;
                outcome.name = "Split to parts".into();
            }
            "lay_on_face" => {
                let normal = numbers(args, "normal")?;
                model
                    .lay_on_face(bed, whole(args, "copy")?, normal)
                    .map_err(failed)?;
                outcome.name = "Gizmo-Place on Face".into();
            }
            other => return Err(format!("there is no edit named {other}")),
        }
        Ok(outcome)
    }

    /// The project and its scene as they are, which is what every call
    /// here answers.
    fn edited(&self, id: u32, more: Value) -> Result<Value, String> {
        let stage = self.stage_of(&json!({ "project": id }))?;
        let scene = self.scene_for(&stage);
        let state = lock(&self.state);
        let at = state
            .index_of(id)
            .ok_or_else(|| format!("there is no project {id}"))?;
        let mut answer = json!({ "project": state.projects[at].detail(), "scene": scene });
        if let (Some(answer), Value::Object(more)) = (answer.as_object_mut(), more) {
            answer.extend(more);
        }
        Ok(answer)
    }

    pub(super) fn edit_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "scene_edit" => self.scene_edit(args),
            "undo" | "redo" => self.step(args, name == "undo"),
            "scene_faces" => self.scene_faces(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// Makes one edit to a project's model.
    fn scene_edit(&self, args: &Value) -> Result<Value, String> {
        let stage: Stage = self.stage_of(args)?;
        let bed = stage.bed();
        // A file to add is read before anything is changed, with the
        // session free to answer other calls.
        let added = match (needed(args, "op")?, text(args, "path")) {
            ("add", Some(path)) => Some(Model::open(Path::new(path)).map_err(|e| e.to_string())?),
            ("add", None) => return Err("path was not given".into()),
            _ => None,
        };
        let mut model = match &stage.file {
            Some(file) => file.model.clone(),
            None => Model::new(),
        };
        let outcome = Self::apply(&mut model, bed.as_ref(), stage.plate, args, added.as_ref())?;
        let next = Arc::new(File::of(model));
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(stage.id)
                .ok_or_else(|| format!("there is no project {}", stage.id))?;
            let project = &mut state.projects[at];
            // The model the edit was made to must still be the project's.
            let still = match (&project.file, &stage.file) {
                (Some(now), Some(then)) => Arc::ptr_eq(now, then),
                (None, None) => true,
                _ => false,
            };
            if !still {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            let before = project.file.replace(next);
            let history = &mut project.history;
            history.undo.push(Step {
                name: outcome.name.clone(),
                file: before.clone(),
            });
            if history.undo.len() > DEPTH {
                history.undo.remove(0);
            }
            history.redo.clear();
            history.revision += 1;
            settle(project, before.as_deref());
            if let Some(show) = outcome.show
                && project
                    .file
                    .as_ref()
                    .is_some_and(|file| file.plates.iter().any(|plate| plate.id == show))
            {
                project.plate = Some(show);
            }
        }
        self.edited(
            stage.id,
            json!({ "done": outcome.name, "made": outcome.made, "note": outcome.note }),
        )
    }

    /// The faces a copy can be laid on ("Lay on face"), largest first: each
    /// with its outward normal in the object's own coordinates (what
    /// `lay_on_face` takes), its area at the printed size, and the outline
    /// to draw over it, also in the object's own coordinates.
    fn scene_faces(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let copy = whole(args, "copy")?;
        let file = stage
            .file
            .ok_or("the project is empty: it has no objects")?;
        let faces = file.model.faces(copy).map_err(|e| e.to_string())?;
        let mm = |value: f64| (value * 1e4).round() / 1e4;
        let faces: Vec<Value> = faces
            .iter()
            .map(|face| {
                json!({
                    "normal": face.normal,
                    "area": mm(face.area),
                    "drawn": face.drawn.iter().map(|p| p.map(mm)).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({ "project": stage.id, "copy": copy, "faces": faces }))
    }

    /// Takes the last edit back, or makes the last one taken back again.
    fn step(&self, args: &Value, back: bool) -> Result<Value, String> {
        let (id, name) = {
            let mut state = lock(&self.state);
            let at = state.target(args)?;
            let project = &mut state.projects[at];
            let history = &mut project.history;
            let (from, onto) = if back {
                (&mut history.undo, &mut history.redo)
            } else {
                (&mut history.redo, &mut history.undo)
            };
            let step = from.pop().ok_or(if back {
                "there is nothing to undo"
            } else {
                "there is nothing to redo"
            })?;
            let before = std::mem::replace(&mut project.file, step.file);
            onto.push(Step {
                name: step.name.clone(),
                file: before.clone(),
            });
            history.revision += 1;
            settle(project, before.as_deref());
            (project.id, step.name)
        };
        self.edited(id, json!({ "done": name }))
    }
}

#[cfg(test)]
mod tests;
