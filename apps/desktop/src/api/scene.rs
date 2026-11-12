//! What the 3D views ask for: a project as a scene (its bed, its plates,
//! its objects as meshes) for Prepare, and a sliced plate as moves for
//! Preview.
//!
//! What is small is answered as JSON through [`Session::call`]. Meshes and
//! moves are large, so they are answered as bytes through
//! [`Session::call_bytes`], in the layouts written out below: the page
//! hands them to the graphics card as they come, without reading them
//! number by number. Every number in them is little-endian.
//!
//! A mesh is sent once for each object, in the object's own coordinates;
//! the copies of an object are only where they stand (`scene`). Moves are
//! asked for a stretch at a time (`toolpath_moves`), so the first layers
//! are on screen before the last have been sent. Nothing is sent twice:
//! `scene` gives each object's mesh a number (`mesh`) and `toolpath` a
//! `run` that change only when what they describe does, so an edit that
//! moves, adds or takes away copies sends no mesh again.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use edge_config::Settings;
use edge_engine::{Bed, Feature, Mark, MoveKind, SceneObject, Shape, Toolpath};
use serde_json::{Value, json};

use super::{Done, File, Session, bed_of, lock};

/// The calls answered here as JSON.
pub(super) const JSON: [&str; 3] = ["scene", "toolpath", "gcode_lines"];

/// The calls answered here as bytes.
pub(super) const BYTES: [&str; 5] = [
    "scene_mesh",
    "bed_model",
    "bed_texture",
    "toolpath_moves",
    "toolpath_lines",
];

/// What is kept between calls so that a file is read once and a G-code
/// file turned into moves once. An entry goes when what it was made from
/// does.
#[derive(Default)]
pub(super) struct Cache {
    meshes: Kept<Shape, SceneObject>,
    paths: Kept<Done, Paths>,
    /// The printers every vendor's profiles describe, for the folder they
    /// were read from.
    models: Mutex<Option<(PathBuf, Arc<Models>)>>,
}

/// What was made from something, by where that something is in memory,
/// for as long as it is still there.
type Kept<From, Made> = Mutex<HashMap<usize, (Weak<From>, Arc<Made>)>>;

/// Printer model name to (vendor, the model's own file under the vendor's folder).
type Models = HashMap<String, (String, String)>;

// ---- meshes ----

/// The first four bytes of a mesh.
pub const MESH_MAGIC: &[u8; 4] = b"EDGM";
/// The size of what comes before a mesh's vertices.
pub const MESH_HEADER: usize = 24;

/// What a triangle's mark byte says when it is not a filament.
pub const MARK_NEGATIVE: u8 = 251;
pub const MARK_MODIFIER: u8 = 252;
pub const MARK_SUPPORT_BLOCKER: u8 = 253;
pub const MARK_SUPPORT_ENFORCER: u8 = 254;
/// The highest filament a mark byte can say; one above is given as this.
pub const MARK_LAST_FILAMENT: u8 = 250;

fn mark_byte(mark: Mark) -> u8 {
    match mark {
        Mark::Filament(filament) => filament.clamp(1, MARK_LAST_FILAMENT),
        Mark::Negative => MARK_NEGATIVE,
        Mark::Modifier => MARK_MODIFIER,
        Mark::SupportBlocker => MARK_SUPPORT_BLOCKER,
        Mark::SupportEnforcer => MARK_SUPPORT_ENFORCER,
    }
}

/// How long [`mesh_bytes`] is for an object, without making it.
fn mesh_size(object: &SceneObject) -> usize {
    let marks = match uniform_mark(object) {
        Some(_) => 0,
        None => object.marks.len().next_multiple_of(4),
    };
    MESH_HEADER + 12 * object.vertices.len() + 12 * object.triangles.len() + marks
}

/// The one mark every triangle has, when they all have the same.
fn uniform_mark(object: &SceneObject) -> Option<u8> {
    let first = object.marks.first().copied().map(mark_byte).unwrap_or(1);
    object
        .marks
        .iter()
        .all(|mark| mark_byte(*mark) == first)
        .then_some(first)
}

/// A mesh as bytes. All numbers little-endian:
///
/// | at | size | what |
/// | --- | --- | --- |
/// | 0 | 4 | the letters `EDGM` |
/// | 4 | 4 | `u32` version, 1 |
/// | 8 | 4 | `u32` V, the number of vertices |
/// | 12 | 4 | `u32` T, the number of triangles |
/// | 16 | 4 | `u32` flags: bit 0 set when a mark for each triangle follows |
/// | 20 | 4 | `u32` the mark every triangle has when none follow, else 0 |
/// | 24 | 12·V | for each vertex `f32` x, y, z in mm, in the object's own coordinates |
/// | 24+12·V | 12·T | for each triangle `u32` a, b, c: its corners as indices of vertices, anticlockwise seen from outside |
/// | 24+12·V+12·T | T | with bit 0 of the flags: for each triangle a `u8` mark |
/// | … | 0–3 | zeros, to a length that is a multiple of 4 |
///
/// A mark is the filament a triangle prints in, 1 to 250 (a higher one is
/// given as 250), or 251 for a negative part, 252 for a modifier, 253 for
/// a support blocker and 254 for a support enforcer.
pub fn mesh_bytes(object: &SceneObject) -> Vec<u8> {
    let uniform = uniform_mark(object);
    let mut out = Vec::with_capacity(mesh_size(object));
    out.extend_from_slice(MESH_MAGIC);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(object.vertices.len() as u32).to_le_bytes());
    out.extend_from_slice(&(object.triangles.len() as u32).to_le_bytes());
    out.extend_from_slice(&u32::from(uniform.is_none()).to_le_bytes());
    out.extend_from_slice(&u32::from(uniform.unwrap_or(0)).to_le_bytes());
    for vertex in &object.vertices {
        for value in vertex {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    for triangle in &object.triangles {
        for corner in triangle {
            out.extend_from_slice(&corner.to_le_bytes());
        }
    }
    if uniform.is_none() {
        out.extend(object.marks.iter().map(|mark| mark_byte(*mark)));
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

// ---- moves ----

/// The first four bytes of a stretch of moves.
pub const MOVES_MAGIC: &[u8; 4] = b"EDGT";
/// The size of what comes before the moves of a stretch.
pub const MOVES_HEADER: usize = 24;
/// The size of one move.
pub const MOVE_SIZE: usize = 32;
/// The most moves one answer carries: 8 MB.
pub const MOVES_AT_ONCE: usize = 1 << 18;

/// One move as bytes. All numbers little-endian:
///
/// | at | size | what |
/// | --- | --- | --- |
/// | 0 | 12 | `f32` x, y, z: where the nozzle is when the move ends, in mm, on its plate |
/// | 12 | 4 | `f32` the speed asked for, in mm/s |
/// | 16 | 2 | `u16` height of the line laid, in thousandths of a mm (65535 for more) |
/// | 18 | 2 | `u16` width of the line laid, likewise |
/// | 20 | 2 | `i16` how far the path turns here into the next move of the same line, in ten-thousandths of a radian, anticlockwise positive |
/// | 22 | 2 | `u16` nozzle temperature last set, in °C |
/// | 24 | 4 | `u32` layer, from 0 |
/// | 28 | 1 | `u8` what the move is: 0 start, 1 travel, 2 line laid, 3 wipe, 4 retraction, 5 unretraction, 6 seam, 7 tool change |
/// | 29 | 1 | `u8` feature of a line laid, by the reference's numbers (libvgcode `EGCodeExtrusionRole`): 1 inner wall, 2 outer wall and so on; 0 for any other move |
/// | 30 | 1 | `u8` filament, from 1 |
/// | 31 | 1 | `u8` part cooling fan, in percent |
///
/// A move goes from where the move before it ended. A start, and the
/// marks from 4 up, draw no line: a start is only a place to go from.
fn move_bytes(out: &mut Vec<u8>, step: &edge_engine::Move) {
    let thousandths = |mm: f32| (f64::from(mm) * 1e3).round().clamp(0.0, 65535.0) as u16;
    for value in step.to {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&step.feedrate.to_le_bytes());
    out.extend_from_slice(&thousandths(step.height).to_le_bytes());
    out.extend_from_slice(&thousandths(step.width).to_le_bytes());
    let turn = (f64::from(step.turn) * 1e4)
        .round()
        .clamp(-32767.0, 32767.0) as i16;
    out.extend_from_slice(&turn.to_le_bytes());
    out.extend_from_slice(&step.temperature.to_le_bytes());
    out.extend_from_slice(&step.layer.to_le_bytes());
    out.extend_from_slice(&[step.kind as u8, step.feature as u8, step.filament, step.fan]);
}

/// A sliced plate as moves: what `toolpath` says of them, and every move
/// as [`move_bytes`] writes it, one after another. For a print read back
/// from its G-code, the line of the file each move was read from, and
/// where each line of the file ends (GCodeProcessor's `lines_ends`): made
/// once for each slice, with the moves, for the G-code window.
pub(super) struct Paths {
    index: Value,
    records: Vec<u8>,
    lines: Vec<u32>,
    line_ends: Vec<usize>,
}

/// The first four bytes of a stretch of moves' lines.
pub const LINES_MAGIC: &[u8; 4] = b"EDGL";

/// The most lines of G-code one answer of `gcode_lines` carries.
pub const GCODE_LINES_AT_ONCE: usize = 2048;

/// The most characters of a line the G-code window shows: a longer one is
/// cut to 52 and given "..." (GCodeViewer.cpp `GCodeWindow::render`).
const LINE_SHOWN: usize = 55;

fn paths_of(done: &Done) -> Paths {
    let (source, path) = match &done.gcode {
        Ok(gcode) => ("gcode", edge_engine::read_toolpath(&gcode.text)),
        Err(_) => ("lines", done.sliced.toolpath_of_lines()),
    };
    let mut records = Vec::with_capacity(path.moves.len() * MOVE_SIZE);
    for step in &path.moves {
        move_bytes(&mut records, step);
    }
    let mut index = index_of(source, &path);
    let line_ends: Vec<usize> = match &done.gcode {
        Ok(gcode) => {
            let text = &gcode.text;
            let mut ends: Vec<usize> = text
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .map(|(at, _)| at + 1)
                .collect();
            if ends.last().copied().unwrap_or(0) < text.len() {
                ends.push(text.len());
            }
            ends
        }
        Err(_) => Vec::new(),
    };
    index["gcode_lines"] = json!(line_ends.len());
    Paths {
        index,
        records,
        lines: path.lines,
        line_ends,
    }
}

/// A line of G-code as the G-code window shows it: no new line, and cut
/// short with "..." past 55 characters.
fn shown_line(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let text = text.trim_end_matches(['\n', '\r']);
    if text.chars().count() <= LINE_SHOWN {
        return text.to_owned();
    }
    let cut: String = text.chars().take(LINE_SHOWN - 3).collect();
    format!("{cut}...")
}

/// What the page needs to know of a toolpath before it asks for moves.
fn index_of(source: &str, path: &Toolpath) -> Value {
    let mut features = BTreeSet::new();
    let mut filaments = BTreeSet::new();
    let mut counts = [0usize; 8];
    // The corners of everything laid, which is what a view is fitted to.
    let mut reach: Option<([f32; 3], [f32; 3])> = None;
    for (at, step) in path.moves.iter().enumerate() {
        counts[step.kind as usize] += 1;
        if step.kind != MoveKind::Extrude {
            continue;
        }
        features.insert(step.feature as u8);
        filaments.insert(step.filament);
        // The file's own lines (a purge at the edge of the bed) are not
        // the part.
        if step.feature == Feature::Custom {
            continue;
        }
        let from = path.moves[at.saturating_sub(1)].to;
        for point in [from, step.to] {
            let (low, high) = reach.get_or_insert((point, point));
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
        }
    }
    let mm = |value: f32| (f64::from(value) * 1e3).round() / 1e3;
    json!({
        // "gcode": the moves of the file that would be exported. "lines":
        // no G-code could be written, so only the lines as sliced, in no
        // printing order and with nothing between them.
        "source": source,
        "moves": path.moves.len(),
        "move_size": MOVE_SIZE,
        "at_once": MOVES_AT_ONCE,
        "layers": {
            "first": path.layers.iter().map(|layer| layer.first).collect::<Vec<_>>(),
            "z": path.layers.iter().map(|layer| mm(layer.z)).collect::<Vec<_>>(),
            "height": path.layers.iter().map(|layer| mm(layer.height)).collect::<Vec<_>>(),
        },
        "features": features,
        "filaments": filaments,
        "counts": {
            "travel": counts[MoveKind::Travel as usize],
            "line": counts[MoveKind::Extrude as usize],
            "wipe": counts[MoveKind::Wipe as usize],
            "retract": counts[MoveKind::Retract as usize],
            "unretract": counts[MoveKind::Unretract as usize],
            "seam": counts[MoveKind::Seam as usize],
            "tool_change": counts[MoveKind::ToolChange as usize],
        },
        "box": reach.map(|(low, high)| [low.map(mm), high.map(mm)]),
    })
}

// ---- the bed ----

/// The corners of `printable_area`, each "XxY", as they are given.
fn bed_shape(settings: &Settings) -> Vec<[f64; 2]> {
    settings
        .texts("printable_area")
        .unwrap_or_default()
        .iter()
        .filter_map(|corner| {
            let (x, y) = corner.split_once('x')?;
            Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
        })
        .collect()
}

/// A file name a profile gives, if it is only that: nothing that could
/// lead out of the vendor's folder.
fn plain_name(name: &str) -> Option<&str> {
    let plain = !name.is_empty()
        && !name.contains(['/', '\\'])
        && name != "."
        && name != ".."
        && !name.contains(':');
    plain.then_some(name)
}

/// Every printer model the vendors' profiles describe, from each vendor's
/// own list (`machine_model_list` in `<Vendor>.json`).
fn models_in(dir: &Path) -> Models {
    let mut models = Models::new();
    let mut vendors: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|entry| Some(entry.ok()?.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect()
        })
        .unwrap_or_default();
    // In order of name, so two vendors with a model of one name always
    // settle the same way: the first has it.
    vendors.sort();
    for path in vendors {
        let Some(vendor) = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
        else {
            continue;
        };
        let Some(listed) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            continue;
        };
        for model in listed["machine_model_list"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let (Some(name), Some(file)) = (model["name"].as_str(), model["sub_path"].as_str()) {
                models
                    .entry(name.to_owned())
                    .or_insert_with(|| (vendor.clone(), file.to_owned()));
            }
        }
    }
    models
}

/// The files a printer's vendor draws its bed with.
#[derive(Default)]
struct BedFiles {
    /// An STL of the bed, its origin at the middle of the printable area.
    model: Option<PathBuf>,
    /// A picture laid over the plate: an SVG or a PNG.
    texture: Option<PathBuf>,
}

impl Session {
    /// An object's shape as one mesh to look at, made once for as long
    /// as any model holds the shape.
    fn mesh_of(&self, shape: &Arc<Shape>) -> Arc<SceneObject> {
        let key = Arc::as_ptr(shape) as usize;
        if let Some((held, mesh)) = lock(&self.scene.meshes).get(&key)
            && held.upgrade().is_some_and(|held| Arc::ptr_eq(&held, shape))
        {
            return mesh.clone();
        }
        let mesh = Arc::new(shape.mesh(""));
        let mut meshes = lock(&self.scene.meshes);
        meshes.retain(|_, (held, _)| held.strong_count() > 0);
        meshes.insert(key, (Arc::downgrade(shape), mesh.clone()));
        mesh
    }

    /// A sliced plate as moves, worked out once.
    fn paths(&self, done: &Arc<Done>) -> Arc<Paths> {
        let key = Arc::as_ptr(done) as usize;
        if let Some((held, paths)) = lock(&self.scene.paths).get(&key)
            && held.upgrade().is_some_and(|held| Arc::ptr_eq(&held, done))
        {
            return paths.clone();
        }
        let made = Arc::new(paths_of(done));
        let mut paths = lock(&self.scene.paths);
        paths.retain(|_, (held, _)| held.strong_count() > 0);
        paths.insert(key, (Arc::downgrade(done), made.clone()));
        made
    }

    /// Where the vendor of a printer keeps the model and the picture of
    /// its bed, as the reference finds them (Preset.cpp
    /// `system_printer_bed_model`, `system_printer_bed_texture`): named
    /// by the printer model's own file, beside the vendor's presets.
    fn bed_files(&self, printer: &Settings) -> BedFiles {
        let Some(dir) = lock(&self.state)
            .library
            .as_ref()
            .map(|library| library.dir().to_owned())
        else {
            return BedFiles::default();
        };
        let models = {
            let mut held = lock(&self.scene.models);
            match &*held {
                Some((from, models)) if *from == dir => models.clone(),
                _ => {
                    let models = Arc::new(models_in(&dir));
                    *held = Some((dir.clone(), models.clone()));
                    models
                }
            }
        };
        let Some((vendor, file)) = printer
            .text("printer_model")
            .and_then(|model| models.get(model))
        else {
            return BedFiles::default();
        };
        let folder = dir.join(vendor);
        let described = file
            .split('/')
            .try_fold(folder.clone(), |path, part| {
                Some(path.join(plain_name(part)?))
            })
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .unwrap_or(Value::Null);
        let beside = |key: &str, kinds: &[&str]| {
            let path = folder.join(plain_name(described[key].as_str()?)?);
            let kind = path.extension()?.to_str()?.to_ascii_lowercase();
            (kinds.contains(&kind.as_str()) && path.is_file()).then_some(path)
        };
        BedFiles {
            model: beside("bed_model", &["stl"]),
            texture: beside("bed_texture", &["svg", "png"]),
        }
    }

    /// The printer a project would be sliced with, and its file.
    pub(super) fn stage_of(&self, args: &Value) -> Result<Stage, String> {
        let (id, choices, file, plate) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.choices.clone(),
                project.file.clone(),
                project.plate,
            )
        };
        let printer = self
            .printer_of(&choices, file.as_deref())?
            .map(|(_, settings)| settings);
        Ok(Stage {
            id,
            file,
            plate,
            printer,
        })
    }

    /// A project as a scene, for Prepare: the bed of its printer, its
    /// plates where they stand, each object once and each copy's place.
    fn scene(&self, args: &Value) -> Result<Value, String> {
        let stage = self.stage_of(args)?;
        Ok(self.scene_for(&stage))
    }

    /// The scene of a project as it stands.
    pub(super) fn scene_for(&self, stage: &Stage) -> Value {
        let printer = stage.printer.as_deref();
        let bed = printer.and_then(bed_of);
        let files = printer
            .map(|printer| self.bed_files(printer))
            .unwrap_or_default();
        let model = stage.file.as_ref().map(|file| &file.model);
        let plates = model.map(|model| model.plates()).unwrap_or_default();
        let objects = model.map(|model| model.objects()).unwrap_or_default();
        // The plates in their grid, as the reference lays them out: a
        // fifth of a bed apart, the bed taken in whole millimetres.
        let grid = bed.map(Bed::of);
        let origin = |index: usize| match &grid {
            Some(grid) => grid.origin(index, plates.len()),
            None => (0.0, 0.0),
        };
        let mm = |value: f64| (value * 1e3).round() / 1e3;
        let meshes: Vec<Arc<SceneObject>> = objects
            .iter()
            .map(|object| self.mesh_of(object.shape()))
            .collect();

        // A bare model has no place of its own: the middle of the bed,
        // standing on it, which is where slicing puts it.
        let bare = model.is_some_and(|model| model.bare());
        let lone = meshes
            .first()
            .and_then(|object| object.bounds)
            .filter(|_| bare);
        let settle = match (lone, bed) {
            (Some((low, high)), Some([x0, y0, x1, y1])) => [
                (x0 + x1) / 2.0 - f64::from(low[0] + high[0]) / 2.0,
                (y0 + y1) / 2.0 - f64::from(low[1] + high[1]) / 2.0,
                -f64::from(low[2]),
            ],
            (Some((low, _)), None) => [0.0, 0.0, -f64::from(low[2])],
            _ => [0.0; 3],
        };

        let mut placings: Vec<Value> = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            for copy in object.copies() {
                let Some((transform, reach)) =
                    model.and_then(|model| model.in_scene(grid.as_ref(), copy.id()))
                else {
                    continue;
                };
                let plate = model.and_then(|model| model.plate_of(copy.id()));
                let rows = transform.rows;
                let at: [f64; 3] = std::array::from_fn(|axis| rows[3][axis] + settle[axis]);
                // Its corners where it stands in the scene.
                let reach = reach.map(|corners| {
                    corners.map(|corner| {
                        std::array::from_fn::<_, 3, _>(|axis| mm(corner[axis] + settle[axis]))
                    })
                });
                placings.push(json!({
                    // A number that stays the copy's through every edit.
                    "id": copy.id(),
                    "object": index,
                    "plate": plate.map(|index| plates[index].id()),
                    "printable": copy.printable(),
                    // Column by column, as the graphics card takes a
                    // matrix: the object's coordinates to the scene's.
                    "matrix": [
                        rows[0][0], rows[0][1], rows[0][2], 0.0,
                        rows[1][0], rows[1][1], rows[1][2], 0.0,
                        rows[2][0], rows[2][1], rows[2][2], 0.0,
                        at[0], at[1], at[2], 1.0,
                    ],
                    "box": reach,
                    // Reaching over the edge of its plate, which cannot be
                    // printed until it is wholly on the plate or off it.
                    "over": model.is_some_and(|model| model.overhangs(grid.as_ref(), copy.id())),
                }));
            }
        }
        let listed: Vec<Value> = objects
            .iter()
            .zip(&meshes)
            .map(|(object, mesh)| {
                let filaments: BTreeSet<u8> = mesh
                    .marks
                    .iter()
                    .filter_map(|mark| match mark {
                        Mark::Filament(filament) => Some(*filament),
                        _ => None,
                    })
                    .collect();
                // The filament the object itself is set to, when its parts agree.
                let own = object.shape().filaments();
                json!({
                    "id": object.id(),
                    "name": object.name(),
                    // The number of its mesh: the same for as long as the
                    // mesh is, whatever is done to its copies.
                    "mesh": object.shape().stamp(),
                    "vertices": mesh.vertices.len(),
                    "triangles": mesh.triangles.len(),
                    "bytes": mesh_size(mesh),
                    "filaments": filaments,
                    "filament": if own.len() == 1 { Some(own[0]) } else { None },
                    "parts": object.parts(),
                    // The filament the object itself is set to, when it is:
                    // what a part set to "default" prints in.
                    "own_filament": object.filament(),
                    // Each part, in order: a number that stays its own
                    // through every edit, its name, what it is, the filament
                    // it prints in and the one it was given itself (null:
                    // "default", its object's), where it stands in the
                    // object (its matrix, column by column, from its own
                    // coordinates to the object's), the box round its own
                    // mesh, where its triangles are in the object's mesh
                    // (the first and how many), whether "Split" can be done
                    // to it, and how many settings of its own it carries.
                    "part_list": object.shape().parts().iter().enumerate().map(|(at, part)| {
                        let rows = part.transform.rows;
                        json!({
                            "id": object.shape().part_ids()[at],
                            "name": part.name,
                            "kind": part_kind(part.kind),
                            "filament": part.filament,
                            "own": part.own_filament,
                            "matrix": [
                                rows[0][0], rows[0][1], rows[0][2], 0.0,
                                rows[1][0], rows[1][1], rows[1][2], 0.0,
                                rows[2][0], rows[2][1], rows[2][2], 0.0,
                                rows[3][0], rows[3][1], rows[3][2], 1.0,
                            ],
                            "box": part.mesh.bounds().map(|(low, high)| [low, high]),
                            "triangles": mesh.parts.get(at),
                            "split": object.shape().part_splits(at),
                            "settings": part.settings().len(),
                        })
                    }).collect::<Vec<_>>(),
                    // What "Split to objects" and "Split to parts" can be
                    // done to (GUI_ObjectList.cpp `is_splittable`).
                    "split": {
                        "objects": object.shape().splits_to_objects(),
                        "parts": object.shape().splits_to_parts(),
                    },
                })
            })
            .collect();
        let plates: Vec<Value> = plates
            .iter()
            .enumerate()
            .map(|(index, plate)| {
                let (x, y) = origin(index);
                json!({
                    "id": plate.id(),
                    "name": plate.name(),
                    "origin": [mm(x), mm(y)],
                    "locked": plate.locked(),
                })
            })
            .collect();
        let named = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
        };
        json!({
            "project": stage.id,
            // The project's own: an object's mesh is kept by the page under
            // this and the mesh's number.
            "key": format!("{}:m", stage.id),
            "bare": bare,
            "bed": {
                "box": bed,
                "shape": printer.map(bed_shape).unwrap_or_default(),
                "height": printer.and_then(|printer| printer.float("printable_height").ok().flatten()),
                "model": named(&files.model),
                "texture": named(&files.texture),
            },
            "plates": plates,
            "plate": stage.plate,
            "colours": stage.file.as_ref().and_then(|file| {
                file.model.underlying().map(|_| file.model.filament_colours())
            }),
            "objects": listed,
            "placings": placings,
        })
    }

    /// What `toolpath_moves` will give for a sliced plate.
    fn toolpath(&self, args: &Value) -> Result<Value, String> {
        let (done, plate) = self.done(args)?;
        let run = {
            let state = lock(&self.state);
            state.projects[state.target(args)?].run
        };
        let mut index = self.paths(&done).index.clone();
        index["plate"] = json!(plate);
        index["run"] = json!(run);
        Ok(index)
    }

    /// Lines of a sliced plate's G-code, for the G-code window: from line
    /// `from` (counting from 1) on, `count` of them, at most
    /// [`GCODE_LINES_AT_ONCE`], each as the window shows it.
    fn gcode_lines(&self, args: &Value) -> Result<Value, String> {
        let (done, _) = self.done(args)?;
        let paths = self.paths(&done);
        let gcode = done.gcode.as_ref().map_err(String::clone)?;
        let total = paths.line_ends.len();
        let from = (super::whole(args, "from")? as usize).max(1);
        if from > total + 1 {
            return Err(format!("there is no line {from}; there are {total}"));
        }
        let count = (super::whole(args, "count")? as usize)
            .min(GCODE_LINES_AT_ONCE)
            .min(total + 1 - from);
        let lines: Vec<String> = (from..from + count)
            .map(|line| {
                let start = if line == 1 {
                    0
                } else {
                    paths.line_ends[line - 2]
                };
                shown_line(&gcode.text[start..paths.line_ends[line - 1]])
            })
            .collect();
        Ok(json!({ "total": total, "from": from, "lines": lines }))
    }

    pub(super) fn scene_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "scene" => self.scene(args),
            "toolpath" => self.toolpath(args),
            "gcode_lines" => self.gcode_lines(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    pub(super) fn scene_bytes(&self, name: &str, args: &Value) -> Result<Vec<u8>, String> {
        match name {
            "scene_mesh" => {
                let stage = self.stage_of(args)?;
                let file = stage
                    .file
                    .ok_or("the project is empty: it has no objects")?;
                let objects = file.model.objects();
                // By the mesh's own number, as `scene` gives it, or by the
                // object's place in the list.
                let object = match args.get("mesh").and_then(Value::as_u64) {
                    Some(mesh) => objects
                        .iter()
                        .find(|object| object.shape().stamp() == mesh)
                        .ok_or_else(|| format!("no object has the mesh {mesh} any more"))?,
                    None => {
                        let index = super::whole(args, "object")? as usize;
                        objects.get(index).ok_or_else(|| {
                            format!("there is no object {index}; there are {}", objects.len())
                        })?
                    }
                };
                Ok(mesh_bytes(&self.mesh_of(object.shape())))
            }
            "bed_model" => {
                let stage = self.stage_of(args)?;
                let path = stage
                    .printer
                    .as_deref()
                    .and_then(|printer| self.bed_files(printer).model)
                    .ok_or("the printer's profile has no model of its bed")?;
                let scene = edge_engine::open(&path, None)
                    .map_err(|e| e.to_string())?
                    .scene();
                Ok(mesh_bytes(&scene.objects[0]))
            }
            "bed_texture" => {
                let stage = self.stage_of(args)?;
                let path = stage
                    .printer
                    .as_deref()
                    .and_then(|printer| self.bed_files(printer).texture)
                    .ok_or("the printer's profile has no picture of its bed")?;
                std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
            "toolpath_moves" => {
                let (done, _) = self.done(args)?;
                let paths = self.paths(&done);
                let total = paths.records.len() / MOVE_SIZE;
                let from = super::whole(args, "from")? as usize;
                if from > total {
                    return Err(format!("there is no move {from}; there are {total}"));
                }
                let count = (super::whole(args, "count")? as usize)
                    .min(total - from)
                    .min(MOVES_AT_ONCE);
                Ok(moves_bytes(&paths.records, from, count))
            }
            "toolpath_lines" => {
                let (done, _) = self.done(args)?;
                let paths = self.paths(&done);
                let total = paths.lines.len();
                let from = super::whole(args, "from")? as usize;
                if from > total {
                    return Err(format!("there is no move {from}; there are {total}"));
                }
                let count = (super::whole(args, "count")? as usize)
                    .min(total - from)
                    .min(MOVES_AT_ONCE);
                Ok(lines_bytes(&paths.lines, from, count))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }
}

/// The line of the G-code each of a stretch of moves was read from, as
/// bytes. All numbers little-endian:
///
/// | at | size | what |
/// | --- | --- | --- |
/// | 0 | 4 | the letters `EDGL` |
/// | 4 | 4 | `u32` version, 1 |
/// | 8 | 4 | `u32` the index of the first move here |
/// | 12 | 4 | `u32` N, how many moves are here: what was asked for, less at the end, and never more than 262144 |
/// | 16 | 4 | `u32` how many moves have a line in all (none for a print that has no G-code) |
/// | 20 | 4 | `u32` the size of one entry, 4 |
/// | 24 | 4·N | `u32` each move's line, counting from 1 |
fn lines_bytes(lines: &[u32], from: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(MOVES_HEADER + count * 4);
    out.extend_from_slice(LINES_MAGIC);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(from as u32).to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    out.extend_from_slice(&(lines.len() as u32).to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    for line in &lines[from..from + count] {
        out.extend_from_slice(&line.to_le_bytes());
    }
    out
}

/// A stretch of moves as bytes. All numbers little-endian:
///
/// | at | size | what |
/// | --- | --- | --- |
/// | 0 | 4 | the letters `EDGT` |
/// | 4 | 4 | `u32` version, 1 |
/// | 8 | 4 | `u32` the index of the first move here |
/// | 12 | 4 | `u32` N, how many moves are here: what was asked for, less at the end, and never more than 262144 |
/// | 16 | 4 | `u32` how many moves the print has in all |
/// | 20 | 4 | `u32` the size of one move, 32 |
/// | 24 | 32·N | the moves, each as [`move_bytes`] has it |
fn moves_bytes(records: &[u8], from: usize, count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(MOVES_HEADER + count * MOVE_SIZE);
    out.extend_from_slice(MOVES_MAGIC);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(from as u32).to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    out.extend_from_slice(&((records.len() / MOVE_SIZE) as u32).to_le_bytes());
    out.extend_from_slice(&(MOVE_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&records[from * MOVE_SIZE..(from + count) * MOVE_SIZE]);
    out
}

/// What a part is, as the list names it (the reference's
/// `ModelVolumeType`).
pub(super) fn part_kind(kind: edge_engine::PartKind) -> &'static str {
    use edge_engine::PartKind;
    match kind {
        PartKind::Normal => "part",
        PartKind::Negative => "negative",
        PartKind::Modifier => "modifier",
        PartKind::SupportBlocker => "support_blocker",
        PartKind::SupportEnforcer => "support_enforcer",
        PartKind::PreciseSeam => "precise_seam",
    }
}

/// A part's kind from the name [`part_kind`] gives it.
pub(super) fn kind_of(name: &str) -> Option<edge_engine::PartKind> {
    use edge_engine::PartKind;
    [
        PartKind::Normal,
        PartKind::Negative,
        PartKind::Modifier,
        PartKind::SupportBlocker,
        PartKind::SupportEnforcer,
        PartKind::PreciseSeam,
    ]
    .into_iter()
    .find(|kind| part_kind(*kind) == name)
}

/// A project as the 3D views need it.
pub(super) struct Stage {
    pub(super) id: u32,
    pub(super) file: Option<Arc<File>>,
    pub(super) plate: Option<u32>,
    pub(super) printer: Option<Arc<Settings>>,
}

impl Stage {
    /// The bed its plates are laid out for; none with no printer chosen.
    pub(super) fn bed(&self) -> Option<Bed> {
        self.printer.as_deref().and_then(bed_of).map(Bed::of)
    }
}

#[cfg(test)]
mod tests;
