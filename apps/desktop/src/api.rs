//! What the page can ask for, by name, with JSON in and JSON out. Nothing
//! here knows about Tauri: the window and the `--serve` mode both come
//! through [`Session::call`].

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use edge_engine::{Chosen, Gcode, Library, Overrides, Shelf, Sliced, Stage, Watcher};
use edge_extrusion::Role;
use edge_gcode::role_name;
use edge_geom::unscaled;
use serde_json::{Value, json};

/// One window's worth of state: where the profiles are, the last slice.
pub struct Session {
    state: Mutex<State>,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    /// A model named when the app was started, for the page to open.
    startup: Option<PathBuf>,
    /// Whether each call is noted on standard error (`EDGE_DESKTOP_TRACE`).
    traced: bool,
}

#[derive(Default)]
struct State {
    library: Option<Library>,
    shelves: HashMap<String, Arc<Shelf>>,
    done: Option<Arc<Done>>,
}

/// A finished slice, and its G-code or why there is none.
struct Done {
    sliced: Sliced,
    gcode: Result<Gcode, String>,
    model: PathBuf,
}

#[derive(Clone, Default)]
struct Progress {
    /// Counts slices started, so a late answer is not taken for a new one.
    run: u64,
    /// idle, slicing, done, failed or cancelled.
    state: &'static str,
    /// What is being done now.
    label: &'static str,
    /// How many of the steps have started, out of `STEPS`.
    step: usize,
    error: Option<String>,
}

/// Reading the model, the engine's stages, and writing the G-code.
const STEPS: usize = Stage::ALL.len() + 2;

struct Watch {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
}

impl Watcher for Watch {
    fn stage(&self, stage: Stage) {
        let mut progress = lock(&self.progress);
        progress.label = stage.label();
        progress.step = 2 + Stage::ALL.iter().position(|s| *s == stage).unwrap_or(0);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A lock that a panic elsewhere does not take away: what it guards is
/// plain data, still good to read.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn text<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn needed<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    text(args, key).ok_or_else(|| format!("{key} was not given"))
}

/// The profiles this build was made beside, when they are still there:
/// the reference's, in the repository.
fn profiles_beside() -> Option<PathBuf> {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer/resources/profiles");
    dir.is_dir().then(|| dir.canonicalize().unwrap_or(dir))
}

impl Session {
    pub fn new(startup: Option<PathBuf>) -> Self {
        let dir = std::env::var_os("EDGE_PROFILES")
            .map(PathBuf::from)
            .or_else(profiles_beside);
        Self {
            state: Mutex::new(State {
                library: dir.map(Library::new),
                ..State::default()
            }),
            progress: Arc::new(Mutex::new(Progress {
                state: "idle",
                ..Progress::default()
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            startup,
            traced: std::env::var_os("EDGE_DESKTOP_TRACE").is_some(),
        }
    }

    /// Answers the page. An error is a sentence to show as it is.
    pub fn call(self: &Arc<Self>, name: &str, args: &Value) -> Result<Value, String> {
        let answer = self.answer(name, args);
        if self.traced && name != "status" {
            match &answer {
                Ok(_) => eprintln!("call {name}: ok"),
                Err(error) => eprintln!("call {name}: {error}"),
            }
        }
        answer
    }

    fn answer(self: &Arc<Self>, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "startup" => Ok(json!({ "model": self.startup })),
            "library" => self.library(),
            "set_profiles" => {
                let dir = PathBuf::from(needed(args, "dir")?);
                if !dir.is_dir() {
                    return Err(format!("{} is not a folder", dir.display()));
                }
                let mut state = lock(&self.state);
                state.library = Some(Library::new(dir));
                state.shelves.clear();
                drop(state);
                self.library()
            }
            "machines" => {
                let shelf = self.shelf(needed(args, "vendor")?)?;
                Ok(json!({ "machines": shelf.machines() }))
            }
            "presets" => {
                let shelf = self.shelf(needed(args, "vendor")?)?;
                let machine = needed(args, "machine")?;
                Ok(json!({
                    "processes": shelf.processes_for(machine),
                    "filaments": shelf.filaments_for(machine),
                }))
            }
            "open" => open(Path::new(needed(args, "path")?)),
            "slice" => self.slice(args),
            "cancel" => {
                self.cancel.store(true, Ordering::Relaxed);
                Ok(Value::Null)
            }
            "status" => Ok(self.status()),
            "summary" => Ok(summary(&*self.done()?)),
            "layer" => {
                let index = args
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or("index was not given")? as usize;
                layer(&self.done()?.sliced, index)
            }
            "export" => {
                let done = self.done()?;
                let gcode = done.gcode.as_ref().map_err(String::clone)?;
                let path = Path::new(needed(args, "path")?);
                std::fs::write(path, &gcode.text)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(json!({ "path": path, "bytes": gcode.text.len() }))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    fn library(&self) -> Result<Value, String> {
        let state = lock(&self.state);
        Ok(match &state.library {
            None => json!({ "dir": null, "vendors": [] }),
            Some(library) => json!({
                "dir": library.dir(),
                "vendors": library.vendors().map_err(|e| e.to_string())?,
            }),
        })
    }

    /// A vendor's presets, read once and kept.
    fn shelf(&self, vendor: &str) -> Result<Arc<Shelf>, String> {
        let mut state = lock(&self.state);
        if let Some(shelf) = state.shelves.get(vendor) {
            return Ok(shelf.clone());
        }
        let library = state
            .library
            .as_ref()
            .ok_or("no profiles folder has been chosen")?;
        let shelf = Arc::new(library.shelf(vendor).map_err(|e| e.to_string())?);
        state.shelves.insert(vendor.to_owned(), shelf.clone());
        Ok(shelf)
    }

    fn done(&self) -> Result<Arc<Done>, String> {
        lock(&self.state)
            .done
            .clone()
            .ok_or_else(|| "nothing has been sliced yet".into())
    }

    fn status(&self) -> Value {
        let progress = lock(&self.progress).clone();
        json!({
            "run": progress.run,
            "state": progress.state,
            "label": progress.label,
            "step": progress.step,
            "steps": STEPS,
            "error": progress.error,
        })
    }

    /// Starts a slice on a thread of its own and comes straight back;
    /// `status` says how it is going.
    fn slice(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        let path = PathBuf::from(needed(args, "path")?);
        let plate = args
            .get("plate")
            .and_then(Value::as_u64)
            .map(|plate| plate as u32);
        // The presets are found before anything starts, so a name that
        // is not there is said at once.
        let chosen = match text(args, "vendor") {
            None => Chosen::default(),
            Some(vendor) => self
                .shelf(vendor)?
                .choose(
                    text(args, "machine"),
                    text(args, "process"),
                    text(args, "filament"),
                )
                .map_err(|e| e.to_string())?,
        };
        let run = {
            let mut progress = lock(&self.progress);
            if progress.state == "slicing" {
                return Err("a slice is already running".into());
            }
            *progress = Progress {
                run: progress.run + 1,
                state: "slicing",
                label: "Reading the model",
                step: 1,
                error: None,
            };
            progress.run
        };
        self.cancel.store(false, Ordering::Relaxed);
        lock(&self.state).done = None;

        let session = self.clone();
        std::thread::spawn(move || {
            let watch = Watch {
                progress: session.progress.clone(),
                cancel: session.cancel.clone(),
            };
            let work = || -> Result<Done, edge_engine::Failure> {
                let loaded = edge_engine::open(&path, plate)?.place()?;
                let job = edge_engine::prepare(loaded, &chosen, &Overrides::default())?;
                let sliced = edge_engine::slice(job, &watch)?;
                {
                    let mut progress = lock(&session.progress);
                    progress.label = "Writing G-code";
                    progress.step = STEPS;
                }
                let gcode = sliced
                    .gcode(&edge_engine::timestamp())
                    .map_err(|e| e.to_string());
                Ok(Done {
                    sliced,
                    gcode,
                    model: path.clone(),
                })
            };
            let outcome = work();
            let mut progress = lock(&session.progress);
            match outcome {
                Ok(done) => {
                    lock(&session.state).done = Some(Arc::new(done));
                    progress.state = "done";
                }
                Err(error) if error.is::<edge_engine::Cancelled>() => {
                    progress.state = "cancelled";
                }
                Err(error) => {
                    progress.state = "failed";
                    progress.error = Some(error.to_string());
                }
            }
        });
        Ok(json!({ "run": run }))
    }
}

/// What a model file is, before slicing it: its plates and the presets
/// it names.
fn open(path: &Path) -> Result<Value, String> {
    let opened = edge_engine::open(path, None).map_err(|e| e.to_string())?;
    let settings = opened.settings();
    let named = |key: &str| settings.and_then(|settings| settings.text(key));
    Ok(json!({
        "path": path,
        "name": path.file_name().map(|name| name.to_string_lossy()),
        "plates": opened.plates(),
        "plate": opened.plate().map(|(id, _)| id),
        "has_settings": settings.is_some(),
        "printer": named("printer_settings_id"),
        "process": named("print_settings_id"),
        "filaments": settings.and_then(|settings| settings.texts("filament_settings_id")),
    }))
}

/// The feature names in the order a legend lists them.
const LEGEND: [Role; 15] = [
    Role::ExternalPerimeter,
    Role::Perimeter,
    Role::OverhangPerimeter,
    Role::TopSolidInfill,
    Role::BottomSurface,
    Role::SolidInfill,
    Role::SparseInfill,
    Role::BridgeInfill,
    Role::InternalBridge,
    Role::GapFill,
    Role::Support,
    Role::SupportInterface,
    Role::Skirt,
    Role::Brim,
    Role::PrimeTower,
];

fn summary(done: &Done) -> Value {
    let sliced = &done.sliced;
    let print = &sliced.print;
    // The features in the print, without copying a line of it.
    let mut present: Vec<&'static str> = Vec::new();
    {
        let loops = sliced
            .walls_by
            .iter()
            .flatten()
            .flat_map(|layer| &layer.loops)
            .chain(sliced.around.iter().flatten())
            .map(|line| line.role);
        let paths = sliced
            .fills_by
            .iter()
            .flatten()
            .flatten()
            .chain(sliced.supports.iter().flatten())
            .map(|line| line.role);
        let tower = sliced
            .tower
            .layers
            .iter()
            .any(|layer| !layer.finish.is_empty())
            .then_some(Role::PrimeTower);
        let mut seen = [false; LEGEND.len()];
        for role in loops.chain(paths).chain(tower) {
            let name = role_name(role);
            if let Some(at) = LEGEND.iter().position(|known| role_name(*known) == name) {
                seen[at] = true;
            }
        }
        for (role, seen) in LEGEND.iter().zip(seen) {
            if seen {
                present.push(role_name(*role));
            }
        }
    }

    let mut notes: Vec<String> = Vec::new();
    if print.machine_start_gcode.is_some() {
        notes.push("Start, end and layer-change G-code come from the printer profile.".to_owned());
    } else {
        notes.push(
            "No printer profile: the G-code has a generic preamble and is only good for viewing."
                .to_owned(),
        );
    }
    let leaky = sliced
        .layers
        .iter()
        .filter(|layer| layer.open_chains > 0)
        .count();
    if leaky > 0 {
        notes.push(format!(
            "{leaky} layers have open contours; the mesh is not watertight there."
        ));
    }
    if let Some(pattern) = &sliced.infill.unsupported_pattern {
        notes.push(format!(
            "Sparse pattern \"{pattern}\" is not implemented; grid was used in its place."
        ));
    }
    let gaps: usize = sliced
        .walls_by
        .iter()
        .flatten()
        .map(|layer| layer.gaps_skipped)
        .sum();
    if gaps > 0 {
        notes.push(format!(
            "{gaps} gap(s) between walls were left empty; their centre line could not be worked out."
        ));
    }
    notes.extend(sliced.notes().cloned());
    let warnings: Vec<String> = sliced
        .without_a_tool()
        .map(|filament| {
            format!(
                "Filament {filament} is used, but the printer has {} tools; T{} is written, as the C++ \
                 slicer writes it, and the printer may not accept it.",
                print.tools,
                filament - 1
            )
        })
        .collect();

    let gcode = match &done.gcode {
        Err(error) => json!({ "error": error }),
        Ok(gcode) => {
            let summary = gcode.summary;
            json!({
                "bytes": gcode.text.len(),
                "filament_m": summary.filament_mm / 1e3,
                "grams": print.filament_density.map(|density| summary.filament_mm3 / 1e3 * density),
                "travel_m": summary.travel_mm / 1e3,
                "minutes": summary.seconds / 60.0,
                "retractions": summary.retractions,
                "tool_changes": summary.tool_changes,
                "slowed_layers": summary.slowed_layers,
            })
        }
    };
    let timings = sliced.timings;
    let ms = |time: std::time::Duration| time.as_secs_f64() * 1e3;
    json!({
        "model": done.model.file_name().map(|name| name.to_string_lossy()),
        "stem": done.model.file_stem().map(|name| name.to_string_lossy()),
        "layers": sliced.layers.len(),
        "layer_height": sliced.layer_height,
        "first_layer_height": sliced.first_layer_height,
        "bed": print.bed,
        "printer_ready": print.machine_start_gcode.is_some(),
        "filaments": sliced.used,
        "colours": sliced.settings.texts("filament_colour"),
        "roles": present,
        "notes": notes,
        "warnings": warnings,
        "gcode": gcode,
        "ms": {
            "layers": ms(timings.slice),
            "walls": ms(timings.perimeters),
            "supports": ms(timings.supports),
            "surfaces": ms(timings.surfaces),
            "fill": ms(timings.fill),
        },
    })
}

/// One layer's lines: feature, width in mm, filament, whether closed, and
/// the points as x, y, x, y in mm.
fn layer(sliced: &Sliced, index: usize) -> Result<Value, String> {
    if index >= sliced.layers.len() {
        return Err(format!(
            "there is no layer {index}; there are {}",
            sliced.layers.len()
        ));
    }
    // To the micrometre: finer than a screen shows and a third the text.
    let mm = |units| (unscaled(units) * 1e3).round() / 1e3;
    let strokes: Vec<Value> = sliced
        .strokes(index)
        .iter()
        .map(|stroke| {
            let points: Vec<f64> = stroke
                .points
                .iter()
                .flat_map(|point| [mm(point.x), mm(point.y)])
                .collect();
            json!({
                "r": role_name(stroke.role),
                "w": (stroke.width * 1e3).round() / 1e3,
                "f": stroke.filament,
                "c": stroke.closed,
                "p": points,
            })
        })
        .collect();
    Ok(json!({
        "index": index,
        "z": (sliced.top_of(index) * 1e3).round() / 1e3,
        "height": sliced.height_of(index),
        "strokes": strokes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(path: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer")
            .join(path)
            .to_string_lossy()
            .into_owned()
    }

    fn wait(session: &Arc<Session>) -> Value {
        loop {
            let status = session.call("status", &Value::Null).unwrap();
            if status["state"] != "slicing" {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_project_is_opened_sliced_looked_through_and_exported() {
        let session = Arc::new(Session::new(None));
        assert!(session.call("summary", &Value::Null).is_err());
        let path = reference("tests/data/support_corpus/twopart_groups.3mf");
        let opened = session.call("open", &json!({ "path": path })).unwrap();
        assert_eq!(opened["has_settings"], true);
        assert_eq!(opened["name"], "twopart_groups.3mf");

        session.call("slice", &json!({ "path": path })).unwrap();
        let status = wait(&session);
        assert_eq!(status["state"], "done", "{status}");
        assert_eq!(status["step"], status["steps"]);

        let summary = session.call("summary", &Value::Null).unwrap();
        assert_eq!(summary["layers"], 60);
        assert!(summary["gcode"]["filament_m"].as_f64().unwrap() > 0.5);
        let roles = summary["roles"].as_array().unwrap();
        assert!(roles.contains(&json!("Outer wall")) && roles.contains(&json!("Bridge")));

        let layer = session.call("layer", &json!({ "index": 3 })).unwrap();
        let strokes = layer["strokes"].as_array().unwrap();
        assert!(!strokes.is_empty());
        assert!(
            strokes
                .iter()
                .all(|stroke| stroke["p"].as_array().unwrap().len() >= 4)
        );
        assert!(session.call("layer", &json!({ "index": 60 })).is_err());

        let out = std::env::temp_dir().join(format!("edge-desktop-{}.gcode", std::process::id()));
        let written = session.call("export", &json!({ "path": out })).unwrap();
        let text = std::fs::read(&out).unwrap();
        std::fs::remove_file(&out).unwrap();
        assert_eq!(written["bytes"], text.len());
        assert!(text.starts_with(b"; HEADER_BLOCK_START"));
    }

    #[test]
    fn a_bare_model_is_sliced_with_presets_picked_from_the_lists() {
        let session = Arc::new(Session::new(None));
        let library = session.call("library", &Value::Null).unwrap();
        assert!(
            library["vendors"]
                .as_array()
                .unwrap()
                .contains(&json!("Snapmaker"))
        );
        let machine = "Snapmaker U1 (0.4 nozzle)";
        let machines = session
            .call("machines", &json!({ "vendor": "Snapmaker" }))
            .unwrap();
        assert!(
            machines["machines"]
                .as_array()
                .unwrap()
                .contains(&json!(machine))
        );
        let presets = session
            .call(
                "presets",
                &json!({ "vendor": "Snapmaker", "machine": machine }),
            )
            .unwrap();
        let process = presets["processes"][0].as_str().unwrap();
        let filament = presets["filaments"][0].as_str().unwrap();

        // A failure is said, and leaves nothing behind to look at.
        let missing = json!({ "path": reference("tests/no_such_model.stl") });
        session.call("slice", &missing).unwrap();
        let status = wait(&session);
        assert_eq!(status["state"], "failed");
        assert!(status["error"].as_str().unwrap().contains("no_such_model"));
        assert!(session.call("summary", &Value::Null).is_err());
        // So is a preset that is not there, before anything starts.
        let unknown = json!({ "path": "x.stl", "vendor": "Snapmaker", "machine": "no such" });
        assert!(session.call("slice", &unknown).is_err());

        let stl = reference("resources/handy_models/OrcaToleranceTest.stl");
        let args = json!({
            "path": stl, "vendor": "Snapmaker", "machine": machine,
            "process": process, "filament": filament,
        });
        session.call("slice", &args).unwrap();
        let status = wait(&session);
        assert_eq!(status["state"], "done", "{status}");
        let summary = session.call("summary", &Value::Null).unwrap();
        assert_eq!(summary["printer_ready"], true);
        assert!(summary["bed"].is_array());
    }
}
