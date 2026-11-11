//! What the page can ask for, by name, with JSON in and JSON out. Nothing
//! here knows about Tauri: the window and the `--serve` mode both come
//! through [`Session::call`].
//!
//! A window holds several projects at once, as the reference's project
//! strip does (ProjectStrip.cpp, ProjectSessions.hpp): each has its file,
//! the plate it is on, the presets chosen for it, a slice of each plate
//! and where its Preview was left. One of them is the active one; a call
//! that names no project means that one. A slice runs on a thread of its
//! own for its project, so another project can be looked at, or sliced,
//! meanwhile.
//!
//! What the 3D views ask for is in [`scene`]. Its meshes and moves are too
//! large for JSON and are answered as bytes, through
//! [`Session::call_bytes`], which is `call` in every other way.

mod edit;
mod scene;

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use edge_config::Settings;
use edge_engine::{
    BedType, Chosen, Gcode, Library, Model, Overrides, PlateInfo, Shelf, Sliced, Stage, Watcher,
};
use edge_extrusion::Role;
use edge_gcode::role_name;
use edge_geom::{Point2, unscaled};
use serde_json::{Value, json};

/// The most projects a window keeps open (ProjectSessions.hpp
/// `default_max_open`): each one stays loaded, slices included.
pub const MAX_OPEN: usize = 8;

/// What a project that has no file yet goes by.
const UNTITLED: &str = "Untitled";

/// The longest note of where a Preview was left that is kept.
const LONGEST_VIEW: usize = 4096;

/// One window's worth of state: where the profiles are, the open projects.
pub struct Session {
    state: Mutex<State>,
    /// A model named when the app was started, for the page to open.
    startup: Option<PathBuf>,
    /// Whether each call is noted on standard error (`EDGE_DESKTOP_TRACE`).
    traced: bool,
    /// What the 3D views' calls keep between them.
    scene: scene::Cache,
}

struct State {
    library: Option<Library>,
    shelves: HashMap<String, Arc<Shelf>>,
    /// Each vendor's printer models, read once and kept.
    catalogue: HashMap<String, Arc<Vec<edge_engine::PrinterModel>>>,
    /// What is kept between runs: the printers added, what was last used.
    store: store::Store,
    /// The open projects, in the order the strip shows them. Never empty.
    projects: Vec<Project>,
    /// The id of the project shown.
    active: u32,
    next_id: u32,
}

/// One open project.
struct Project {
    id: u32,
    /// What the strip shows it as.
    name: String,
    /// Its model as it stands; none for one that is new and empty.
    file: Option<Arc<File>>,
    /// Its model as it was opened: it is unsaved once the two differ.
    opened: Option<Arc<File>>,
    /// The edits made to its model, to take back, and those taken back.
    history: edit::History,
    /// The plate it is on, by number; none for a file without plates.
    plate: Option<u32>,
    choices: Choices,
    /// The choices it was opened with: it is unsaved once they differ.
    baseline: Choices,
    /// Prepare or Preview, whichever it was last on.
    tab: &'static str,
    /// Where its Preview was left, as the page noted it.
    view: Value,
    /// Counts its slices, so a late answer is not taken for a new one.
    run: u64,
    /// Its slice, running or last run.
    work: Option<Arc<Work>>,
    /// The plates sliced, by number; 0 for a file without plates.
    results: BTreeMap<u32, Arc<Done>>,
    /// Why a plate's slice failed, for the plates that did.
    failures: BTreeMap<u32, String>,
    /// What stands on each plate looked at, seen from above.
    footprints: HashMap<u32, Arc<Vec<Footprint>>>,
}

/// A project's model as it stands at one moment: as it was opened, or as
/// an edit left it. An edit makes another; this one is never changed, so
/// a slice that is running, and every step of the undo history, keeps the
/// one it has.
struct File {
    /// What it was opened from; none for one put together in the window.
    path: Option<PathBuf>,
    /// Its plates; empty for a bare model or a project without plates,
    /// which is sliced whole.
    plates: Vec<PlateInfo>,
    /// The filaments all of it prints in, by number from 1.
    used: Vec<usize>,
    /// The settings a project was saved with.
    settings: Option<Arc<Settings>>,
    /// The plates, the objects and where each copy stands.
    model: Model,
}

impl File {
    /// A model with what the page lists of it read off.
    fn of(model: Model) -> Self {
        Self {
            path: model.path().map(Path::to_owned),
            plates: model.plate_list(),
            used: model.filaments(),
            settings: model.settings().cloned(),
            model,
        }
    }

    /// What it goes by where a file is named: its own, else the name of a
    /// project that has none.
    fn named(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(|| UNTITLED.into())
    }
}

/// What a project is sliced with, as chosen in its sidebar.
#[derive(Clone, Debug, Default, PartialEq)]
struct Choices {
    /// Presets chosen here, rather than the project's own settings.
    from_presets: bool,
    vendor: Option<String>,
    machine: Option<String>,
    process: Option<String>,
    /// A preset for filament 1, filament 2 and so on.
    filaments: Vec<String>,
    bed_type: Option<String>,
    layer_height: Option<f64>,
    first_layer_height: Option<f64>,
    wall_loops: Option<usize>,
    closing_radius: Option<f64>,
}

impl Choices {
    /// Whether two sets of choices slice alike. Presets that are picked
    /// but not in use (the project's own settings are) do not count.
    fn settle_as(&self, other: &Choices) -> bool {
        let over = |choices: &Choices| {
            (
                choices.bed_type.clone(),
                choices.layer_height,
                choices.first_layer_height,
                choices.wall_loops,
                choices.closing_radius,
            )
        };
        match (self.from_presets, other.from_presets) {
            (false, false) => over(self) == over(other),
            _ => self == other,
        }
    }

    fn overrides(&self) -> Overrides {
        Overrides {
            layer_height: self.layer_height,
            first_layer_height: self.first_layer_height,
            closing_radius: self.closing_radius,
            wall_loops: self.wall_loops,
            bed_type: self.bed_type.clone(),
        }
    }

    fn json(&self) -> Value {
        json!({
            "source": if self.from_presets { "presets" } else { "project" },
            "vendor": self.vendor,
            "machine": self.machine,
            "process": self.process,
            "filaments": self.filaments,
            "bed_type": self.bed_type,
            "layer_height": self.layer_height,
            "first_layer_height": self.first_layer_height,
            "wall_loops": self.wall_loops,
            "closing_radius": self.closing_radius,
        })
    }
}

/// A finished slice, and its G-code or why there is none.
struct Done {
    sliced: Sliced,
    gcode: Result<Gcode, String>,
    model: PathBuf,
    /// How the bed surface was settled when none was chosen.
    bed_type: BedType,
}

/// One object on a plate seen from above: its outline in mm where the
/// file puts it, and how high it stands.
struct Footprint {
    name: String,
    outline: Vec<[f64; 2]>,
    /// The least and greatest x and y.
    extent: [f64; 4],
    height: f64,
}

/// One slice of a project, as its thread and the calls share it.
struct Work {
    progress: Mutex<Progress>,
    cancel: AtomicBool,
}

#[derive(Clone, Default)]
struct Progress {
    run: u64,
    /// idle, slicing, done, failed or cancelled.
    state: &'static str,
    /// What is being done now.
    label: &'static str,
    /// How many of the steps have started, out of `STEPS`.
    step: usize,
    /// The plate being sliced, and which of how many it is.
    plate: Option<u32>,
    at: usize,
    of: usize,
    error: Option<String>,
}

/// Reading the model, the engine's stages, and writing the G-code.
const STEPS: usize = Stage::ALL.len() + 2;

struct Watch<'a>(&'a Work);

impl Watcher for Watch<'_> {
    fn stage(&self, stage: Stage) {
        let mut progress = lock(&self.0.progress);
        progress.label = stage.label();
        progress.step = 2 + Stage::ALL.iter().position(|s| *s == stage).unwrap_or(0);
    }

    fn cancelled(&self) -> bool {
        self.0.cancel.load(Ordering::Relaxed)
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

/// A whole number the call must carry.
fn whole(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{key} was not given"))
}

fn shown(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The profiles this build was made beside, when they are still there:
/// the reference's, in the repository.
fn profiles_beside() -> Option<PathBuf> {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer/resources/profiles");
    dir.is_dir().then(|| dir.canonicalize().unwrap_or(dir))
}

/// Two spellings of one file are one project: separators and, on Windows,
/// case do not count (ProjectSessions.hpp `normalized_path`).
fn normalized(path: &str, case_insensitive: bool) -> String {
    let mut out: String = path
        .chars()
        .map(|c| match c {
            '\\' => '/',
            c if case_insensitive => c.to_ascii_lowercase(),
            c => c,
        })
        .collect();
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

/// What a file is shown as: its name without the folder and the last
/// extension (ProjectSessions.hpp `name_from_path`).
fn name_from_path(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(dot) if dot > 0 => name[..dot].to_owned(),
        _ => name.to_owned(),
    }
}

/// The slot a project is kept under for its plate.
fn slot(plate: Option<u32>) -> u32 {
    plate.unwrap_or(0)
}

/// The bed as the box around the corners of `printable_area`, each "XxY".
fn bed_of(settings: &Settings) -> Option<[f64; 4]> {
    let mut extent: Option<[f64; 4]> = None;
    for corner in settings.texts("printable_area")? {
        let (x, y) = corner.split_once('x')?;
        let (x, y): (f64, f64) = (x.trim().parse().ok()?, y.trim().parse().ok()?);
        let [min_x, min_y, max_x, max_y] = extent.unwrap_or([x, y, x, y]);
        extent = Some([min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)]);
    }
    extent
}

mod printers;
mod store;

/// The build plates the engine knows the bed temperature of, each with the
/// name the reference shows it by (PrintConfig.cpp `curr_bed_type`).
const PLATES: [(&str, &str); 6] = [
    ("Cool Plate", "Smooth Cool Plate"),
    ("Engineering Plate", "Engineering Plate"),
    ("High Temp Plate", "Smooth High Temp Plate"),
    ("Textured PEI Plate", "Textured PEI Plate"),
    ("Textured Cool Plate", "Textured Cool Plate"),
    ("Supertack Plate", "Cool Plate (SuperTack)"),
];

/// A Snapmaker U1's plates, as the reference names them there
/// (`enum_labels_u1`). Its Graphic Effect Plate is left out: the engine
/// has no bed temperature for it yet.
const PLATES_U1: [(&str, &str); 3] = [
    ("Textured PEI Plate", "Textured PEI Plate"),
    ("High Temp Plate", "Smooth PEI Plate"),
    ("Cool Plate", "Cool Plate"),
];

/// A U1 that takes every plate (`enum_labels_ex`), again without the
/// Graphic Effect Plate.
const PLATES_U1_ALL: [(&str, &str); 6] = [
    ("Cool Plate", "Smooth Cool Plate"),
    ("Engineering Plate", "Engineering Plate"),
    ("High Temp Plate", "Smooth PEI Plate"),
    ("Textured PEI Plate", "Textured PEI Plate"),
    ("Textured Cool Plate", "Textured Cool Plate"),
    ("Supertack Plate", "Cool Steel Plate"),
];

/// The build plates to choose from for a printer, whether there is a
/// choice at all, and the one in force when none is chosen: as the
/// reference's sidebar fills its "Bed type" list (Plater.cpp
/// `Sidebar::update_presets`). A printer that is not Bambu's, not a U1 and
/// does not say it takes several plates has its own default and no choice.
fn bed_types(printer: &Settings, vendor: Option<&str>, chosen: Option<&str>) -> Value {
    let plates = plates_for(printer, vendor);
    // A profile gives its default by name or by its number in the
    // reference's list (PrintConfig.hpp `BedType`), as `prepare` reads it.
    let default = match printer.text("default_bed_type").map(str::trim) {
        Some("1") => Some("Cool Plate"),
        Some("2") => Some("Engineering Plate"),
        Some("3") => Some("High Temp Plate"),
        Some("4") => Some("Textured PEI Plate"),
        Some("5") => Some("Textured Cool Plate"),
        Some("7") => Some("Supertack Plate"),
        Some(name) if name.ends_with("Plate") => Some(name),
        _ => None,
    };
    let in_force = chosen
        .or(printer.text("curr_bed_type"))
        .or(default)
        .unwrap_or("Cool Plate");
    json!({
        "enabled": plates.enabled,
        "in_force": in_force,
        "choices": plates
            .offered
            .iter()
            .map(|(value, label)| json!({ "value": value, "label": label }))
            .collect::<Vec<_>>(),
    })
}

/// The plates a printer's "Bed type" list offers, and whether there is a
/// choice among them.
struct Plates {
    enabled: bool,
    /// A Snapmaker U1, which has plates of its own.
    u1: bool,
    /// The printer says it takes every plate (`support_multi_bed_types`).
    several: bool,
    offered: &'static [(&'static str, &'static str)],
}

fn plates_for(printer: &Settings, vendor: Option<&str>) -> Plates {
    let model = printer.text("printer_model").unwrap_or_default();
    let name = printer.text("printer_settings_id").unwrap_or_default();
    let is_u1 = [model, name]
        .iter()
        .any(|text| text.contains("Snapmaker") && text.contains("U1"));
    let several = matches!(
        printer.text("support_multi_bed_types"),
        Some("1") | Some("true")
    );
    let is_bambu = vendor == Some("BBL") || model.starts_with("Bambu Lab");
    let offered: &'static [(&'static str, &'static str)] = match (is_u1, several) {
        (true, false) => &PLATES_U1,
        (true, true) => &PLATES_U1_ALL,
        (false, _) => &PLATES,
    };
    Plates {
        enabled: is_bambu || several || is_u1,
        u1: is_u1,
        several,
        offered,
    }
}

impl Project {
    fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            file: None,
            opened: None,
            history: edit::History::default(),
            plate: None,
            choices: Choices {
                from_presets: true,
                ..Choices::default()
            },
            baseline: Choices {
                from_presets: true,
                ..Choices::default()
            },
            tab: "prepare",
            view: json!({}),
            run: 0,
            work: None,
            results: BTreeMap::new(),
            failures: BTreeMap::new(),
            footprints: HashMap::new(),
        }
    }

    /// Whether what was chosen for it, or done to its model, since it was
    /// opened would be lost.
    fn dirty(&self) -> bool {
        let edited = match (&self.file, &self.opened) {
            (Some(now), Some(opened)) => !Arc::ptr_eq(now, opened),
            (None, None) => false,
            _ => true,
        };
        edited || !self.choices.settle_as(&self.baseline)
    }

    /// New, empty and untouched: opening a file takes its place.
    fn blank(&self) -> bool {
        self.file.is_none() && !self.dirty()
    }

    fn slicing(&self) -> bool {
        self.work
            .as_ref()
            .is_some_and(|work| lock(&work.progress).state == "slicing")
    }

    /// The slots of the plates that hold something to print.
    fn sliceable(&self) -> Vec<Option<u32>> {
        match &self.file {
            None => Vec::new(),
            Some(file) if file.plates.is_empty() => vec![None],
            Some(file) => file
                .plates
                .iter()
                .filter(|plate| !plate.filaments.is_empty())
                .map(|plate| Some(plate.id))
                .collect(),
        }
    }

    /// Puts a file in it, on the file's first plate and with nothing sliced.
    fn load(&mut self, file: Arc<File>) {
        self.name = name_from_path(&shown(&file.named()));
        // Plate 1, as the engine takes when none is named; else the first.
        self.plate = file
            .plates
            .iter()
            .find(|plate| plate.id == 1)
            .or(file.plates.first())
            .map(|plate| plate.id);
        self.choices.from_presets = file.settings.is_none();
        // A project's own bed type is in force until another is chosen
        // for it: the one that went with the printer the empty project
        // was on is not a choice made for this file.
        if file.settings.is_some() {
            self.choices.bed_type = None;
        }
        self.baseline = self.choices.clone();
        self.opened = Some(file.clone());
        self.file = Some(file);
        self.history = edit::History::default();
        self.tab = "prepare";
        self.view = json!({});
        self.footprints.clear();
        self.forget();
    }

    /// Lets go of every slice: what it would be sliced with has changed.
    fn forget(&mut self) {
        if let Some(work) = &self.work {
            work.cancel.store(true, Ordering::Relaxed);
        }
        self.results.clear();
        self.failures.clear();
    }

    /// The highest-numbered filament anything in the file prints in.
    fn filament_count(&self) -> usize {
        self.file
            .as_ref()
            .and_then(|file| file.used.iter().max().copied())
            .unwrap_or(1)
    }

    /// One line of the strip.
    fn tile(&self) -> Value {
        let sliceable = self.sliceable();
        json!({
            "id": self.id,
            "name": self.name,
            "path": self.file.as_ref().and_then(|file| file.path.as_deref()).map(shown),
            "dirty": self.dirty(),
            "empty": self.file.is_none(),
            "plate": self.plate,
            "tab": self.tab,
            // Counts the edits to its model, for what is drawn of it.
            "revision": self.history.revision(),
            "slicing": self.slicing(),
            "plates_with_objects": sliceable.len(),
            "plates_sliced": sliceable
                .iter()
                .filter(|plate| self.results.contains_key(&slot(**plate)))
                .count(),
        })
    }

    /// Everything the page shows of it.
    fn detail(&self) -> Value {
        let file = self.file.as_deref();
        let settings = file.and_then(|file| file.settings.as_deref());
        let named = |key: &str| settings.and_then(|settings| settings.text(key));
        let plates: Vec<Value> = file
            .map(|file| {
                file.plates
                    .iter()
                    .map(|plate| {
                        json!({
                            "id": plate.id,
                            "name": plate.name,
                            "filaments": plate.filaments,
                            "objects": plate.objects,
                            "sliced": self.results.contains_key(&plate.id),
                            "failed": self.failures.get(&plate.id),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut detail = self.tile();
        let more = json!({
            "file": file
                .and_then(|file| file.path.as_deref())
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy()),
            "plates": plates,
            // A file without plates is sliced whole, under plate 0.
            "sliced": self.results.contains_key(&slot(self.plate)),
            "failed": self.failures.get(&slot(self.plate)),
            "has_settings": settings.is_some(),
            // The filaments all of it prints in, by number from 1.
            "used": file.map_or(vec![1], |file| file.used.clone()),
            "filament_count": self.filament_count(),
            "colours": settings.and_then(|settings| settings.texts("filament_colour")),
            // The presets the file names itself as saved with.
            "saved": {
                "printer": named("printer_settings_id"),
                "process": named("print_settings_id"),
                "filaments": settings.and_then(|settings| settings.texts("filament_settings_id")),
            },
            "choices": self.choices.json(),
            "view": self.view,
            "edit": self.history.marks(),
            "status": status_of(self),
        });
        if let (Some(detail), Value::Object(more)) = (detail.as_object_mut(), more) {
            detail.extend(more);
        }
        detail
    }
}

fn status_of(project: &Project) -> Value {
    let progress = match &project.work {
        Some(work) => lock(&work.progress).clone(),
        None => Progress {
            state: "idle",
            ..Progress::default()
        },
    };
    json!({
        "project": project.id,
        "run": progress.run,
        "state": progress.state,
        "label": progress.label,
        "step": progress.step,
        "steps": STEPS,
        "plate": progress.plate,
        "at": progress.at,
        "of": progress.of,
        "error": progress.error,
    })
}

impl State {
    fn index_of(&self, id: u32) -> Option<usize> {
        self.projects.iter().position(|project| project.id == id)
    }

    fn active_index(&self) -> usize {
        self.index_of(self.active).unwrap_or(0)
    }

    /// The project a call is about: the one it names, else the active one.
    fn target(&self, args: &Value) -> Result<usize, String> {
        match args.get("project").filter(|id| !id.is_null()) {
            None => Ok(self.active_index()),
            Some(id) => id
                .as_u64()
                .and_then(|id| self.index_of(u32::try_from(id).ok()?))
                .ok_or_else(|| format!("there is no project {id}")),
        }
    }

    /// The project holding this file, whichever way its path is spelled.
    fn holding(&self, path: &Path) -> Option<u32> {
        let key = normalized(&shown(path), cfg!(windows));
        self.projects
            .iter()
            .find(|project| {
                project
                    .file
                    .as_ref()
                    .and_then(|file| file.path.as_deref())
                    .is_some_and(|held| normalized(&shown(held), cfg!(windows)) == key)
            })
            .map(|project| project.id)
    }

    /// "Untitled", then "Untitled 2", "Untitled 3": the lowest number no
    /// open project shows (ProjectSessions.hpp `next_untitled_name`).
    fn untitled(&self) -> String {
        let taken = |name: &str| self.projects.iter().any(|project| project.name == name);
        if !taken(UNTITLED) {
            return UNTITLED.to_owned();
        }
        (2..)
            .map(|number| format!("{UNTITLED} {number}"))
            .find(|name| !taken(name))
            .expect("some number is free")
    }

    /// A new project right after the active one, shown at once, on the
    /// printer of the project it was started from.
    fn add(&mut self, name: String) -> usize {
        let from = self.active_index();
        let mut project = Project::new(self.next_id, name);
        self.next_id += 1;
        let started_from = &self.projects[from].choices;
        // The bed type goes with the printer: the one the project it was
        // started from is on, or for one sliced with its own settings the
        // one last used with that printer.
        let bed_type = if started_from.from_presets {
            started_from.bed_type.clone()
        } else {
            started_from
                .machine
                .as_ref()
                .and_then(|machine| self.store.data.printers.get(machine))
                .and_then(|kept| kept.bed_type.clone())
        };
        project.choices = Choices {
            from_presets: true,
            vendor: started_from.vendor.clone(),
            machine: started_from.machine.clone(),
            process: started_from.process.clone(),
            filaments: started_from.filaments.clone(),
            bed_type,
            ..Choices::default()
        };
        project.baseline = project.choices.clone();
        self.active = project.id;
        self.projects.insert(from + 1, project);
        from + 1
    }

    /// The strip: every open project in order, and which is shown.
    fn listing(&self) -> Value {
        json!({
            "active": self.active,
            "max_open": MAX_OPEN,
            "projects": self.projects.iter().map(Project::tile).collect::<Vec<_>>(),
        })
    }

    /// The strip, with something more to say about the call just answered.
    fn listing_with(&self, more: Value) -> Value {
        let mut listing = self.listing();
        if let (Some(listing), Value::Object(more)) = (listing.as_object_mut(), more) {
            listing.extend(more);
        }
        listing
    }
}

/// Reads a model file for a project: its plates, what each holds and the
/// settings it was saved with.
fn read(path: &Path) -> Result<File, String> {
    Ok(File::of(Model::open(path).map_err(|e| e.to_string())?))
}

/// What stands on a plate, seen from above, where the model has it.
fn footprints(file: &File, plate: Option<u32>) -> Result<Vec<Footprint>, String> {
    let loaded = file.model.place(plate).map_err(|e| e.to_string())?;
    // A project's objects each by itself; a bare model is one.
    let solids: Vec<_> = match loaded.objects.as_slice() {
        [] => loaded.bodies.iter().map(|body| &body.solid).collect(),
        objects => objects.iter().collect(),
    };
    let names = file
        .plates
        .iter()
        .find(|listed| Some(listed.id) == plate)
        .map(|listed| listed.objects.as_slice())
        .filter(|names| names.len() == solids.len())
        .unwrap_or_default();
    let mm = |units| (unscaled(units) * 1e3).round() / 1e3;
    Ok(solids
        .iter()
        .enumerate()
        .filter_map(|(at, solid)| {
            let (low, high) = solid.bounds()?;
            let hull = edge_geom::convex_hull(
                solid
                    .vertices
                    .iter()
                    .map(|vertex| Point2::from_mm(vertex[0].into(), vertex[1].into())),
            );
            Some(Footprint {
                name: names.get(at).cloned().unwrap_or_default(),
                outline: hull
                    .points
                    .iter()
                    .map(|point| [mm(point.x), mm(point.y)])
                    .collect(),
                extent: [low[0].into(), low[1].into(), high[0].into(), high[1].into()],
                height: f64::from(high[2]),
            })
        })
        .collect())
}

/// Slices one plate of a model as it stands and writes its G-code.
fn slice_plate(
    file: &File,
    plate: Option<u32>,
    chosen: &Chosen,
    overrides: &Overrides,
    work: &Work,
) -> Result<Done, edge_engine::Failure> {
    let loaded = file.model.place(plate)?;
    let job = edge_engine::prepare(loaded, chosen, overrides)?;
    let bed_type = job.bed_type.clone();
    let sliced = edge_engine::slice(job, &Watch(work))?;
    {
        let mut progress = lock(&work.progress);
        progress.label = "Writing G-code";
        progress.step = STEPS;
    }
    let gcode = sliced
        .gcode(&edge_engine::timestamp())
        .map_err(|e| e.to_string());
    Ok(Done {
        sliced,
        gcode,
        model: file.named(),
        bed_type,
    })
}

/// A number the person set over the presets, or none to leave it to them.
fn over(args: &Value, key: &str, least: f64) -> Result<Option<Option<f64>>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(value) => match value.as_f64() {
            Some(number) if number.is_finite() && number >= least => Ok(Some(Some(number))),
            _ => Err(format!("{key} cannot be {value}")),
        },
    }
}

impl Session {
    /// A session that keeps its settings where this user's are kept. A
    /// test build keeps them for the run only, so no test reads or
    /// writes the settings of whoever runs it.
    pub fn new(startup: Option<PathBuf>) -> Self {
        let settings = if cfg!(test) {
            None
        } else {
            store::default_dir()
        };
        Self::with_settings(startup, settings)
    }

    /// A session that keeps its settings in `settings`, or with none for
    /// the run only. The profiles are those `EDGE_PROFILES` names, else
    /// the folder last chosen, else the ones beside this build.
    pub fn with_settings(startup: Option<PathBuf>, settings: Option<PathBuf>) -> Self {
        let store = store::Store::open(settings);
        let dir = std::env::var_os("EDGE_PROFILES")
            .map(PathBuf::from)
            .or_else(|| {
                let chosen = store.data.profiles_dir.as_ref().map(PathBuf::from);
                chosen.filter(|dir| dir.is_dir())
            })
            .or_else(profiles_beside);
        Self {
            state: Mutex::new(State {
                library: dir.map(Library::new),
                shelves: HashMap::new(),
                catalogue: HashMap::new(),
                store,
                // The project a window starts with.
                projects: vec![Project::new(1, UNTITLED.to_owned())],
                active: 1,
                next_id: 2,
            }),
            startup,
            traced: std::env::var_os("EDGE_DESKTOP_TRACE").is_some(),
            scene: scene::Cache::default(),
        }
    }

    /// Answers the page. An error is a sentence to show as it is.
    pub fn call(self: &Arc<Self>, name: &str, args: &Value) -> Result<Value, String> {
        let answer = self.answer(name, args);
        if self.traced && !matches!(name, "status" | "projects") {
            match &answer {
                Ok(_) => eprintln!("call {name}: ok"),
                Err(error) => eprintln!("call {name}: {error}"),
            }
        }
        answer
    }

    /// Answers the page with bytes: a mesh, or a stretch of a print's
    /// moves, in the layouts [`scene`] writes out. An error is a sentence
    /// to show as it is.
    pub fn call_bytes(self: &Arc<Self>, name: &str, args: &Value) -> Result<Vec<u8>, String> {
        let answer = match name {
            name if scene::BYTES.contains(&name) => self.scene_bytes(name, args),
            other => Err(format!(
                "there is no call named {other} that answers in bytes"
            )),
        };
        if self.traced {
            match &answer {
                Ok(bytes) => eprintln!("call {name}: {} bytes", bytes.len()),
                Err(error) => eprintln!("call {name}: {error}"),
            }
        }
        answer
    }

    fn answer(self: &Arc<Self>, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "startup" => Ok(json!({ "model": self.startup.as_deref().map(shown) })),
            "library" => self.library(),
            "set_profiles" => {
                let dir = PathBuf::from(needed(args, "dir")?);
                if !dir.is_dir() {
                    return Err(format!("{} is not a folder", dir.display()));
                }
                let mut state = lock(&self.state);
                // Kept for the next run; if it cannot be, it is said and
                // the folder is not changed.
                let chosen = shown(&dir);
                state
                    .store
                    .change(|data| data.profiles_dir = Some(chosen))?;
                state.library = Some(Library::new(dir));
                state.shelves.clear();
                state.catalogue.clear();
                drop(state);
                self.library()
            }
            "printers" => self.printers(args),
            "printer_catalogue" => self.printer_catalogue(),
            "printer_cover" => self.printer_cover(args),
            "printers_save" => self.printers_save(args),
            "printers_default" => self.printers_default(args),
            "printer_select" => self.printer_select(args),
            "machines" => {
                let shelf = self.shelf(needed(args, "vendor")?)?;
                Ok(json!({ "machines": shelf.machines() }))
            }
            "presets" => {
                let shelf = self.shelf(needed(args, "vendor")?)?;
                let machine = needed(args, "machine")?;
                let (process, filament) = shelf.usual_for(machine);
                Ok(json!({
                    "processes": shelf.processes_for(machine),
                    "filaments": shelf.filaments_for(machine),
                    // What the printer's own profile usually prints with.
                    "usual_process": process,
                    "usual_filament": filament,
                }))
            }

            "projects" => Ok(lock(&self.state).listing()),
            "project" => {
                let state = lock(&self.state);
                Ok(state.projects[state.target(args)?].detail())
            }
            "project_new" => Ok(self.project_new()),
            "project_open" => self.project_open(Path::new(needed(args, "path")?)),
            "project_select" => {
                let mut state = lock(&self.state);
                let at = state.target(&json!({ "project": whole(args, "id")? }))?;
                state.active = state.projects[at].id;
                Ok(state.listing())
            }
            "project_cycle" => {
                // The project `step` places on from the one shown, round
                // the ends (ProjectSessions.hpp `neighbour_id`).
                let step = args
                    .get("step")
                    .and_then(Value::as_i64)
                    .ok_or("step was not given")?;
                let mut state = lock(&self.state);
                let count = state.projects.len() as i64;
                let to = (state.active_index() as i64 + step).rem_euclid(count);
                state.active = state.projects[to as usize].id;
                Ok(state.listing())
            }
            "project_move" => {
                let mut state = lock(&self.state);
                let from = state.target(&json!({ "project": whole(args, "id")? }))?;
                let to = whole(args, "index")? as usize;
                if to >= state.projects.len() {
                    return Err(format!(
                        "there is no place {to} in the strip; it holds {}",
                        state.projects.len()
                    ));
                }
                let project = state.projects.remove(from);
                state.projects.insert(to, project);
                Ok(state.listing())
            }
            "project_close" => self.project_close(args),
            "project_set" => self.project_set(args),

            "settled" => self.settled(args),
            "plate_view" => self.plate_view(args),
            "slice" => self.slice(args),
            "cancel" => {
                let state = lock(&self.state);
                if let Some(work) = &state.projects[state.target(args)?].work {
                    work.cancel.store(true, Ordering::Relaxed);
                }
                Ok(Value::Null)
            }
            "status" => {
                let state = lock(&self.state);
                Ok(status_of(&state.projects[state.target(args)?]))
            }
            "summary" => {
                let (done, plate) = self.done(args)?;
                Ok(summary(&done, plate))
            }
            "layer" => {
                let index = whole(args, "index")? as usize;
                layer(&self.done(args)?.0.sliced, index)
            }
            "export" => {
                let (done, _) = self.done(args)?;
                let gcode = done.gcode.as_ref().map_err(String::clone)?;
                let path = Path::new(needed(args, "path")?);
                std::fs::write(path, &gcode.text)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                Ok(json!({ "path": shown(path), "bytes": gcode.text.len() }))
            }
            name if edit::JSON.contains(&name) => self.edit_json(name, args),
            name if scene::JSON.contains(&name) => self.scene_json(name, args),
            name if scene::BYTES.contains(&name) => {
                Err(format!("{name} answers in bytes, not in JSON"))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    fn library(&self) -> Result<Value, String> {
        let state = lock(&self.state);
        Ok(match &state.library {
            None => json!({ "dir": null, "vendors": [] }),
            Some(library) => json!({
                "dir": shown(library.dir()),
                "vendors": library.vendors().map_err(|e| e.to_string())?,
            }),
        })
    }

    /// A vendor's presets, read once and kept. They are read with the
    /// session free to answer other calls.
    fn shelf(&self, vendor: &str) -> Result<Arc<Shelf>, String> {
        let library = {
            let state = lock(&self.state);
            if let Some(shelf) = state.shelves.get(vendor) {
                return Ok(shelf.clone());
            }
            state
                .library
                .clone()
                .ok_or("no profiles folder has been chosen")?
        };
        let shelf = Arc::new(library.shelf(vendor).map_err(|e| e.to_string())?);
        let mut state = lock(&self.state);
        // Kept, unless another folder of profiles was chosen meanwhile.
        if state.library.as_ref().map(Library::dir) == Some(library.dir()) {
            return Ok(state
                .shelves
                .entry(vendor.to_owned())
                .or_insert(shelf)
                .clone());
        }
        Ok(shelf)
    }

    /// File > New project and the strip's "+": a new, empty project next to
    /// the one shown (ProjectStrip.cpp `new_project`). One that is itself
    /// new and untouched is simply started again; at the most a window
    /// keeps, nothing is added and the answer says so.
    fn project_new(&self) -> Value {
        let mut state = lock(&self.state);
        let shown = state.active_index();
        if state.projects[shown].blank() {
            state.projects[shown].tab = "prepare";
            let id = state.active;
            return state.listing_with(json!({ "id": id }));
        }
        if state.projects.len() >= MAX_OPEN {
            return state.listing_with(json!({ "full": true }));
        }
        let name = state.untitled();
        let at = state.add(name);
        let id = state.projects[at].id;
        state.listing_with(json!({ "id": id }))
    }

    /// File > Open project and the strip's folder button (ProjectStrip.cpp
    /// `open_project`): a file that is open already is shown; a new,
    /// untouched project takes the file itself; otherwise it goes in a new
    /// project next to the one shown, unless the window is full.
    fn project_open(&self, path: &Path) -> Result<Value, String> {
        // Before the file is read, which may take a while, and again after.
        let place = |state: &mut State| -> Option<Value> {
            if let Some(id) = state.holding(path) {
                state.active = id;
                return Some(state.listing_with(json!({ "id": id, "already": true })));
            }
            let blank = state.projects[state.active_index()].blank();
            (!blank && state.projects.len() >= MAX_OPEN)
                .then(|| state.listing_with(json!({ "full": true })))
        };
        if let Some(answer) = place(&mut lock(&self.state)) {
            return Ok(answer);
        }
        let file = Arc::new(read(path)?);
        let mut state = lock(&self.state);
        if let Some(answer) = place(&mut state) {
            return Ok(answer);
        }
        let shown = state.active_index();
        let at = if state.projects[shown].blank() {
            shown
        } else {
            state.add(String::new())
        };
        state.projects[at].load(file);
        let id = state.projects[at].id;
        Ok(state.listing_with(json!({ "id": id })))
    }

    /// Closes a project (ProjectStrip.cpp `close_project`). One with
    /// choices that would be lost, or one that is slicing, is not closed
    /// until the call says to (`discard`, `stop`): the answer asks instead.
    /// The next project is shown in its place, else the one before; the
    /// last one leaves a new, empty project behind, so there is always one.
    fn project_close(&self, args: &Value) -> Result<Value, String> {
        let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
        let mut state = lock(&self.state);
        let at = state.target(&json!({ "project": whole(args, "id")? }))?;
        let project = &state.projects[at];
        let ask = if project.dirty() && !flag("discard") {
            Some("unsaved")
        } else if project.slicing() && !flag("stop") {
            Some("slicing")
        } else {
            None
        };
        if let Some(ask) = ask {
            let name = project.name.clone();
            return Ok(state.listing_with(json!({ "closed": false, "ask": ask, "name": name })));
        }
        let mut closed = state.projects.remove(at);
        closed.forget();
        if state.projects.is_empty() {
            let fresh = Project::new(state.next_id, UNTITLED.to_owned());
            state.next_id += 1;
            state.active = fresh.id;
            state.projects.push(fresh);
        } else if state.active == closed.id {
            let next = at.min(state.projects.len() - 1);
            state.active = state.projects[next].id;
        }
        Ok(state.listing_with(json!({ "closed": true })))
    }

    /// Sets what is chosen for a project: the plate it is on, the tab it
    /// shows, where its Preview is, and what it is sliced with. A change to
    /// the last lets go of its slices, as the reference's do.
    fn project_set(&self, args: &Value) -> Result<Value, String> {
        let mut state = lock(&self.state);
        let at = state.target(args)?;
        let project = &mut state.projects[at];
        let mut choices = project.choices.clone();
        let named = |key: &str| -> Option<Option<String>> {
            args.get(key).map(|value| {
                value
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned)
            })
        };
        match text(args, "source") {
            None => {}
            Some("presets") => choices.from_presets = true,
            Some("project") => {
                let has_settings = project
                    .file
                    .as_ref()
                    .is_some_and(|file| file.settings.is_some());
                if !has_settings {
                    return Err("the model has no settings of its own".into());
                }
                choices.from_presets = false;
            }
            Some(other) => return Err(format!("source cannot be {other}")),
        }
        if let Some(vendor) = named("vendor") {
            choices.vendor = vendor;
        }
        if let Some(machine) = named("machine") {
            choices.machine = machine;
        }
        if let Some(process) = named("process") {
            choices.process = process;
        }
        if let Some(bed_type) = named("bed_type") {
            choices.bed_type = bed_type;
        }
        if let Some(filaments) = args.get("filaments") {
            choices.filaments = filaments
                .as_array()
                .ok_or("filaments is not a list")?
                .iter()
                .map(|name| name.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or("filaments is not a list of names")?;
        }
        if let Some(height) = over(args, "layer_height", f64::MIN_POSITIVE)? {
            choices.layer_height = height;
        }
        if let Some(height) = over(args, "first_layer_height", f64::MIN_POSITIVE)? {
            choices.first_layer_height = height;
        }
        if let Some(radius) = over(args, "closing_radius", 0.0)? {
            choices.closing_radius = radius;
        }
        if let Some(loops) = over(args, "wall_loops", 0.0)? {
            if loops.is_some_and(|loops| loops.fract() != 0.0) {
                return Err("wall_loops is not a whole number".into());
            }
            choices.wall_loops = loops.map(|loops| loops as usize);
        }
        let plate = match args.get("plate").filter(|plate| !plate.is_null()) {
            None => None,
            Some(plate) => {
                let ids: Vec<u32> = project
                    .file
                    .iter()
                    .flat_map(|file| file.plates.iter().map(|plate| plate.id))
                    .collect();
                let id = plate
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .filter(|id| ids.contains(id))
                    .ok_or_else(|| match ids.as_slice() {
                        [] => format!("there is no plate {plate}; the project has no plates"),
                        ids => format!(
                            "there is no plate {plate}; the project has plates {}",
                            ids.iter()
                                .map(u32::to_string)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    })?;
                Some(id)
            }
        };
        let tab = match text(args, "tab") {
            None => None,
            Some("prepare") => Some("prepare"),
            Some("preview") => Some("preview"),
            Some(other) => return Err(format!("tab cannot be {other}")),
        };
        let view = match args.get("view") {
            None => None,
            Some(view) if view.is_object() && view.to_string().len() <= LONGEST_VIEW => {
                Some(view.clone())
            }
            Some(_) => return Err("view is not a short note".into()),
        };
        // Nothing is changed unless all of it can be.
        if let Some(plate) = plate {
            project.plate = Some(plate);
        }
        if let Some(tab) = tab {
            project.tab = tab;
        }
        if let Some(view) = view {
            project.view = view;
        }
        if !choices.settle_as(&project.choices) {
            project.forget();
        }
        project.choices = choices;
        // The choices a project starts with are not a change to it.
        if args.get("initial").and_then(Value::as_bool) == Some(true) {
            project.baseline = project.choices.clone();
        }
        let detail = project.detail();
        printers::remember(&mut state, at, args);
        Ok(detail)
    }

    /// The printer's settings a project would be sliced with and where
    /// they come from: the preset chosen, or the project's own.
    fn printer_of(
        &self,
        choices: &Choices,
        file: Option<&File>,
    ) -> Result<Option<(&'static str, Arc<Settings>)>, String> {
        if !choices.from_presets {
            return Ok(file
                .and_then(|file| file.settings.clone())
                .map(|settings| ("project", settings)));
        }
        let (Some(vendor), Some(machine)) = (&choices.vendor, &choices.machine) else {
            return Ok(None);
        };
        // A name that is not among the vendor's is no printer yet: it is
        // said when slicing is asked for.
        let chosen = self.shelf(vendor)?.choose(Some(machine), None, &[]).ok();
        Ok(chosen
            .and_then(|chosen| chosen.machine)
            .map(|(_, settings)| ("preset", Arc::new(settings))))
    }

    /// What a project would be sliced with, as far as can be said before
    /// slicing: the printer (for the sidebar and the Device tab), and the
    /// values of the process settings that can be set over the presets.
    fn settled(&self, args: &Value) -> Result<Value, String> {
        let (choices, file) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.choices.clone(), project.file.clone())
        };
        let printer = self.printer_of(&choices, file.as_deref())?;
        let process: Option<Arc<Settings>> = if !choices.from_presets {
            file.as_ref().and_then(|file| file.settings.clone())
        } else {
            match (&choices.vendor, &choices.process) {
                (Some(vendor), Some(process)) => self
                    .shelf(vendor)?
                    .choose(None, Some(process), &[])
                    .ok()
                    .and_then(|chosen| chosen.process)
                    .map(|(_, settings)| Arc::new(settings)),
                _ => None,
            }
        };
        let printer = printer.map(|(source, settings)| {
            let text = |key: &str| settings.text(key).filter(|text| !text.is_empty());
            let number = |key: &str| settings.float(key).ok().flatten();
            let vendor = choices.vendor.as_deref().filter(|_| choices.from_presets);
            json!({
                "source": source,
                "name": if choices.from_presets { choices.machine.as_deref() } else { text("printer_settings_id") },
                "vendor": vendor,
                "model": text("printer_model"),
                "variant": text("printer_variant"),
                "nozzle_diameter": settings.texts("nozzle_diameter"),
                "bed": bed_of(&settings),
                "height": number("printable_height"),
                "gcode_flavor": text("gcode_flavor"),
                // Where the printer is, when its preset says: nothing is
                // sent there. A key or a password is never passed on, only
                // whether there is one.
                "host_type": text("host_type"),
                "print_host": text("print_host"),
                "print_host_webui": text("print_host_webui"),
                "has_api_key": text("printhost_apikey").is_some(),
                "bed_types": bed_types(&settings, vendor, choices.bed_type.as_deref()),
            })
        });
        let process = process.map(|settings| {
            let number = |key: &str| settings.float(key).ok().flatten();
            json!({
                "name": if choices.from_presets { choices.process.as_deref() } else { settings.text("print_settings_id") },
                "layer_height": number("layer_height"),
                "first_layer_height": number("initial_layer_print_height"),
                "wall_loops": number("wall_loops"),
                "closing_radius": number("slice_closing_radius"),
            })
        });
        Ok(json!({ "printer": printer, "process": process }))
    }

    /// A plate seen from above, for Prepare: the bed of the printer chosen
    /// and the outline of each object on it. A bare model is shown in the
    /// middle of the bed, where slicing puts it.
    fn plate_view(&self, args: &Value) -> Result<Value, String> {
        let (id, choices, file, plate, known) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            let plate = match args.get("plate").and_then(Value::as_u64) {
                Some(plate) => u32::try_from(plate).ok(),
                None => project.plate,
            };
            (
                project.id,
                project.choices.clone(),
                project.file.clone(),
                plate,
                project.footprints.get(&slot(plate)).cloned(),
            )
        };
        let printer = self.printer_of(&choices, file.as_deref())?;
        let bed = printer.as_ref().and_then(|(_, settings)| bed_of(settings));
        let height = printer
            .as_ref()
            .and_then(|(_, settings)| settings.float("printable_height").ok().flatten());
        let objects = match (&file, known) {
            (None, _) => Arc::new(Vec::new()),
            (Some(_), Some(known)) => known,
            (Some(file), None) => {
                let found = Arc::new(footprints(file, plate)?);
                let mut state = lock(&self.state);
                // Kept for the project, if it is still open on this file.
                if let Some(at) = state.index_of(id) {
                    let project = &mut state.projects[at];
                    if project
                        .file
                        .as_ref()
                        .is_some_and(|held| Arc::ptr_eq(held, file))
                    {
                        project.footprints.insert(slot(plate), found.clone());
                    }
                }
                found
            }
        };
        // A model without settings that nothing has been put anywhere in
        // has no place of its own: the middle of the bed.
        let bare = file
            .as_ref()
            .is_some_and(|file| file.settings.is_none() && !file.model.placed());
        let extent = objects.iter().map(|object| object.extent).reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        });
        let (dx, dy) = match (bare, bed, extent) {
            (true, Some(bed), Some(extent)) => (
                (bed[0] + bed[2]) / 2.0 - (extent[0] + extent[2]) / 2.0,
                (bed[1] + bed[3]) / 2.0 - (extent[1] + extent[3]) / 2.0,
            ),
            _ => (0.0, 0.0),
        };
        let mm = |value: f64| (value * 1e3).round() / 1e3;
        let objects: Vec<Value> = objects
            .iter()
            .map(|object| {
                let outline: Vec<f64> = object
                    .outline
                    .iter()
                    .flat_map(|point| [mm(point[0] + dx), mm(point[1] + dy)])
                    .collect();
                let [x0, y0, x1, y1] = object.extent;
                json!({
                    "name": object.name,
                    "outline": outline,
                    "box": [mm(x0 + dx), mm(y0 + dy), mm(x1 + dx), mm(y1 + dy)],
                    "height": mm(object.height),
                })
            })
            .collect();
        Ok(json!({
            "project": id,
            "plate": plate,
            "bed": bed,
            "height": height,
            "objects": objects,
        }))
    }

    /// The slice of a project's plate: the plate named, else the one it
    /// is on.
    fn done(&self, args: &Value) -> Result<(Arc<Done>, Option<u32>), String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let plate = match args.get("plate").and_then(Value::as_u64) {
            Some(plate) => u32::try_from(plate).ok(),
            None => project.plate,
        };
        project
            .results
            .get(&slot(plate))
            .cloned()
            .map(|done| (done, plate))
            .ok_or_else(|| "nothing has been sliced yet".into())
    }

    /// Starts slicing a project on a thread of its own and comes straight
    /// back; `status` says how it is going. One plate (the one named, else
    /// the one the project is on), or with `all` every plate that holds
    /// something and is not sliced yet, one after another.
    fn slice(self: &Arc<Self>, args: &Value) -> Result<Value, String> {
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
        let (id, file, choices, plates) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            let file = project
                .file
                .clone()
                .ok_or("the project is empty: there is nothing to slice")?;
            let sliceable = project.sliceable();
            let plates: Vec<Option<u32>> = if all {
                sliceable
                    .into_iter()
                    .filter(|plate| !project.results.contains_key(&slot(*plate)))
                    .collect()
            } else {
                let plate = match args.get("plate").and_then(Value::as_u64) {
                    Some(plate) => u32::try_from(plate).ok(),
                    None => project.plate,
                };
                if !sliceable.contains(&plate) {
                    return Err(match plate {
                        Some(plate) => format!("plate {plate} has nothing printable on it"),
                        None => "the project has plates; none was named".into(),
                    });
                }
                vec![plate]
            };
            if plates.is_empty() {
                return Err("every plate that holds something is sliced already".into());
            }
            (project.id, file, project.choices.clone(), plates)
        };
        // The presets are found before anything starts, so a name that
        // is not there is said at once.
        let chosen = match (choices.from_presets, &choices.vendor) {
            (true, Some(vendor)) => {
                let filaments: Vec<&str> = choices.filaments.iter().map(String::as_str).collect();
                self.shelf(vendor)?
                    .choose(
                        choices.machine.as_deref(),
                        choices.process.as_deref(),
                        &filaments,
                    )
                    .map_err(|e| e.to_string())?
            }
            _ => Chosen::default(),
        };
        let overrides = choices.overrides();

        let (work, run) = {
            let mut state = lock(&self.state);
            let at = state
                .index_of(id)
                .ok_or_else(|| format!("there is no project {id}"))?;
            let project = &mut state.projects[at];
            // One that was called off may still be winding down; its
            // answer is no longer wanted, so a new one may start.
            let running = project.work.as_ref().is_some_and(|work| {
                !work.cancel.load(Ordering::Relaxed) && lock(&work.progress).state == "slicing"
            });
            if running {
                return Err("a slice is already running".into());
            }
            project.run += 1;
            let work = Arc::new(Work {
                progress: Mutex::new(Progress {
                    run: project.run,
                    state: "slicing",
                    label: "Reading the model",
                    step: 1,
                    plate: plates[0],
                    at: 0,
                    of: plates.len(),
                    error: None,
                }),
                cancel: AtomicBool::new(false),
            });
            for plate in &plates {
                project.results.remove(&slot(*plate));
                project.failures.remove(&slot(*plate));
            }
            project.work = Some(work.clone());
            (work, project.run)
        };

        let session = self.clone();
        std::thread::spawn(move || {
            let mut errors: Vec<String> = Vec::new();
            let mut cancelled = false;
            for (at, plate) in plates.iter().enumerate() {
                {
                    let mut progress = lock(&work.progress);
                    progress.label = "Reading the model";
                    progress.step = 1;
                    progress.plate = *plate;
                    progress.at = at;
                }
                let outcome = slice_plate(&file, *plate, &chosen, &overrides, &work);
                if work.cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                let outcome = match outcome {
                    Ok(done) => Ok(Arc::new(done)),
                    Err(error) => {
                        let error = error.to_string();
                        errors.push(match (plate, plates.len()) {
                            (Some(plate), 2..) => format!("Plate {plate}: {error}"),
                            _ => error.clone(),
                        });
                        Err(error)
                    }
                };
                // Kept before it is said to be done, and only while this
                // is still the project's slice and was not called off (an
                // edit calls it off with the session locked, so what is
                // kept here is never of a model that has changed since).
                // The two locks are never held together.
                let mut state = lock(&session.state);
                if work.cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                let project =
                    state
                        .index_of(id)
                        .map(|at| &mut state.projects[at])
                        .filter(|project| {
                            project
                                .work
                                .as_ref()
                                .is_some_and(|current| Arc::ptr_eq(current, &work))
                        });
                match (project, outcome) {
                    (Some(project), Ok(done)) => {
                        project.results.insert(slot(*plate), done);
                    }
                    (Some(project), Err(error)) => {
                        project.failures.insert(slot(*plate), error);
                    }
                    (None, _) => {}
                }
            }
            let mut progress = lock(&work.progress);
            if cancelled {
                progress.state = "cancelled";
            } else if errors.is_empty() {
                progress.state = "done";
                progress.at = plates.len();
            } else {
                progress.state = "failed";
                progress.error = Some(errors.join("\n"));
            }
        });
        Ok(json!({ "project": id, "run": run }))
    }
}

/// The features in the order the reference's legend lists them: that of
/// its roles (libvgcode `EGCodeExtrusionRole`).
const LEGEND: [Role; 16] = [
    Role::Perimeter,
    Role::ExternalPerimeter,
    Role::OverhangPerimeter,
    Role::SparseInfill,
    Role::SolidInfill,
    Role::TopSolidInfill,
    Role::Ironing,
    Role::BridgeInfill,
    Role::GapFill,
    Role::Skirt,
    Role::Support,
    Role::SupportInterface,
    Role::PrimeTower,
    Role::BottomSurface,
    Role::InternalBridge,
    Role::Brim,
];

fn summary(done: &Done, plate: Option<u32>) -> Value {
    let sliced = &done.sliced;
    let print = &sliced.print;
    // The features in the print, without copying a line of it.
    let mut present: Vec<&'static str> = Vec::new();
    // The narrowest and the widest line of the walls, fill, supports,
    // skirt and brim, for colouring by width.
    let mut widths: Option<(f64, f64)> = None;
    {
        let loops = sliced
            .walls_by
            .iter()
            .flatten()
            .flat_map(|layer| &layer.loops)
            .chain(sliced.around.iter().flatten())
            .map(|line| (line.role, Some(line.flow.width)));
        let paths = sliced
            .fills_by
            .iter()
            .flatten()
            .flatten()
            .chain(sliced.supports.iter().flatten())
            .map(|line| (line.role, Some(line.flow.width)));
        let tower = sliced
            .tower
            .layers
            .iter()
            .any(|layer| !layer.finish.is_empty())
            .then_some((Role::PrimeTower, None));
        let mut seen = [false; LEGEND.len()];
        for (role, width) in loops.chain(paths).chain(tower) {
            let name = role_name(role);
            if let Some(at) = LEGEND.iter().position(|known| role_name(*known) == name) {
                seen[at] = true;
            }
            if let Some(width) = width {
                let (least, most) = widths.unwrap_or((width, width));
                widths = Some((least.min(width), most.max(width)));
            }
        }
        for (role, seen) in LEGEND.iter().zip(seen) {
            if seen {
                present.push(role_name(*role));
            }
        }
    }

    // Three kinds of thing to say, kept apart: what is wrong with this
    // print, what was settled for the person, and what the settings ask
    // for that this engine does not do yet.
    let sentence = |phrase: &str| {
        let mut letters = phrase.chars();
        letters.next().map_or(String::new(), |first| {
            format!("{}{}.", first.to_uppercase(), letters.as_str())
        })
    };
    let mut problems: Vec<String> = Vec::new();
    if print.machine_start_gcode.is_none() {
        problems.push(
            "No printer was chosen: the G-code has a generic start and end, and is only good for looking at."
                .to_owned(),
        );
    }
    problems.extend(sliced.without_a_tool().map(|filament| {
        format!(
            "Filament {filament} is used, but the printer has {} tools; T{} is written, as the C++ \
             slicer writes it, and the printer may not accept it.",
            print.tools,
            filament - 1
        )
    }));
    let leaky = sliced
        .layers
        .iter()
        .filter(|layer| layer.open_chains > 0)
        .count();
    if leaky > 0 {
        problems.push(format!(
            "{leaky} layers have open contours; the mesh is not watertight there."
        ));
    }
    let skipped: usize = sliced
        .walls_by
        .iter()
        .flatten()
        .map(|layer| layer.gaps_skipped)
        .sum();
    if skipped > 0 {
        problems.push(format!(
            "{skipped} gap(s) between walls were left empty; their centre line could not be worked out."
        ));
    }
    let mut remarks: Vec<String> = sliced
        .remarks
        .iter()
        .map(|remark| sentence(remark))
        .collect();
    match &done.bed_type {
        BedType::Settled => {}
        BedType::PrinterDefault(plate) => remarks.push(format!(
            "No bed surface was chosen; the printer's own default, {plate}, sets the bed temperature."
        )),
        BedType::Assumed => remarks.push(
            "No bed surface was chosen; Cool Plate is assumed for the bed temperature.".to_owned(),
        ),
    }
    let mut gaps: Vec<String> = Vec::new();
    if let Some(pattern) = &sliced.infill.unsupported_pattern {
        gaps.push(format!(
            "sparse_infill_pattern \"{pattern}\": grid is used in its place"
        ));
    }
    gaps.extend(sliced.notes().cloned());

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
                "timed": summary.timed.map(|model| match model {
                    edge_gcode::TimeModel::Klipper => "klipper",
                    edge_gcode::TimeModel::Reference => "acceleration",
                }),
                "retractions": summary.retractions,
                "tool_changes": summary.tool_changes,
                "slowed_layers": summary.slowed_layers,
            })
        }
    };
    let timings = sliced.timings;
    let ms = |time: std::time::Duration| time.as_secs_f64() * 1e3;
    let heights = (0..sliced.layers.len())
        .map(|index| sliced.height_of(index))
        .fold(None, |range: Option<(f64, f64)>, height| {
            let (least, most) = range.unwrap_or((height, height));
            Some((least.min(height), most.max(height)))
        });
    let round = |range: Option<(f64, f64)>| {
        range.map(|(least, most)| [(least * 1e3).round() / 1e3, (most * 1e3).round() / 1e3])
    };
    json!({
        "plate": plate,
        "widths": round(widths),
        "heights": round(heights),
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
        "problems": problems,
        "remarks": remarks,
        "gaps": gaps,
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

    const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
    const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
    const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
    const MACHINE: &str = "Snapmaker U1 (0.4 nozzle)";

    fn reference(path: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer")
            .join(path)
            .to_string_lossy()
            .into_owned()
    }

    fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
        session
            .call(name, &args)
            .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
    }

    fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
        session
            .call(name, &args)
            .expect_err("the call should be refused")
    }

    fn open(session: &Arc<Session>, path: &str) -> u64 {
        ask(session, "project_open", json!({ "path": reference(path) }))["id"]
            .as_u64()
            .unwrap()
    }

    /// The strip as it reads: each project's name, the shown one starred.
    fn strip(session: &Arc<Session>) -> Vec<String> {
        let listing = ask(session, "projects", Value::Null);
        listing["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|project| {
                let star = if project["id"] == listing["active"] {
                    "*"
                } else {
                    ""
                };
                format!("{}{star}", project["name"].as_str().unwrap())
            })
            .collect()
    }

    fn wait(session: &Arc<Session>, project: u64) -> Value {
        loop {
            let status = ask(session, "status", json!({ "project": project }));
            if status["state"] != "slicing" {
                return status;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The printer's usual process and filament, chosen for a project as
    /// the page chooses them when it is opened.
    fn choose_presets(session: &Arc<Session>, project: u64, filaments: usize) -> Value {
        let presets = ask(
            session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": MACHINE }),
        );
        let filament = presets["filaments"][0].as_str().unwrap();
        ask(
            session,
            "project_set",
            json!({
                "project": project, "source": "presets", "vendor": "Snapmaker",
                "machine": MACHINE, "process": presets["processes"][0],
                "filaments": vec![filament; filaments], "initial": true,
            }),
        )
    }

    #[test]
    fn a_project_is_opened_sliced_looked_through_and_exported() {
        let session = Arc::new(Session::new(None));
        // A window starts with one project, new and empty.
        assert_eq!(strip(&session), ["Untitled*"]);
        assert!(session.call("summary", &Value::Null).is_err());
        assert!(refused(&session, "slice", json!({})).contains("empty"));
        // The file takes its place.
        let id = open(&session, SMALL);
        assert_eq!(id, 1);
        assert_eq!(strip(&session), ["twopart_groups*"]);
        let opened = ask(&session, "project", Value::Null);
        assert_eq!(opened["has_settings"], true);
        assert_eq!(opened["used"], json!([1]));
        assert_eq!(opened["file"], "twopart_groups.3mf");
        assert_eq!(opened["choices"]["source"], "project");
        assert_eq!(opened["dirty"], false);
        assert_eq!(opened["sliced"], false);
        assert_eq!(opened["status"]["state"], "idle");

        ask(&session, "slice", json!({}));
        let status = wait(&session, id);
        assert_eq!(status["state"], "done", "{status}");
        assert_eq!(status["step"], status["steps"]);

        let summary = ask(&session, "summary", Value::Null);
        assert_eq!(summary["layers"], 60);
        assert!(summary["gcode"]["filament_m"].as_f64().unwrap() > 0.5);
        // Nothing is wrong with it; what the profile asks for and is not
        // done is listed apart, and may be nothing: this project's one
        // such setting, loops in narrow solid areas, is followed now.
        assert_eq!(summary["problems"], json!([]));
        assert!(summary["gaps"].is_array());
        let roles = summary["roles"].as_array().unwrap();
        assert!(roles.contains(&json!("Outer wall")) && roles.contains(&json!("Bridge")));
        // In the reference's order: the inner wall is listed first.
        assert_eq!(roles[0], "Inner wall");
        let [narrow, wide] =
            [&summary["widths"][0], &summary["widths"][1]].map(|w| w.as_f64().unwrap());
        assert!(
            0.1 < narrow && narrow <= wide && wide < 2.0,
            "{narrow} {wide}"
        );
        // The thinnest and the thickest layer: the first is the thicker.
        assert_eq!(summary["heights"][0], summary["layer_height"]);
        assert_eq!(summary["heights"][1], summary["first_layer_height"]);

        let layer = ask(&session, "layer", json!({ "index": 3 }));
        let strokes = layer["strokes"].as_array().unwrap();
        assert!(!strokes.is_empty());
        assert!(
            strokes
                .iter()
                .all(|stroke| stroke["p"].as_array().unwrap().len() >= 4)
        );
        assert!(session.call("layer", &json!({ "index": 60 })).is_err());
        assert!(session.call("layer", &json!({})).is_err());

        let out = std::env::temp_dir().join(format!("edge-desktop-{}.gcode", std::process::id()));
        let written = ask(&session, "export", json!({ "path": out }));
        let text = std::fs::read(&out).unwrap();
        std::fs::remove_file(&out).unwrap();
        assert_eq!(written["bytes"], text.len());
        assert!(text.starts_with(b"; HEADER_BLOCK_START"));

        // The strip's tick: every plate that holds something is sliced.
        let tile = &ask(&session, "projects", Value::Null)["projects"][0];
        assert_eq!(tile["plates_with_objects"], 1);
        assert_eq!(tile["plates_sliced"], 1);
        assert_eq!(tile["slicing"], false);
        // There is nothing left for "Slice all" to do.
        assert!(refused(&session, "slice", json!({ "all": true })).contains("already"));
    }

    #[test]
    fn a_bare_model_is_sliced_with_presets_picked_from_the_lists() {
        let session = Arc::new(Session::new(None));
        let library = ask(&session, "library", Value::Null);
        assert!(
            library["vendors"]
                .as_array()
                .unwrap()
                .contains(&json!("Snapmaker"))
        );
        let machines = ask(&session, "machines", json!({ "vendor": "Snapmaker" }));
        assert!(
            machines["machines"]
                .as_array()
                .unwrap()
                .contains(&json!(MACHINE))
        );

        // A file that is not there is said, and opens nothing.
        let missing = refused(
            &session,
            "project_open",
            json!({ "path": reference("tests/no_such_model.stl") }),
        );
        assert!(missing.contains("no_such_model"), "{missing}");
        assert_eq!(strip(&session), ["Untitled*"]);

        let id = open(&session, BARE);
        let opened = ask(&session, "project", Value::Null);
        assert_eq!(opened["has_settings"], false);
        assert_eq!(opened["choices"]["source"], "presets");
        assert_eq!(opened["plates"], json!([]));
        // It has no settings of its own to go back to.
        assert!(
            refused(&session, "project_set", json!({ "source": "project" }))
                .contains("no settings")
        );
        // A preset that is not there is said before anything starts.
        ask(
            &session,
            "project_set",
            json!({ "vendor": "Snapmaker", "machine": "no such" }),
        );
        assert!(session.call("slice", &json!({})).is_err());
        assert_eq!(ask(&session, "status", Value::Null)["state"], "idle");

        choose_presets(&session, id, 1);
        ask(&session, "slice", json!({}));
        let status = wait(&session, id);
        assert_eq!(status["state"], "done", "{status}");
        let summary = ask(&session, "summary", Value::Null);
        assert_eq!(summary["printer_ready"], true);
        assert!(summary["bed"].is_array());
        // No bed surface was chosen, which is said; one chosen is not.
        let said = |summary: &Value| {
            summary["remarks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|remark| remark.as_str().unwrap().contains("bed surface"))
        };
        assert!(said(&summary), "{}", summary["remarks"]);

        // A plate of several filaments, with a preset for each and a bed
        // chosen, in a project of its own beside the first.
        let project = open(&session, THREE_PLATES);
        assert_eq!(strip(&session), ["OrcaToleranceTest", "mcbrim_test*"]);
        let opened = ask(&session, "project_set", json!({ "plate": 2 }));
        let used = opened["plates"][1]["filaments"].as_array().unwrap();
        assert!(used.len() > 1, "{used:?}");
        let highest = opened["filament_count"].as_u64().unwrap() as usize;
        assert!(highest >= used.len());
        choose_presets(&session, project, highest);
        ask(
            &session,
            "project_set",
            json!({ "bed_type": "Textured PEI Plate" }),
        );
        ask(&session, "slice", json!({}));
        let status = wait(&session, project);
        assert_eq!(status["state"], "done", "{status}");
        assert_eq!(status["plate"], 2);
        let summary = ask(&session, "summary", Value::Null);
        assert_eq!(summary["plate"], 2);
        assert!(!said(&summary), "{}", summary["remarks"]);
        assert!(summary["gcode"]["tool_changes"].as_u64().unwrap() > 0);
        // More filaments than the printer has tools is a problem with this print.
        assert!(
            summary["problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|problem| problem.as_str().unwrap().contains("tools"))
        );
        // The first project still has its own slice.
        let first = ask(&session, "summary", json!({ "project": id }));
        assert_eq!(first["model"], "OrcaToleranceTest.stl");
        // Only plate 2 of the second is sliced.
        assert!(session.call("summary", &json!({ "plate": 1 })).is_err());
    }

    #[test]
    fn projects_are_opened_side_by_side_switched_reordered_and_closed() {
        let session = Arc::new(Session::new(None));
        let small = open(&session, SMALL);
        let bare = open(&session, BARE);
        // A file goes in a new project right after the one shown.
        assert_eq!(strip(&session), ["twopart_groups", "OrcaToleranceTest*"]);
        // Open already: its own project is shown, and no second one made.
        let again = ask(
            &session,
            "project_open",
            json!({ "path": reference(SMALL) }),
        );
        assert_eq!(again["already"], true);
        assert_eq!(again["id"], small);
        assert_eq!(strip(&session), ["twopart_groups*", "OrcaToleranceTest"]);

        // A new project goes next to the one shown.
        let untitled = ask(&session, "project_new", json!({}))["id"]
            .as_u64()
            .unwrap();
        assert_eq!(
            strip(&session),
            ["twopart_groups", "Untitled*", "OrcaToleranceTest"]
        );
        // One that is new and untouched is simply started again.
        assert_eq!(ask(&session, "project_new", json!({}))["id"], untitled);
        assert_eq!(strip(&session).len(), 3);
        // Touched, it stays, and the next one is numbered.
        let touched = ask(&session, "project_set", json!({ "bed_type": "Cool Plate" }));
        assert_eq!(touched["dirty"], true);
        ask(&session, "project_new", json!({}));
        let all = [
            "twopart_groups",
            "Untitled",
            "Untitled 2*",
            "OrcaToleranceTest",
        ];
        assert_eq!(strip(&session), all);

        // Next and previous go round the ends.
        ask(&session, "project_cycle", json!({ "step": 1 }));
        assert_eq!(strip(&session)[3], "OrcaToleranceTest*");
        ask(&session, "project_cycle", json!({ "step": 1 }));
        assert_eq!(strip(&session)[0], "twopart_groups*");
        ask(&session, "project_cycle", json!({ "step": -1 }));
        assert_eq!(strip(&session)[3], "OrcaToleranceTest*");
        assert!(refused(&session, "project_cycle", json!({})).contains("step"));
        ask(&session, "project_select", json!({ "id": untitled }));
        assert_eq!(strip(&session)[1], "Untitled*");
        assert!(refused(&session, "project_select", json!({ "id": 99 })).contains("no project 99"));
        assert!(refused(&session, "project", json!({ "project": 99 })).contains("no project 99"));

        // Dragged to another place in the strip.
        ask(&session, "project_move", json!({ "id": small, "index": 3 }));
        assert_eq!(
            strip(&session),
            [
                "Untitled*",
                "Untitled 2",
                "OrcaToleranceTest",
                "twopart_groups"
            ]
        );
        ask(&session, "project_move", json!({ "id": bare, "index": 0 }));
        assert_eq!(
            strip(&session),
            [
                "OrcaToleranceTest",
                "Untitled*",
                "Untitled 2",
                "twopart_groups"
            ]
        );
        assert!(
            refused(&session, "project_move", json!({ "id": bare, "index": 4 }))
                .contains("no place 4")
        );
        assert!(
            refused(&session, "project_move", json!({ "id": 99, "index": 0 }))
                .contains("no project")
        );

        // What was chosen for a project would be lost: closing it asks.
        let asked = ask(&session, "project_close", json!({ "id": untitled }));
        assert_eq!(asked["closed"], false);
        assert_eq!(asked["ask"], "unsaved");
        assert_eq!(asked["name"], "Untitled");
        assert_eq!(strip(&session).len(), 4);
        // Closing the one shown shows the next one.
        let closed = ask(
            &session,
            "project_close",
            json!({ "id": untitled, "discard": true }),
        );
        assert_eq!(closed["closed"], true);
        assert_eq!(
            strip(&session),
            ["OrcaToleranceTest", "Untitled 2*", "twopart_groups"]
        );
        // One out of sight goes and nothing on screen changes.
        ask(&session, "project_close", json!({ "id": bare }));
        assert_eq!(strip(&session), ["Untitled 2*", "twopart_groups"]);
        // The last in the strip gives way to the one before it.
        ask(&session, "project_select", json!({ "id": small }));
        ask(&session, "project_close", json!({ "id": small }));
        assert_eq!(strip(&session), ["Untitled 2*"]);
        assert!(refused(&session, "project_close", json!({ "id": small })).contains("no project"));
        // The last project of all leaves a new, empty one: there is always one.
        let listing = ask(&session, "projects", Value::Null);
        let last = listing["active"].clone();
        let left = ask(&session, "project_close", json!({ "id": last }));
        assert_eq!(left["closed"], true);
        assert_eq!(strip(&session), ["Untitled*"]);
        assert_ne!(left["active"], last);
        assert_eq!(left["projects"][0]["empty"], true);
    }

    #[test]
    fn a_window_keeps_no_more_projects_than_it_may() {
        let session = Arc::new(Session::new(None));
        for _ in 1..MAX_OPEN {
            ask(&session, "project_set", json!({ "wall_loops": 3 }));
            assert!(
                ask(&session, "project_new", json!({}))
                    .get("full")
                    .is_none()
            );
        }
        let names = strip(&session);
        assert_eq!(names.len(), MAX_OPEN);
        assert_eq!(names[MAX_OPEN - 1], format!("Untitled {MAX_OPEN}*"));
        // The last one is untouched, so a file may still take its place.
        let small = open(&session, SMALL);
        assert_eq!(strip(&session)[MAX_OPEN - 1], "twopart_groups*");
        // Now the window is full: nothing more is added, and it says so.
        let full = ask(&session, "project_new", json!({}));
        assert_eq!(full["full"], true);
        assert_eq!(full["max_open"], MAX_OPEN);
        let full = ask(&session, "project_open", json!({ "path": reference(BARE) }));
        assert_eq!(full["full"], true);
        assert_eq!(strip(&session).len(), MAX_OPEN);
        // A file open already is still shown.
        ask(&session, "project_select", json!({ "id": 1 }));
        let shown = ask(
            &session,
            "project_open",
            json!({ "path": reference(SMALL) }),
        );
        assert_eq!(shown["active"], small);
        // With one closed there is room, and its number is free again.
        let third = ask(&session, "projects", Value::Null)["projects"][2].clone();
        assert_eq!(third["name"], "Untitled 3");
        ask(
            &session,
            "project_close",
            json!({ "id": third["id"], "discard": true }),
        );
        ask(&session, "project_new", json!({}));
        let names = strip(&session);
        assert_eq!(names.len(), MAX_OPEN);
        assert_eq!(names[MAX_OPEN - 1], "Untitled 3*");
    }

    #[test]
    fn one_project_slices_while_another_is_looked_at() {
        let session = Arc::new(Session::new(None));
        let small = open(&session, SMALL);
        let plates = open(&session, THREE_PLATES);
        ask(&session, "slice", json!({ "project": small }));
        assert_eq!(wait(&session, small)["state"], "done");

        // Every plate of the second, one after another, out of sight of
        // the first.
        ask(&session, "project_select", json!({ "id": small }));
        ask(&session, "slice", json!({ "project": plates, "all": true }));
        let status = ask(&session, "status", json!({ "project": plates }));
        assert_eq!(status["state"], "slicing");
        assert_eq!(status["of"], 3);
        assert!(
            refused(&session, "slice", json!({ "project": plates })).contains("already running")
        );
        // The first is looked through meanwhile, and sliced again.
        let listing = ask(&session, "projects", Value::Null);
        assert_eq!(listing["projects"][1]["slicing"], true);
        assert_eq!(listing["projects"][0]["slicing"], false);
        assert_eq!(ask(&session, "summary", Value::Null)["layers"], 60);
        assert!(
            !ask(&session, "layer", json!({ "index": 0 }))["strokes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(ask(&session, "status", Value::Null)["state"], "done");
        ask(&session, "slice", json!({}));
        assert_eq!(wait(&session, small)["state"], "done");
        assert_eq!(ask(&session, "summary", Value::Null)["layers"], 60);

        let status = wait(&session, plates);
        assert_eq!(status["state"], "done", "{status}");
        assert_eq!(status["at"], 3);
        let tile = &ask(&session, "projects", Value::Null)["projects"][1];
        assert_eq!(tile["plates_with_objects"], 3);
        assert_eq!(tile["plates_sliced"], 3);
        let detail = ask(&session, "project", json!({ "project": plates }));
        for plate in detail["plates"].as_array().unwrap() {
            assert_eq!(plate["sliced"], true);
            let summary = ask(
                &session,
                "summary",
                json!({ "project": plates, "plate": plate["id"] }),
            );
            assert_eq!(summary["plate"], plate["id"]);
            assert_eq!(summary["model"], "mcbrim_test.3mf");
        }

        // A slice called off leaves nothing behind, and another may start
        // at once.
        ask(
            &session,
            "project_set",
            json!({ "project": plates, "layer_height": 0.28 }),
        );
        ask(&session, "slice", json!({ "project": plates, "plate": 3 }));
        ask(&session, "cancel", json!({ "project": plates }));
        assert_eq!(wait(&session, plates)["state"], "cancelled");
        assert!(
            session
                .call("summary", &json!({ "project": plates, "plate": 3 }))
                .is_err()
        );
        ask(&session, "slice", json!({ "project": plates, "plate": 3 }));
        // Closing one that is slicing asks first, then stops it.
        let asked = ask(
            &session,
            "project_close",
            json!({ "id": plates, "discard": true }),
        );
        assert_eq!(asked["ask"], "slicing");
        let closed = ask(
            &session,
            "project_close",
            json!({ "id": plates, "discard": true, "stop": true }),
        );
        assert_eq!(closed["closed"], true);
        assert_eq!(strip(&session), ["twopart_groups*"]);
        assert_eq!(ask(&session, "summary", Value::Null)["layers"], 60);
    }

    #[test]
    fn what_is_chosen_for_a_project_stays_with_it() {
        let session = Arc::new(Session::new(None));
        let small = open(&session, SMALL);
        let plates = open(&session, THREE_PLATES);
        let opened = ask(&session, "project", Value::Null);
        assert_eq!(opened["plate"], 1);
        assert_eq!(opened["tab"], "prepare");
        assert_eq!(opened["plates"].as_array().unwrap().len(), 3);

        // The plate, the tab and where Preview was left are the project's
        // own, and no change to it.
        let view = json!({ "type": "Filament", "layers": { "2": 17 } });
        let set = ask(
            &session,
            "project_set",
            json!({ "plate": 2, "tab": "preview", "view": view }),
        );
        assert_eq!(
            (&set["plate"], &set["tab"], &set["view"]),
            (&json!(2), &json!("preview"), &view)
        );
        assert_eq!(set["dirty"], false);
        let other = ask(&session, "project", json!({ "project": small }));
        assert_eq!(other["tab"], "prepare");
        assert_eq!(other["view"], json!({}));
        // What cannot be set is said, and none of the call is taken.
        let no_plate = refused(
            &session,
            "project_set",
            json!({ "tab": "prepare", "plate": 9 }),
        );
        assert!(no_plate.contains("plates 1, 2, 3"), "{no_plate}");
        assert_eq!(ask(&session, "project", Value::Null)["tab"], "preview");
        for bad in [
            json!({ "tab": "device" }),
            json!({ "view": "top" }),
            json!({ "view": { "note": "x".repeat(LONGEST_VIEW) } }),
            json!({ "source": "other" }),
            json!({ "layer_height": -1 }),
            json!({ "layer_height": 0 }),
            json!({ "wall_loops": 1.5 }),
            json!({ "closing_radius": "wide" }),
            json!({ "filaments": "PLA" }),
            json!({ "project": small, "plate": 99 }),
        ] {
            assert!(session.call("project_set", &bad).is_err(), "{bad}");
        }

        // A change to what it is sliced with lets go of its slices, and
        // marks it; taken back, the mark goes.
        ask(&session, "slice", json!({ "project": small }));
        assert_eq!(wait(&session, small)["state"], "done");
        let kept = ask(
            &session,
            "project_set",
            json!({ "project": small, "tab": "preview" }),
        );
        assert_eq!(kept["sliced"], true);
        let changed = ask(
            &session,
            "project_set",
            json!({ "project": small, "layer_height": 0.3 }),
        );
        assert_eq!(changed["dirty"], true);
        assert_eq!(changed["sliced"], false);
        assert_eq!(changed["choices"]["layer_height"], 0.3);
        assert!(
            session
                .call("summary", &json!({ "project": small }))
                .is_err()
        );
        // What was set over the settings is sliced with.
        ask(&session, "slice", json!({ "project": small }));
        assert_eq!(wait(&session, small)["state"], "done");
        let summary = ask(&session, "summary", json!({ "project": small }));
        assert_eq!(summary["layer_height"], 0.3);
        assert!(summary["layers"].as_u64().unwrap() < 60);
        let back = ask(
            &session,
            "project_set",
            json!({ "project": small, "layer_height": null }),
        );
        assert_eq!(back["dirty"], false);
        assert_eq!(back["sliced"], false);

        // Presets picked while the project's own settings are in use
        // change nothing; in use, they do, unless they are what it
        // starts with.
        let picked = ask(
            &session,
            "project_set",
            json!({ "vendor": "Snapmaker", "machine": MACHINE }),
        );
        assert_eq!(picked["dirty"], false);
        assert_eq!(
            ask(&session, "project_set", json!({ "source": "presets" }))["dirty"],
            true
        );
        assert_eq!(
            ask(&session, "project_set", json!({ "source": "project" }))["dirty"],
            false
        );
        // A new project is on the printer of the one it was started from.
        let fresh = ask(&session, "project_new", json!({}))["id"]
            .as_u64()
            .unwrap();
        let fresh = ask(&session, "project", json!({ "project": fresh }));
        assert_eq!(fresh["choices"]["source"], "presets");
        assert_eq!(fresh["choices"]["machine"], MACHINE);
        assert_eq!(fresh["dirty"], false);
        assert_eq!(fresh["empty"], true);
        let initial = choose_presets(&session, fresh["id"].as_u64().unwrap(), 1);
        assert_eq!(initial["dirty"], false);
        assert_eq!(
            ask(&session, "project", json!({ "project": plates }))["plate"],
            2
        );
    }

    #[test]
    fn a_plate_is_seen_from_above_and_its_printer_described() {
        let session = Arc::new(Session::new(None));
        // Nothing chosen: nothing can be said.
        let nothing = ask(&session, "settled", json!({}));
        assert_eq!(nothing, json!({ "printer": null, "process": null }));
        assert_eq!(ask(&session, "plate_view", json!({}))["bed"], Value::Null);

        // An empty project shows the bed of the printer chosen.
        choose_presets(&session, 1, 1);
        let empty = ask(&session, "plate_view", json!({}));
        let bed: Vec<f64> = empty["bed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!(
            bed[2] - bed[0] > 200.0 && bed[3] - bed[1] > 200.0,
            "{bed:?}"
        );
        assert_eq!(empty["objects"], json!([]));
        let settled = ask(&session, "settled", json!({}));
        let printer = &settled["printer"];
        assert_eq!(printer["source"], "preset");
        assert_eq!(printer["name"], MACHINE);
        assert_eq!(printer["vendor"], "Snapmaker");
        assert_eq!(printer["model"], "Snapmaker U1");
        assert_eq!(printer["bed"], empty["bed"]);
        assert!(printer["height"].as_f64().unwrap() > 200.0);
        assert_eq!(printer["gcode_flavor"], "klipper");
        // The preset names no address, and nothing is made up.
        assert_eq!(printer["print_host"], Value::Null);
        assert_eq!(printer["has_api_key"], false);
        // A U1's plates, by the names the reference gives them there.
        let plates = &printer["bed_types"];
        assert_eq!(plates["enabled"], true);
        assert_eq!(plates["in_force"], "Textured PEI Plate");
        assert!(
            plates["choices"]
                .as_array()
                .unwrap()
                .contains(&json!({ "value": "High Temp Plate", "label": "Smooth PEI Plate" }))
        );
        assert_eq!(settled["process"]["layer_height"], 0.2);
        assert!(settled["process"]["wall_loops"].as_f64().unwrap() >= 1.0);

        // A bare model stands in the middle of that bed, as it is sliced.
        open(&session, BARE);
        assert_eq!(strip(&session), ["OrcaToleranceTest*"]);
        let view = ask(&session, "plate_view", json!({}));
        let objects = view["objects"].as_array().unwrap();
        assert_eq!(objects.len(), 1);
        let numbers = |value: &Value| -> Vec<f64> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect()
        };
        let extent = numbers(&objects[0]["box"]);
        assert!(((extent[0] + extent[2]) / 2.0 - (bed[0] + bed[2]) / 2.0).abs() < 0.01);
        assert!(((extent[1] + extent[3]) / 2.0 - (bed[1] + bed[3]) / 2.0).abs() < 0.01);
        let outline = numbers(&objects[0]["outline"]);
        assert!(outline.len() >= 6 && outline.len() % 2 == 0);
        for point in outline.chunks(2) {
            assert!(extent[0] - 0.01 <= point[0] && point[0] <= extent[2] + 0.01);
            assert!(extent[1] - 0.01 <= point[1] && point[1] <= extent[3] + 0.01);
        }
        assert!(objects[0]["height"].as_f64().unwrap() > 1.0);

        // A project's plate: each object where the project puts it, on
        // the project's own bed, whichever plate it is.
        let project = open(&session, THREE_PLATES);
        let detail = ask(&session, "project", Value::Null);
        for plate in detail["plates"].as_array().unwrap() {
            let view = ask(&session, "plate_view", json!({ "plate": plate["id"] }));
            assert_eq!(view["plate"], plate["id"]);
            let bed = numbers(&view["bed"]);
            let objects = view["objects"].as_array().unwrap();
            assert_eq!(objects.len(), plate["objects"].as_array().unwrap().len());
            for (object, name) in objects.iter().zip(plate["objects"].as_array().unwrap()) {
                assert_eq!(&object["name"], name);
                let extent = numbers(&object["box"]);
                assert!(
                    bed[0] <= extent[0] && extent[2] <= bed[2],
                    "plate {}",
                    plate["id"]
                );
                assert!(
                    bed[1] <= extent[1] && extent[3] <= bed[3],
                    "plate {}",
                    plate["id"]
                );
            }
        }
        let own = ask(&session, "settled", json!({}));
        assert_eq!(own["printer"]["source"], "project");
        assert_eq!(own["printer"]["name"], detail["saved"]["printer"]);
        assert_eq!(own["process"]["name"], detail["saved"]["process"]);
        assert!(
            session
                .call("plate_view", &json!({ "project": project, "plate": 9 }))
                .is_err()
        );

        // A preset that says where its printer is has it shown; one that
        // takes one plate only has no choice of plate.
        let fresh = ask(&session, "project_new", json!({}))["id"].clone();
        ask(
            &session,
            "project_set",
            json!({ "project": fresh, "vendor": "Volumic", "machine": "SH65 Stage 2 (0.4 nozzle)" }),
        );
        let there = ask(&session, "settled", json!({ "project": fresh }));
        assert_eq!(there["printer"]["print_host"], "192.168.0.60");
        assert_eq!(there["printer"]["bed_types"]["enabled"], false);
        assert_eq!(there["process"], Value::Null);
    }

    #[test]
    fn two_spellings_of_a_file_are_one_project() {
        assert_eq!(
            normalized("C:\\Prints\\Cube.3mf", true),
            "c:/prints/cube.3mf"
        );
        assert_eq!(
            normalized("C:\\Prints\\Cube.3mf", false),
            "C:/Prints/Cube.3mf"
        );
        assert_eq!(normalized("/prints/cube.3mf/", false), "/prints/cube.3mf");
        assert_eq!(normalized("/", false), "/");
        assert_eq!(name_from_path("C:\\Prints\\Cube.3mf"), "Cube");
        assert_eq!(name_from_path("/prints/plate.gcode.3mf"), "plate.gcode");
        assert_eq!(name_from_path("/prints/.hidden"), ".hidden");
        assert_eq!(name_from_path("bare"), "bare");
    }

    #[test]
    fn every_feature_in_the_legend_has_a_colour_on_the_page() {
        // Ironing stands where the reference's legend has it, after the
        // top surface, in the reference's colour for it (libvgcode
        // `DEFAULT_EXTRUSION_ROLES_COLORS`: 255, 140, 105).
        let at = |role: Role| LEGEND.iter().position(|known| *known == role).unwrap();
        assert_eq!(at(Role::Ironing), at(Role::TopSolidInfill) + 1);
        let page = include_str!("../ui/app.js");
        assert!(page.contains("'Ironing': '#ff8c69'"));
        for role in LEGEND {
            let named = format!("'{}': '#", role_name(role));
            assert!(page.contains(&named), "no colour for {}", role_name(role));
        }
    }
}
