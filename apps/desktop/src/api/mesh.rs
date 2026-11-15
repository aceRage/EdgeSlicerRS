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

use edge_engine::{MeshErrors, MeshStat, remesh, simplify};
use serde_json::{Value, json};

use super::{
    File, Session,
    edit::push_step,
    i18n::{fill, tr},
    lock, whole,
};

/// The calls answered here.
pub(super) const JSON: [&str; 6] = [
    "mesh_simplify",
    "mesh_progress",
    "mesh_cancel",
    "mesh_apply",
    "mesh_repair_info",
    "mesh_repair",
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

/// The parts a call names (`parts`), none for all.
fn part_ids(args: &Value) -> Vec<u64> {
    args.get("parts")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default()
}

/// A mesh's surface area in mm² (`repair_by_remesh`, for the dialog's
/// estimate of the triangles a remesh makes).
fn area(mesh: &edge_engine::Mesh) -> f64 {
    mesh.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.vertices[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            0.5 * f64::from((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt())
        })
        .sum()
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
            "mesh_repair_info" => self.mesh_repair_info(args),
            "mesh_repair" => self.mesh_repair(args),
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

    /// The parts a "Repair" is for (`object`, `parts`: none for every
    /// printed part of it), and what its dialog shows of the first
    /// (`repair_by_remesh`): `{ parts, auto_voxel, triangles, area }`, the
    /// voxel size chosen for it, its triangles and its surface's area in mm².
    fn mesh_repair_info(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let file = stage.file.clone().ok_or("the project has no model")?;
        let targets = file
            .model
            .printed_meshes(whole(args, "object")?, &part_ids(args))
            .map_err(|error| error.to_string())?;
        let Some((_, first)) = targets.first() else {
            return Ok(json!({ "parts": 0, "auto_voxel": 0.1, "triangles": 0, "area": 0.0 }));
        };
        Ok(json!({
            "parts": targets.len(),
            "auto_voxel": remesh::auto_voxel_size(first),
            "triangles": first.triangles.len(),
            "area": area(first),
        }))
    }

    /// "Repair/Remesh" (`repair_by_remesh`): each part named (none: every
    /// printed part of `object`) rebuilt with `options` (as the dialog has
    /// them: `voxel_size`, `keep_bottom_flat`, `bottom_margin`,
    /// `preserve_sharp_edges`, `sharp_feature_angle`), put in as one step,
    /// "Repair by remeshing", while `mesh_progress` says how far it has got.
    /// A voxel size left at the first part's own is each part's own. Answers
    /// as `scene_edit`, `note` the reference's words on what was done.
    fn mesh_repair(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let file = stage.file.clone().ok_or("the project has no model")?;
        let object = whole(args, "object")?;
        let targets = file
            .model
            .printed_meshes(object, &part_ids(args))
            .map_err(|error| error.to_string())?;
        if targets.is_empty() {
            return Err("there is no printed part to repair".into());
        }
        let given = args.get("options").cloned().unwrap_or(Value::Null);
        let number = |key: &str, or: f64| given.get(key).and_then(Value::as_f64).unwrap_or(or);
        let flag = |key: &str, or: bool| given.get(key).and_then(Value::as_bool).unwrap_or(or);
        let defaults = remesh::Options::default();
        let mut options = remesh::Options {
            // 0: each part's own.
            voxel_size: match number("voxel_size", 0.0) {
                size if size <= 0.0 => 0.0,
                size => size.clamp(remesh::VOXEL_MIN, remesh::VOXEL_MAX),
            },
            keep_bottom_flat: flag("keep_bottom_flat", defaults.keep_bottom_flat),
            bottom_margin: number("bottom_margin", 0.0).clamp(0.0, remesh::BOTTOM_MARGIN_MAX),
            preserve_sharp_edges: flag("preserve_sharp_edges", defaults.preserve_sharp_edges),
            sharp_feature_angle: number("sharp_feature_angle", defaults.sharp_feature_angle)
                .clamp(remesh::ANGLE_MIN, remesh::ANGLE_MAX),
        };
        // A voxel size left at the first part's own is each part's own.
        if (options.voxel_size - remesh::auto_voxel_size(&targets[0].1)).abs() < 1e-9 {
            options.voxel_size = 0.0;
        }
        let (id, cancel, progress) = self.mesh_start();
        let mut made = Vec::new();
        let (mut before, mut after, mut failed, mut declined) = (0usize, 0usize, 0usize, 0usize);
        for (at, (part, mesh)) in targets.iter().enumerate() {
            progress.store((at * 100 / targets.len()) as i32, Ordering::Relaxed);
            let bed = file
                .model
                .part_bed_direction(object, *part)
                .map_err(|error| error.to_string())?;
            match remesh::remesh_with_options(mesh, &options, bed, &|| {
                cancel.load(Ordering::Relaxed)
            }) {
                Ok(Some((mesh, report))) => {
                    before += report.triangles_before;
                    after += report.triangles_after;
                    if report.fell_back {
                        declined += 1;
                    }
                    made.push((*part, mesh, MeshStat::default()));
                }
                Ok(None) => failed += 1,
                Err(_) => {
                    self.mesh_finish(id, None, &cancel).ok();
                    return Err("stopped".into());
                }
            }
        }
        self.mesh_finish(id, None, &cancel).ok();
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let repaired = made.len();
        // GUI_ObjectList.cpp `repair_by_remesh`, the notification.
        let mut note = if failed == 0 {
            fill(&tr("Repaired %1% part(s) by remeshing."), &[&repaired])
        } else {
            fill(
                &tr("Repaired %1% part(s), %2% failed."),
                &[&repaired, &failed],
            )
        };
        if repaired > 0 {
            note.push(' ');
            note.push_str(&fill(&tr("Triangles: %1% -> %2%."), &[&before, &after]));
        }
        if options.keep_bottom_flat && declined > 0 {
            note.push(' ');
            note.push_str(&fill(
                &tr("%1% part(s) had no flat bottom to protect."),
                &[&declined],
            ));
        }
        if made.is_empty() {
            return Err(note);
        }
        let mut model = file.model.clone();
        let painted = model
            .set_part_meshes(stage.bed().as_ref(), object, made)
            .map_err(|error| error.to_string())?;
        let step = "Repair by remeshing";
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
            push_step(project, step, before, None);
        }
        // Plater.cpp `clear_before_change_mesh`, said before the result.
        if painted {
            note = format!(
                "{}\n{note}",
                tr("Custom supports and color painting were removed before repairing.")
            );
        }
        self.edited(stage.id, json!({ "done": step, "note": note }))
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
