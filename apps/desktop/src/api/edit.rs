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

use edge_engine::{Bed, LAST_LAYER, LayerSequence, Model, PartKind, PlateSettings, Transform};
use serde_json::{Value, json};

use super::{
    File, Project, Session,
    filaments::FilamentChoices,
    lock, needed,
    scene::{Stage, kind_of},
    settings::items,
    text, whole,
};

/// The calls answered here.
pub(super) const JSON: [&str; 8] = [
    "scene_edit",
    "undo",
    "redo",
    "scene_faces",
    "cut_section",
    "cut_pick",
    "cut_parts",
    "cut_recipe",
];

/// The calls here answered in bytes (`Session::call_bytes`).
pub(super) const BYTES: [&str; 2] = ["cut_parts_mesh", "cut_connector_mesh"];

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

/// One step of the history: what was done, and the model on its far side;
/// for an edit of the filament list, the presets and what was set over
/// them on its far side too (`filaments`).
struct Step {
    name: String,
    file: Option<Arc<File>>,
    filaments: Option<FilamentChoices>,
}

/// Puts an edit made to a project on its undo history, `before` being the
/// model it was made to (and, for one of the filament list, the choices
/// it was made to), and lets go of what it made stale.
pub(super) fn push_step(
    project: &mut Project,
    name: &str,
    before: Option<Arc<File>>,
    filaments: Option<FilamentChoices>,
) {
    let history = &mut project.history;
    history.undo.push(Step {
        name: name.to_owned(),
        file: before.clone(),
        filaments,
    });
    if history.undo.len() > DEPTH {
        history.undo.remove(0);
    }
    history.redo.clear();
    history.revision += 1;
    settle(project, before.as_deref());
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
    /// Takes the last step back off when it is `name` and nothing came
    /// after it, giving what it was made to: the step to come is then
    /// one with it.
    fn amend(&mut self, name: &str, before: &mut Option<Arc<File>>) -> bool {
        if self.redo.is_empty() && self.undo.last().is_some_and(|step| step.name == name) {
            let step = self.undo.pop().expect("a last step");
            *before = step.file;
            true
        } else {
            false
        }
    }

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
    // The model's settings (its filaments) are in every plate's slice.
    let settings_of = |file: Option<&File>| file.and_then(|file| file.model.settings().cloned());
    if !match (settings_of(before), settings_of(project.file.as_deref())) {
        (Some(was), Some(now)) => Arc::ptr_eq(&was, &now),
        (None, None) => true,
        _ => false,
    } {
        project.forget();
    }
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

/// Takes away each plate's own bed type that `allowed` does not take, as
/// one step of the history, and answers whether there was one (Plater.cpp
/// `normalize_bed_types`, which runs when another printer is chosen).
pub(super) fn reset_bed_types(project: &mut Project, allowed: impl Fn(&str) -> bool) -> bool {
    let Some(file) = project.file.clone() else {
        return false;
    };
    let mut model = file.model.clone();
    if !model.reset_bed_types(allowed) {
        return false;
    }
    let before = project.file.replace(Arc::new(File::of(model)));
    push_step(project, "Plate bed types reset", before, None);
    true
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

/// What "Cut" is to keep and how ("After cut", the thickness), as a call
/// gives it: each left out is as the tool starts (both kept, laid as they
/// stood, no thickness).
fn cut_options(args: &Value) -> Result<edge_engine::CutOptions, String> {
    let flag = |key: &str, default: bool| -> Result<bool, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(value) => value
                .as_bool()
                .ok_or_else(|| format!("{key} is not true or false")),
        }
    };
    let thickness = match args.get("thickness") {
        None | Some(Value::Null) => 0.0,
        Some(value) => value.as_f64().ok_or("thickness is not a number")?,
    };
    let thickness_from = match text(args, "thickness_from") {
        None | Some("centred") => edge_engine::CutThickness::Centred,
        Some("above") => edge_engine::CutThickness::Above,
        Some("below") => edge_engine::CutThickness::Below,
        Some(other) => return Err(format!("thickness_from cannot be {other}")),
    };
    Ok(edge_engine::CutOptions {
        keep_upper: flag("keep_upper", true)?,
        keep_lower: flag("keep_lower", true)?,
        place_on_cut_upper: flag("place_on_cut_upper", false)?,
        place_on_cut_lower: flag("place_on_cut_lower", false)?,
        flip_upper: flag("flip_upper", false)?,
        flip_lower: flag("flip_lower", false)?,
        to_parts: flag("to_parts", false)?,
        thickness,
        thickness_from,
        groove: match args.get("groove") {
            None | Some(Value::Null) => None,
            Some(groove) => {
                let number = |key: &str| -> Result<f64, String> {
                    groove
                        .get(key)
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| format!("the groove's {key} was not given"))
                };
                Some(edge_engine::Groove {
                    depth: number("depth")?,
                    width: number("width")?,
                    flaps_angle: number("flaps_angle")?,
                    angle: number("angle")?,
                    depth_tolerance: number("depth_tolerance")?,
                    width_tolerance: number("width_tolerance")?,
                    count: number("count")?.clamp(1.0, 20.0) as usize,
                    gap: number("gap")?,
                    plane_radius: number("plane_radius")?,
                })
            }
        },
        groove_init: match args.get("groove").and_then(|groove| groove.get("init")) {
            None | Some(Value::Null) => None,
            Some(init) => {
                let list: Vec<f64> = init
                    .as_array()
                    .map(|list| list.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                Some(
                    <[f64; 4]>::try_from(list.as_slice())
                        .map_err(|_| "the groove's init is not four numbers")?,
                )
            }
        },
        editable: flag("editable", true)?,
        connectors: match args.get("connectors") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list
                .iter()
                .map(connector_of)
                .collect::<Result<Vec<_>, String>>()?,
            Some(_) => return Err("connectors is not a list".into()),
        },
        snap: match args.get("snap") {
            None | Some(Value::Null) => [0.3, 0.15],
            Some(snap) => {
                let number = |key: &str, least: f64, most: f64| -> Result<f64, String> {
                    snap.get(key)
                        .and_then(Value::as_f64)
                        .filter(|value| (least..=most).contains(value))
                        .ok_or_else(|| format!("the snap's {key} is not from {least} to {most}"))
                };
                [number("space", 0.1, 0.5)?, number("bulge", 0.05, 1.0)?]
            }
        },
        sides: match args.get("sides") {
            None | Some(Value::Null) => None,
            Some(Value::Array(list)) => Some(
                list.iter()
                    .map(|side| side.as_bool().ok_or("sides are true or false"))
                    .collect::<Result<Vec<bool>, _>>()?,
            ),
            Some(_) => return Err("sides is not a list".into()),
        },
    })
}

/// A connector as the page gives it: `at` (on the plane, in the scene),
/// `radius`, `height`, `radius_tolerance`, `height_tolerance` (mm),
/// `z_angle` (radians), `type` (`plug`, `dowel`, `snap`), `style`
/// (`prism`, `frustum`), `shape` (`triangle`, `square`, `hexagon`,
/// `circle`).
fn connector_of(value: &Value) -> Result<edge_engine::Connector, String> {
    use edge_engine::{ConnectorKind, ConnectorShape, ConnectorStyle};
    let number = |key: &str| -> Result<f64, String> {
        value
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("a connector's {key} was not given"))
    };
    let at = numbers(value, "at")?;
    let (radius, height) = (number("radius")?, number("height")?);
    if radius <= 0.0 || height <= 0.0 {
        return Err("a connector's radius and height are more than 0".into());
    }
    let word = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    Ok(edge_engine::Connector {
        at,
        radius,
        height,
        radius_tolerance: number("radius_tolerance")?.max(0.0),
        height_tolerance: number("height_tolerance")?.max(0.0),
        z_angle: number("z_angle")?,
        kind: match word("type") {
            "plug" => ConnectorKind::Plug,
            "dowel" => ConnectorKind::Dowel,
            "snap" => ConnectorKind::Snap,
            other => return Err(format!("a connector cannot be a {other}")),
        },
        style: match word("style") {
            "prism" => ConnectorStyle::Prism,
            "frustum" => ConnectorStyle::Frustum,
            other => return Err(format!("a connector's style cannot be {other}")),
        },
        shape: match word("shape") {
            "triangle" => ConnectorShape::Triangle,
            "square" => ConnectorShape::Square,
            "hexagon" => ConnectorShape::Hexagon,
            "circle" => ConnectorShape::Circle,
            other => return Err(format!("a connector's shape cannot be {other}")),
        },
    })
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
    /// The part it made, for the page to select.
    part: Option<u64>,
    /// Nothing changed (a plate given the name it had): no step is taken.
    unchanged: bool,
    /// The cut put back to be made again ("Edit cut...").
    recipe: Option<Value>,
}

/// A matrix as the page takes it: sixteen numbers, column by column.
fn matrix_json(transform: &Transform) -> Value {
    let mut numbers = Vec::with_capacity(16);
    for (row, values) in transform.rows.iter().enumerate() {
        numbers.extend_from_slice(values);
        numbers.push(if row == 3 { 1.0 } else { 0.0 });
    }
    json!(numbers)
}

/// How a cut was made, for the tool to be set up with it: its plane in
/// the scene and every setting it was made with (GLGizmoCut.cpp
/// `apply_recipe_to_gizmo`).
fn recipe_json(recipe: &edge_engine::CutRecipe, plane: &Transform) -> Value {
    let from = ["centred", "above", "below"][recipe.thickness_offset.min(2) as usize];
    // Where the recipe's coordinates are in the scene.
    let shift: [f64; 3] =
        std::array::from_fn(|axis| plane.rows[3][axis] - recipe.plane.rows[3][axis]);
    let connectors: Vec<Value> = recipe
        .connectors
        .iter()
        .map(|c| {
            let at: [f64; 3] = std::array::from_fn(|axis| c.pos[axis] + shift[axis]);
            let kind = ["plug", "dowel", "snap"]
                .get(c.kind as usize)
                .copied()
                .unwrap_or("plug");
            let style = ["prism", "frustum"]
                .get(c.style as usize)
                .copied()
                .unwrap_or("prism");
            let shape = ["triangle", "square", "hexagon", "circle"]
                .get(c.shape as usize)
                .copied()
                .unwrap_or("circle");
            json!({
                "at": at,
                "radius": c.radius,
                "height": c.height,
                "radius_tolerance": c.radius_tolerance,
                "height_tolerance": c.height_tolerance,
                "z_angle": c.z_angle,
                "type": kind,
                "style": style,
                "shape": shape,
            })
        })
        .collect();
    json!({
        "plane": matrix_json(plane),
        "mode": if recipe.groove.is_some() { "dovetail" } else { "planar" },
        "thickness": recipe.thickness,
        "thickness_from": from,
        "keep_upper": recipe.keep_upper,
        "keep_lower": recipe.keep_lower,
        "to_parts": recipe.keep_as_parts,
        "place_on_cut_upper": recipe.place_on_cut_upper,
        "place_on_cut_lower": recipe.place_on_cut_lower,
        "flip_upper": recipe.flip_upper,
        "flip_lower": recipe.flip_lower,
        "connectors": connectors,
        "groove": recipe.groove.map(|g| json!({
            "depth": g.depth, "width": g.width, "flaps_angle": g.flaps_angle, "angle": g.angle,
            "depth_tolerance": g.depth_tolerance, "width_tolerance": g.width_tolerance,
            "count": g.count, "gap": g.gap, "plane_radius": g.plane_radius,
            "init": [g.depth_init, g.width_init, g.flaps_angle_init, g.angle_init],
        })),
    })
}

/// Filament numbers a call gives under `key`; none given is an empty list.
fn filament_order(value: Option<&Value>, key: &str) -> Result<Vec<u32>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .and_then(|list| {
                list.iter()
                    .map(|item| item.as_u64().and_then(|n| u32::try_from(n).ok()))
                    .collect::<Option<Vec<u32>>>()
            })
            .ok_or_else(|| format!("{key} is not a list of filament numbers")),
    }
}

/// What a plate sets for itself, as `plate_settings` gives it: each left
/// out, or null, is the project's.
fn plate_settings_of(args: &Value) -> Result<PlateSettings, String> {
    let named = |key: &str| -> Result<Option<String>, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) if text.is_empty() => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(other) => Err(format!("{key} cannot be {other}")),
        }
    };
    let spiral_mode = match args.get("spiral_mode") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(on)) => Some(*on),
        Some(other) => return Err(format!("spiral_mode cannot be {other}")),
    };
    let other_layers_sequence = match args.get("other_layers_sequence") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(ranges)) => ranges
            .iter()
            .map(|range| {
                let layer = |key: &str| -> Result<u32, String> {
                    match range.get(key) {
                        // "End": to the last layer.
                        None | Some(Value::Null) if key == "end" => Ok(LAST_LAYER),
                        Some(value) => value
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .filter(|n| *n <= LAST_LAYER)
                            .ok_or_else(|| super::i18n::tr("Please input layer value (>= 2).")),
                        None => Err(super::i18n::tr("Please input layer value (>= 2).")),
                    }
                };
                Ok(LayerSequence {
                    begin: layer("begin")?,
                    end: layer("end")?,
                    order: filament_order(range.get("order"), "order")?,
                })
            })
            .collect::<Result<_, String>>()?,
        Some(other) => return Err(format!("other_layers_sequence cannot be {other}")),
    };
    Ok(PlateSettings {
        bed_type: named("bed_type")?,
        print_sequence: named("print_sequence")?,
        spiral_mode,
        first_layer_sequence: filament_order(
            args.get("first_layer_sequence"),
            "first_layer_sequence",
        )?,
        other_layers_sequence,
    })
}

/// A plate's own settings as the page reads them.
pub(super) fn plate_settings_json(own: &PlateSettings) -> Value {
    json!({
        "bed_type": own.bed_type,
        "print_sequence": own.print_sequence,
        "spiral_mode": own.spiral_mode,
        "first_layer_sequence": own.first_layer_sequence,
        "other_layers_sequence": own
            .other_layers_sequence
            .iter()
            .map(|range| json!({
                "begin": range.begin,
                // "End" is no number to the page.
                "end": (range.end != LAST_LAYER).then_some(range.end),
                "order": range.order,
            }))
            .collect::<Vec<_>>(),
    })
}

/// The parts a call names under `parts`: each a number, or for
/// `part_transform` each `{ id, matrix }`.
fn part_places(args: &Value) -> Result<Vec<(u64, Transform)>, String> {
    args.get("parts")
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
        .ok_or("parts was not given")?
        .iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_u64)
                .ok_or("a part's id was not given")?;
            Ok((
                id,
                transform_of(item.get("matrix").unwrap_or(&Value::Null))?,
            ))
        })
        .collect()
}

impl Session {
    /// Makes the edit `args` names to a model. `shown` is the plate the
    /// project is on.
    // What an edit needs beside the model differs from op to op; each is
    // given on its own rather than gathered for one op's sake.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        model: &mut Model,
        bed: Option<&Bed>,
        shown: Option<u32>,
        args: &Value,
        added: Option<&Model>,
        leave: &[u32],
        global: Option<&edge_config::Settings>,
        arrange: Option<&edge_engine::ArrangeOptions>,
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
                // Every plate (`all`), the copies named (`copies`), or a
                // plate (the one shown when none is named), with the
                // options beside the Arrange button and the settings the
                // project is sliced with (ArrangeJob.cpp).
                let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
                let chosen = match args.get("copies") {
                    Some(Value::Array(_)) => Some(ids(args, "copies")?),
                    _ => None,
                };
                let scope = match (&chosen, all) {
                    (Some(copies), _) => edge_engine::ArrangeScope::Copies(copies),
                    (None, true) => edge_engine::ArrangeScope::All { leave },
                    (None, false) => edge_engine::ArrangeScope::Plate(
                        plate(args).or(shown).ok_or("plate was not given")?,
                    ),
                };
                let options = arrange.cloned().unwrap_or_default();
                let arranged = model
                    .arrange_with(bed, global, &options, scope)
                    .map_err(|error| super::i18n::tr(&error.to_string()))?;
                let mut notes = Vec::new();
                for name in &arranged.zero_size {
                    notes.push(super::i18n::fill(
                        &super::i18n::tr("Object %s has zero size and can't be arranged."),
                        &[name],
                    ));
                }
                if !arranged.unplaced.is_empty() {
                    notes.push(super::i18n::tr(
                        "Arranging is done but there are unpacked items. Reduce spacing and try again.",
                    ));
                }
                outcome.note = (!notes.is_empty()).then(|| notes.join("\n"));
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
                    outcome.note = Some(super::i18n::tr(
                        "All non-solid parts (modifiers) were deleted",
                    ));
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
            "part_filament" => {
                // 0 or null: "default", the object's.
                let filament = match args.get("filament") {
                    None | Some(Value::Null) => None,
                    Some(value) => {
                        Some(value.as_u64().ok_or("filament is not a whole number")? as usize)
                            .filter(|filament| *filament > 0)
                    }
                };
                model
                    .set_part_filament(bed, whole(args, "object")?, &ids(args, "parts")?, filament)
                    .map_err(failed)?;
                outcome.name = "Change Filament".into();
            }
            "part_rename" => {
                model
                    .rename_part(
                        bed,
                        whole(args, "object")?,
                        whole(args, "part")?,
                        needed(args, "name")?,
                    )
                    .map_err(failed)?;
                outcome.name = "Rename Part".into();
            }
            "part_delete" => {
                model
                    .remove_parts(bed, whole(args, "object")?, &ids(args, "parts")?)
                    .map_err(failed)?;
                outcome.name = "Delete part".into();
            }
            "part_transform" => {
                let name = text(args, "name").unwrap_or("Move Object");
                if !TRANSFORMS.contains(&name) {
                    return Err(format!("name cannot be {name}"));
                }
                model
                    .set_part_transforms(bed, whole(args, "object")?, &part_places(args)?)
                    .map_err(failed)?;
                outcome.name = name.to_owned();
            }
            "part_mirror" => {
                let axis = match needed(args, "axis")? {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    other => return Err(format!("axis cannot be {other}")),
                };
                model
                    .mirror_parts(bed, whole(args, "copy")?, &ids(args, "parts")?, axis)
                    .map_err(failed)?;
                outcome.name = "Mirror Object".into();
            }
            "part_split" => {
                model
                    .split_part(bed, whole(args, "object")?, whole(args, "part")?)
                    .map_err(failed)?;
                outcome.name = "Split to parts".into();
            }
            "part_type" => {
                let kind = needed(args, "kind")?;
                let kind = kind_of(kind)
                    .ok_or_else(|| format!("there is no kind of part named {kind}"))?;
                model
                    .set_part_kind(bed, whole(args, "object")?, &ids(args, "parts")?, kind)
                    .map_err(failed)?;
                outcome.name = "Change part type".into();
            }
            "part_add" => {
                let object = whole(args, "object")?;
                let kind = needed(args, "kind")?;
                let kind = kind_of(kind)
                    .ok_or_else(|| format!("there is no kind of part named {kind}"))?;
                let part = match (added, text(args, "shape")) {
                    (Some(file), _) => {
                        outcome.name = if kind == PartKind::Normal {
                            "Load Part"
                        } else {
                            "Load Modifier"
                        }
                        .into();
                        model.add_from(bed, object, file, kind)
                    }
                    (None, Some(shape)) => {
                        outcome.name = "Add primitive".into();
                        let copy = args.get("copy").and_then(Value::as_u64);
                        model.add_generic(bed, object, copy, kind, shape)
                    }
                    (None, None) => return Err("path or shape was not given".into()),
                };
                outcome.part = Some(part.map_err(failed)?);
            }
            // What is done to a plate as a plate. The steps' names are the
            // reference's where it takes a snapshot (Plater.cpp
            // `rename_plate`, `select_plate_by_hover_id`); it takes none
            // for the plate's own settings or for duplicating a plate,
            // which are named here for the dialog and the menu.
            "plate_rename" => {
                let id = plate(args).or(shown).ok_or("plate was not given")?;
                let name = args
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("name was not given")?;
                let was = model.plates().iter().find(|held| held.id() == id);
                outcome.unchanged = was.is_some_and(|held| held.name() == name);
                model.rename_plate(id, name).map_err(failed)?;
                outcome.name = "Rename Plate".into();
            }
            "plate_lock" => {
                let id = plate(args).or(shown).ok_or("plate was not given")?;
                let value = args
                    .get("value")
                    .and_then(Value::as_bool)
                    .ok_or("value was not given")?;
                let was = model.plates().iter().find(|held| held.id() == id);
                outcome.unchanged = was.is_some_and(|held| held.locked() == value);
                model.lock_plate(id, value).map_err(failed)?;
                outcome.name = "lock partplate".into();
            }
            "plate_settings" => {
                let id = plate(args).or(shown).ok_or("plate was not given")?;
                let own = plate_settings_of(args)?;
                let was = model.plates().iter().find(|held| held.id() == id);
                outcome.unchanged = was.is_some_and(|held| held.settings() == own);
                model.set_plate_settings(id, &own).map_err(failed)?;
                outcome.name = "Plate Settings".into();
            }
            "plate_front" => {
                let id = plate(args).or(shown).ok_or("plate was not given")?;
                model.move_plate_to_front(bed, id).map_err(failed)?;
                // The plate moved is the one shown (Plater.cpp
                // `select_plate_by_hover_id`, `select_plate(0)`).
                outcome.show = Some(1);
                outcome.name = "move plate to the front".into();
            }
            "plate_duplicate" => {
                let id = plate(args).or(shown).ok_or("plate was not given")?;
                let (new, made) = model.duplicate_plate(bed, id).map_err(failed)?;
                outcome.show = Some(new);
                outcome.made = made;
                outcome.name = "Duplicate Current Plate".into();
            }
            "cut" => {
                let options = cut_options(args)?;
                let plane = transform_of(args.get("plane").unwrap_or(&Value::Null))?;
                let done = model
                    .cut(bed, whole(args, "copy")?, &plane, &options)
                    .map_err(|error| super::i18n::tr(&error.to_string()))?;
                outcome.made = done.made;
                if done.open > 0 {
                    outcome.note = Some(super::i18n::fill(
                        &super::i18n::tr(
                            "The object is open where it was cut: %d of the cut's outlines could not be closed.",
                        ),
                        &[&done.open.to_string()],
                    ));
                }
                outcome.name = "Cut by Plane".into();
            }
            "cut_edit" => {
                let edit = model
                    .cut_edit(bed, whole(args, "copy")?)
                    .map_err(|error| super::i18n::tr(&error.to_string()))?;
                outcome.made = vec![edit.copy];
                let mut recipe = recipe_json(&edit.recipe, &edit.plane);
                recipe["missing"] = json!(edit.missing);
                outcome.recipe = Some(recipe);
                outcome.name = "Edit cut".into();
            }
            // An object's, a part's or a height range's own settings, and
            // the height ranges (settings/items.rs).
            op if items::OPS.contains(&op) => {
                let global = global.ok_or("the project's settings were not given")?;
                match items::apply(model, bed, op, args, global)? {
                    Some(name) => outcome.name = name,
                    None => outcome.unchanged = true,
                }
            }
            other => return Err(format!("there is no edit named {other}")),
        }
        Ok(outcome)
    }

    /// The project and its scene as they are, which is what every call
    /// here answers.
    pub(super) fn edited(&self, id: u32, more: Value) -> Result<Value, String> {
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
            "cut_section" => self.cut_section(args),
            "cut_pick" => self.cut_pick(args),
            "cut_parts" => self.cut_parts(args),
            "cut_recipe" => self.cut_recipe(args),
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
            ("add" | "part_add", Some(path)) => {
                Some(Model::open(Path::new(path)).map_err(|e| e.to_string())?)
            }
            ("add", None) => return Err("path was not given".into()),
            _ => None,
        };
        let mut model = match &stage.file {
            Some(file) => file.model.clone(),
            None => Model::new(),
        };
        // A plate's own bed type is one the printer takes (the dialog
        // offers no other: PlateSettingsDialog.cpp, `allowed_bed_types`).
        if needed(args, "op")? == "plate_settings"
            && let Some(bed) = text(args, "bed_type")
            && let Some(printer) = stage.printer.as_deref()
            && !super::plates_for(printer, None).takes(bed)
        {
            return Err(format!(
                "the printer chosen does not take a {bed} of a plate's own"
            ));
        }
        // Arranging every plate leaves alone, besides the locked ones, a
        // plate whose own print sequence is not the project's
        // (ArrangeJob.cpp `prepare_all`).
        let leave: Vec<u32> = if needed(args, "op")? == "arrange"
            && args.get("all").and_then(Value::as_bool) == Some(true)
        {
            let global = self.print_sequence(stage.id)?;
            model
                .plates()
                .iter()
                .filter(|plate| {
                    plate
                        .settings()
                        .print_sequence
                        .is_some_and(|own| own != global)
                })
                .map(|plate| plate.id())
                .collect()
        } else {
            Vec::new()
        };
        let (global, arrange) = if needed(args, "op")? == "arrange" {
            let (options, global) = self.arrange_choice(stage.id)?;
            (Some(global), Some(options))
        } else if items::OPS.contains(&needed(args, "op")?) {
            (Some(self.items_global(stage.id)?), None)
        } else {
            (None, None)
        };
        let mut outcome = Self::apply(
            &mut model,
            bed.as_ref(),
            stage.plate,
            args,
            added.as_ref(),
            &leave,
            global.as_ref(),
            arrange.as_ref(),
        )?;
        if outcome.unchanged {
            return self.edited(
                stage.id,
                json!({ "done": null, "made": [], "note": null, "part": null }),
            );
        }
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
            let mut before = project.file.replace(next);
            // The cut made of a cut put back ("Edit cut...") is one step
            // with putting it back, so one undo brings the halves back
            // (GUI_ObjectList.cpp `edit_cut`: one snapshot for the whole).
            if text(args, "amend") == Some("Edit cut")
                && project.history.amend("Edit cut", &mut before)
            {
                outcome.name = "Edit cut".into();
            }
            push_step(project, &outcome.name, before, None);
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
            json!({ "done": outcome.name, "made": outcome.made, "note": outcome.note, "part": outcome.part, "recipe": outcome.recipe }),
        )
    }

    /// How a copy's object was cut, when the cut was kept editable, set
    /// up on `onto` ("Copy cut to..."; the copy itself when not given).
    fn cut_recipe(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let bed = stage.bed();
        let file = stage.file.as_ref().ok_or("the project has no model")?;
        let copy = whole(args, "copy")?;
        let onto = args.get("onto").and_then(Value::as_u64).unwrap_or(copy);
        let (recipe, plane) = file
            .model
            .cut_copied(bed.as_ref(), copy, onto)
            .map_err(|error| super::i18n::tr(&error.to_string()))?;
        Ok(recipe_json(&recipe, &plane))
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

    /// Where a plane cuts a copy's object ("Cut", while the plane is
    /// placed): the lines of its section in the scene, as six numbers
    /// each, and how far the object reaches over and under the plane.
    fn cut_section(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let copy = whole(args, "copy")?;
        let plane = transform_of(args.get("plane").unwrap_or(&Value::Null))?;
        let bed = stage.bed();
        let file = stage
            .file
            .ok_or("the project is empty: it has no objects")?;
        let section = file
            .model
            .cut_section(bed.as_ref(), copy, &plane)
            .map_err(|e| e.to_string())?;
        // With a dovetail, whether it can be cut there.
        let groove = match cut_options(args)?.groove {
            Some(groove) => Some(
                file.model
                    .groove_meets(bed.as_ref(), copy, &plane, &groove)
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        };
        let mm = |value: f64| (value * 1e4).round() / 1e4;
        // The cut face itself, while connectors are placed on it.
        let cap = match args.get("cap").and_then(Value::as_bool) {
            Some(true) => Some(
                file.model
                    .cut_cap(bed.as_ref(), copy, &plane)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .flatten()
                    .flatten()
                    .map(|value| mm(*value))
                    .collect::<Vec<f64>>(),
            ),
            _ => None,
        };
        let lines: Vec<f64> = section
            .lines
            .iter()
            .flat_map(|[a, b]| {
                a.iter()
                    .chain(b)
                    .map(|value| mm(*value))
                    .collect::<Vec<_>>()
            })
            .collect();
        Ok(json!({
            "project": stage.id,
            "copy": copy,
            "lines": lines,
            "above": mm(section.above),
            "below": mm(section.below),
            "reach": section.reach.map(|corners| corners.map(|corner| corner.map(mm))),
            "groove": groove,
            "cap": cap,
        }))
    }

    /// The face of a copy a line of sight meets ("Cut"'s "Pick face"):
    /// `{ hit: { point, normal, corners } }`, or `hit` null.
    fn cut_pick(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        let copy = whole(args, "copy")?;
        let origin = numbers(args, "origin")?;
        let direction = numbers(args, "direction")?;
        let bed = stage.bed();
        let file = stage
            .file
            .ok_or("the project is empty: it has no objects")?;
        let hit = file
            .model
            .face_hit(bed.as_ref(), copy, origin, direction)
            .map_err(|e| e.to_string())?;
        Ok(json!({
            "project": stage.id,
            "copy": copy,
            "hit": hit.map(|hit| json!({ "point": hit.point, "normal": hit.normal, "corners": hit.corners })),
        }))
    }

    /// The pieces a cut makes of a copy's object, for a piece to be given
    /// to the other side: the model's [`edge_engine::CutPieces`].
    fn pieces_of(&self, args: &Value) -> Result<(u32, edge_engine::CutPieces), String> {
        let stage = self.stage_of(args)?;
        let copy = whole(args, "copy")?;
        let plane = transform_of(args.get("plane").unwrap_or(&Value::Null))?;
        let options = cut_options(args)?;
        let bed = stage.bed();
        let file = stage
            .file
            .ok_or("the project is empty: it has no objects")?;
        let pieces = file
            .model
            .cut_pieces(bed.as_ref(), copy, &plane, &options)
            .map_err(|e| e.to_string())?;
        Ok((stage.id, pieces))
    }

    /// `cut_parts`: how many pieces, the side each was cut on, the
    /// triangles of each (`[first, how many]` of `cut_parts_mesh`'s) and
    /// the copy's move in the scene.
    fn cut_parts(&self, args: &Value) -> Result<Value, String> {
        let (id, pieces) = self.pieces_of(args)?;
        let mut ranges: Vec<[u32; 2]> = vec![[0, 0]; pieces.upper.len()];
        for (at, piece) in pieces.piece_of.iter().enumerate() {
            let range = &mut ranges[*piece as usize];
            if range[1] == 0 {
                range[0] = at as u32;
            }
            range[1] += 1;
        }
        Ok(json!({
            "project": id,
            "count": pieces.upper.len(),
            "upper": pieces.upper,
            "ranges": ranges,
            "offset": pieces.offset,
        }))
    }

    pub(super) fn edit_bytes(&self, name: &str, args: &Value) -> Result<Vec<u8>, String> {
        match name {
            "cut_parts_mesh" => {
                let (_, pieces) = self.pieces_of(args)?;
                let object = edge_engine::SceneObject {
                    name: String::new(),
                    marks: vec![edge_engine::Mark::Filament(1); pieces.mesh.triangles.len()],
                    bounds: pieces.mesh.bounds(),
                    parts: Vec::new(),
                    vertices: pieces.mesh.vertices,
                    triangles: pieces.mesh.triangles,
                };
                Ok(super::scene::mesh_bytes(&object))
            }
            // A connector's shape at unit size, as the cut makes it, for the
            // tool to draw (`type`, `style`, `shape`, `snap` as `cut` takes
            // them).
            "cut_connector_mesh" => {
                let mut asked = args.clone();
                if let Some(object) = asked.as_object_mut() {
                    object.insert("at".into(), json!([0.0, 0.0, 0.0]));
                    object.insert("radius".into(), json!(1.0));
                    object.insert("height".into(), json!(1.0));
                    for key in ["radius_tolerance", "height_tolerance", "z_angle"] {
                        object.insert(key.into(), json!(0.0));
                    }
                }
                let c = connector_of(&asked)?;
                let snap = cut_options(&json!({ "snap": args.get("snap") }))?.snap;
                let mesh = edge_engine::connector_mesh(c.kind, c.style, c.shape, snap);
                let object = edge_engine::SceneObject {
                    name: String::new(),
                    marks: vec![edge_engine::Mark::Filament(1); mesh.triangles.len()],
                    bounds: mesh.bounds(),
                    parts: Vec::new(),
                    vertices: mesh.vertices,
                    triangles: mesh.triangles,
                };
                Ok(super::scene::mesh_bytes(&object))
            }
            other => Err(format!(
                "there is no call named {other} that answers in bytes"
            )),
        }
    }

    /// Takes the last edit back, or makes the last one taken back again.
    fn step(&self, args: &Value, back: bool) -> Result<Value, String> {
        let (id, name) = {
            let mut state = lock(&self.state);
            let at = state.target(args)?;
            let project = &mut state.projects[at];
            let history = &mut project.history;
            let from = if back {
                &mut history.undo
            } else {
                &mut history.redo
            };
            let step = from.pop().ok_or(if back {
                "there is nothing to undo"
            } else {
                "there is nothing to redo"
            })?;
            // What is set at the layers is not in the history, as in the
            // reference: it stays as it is (`ticks`). A step of the
            // filament list renumbers it, and takes it back with it.
            let file = match (&project.file, step.file) {
                (Some(now), Some(then)) if step.filaments.is_none() => {
                    Some(super::ticks::carried(now, &then).map_or(then, Arc::new))
                }
                (_, then) => then,
            };
            let before = std::mem::replace(&mut project.file, file);
            // The filament list's presets go back with it.
            let filaments = step.filaments.map(|choices| choices.swap_into(project));
            let history = &mut project.history;
            let onto = if back {
                &mut history.redo
            } else {
                &mut history.undo
            };
            onto.push(Step {
                name: step.name.clone(),
                file: before.clone(),
                filaments: filaments.clone(),
            });
            history.revision += 1;
            if filaments.is_some() {
                project.forget();
            }
            settle(project, before.as_deref());
            (project.id, step.name)
        };
        self.edited(id, json!({ "done": name }))
    }
}

#[cfg(test)]
mod tests;
