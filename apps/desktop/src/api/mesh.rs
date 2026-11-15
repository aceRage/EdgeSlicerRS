//! The mesh operations of a part: "Simplify" (Gizmos/GLGizmoSimplify.cpp)
//! and "Repair" (GUI_ObjectList.cpp `repair_by_remesh`), and the object
//! list's warning of a mesh's errors (`get_mesh_errors_info`).
//!
//! A mesh operation is worked out by one call that holds on until it is
//! done (`mesh_simplify`), while the page asks how far it has got
//! (`mesh_progress`) and can call it off (`mesh_cancel`), as the
//! reference's worker thread reports its progress and is cancelled; a
//! later one calls off the one before, as the gizmo restarts its worker
//! when the settings change. The result is kept, for the page to draw in
//! place of the part (`mesh_result`, in bytes) and to put in the model
//! (`mesh_apply`, one step of the history named as the reference's
//! snapshot). It is put in only while the model is still the one it was
//! made from.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicI32, Ordering},
};

use edge_engine::{MeshErrors, MeshStat, simplify};
use serde_json::{Value, json};

use super::{File, Session, edit::push_step, lock, whole};

/// The calls answered here.
pub(super) const JSON: [&str; 4] = [
    "mesh_simplify",
    "mesh_progress",
    "mesh_cancel",
    "mesh_apply",
];

/// The calls here answered in bytes.
pub(super) const BYTES: [&str; 1] = ["mesh_result"];

/// The mesh operation under way and the last one done.
#[derive(Default)]
pub(super) struct Jobs {
    /// How many have been asked for: the number of the latest.
    asked: u64,
    running: Option<Running>,
    done: Option<Held>,
}

struct Running {
    id: u64,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicI32>,
}

/// What an operation made, and what it was made from.
struct Held {
    id: u64,
    project: u32,
    /// The model it was made from, by where it is in memory.
    file: usize,
    object: u64,
    part: u64,
    /// The history's name for putting it in.
    step: String,
    mesh: edge_engine::Mesh,
    stat: MeshStat,
    /// The filament the part prints in, to draw it in its colour.
    filament: u8,
}

/// The filament a part prints in, for drawing it.
pub(super) fn part_filament(model: &edge_engine::Model, object: u64, part: u64) -> u8 {
    model
        .objects()
        .iter()
        .find(|each| each.id() == object)
        .and_then(|each| {
            let shape = each.shape();
            let at = shape.part_ids().iter().position(|id| *id == part)?;
            u8::try_from(shape.parts()[at].filament).ok()
        })
        .unwrap_or(1)
}

/// What the page is told of a mesh's errors (`get_mesh_errors_info`):
/// null when there is no warning, else the open edges left and how many
/// errors were repaired.
pub(super) fn errors_json(errors: &MeshErrors) -> Value {
    if !errors.shown() {
        return Value::Null;
    }
    json!({ "open_edges": errors.open_edges, "repaired": errors.repaired.count() })
}

impl Session {
    pub(super) fn mesh_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "mesh_simplify" => self.mesh_simplify(args),
            "mesh_progress" => {
                let jobs = lock(&self.mesh_jobs);
                Ok(match &jobs.running {
                    Some(running) => json!({
                        "id": running.id,
                        "running": true,
                        "progress": running.progress.load(Ordering::Relaxed),
                    }),
                    None => {
                        json!({ "id": jobs.done.as_ref().map(|held| held.id), "running": false })
                    }
                })
            }
            "mesh_cancel" => {
                if let Some(running) = &lock(&self.mesh_jobs).running {
                    running.cancel.store(true, Ordering::Relaxed);
                }
                Ok(json!({}))
            }
            "mesh_apply" => self.mesh_apply(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    pub(super) fn mesh_bytes(&self, name: &str, args: &Value) -> Result<Vec<u8>, String> {
        match name {
            // The last result, in the part's own coordinates, by its number.
            "mesh_result" => {
                let jobs = lock(&self.mesh_jobs);
                let held = jobs.done.as_ref().ok_or("there is no result")?;
                if args.get("id").and_then(Value::as_u64) != Some(held.id) {
                    return Err("that result has been made again".into());
                }
                let mesh = &held.mesh;
                let object = edge_engine::SceneObject {
                    name: String::new(),
                    marks: vec![edge_engine::Mark::Filament(held.filament); mesh.triangles.len()],
                    bounds: mesh.bounds(),
                    parts: Vec::new(),
                    vertices: mesh.vertices.clone(),
                    triangles: mesh.triangles.clone(),
                };
                Ok(super::scene::mesh_bytes(&object))
            }
            other => Err(format!(
                "there is no call named {other} that answers in bytes"
            )),
        }
    }

    /// Starts an operation: the one under way is called off. Answers its
    /// number, the flag that calls it off and where its progress goes.
    fn mesh_start(&self) -> (u64, Arc<AtomicBool>, Arc<AtomicI32>) {
        let mut jobs = lock(&self.mesh_jobs);
        if let Some(running) = jobs.running.take() {
            running.cancel.store(true, Ordering::Relaxed);
        }
        jobs.asked += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicI32::new(0));
        jobs.running = Some(Running {
            id: jobs.asked,
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        (jobs.asked, cancel, progress)
    }

    /// Keeps what an operation made, unless it was called off or a later
    /// one was asked for ("stopped").
    fn mesh_finish(&self, id: u64, held: Option<Held>, cancel: &AtomicBool) -> Result<(), String> {
        let mut jobs = lock(&self.mesh_jobs);
        if jobs.running.as_ref().map(|running| running.id) == Some(id) {
            jobs.running = None;
        }
        match held {
            Some(held) if !cancel.load(Ordering::Relaxed) && jobs.asked == held.id => {
                jobs.done = Some(held);
                Ok(())
            }
            _ => Err("stopped".into()),
        }
    }

    /// "Simplify" of a part (`GLGizmoSimplify::process`): `object`, `part`
    /// (none: the object's one part), and either `wanted` (a triangle
    /// count: "Decimate ratio") or `max_error` (the quadric error of
    /// "Detail level"). Answers `{ id, part, original, triangles }`; the
    /// mesh then from `mesh_result`.
    fn mesh_simplify(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let file = stage.file.clone().ok_or("the project has no model")?;
        let object = whole(args, "object")?;
        let part = args.get("part").and_then(Value::as_u64);
        let (part, mesh) = file
            .model
            .mesh_of_part(object, part)
            .map_err(|error| super::i18n::tr(&error.to_string()))?;
        let name = file
            .model
            .part_name(object, part)
            .map_err(|error| error.to_string())?;
        // `use_count` takes the count, else the error ("Detail level").
        let (count, max_error) = match args.get("wanted").and_then(Value::as_u64) {
            Some(wanted) => (u32::try_from(wanted).unwrap_or(u32::MAX), f32::MAX),
            None => (
                0,
                args.get("max_error")
                    .and_then(Value::as_f64)
                    .ok_or("give wanted or max_error")? as f32,
            ),
        };
        let (id, cancel, progress) = self.mesh_start();
        let mut work = (*mesh).clone();
        let done = simplify::quadric_edge_collapse(
            &mut work,
            count,
            Some(max_error),
            &mut |percent| progress.store(percent, Ordering::Relaxed),
            &|| cancel.load(Ordering::Relaxed),
        );
        let filament = part_filament(&file.model, object, part);
        let held = done.ok().map(|_| Held {
            filament,
            id,
            project: stage.id,
            file: Arc::as_ptr(&file) as usize,
            object,
            part,
            // `apply_simplify`: `take_snapshot(GUI::format("Simplify %1%", name))`.
            step: format!("Simplify {name}"),
            mesh: work,
            stat: MeshStat::default(),
        });
        let triangles = held.as_ref().map(|held| held.mesh.triangles.len());
        self.mesh_finish(id, held, &cancel)?;
        Ok(json!({
            "id": id,
            "part": part,
            "original": mesh.triangles.len(),
            "triangles": triangles,
        }))
    }

    /// Puts the result numbered `id` in place of its part, as one step of
    /// the history; refused when the project changed since it was made.
    /// Answers the project and its scene, `done` (the step's name) and
    /// `note` (what was said of paint taken off, when any was).
    fn mesh_apply(&self, args: &Value) -> Result<Value, String> {
        let id = whole(args, "id")?;
        let stage = self.stage_of(args)?;
        let (held_project, file_key, object, part, step, mesh, stat) = {
            let jobs = lock(&self.mesh_jobs);
            let held = jobs
                .done
                .as_ref()
                .filter(|held| held.id == id)
                .ok_or("that result is no longer kept")?;
            (
                held.project,
                held.file,
                held.object,
                held.part,
                held.step.clone(),
                held.mesh.clone(),
                held.stat,
            )
        };
        let file = stage.file.clone().ok_or("the project has no model")?;
        if held_project != stage.id || Arc::as_ptr(&file) as usize != file_key {
            return Err("the project changed meanwhile; nothing was done".into());
        }
        let mut model = file.model.clone();
        let bed = stage.bed();
        let painted = model
            .set_part_mesh(bed.as_ref(), object, part, mesh, stat)
            .map_err(|error| error.to_string())?;
        {
            let mut state = lock(&self.state);
            let at = state
                .index_of(stage.id)
                .ok_or_else(|| format!("there is no project {}", stage.id))?;
            let project = &mut state.projects[at];
            if !project
                .file
                .as_ref()
                .is_some_and(|now| Arc::ptr_eq(now, &file))
            {
                return Err("the project changed meanwhile; nothing was done".into());
            }
            let before = project.file.replace(Arc::new(File::of(model)));
            push_step(project, &step, before, None);
        }
        lock(&self.mesh_jobs).done = None;
        // Plater.cpp `clear_before_change_mesh`.
        let note = painted.then(|| {
            super::i18n::tr("Custom supports and color painting were removed before repairing.")
        });
        self.edited(
            stage.id,
            json!({ "done": step, "part": part, "note": note }),
        )
    }
}

#[cfg(test)]
mod tests;
