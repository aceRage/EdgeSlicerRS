//! A person's presets brought in from files and sent out in archives, as
//! the reference's File > Import > "Import Configs" and File > Export >
//! "Export Preset Bundle" do (MainFrame.cpp `load_config_file`,
//! PresetBundle.cpp `import_presets`, `import_json_presets`;
//! CreatePresetsDialog.cpp `ExportConfigsDialog`). The formats are
//! `edge_config::bundle`'s.
//!
//! Import: a preset of the person's is saved over only once they have
//! been asked, in the reference's words ("A config exists with the same
//! name: %s, do you want to overwrite it?"); a name a system preset has is
//! passed over; a preset whose parent is not among the presets installed
//! is passed over ("Make sure you have added the corresponding printer
//! before importing the configs."). Each is written as the reference
//! writes it (`Preset::save` with the parent's config): what differs
//! from the preset it inherits from, the settings of other kinds left out
//! (`remove_invalid_keys`). Settings that run programs or write outside
//! the output folder are asked about first, as EdgeSlicer asks
//! (UntrustedSettingsGuard.cpp `install_untrusted_config_filter`).
//!
//! What differs from the reference, on purpose:
//! - Every file is read before anything is written: one that is not
//!   preset JSON at all refuses the whole import, where the reference
//!   logs it and imports the rest.
//! - The questions are asked once for every preset of the import, not
//!   one dialog a preset.
//! - A preset of the import laid over another of the same import is
//!   imported after it, whatever order the archive holds them in; the
//!   reference passes over one whose parent comes later.
//! - A preset that stands alone is written as the file has it; the
//!   reference fills in every setting of its kind from the defaults
//!   first.
//! - The settings being edited in a project do not count among the
//!   person's own for the question on untrusted settings; their saved
//!   process presets do.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use edge_config::{
    ConfigError, Kind, Settings, UserPresets, Value as Setting,
    bundle::{self, Found, Incoming, Passed},
    user::{self, ToSave},
};
use edge_engine::Shelf;
use serde_json::{Value, json};

use super::{
    Session,
    i18n::{fill, tr, trn},
    lock, needed,
    settings::{Book, kept},
    shown, text,
};

/// Said when the session keeps no presets (no data folder).
const NO_FOLDER: &str = "There is no data folder to keep presets in.";

/// The export types, as the page names them, with the reference's title
/// for each (`ExportConfigsDialog::ExportConfigsDialog`) and the line it
/// shows under the bundles.
const TYPES: [(&str, &str, Option<&str>); 5] = [
    (
        "printer_bundle",
        "Printer config bundle(.orca_printer)",
        Some(
            "Printer and all the filament&&process presets that belongs to the printer.\nCan be shared with others.",
        ),
    ),
    (
        "filament_bundle",
        "Filament bundle(.orca_filament)",
        Some("User's filament preset set.\nCan be shared with others."),
    ),
    ("printer", "Printer presets(.zip)", None),
    ("filament", "Filament presets(.zip)", None),
    ("process", "Process presets(.zip)", None),
];

/// The vendors' presets the person has installed: those of every vendor
/// a printer has been added from.
struct Shelves(Vec<Arc<Shelf>>);

impl Shelves {
    fn of(&self, kind: Kind, name: &str) -> Option<&Shelf> {
        self.0
            .iter()
            .find(|shelf| shelf.knows(kind, name))
            .map(Arc::as_ref)
    }

    fn knows(&self, kind: Kind, name: &str) -> bool {
        self.of(kind, name).is_some()
    }

    fn resolve(&self, kind: Kind, name: &str) -> Result<Settings, ConfigError> {
        match self.of(kind, name) {
            Some(shelf) => shelf.resolve(kind, name),
            None => Err(ConfigError::UnknownPreset {
                vendor: "installed".to_owned(),
                kind,
                name: name.to_owned(),
            }),
        }
    }

    /// The shelf a system printer is on.
    fn with_machine(&self, machine: &str) -> Option<&Arc<Shelf>> {
        self.0
            .iter()
            .find(|shelf| shelf.machines().iter().any(|m| m == machine))
    }
}

fn book_of(kind: Kind) -> Book {
    match kind {
        Kind::Process => Book::Process,
        Kind::Filament => Book::Filament,
        Kind::Machine | Kind::MachineModel => Book::Printer,
    }
}

fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Process => "process",
        Kind::Filament => "filament",
        Kind::Machine | Kind::MachineModel => "printer",
    }
}

/// Seconds since 1970 for what is stamped now: `SOURCE_DATE_EPOCH` when
/// set, so two exports can be compared byte for byte.
fn now() -> i64 {
    std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0))
        })
}

/// The lines of `post_process` joined, blank ones dropped, each trimmed
/// (UntrustedInput.cpp `joined_post_process`).
fn joined_post_process(settings: &Settings) -> String {
    let values: Vec<String> = match settings.get("post_process") {
        Some(Setting::One(text)) => vec![text.clone()],
        Some(Setting::Many(items)) => items.clone(),
        None => Vec::new(),
    };
    values
        .iter()
        .flat_map(|value| value.split(['\r', '\n']))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether an output name template names a folder in its own text
/// (UntrustedInput.cpp `filename_format_leaves_folder`).
fn leaves_folder(format: &str) -> bool {
    if format.contains('\\') || format.contains("..") {
        return true;
    }
    let (mut depth, mut in_string) = (0usize, false);
    for c in format.chars() {
        if depth > 0 {
            match c {
                '"' => in_string = !in_string,
                '/' if in_string => return true,
                '}' | ']' if !in_string => depth -= 1,
                '{' | '[' if !in_string => depth += 1,
                _ => {}
            }
            continue;
        }
        match c {
            '{' | '[' => depth += 1,
            '/' | ':' => return true,
            _ => {}
        }
    }
    false
}

/// What the person has in their own process presets, which an imported
/// file may carry without being asked about (`collect_trusted`).
#[derive(Default)]
struct Trusted {
    post_process: BTreeSet<String>,
    filename_format: BTreeSet<String>,
}

/// A setting an imported preset carries that is asked about
/// (`find_untrusted_settings`): the preset, the key, its value.
struct Untrusted {
    at: usize,
    key: &'static str,
    value: String,
}

fn untrusted(at: usize, settings: &Settings, trusted: &Trusted) -> Vec<Untrusted> {
    let mut found = Vec::new();
    let post = joined_post_process(settings);
    if !post.is_empty() && !trusted.post_process.contains(&post) {
        found.push(Untrusted {
            at,
            key: "post_process",
            value: post,
        });
    }
    if let Some(format) = settings.text("filename_format")
        && leaves_folder(format)
        && !trusted.filename_format.contains(format)
    {
        found.push(Untrusted {
            at,
            key: "filename_format",
            value: format.to_owned(),
        });
    }
    found
}

/// At most 6 lines of 160 characters of a value, as the question shows
/// it (`shorten`).
fn shorten(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = lines
        .iter()
        .take(6)
        .map(|line| {
            if line.chars().count() > 160 {
                format!("{}...", line.chars().take(157).collect::<String>())
            } else {
                (*line).to_owned()
            }
        })
        .map(|line| format!("    {line}"))
        .collect();
    if lines.len() > 6 {
        out.push("    ...".to_owned());
    }
    out.join("\n")
}

/// What a preset read from a file writes over the preset it inherits
/// from (`Preset::save` with the parent's config, after
/// `remove_invalid_keys`): the settings of its kind that differ from the
/// parent's, and one the parent lacks that differs from the default; a
/// flow-variant list of more than one entry always. With no parent,
/// every setting of its kind.
fn own_of(kind: Kind, settings: &Settings, parent: Option<&Settings>) -> Settings {
    let book = book_of(kind);
    let defaults = edge_config::reference_defaults();
    let mut own = Settings::default();
    for (key, value) in settings.iter() {
        if !kept::is_of(book, key) {
            continue;
        }
        let differs = match parent {
            None => true,
            Some(parent) => match parent.get(key) {
                Some(theirs) => !user::same(value, theirs),
                None => defaults
                    .get(key)
                    .is_none_or(|default| !user::same(value, default)),
            },
        };
        let flows = key.ends_with("_flow_support")
            && matches!(value, Setting::Many(items) if items.len() > 1);
        if differs || flows {
            own.insert(key, value.clone());
        }
    }
    own
}

/// The paths a call names: `path`, or a list in `paths`.
fn paths_of(args: &Value) -> Result<Vec<PathBuf>, String> {
    if let Some(path) = text(args, "path") {
        return Ok(vec![PathBuf::from(path)]);
    }
    let paths: Vec<PathBuf> = args
        .get("paths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        return Err("path was not given".into());
    }
    Ok(paths)
}

fn passed_json(passed: &[Passed]) -> Vec<Value> {
    passed
        .iter()
        .map(|p| json!({ "file": p.file, "reason": p.reason }))
        .collect()
}

/// One printer for "Printer config bundle(.orca_printer)": a printer of
/// the person's, or a system one their presets go with.
struct BundlePrinter {
    name: String,
    mine: bool,
    shelf: Option<Arc<Shelf>>,
    filaments: Vec<String>,
    processes: Vec<String>,
}

/// What can be exported, by type (`ExportConfigsDialog::data_init`).
struct Exportable {
    printers: Vec<BundlePrinter>,
    /// A filament's name, and the presets of it with the vendor of the
    /// printer each is for (empty when its name says none).
    filaments: BTreeMap<String, Vec<(String, String)>>,
    user_printers: Vec<String>,
    user_filaments: Vec<String>,
    user_processes: Vec<String>,
}

impl Session {
    /// The account folder presets are kept in, or the reason there is
    /// none.
    fn presets_dir(&self) -> Result<PathBuf, String> {
        lock(&self.state)
            .users
            .dir
            .clone()
            .ok_or_else(|| NO_FOLDER.to_owned())
    }

    /// The vendors' presets installed: those of each vendor a printer has
    /// been added from.
    fn installed_shelves(&self) -> Shelves {
        let vendors: Vec<String> = lock(&self.state)
            .store
            .data
            .models
            .keys()
            .cloned()
            .collect();
        Shelves(
            vendors
                .iter()
                .filter_map(|vendor| self.shelf(vendor).ok())
                .collect(),
        )
    }

    /// `preset_import`: the presets in a file (`path`) or files
    /// (`paths`): a preset's `.json`, or an archive of them (`.zip`,
    /// `.orca_printer`, `.orca_filament`), saved among the person's.
    /// `keep_untrusted` answers the question on untrusted settings,
    /// `overwrite` the one on presets of theirs of the same name (true:
    /// "Yes to All", false: "No to All").
    pub(super) fn preset_import(&self, args: &Value) -> Result<Value, String> {
        let dir = self.presets_dir()?;
        let paths = paths_of(args)?;
        let mut found = Found::default();
        for path in &paths {
            let one = bundle::read_path(path).map_err(|e| e.to_string())?;
            found.passed.extend(one.passed);
            found.broken.extend(one.broken);
            for preset in one.presets {
                if found
                    .presets
                    .iter()
                    .any(|other| other.kind == preset.kind && other.name == preset.name)
                {
                    found.passed.push(Passed {
                        file: preset.file,
                        reason: format!(
                            "another file holds a {} preset named \"{}\"",
                            kind_word(preset.kind),
                            preset.name
                        ),
                    });
                } else {
                    found.presets.push(preset);
                }
            }
        }
        if !found.broken.is_empty() {
            let mut message = String::from("Nothing was imported: these files are not presets:");
            for broken in &found.broken {
                message.push_str(&format!("\n - {}: {}", broken.file, broken.reason));
            }
            return Err(message);
        }
        let shelves = self.installed_shelves();
        let users = self.users();
        let mut passed = found.passed;
        let mut incoming: Vec<Incoming> = Vec::new();
        for preset in found.presets {
            if shelves.knows(preset.kind, &preset.name) {
                passed.push(Passed {
                    file: preset.file,
                    reason: format!("a system preset is named \"{}\"", preset.name),
                });
            } else {
                incoming.push(preset);
            }
        }

        // Settings that run programs or write outside the output folder.
        let trusted = self.trusted(&users, &shelves);
        let flagged: Vec<Untrusted> = incoming
            .iter()
            .enumerate()
            .flat_map(|(at, preset)| untrusted(at, &preset.settings, &trusted))
            .collect();
        let mut removed = Vec::new();
        if !flagged.is_empty() {
            match args.get("keep_untrusted").and_then(Value::as_bool) {
                None => return Ok(self.untrusted_question(&incoming, &flagged)),
                Some(true) => {}
                Some(false) => {
                    for item in &flagged {
                        let preset = &incoming[item.at];
                        // What it goes back to (`baseline_for`): the
                        // person's own process preset of the name, else
                        // the default.
                        let baseline = (preset.kind == Kind::Process)
                            .then(|| users.get(Kind::Process, &preset.name))
                            .flatten()
                            .and_then(|_| {
                                users
                                    .resolve(Kind::Process, &preset.name, &|parent| {
                                        shelves.resolve(Kind::Process, parent)
                                    })
                                    .ok()
                            })
                            .and_then(|own| own.get(item.key).cloned())
                            .or_else(|| edge_config::reference_defaults().get(item.key).cloned());
                        removed.push(json!({ "name": preset.name, "key": item.key }));
                        let settings = &mut incoming[item.at].settings;
                        match baseline {
                            Some(value) => {
                                settings.insert(item.key, value);
                            }
                            None => {
                                settings.remove(item.key);
                            }
                        }
                    }
                }
            }
        }

        // Presets of the person's of the same names.
        let conflicts: Vec<(Kind, String)> = incoming
            .iter()
            .filter(|preset| users.get(preset.kind, &preset.name).is_some())
            .map(|preset| (preset.kind, preset.name.clone()))
            .collect();
        if !conflicts.is_empty() {
            match args.get("overwrite").and_then(Value::as_bool) {
                None => {
                    let names: Vec<&str> =
                        conflicts.iter().map(|(_, name)| name.as_str()).collect();
                    return Ok(json!({
                        "imported": false,
                        "title": tr("Overwrite config"),
                        "ask": fill(
                            &tr("A config exists with the same name: %s, do you want to overwrite it?"),
                            &[&names.join(", ")],
                        ),
                        "conflicts": conflicts
                            .iter()
                            .map(|(kind, name)| json!({ "kind": kind_word(*kind), "name": name }))
                            .collect::<Vec<_>>(),
                    }));
                }
                Some(true) => {}
                Some(false) => {
                    incoming.retain(|preset| {
                        let mine = users.get(preset.kind, &preset.name).is_some();
                        if mine {
                            passed.push(Passed {
                                file: preset.file.clone(),
                                reason: format!(
                                    "a preset of yours is named \"{}\" and was kept",
                                    preset.name
                                ),
                            });
                        }
                        !mine
                    });
                }
            }
        }

        // Parents first: a preset is imported once the preset it
        // inherits from is there, installed, the person's or imported
        // before it.
        let in_import: BTreeSet<(&str, String)> = incoming
            .iter()
            .map(|preset| (kind_word(preset.kind), preset.name.clone()))
            .collect();
        let mut ordered: Vec<Incoming> = Vec::new();
        let mut waiting = incoming;
        loop {
            let before = waiting.len();
            let mut still = Vec::new();
            for preset in waiting {
                let ready = match &preset.inherits {
                    None => true,
                    Some(parent)
                        if in_import.contains(&(kind_word(preset.kind), parent.clone())) =>
                    {
                        ordered
                            .iter()
                            .any(|done| done.kind == preset.kind && done.name == *parent)
                    }
                    Some(parent) => {
                        users.get(preset.kind, parent).is_some()
                            || shelves.knows(preset.kind, parent)
                    }
                };
                if ready {
                    ordered.push(preset);
                } else {
                    still.push(preset);
                }
            }
            waiting = still;
            if waiting.is_empty() || waiting.len() == before {
                break;
            }
        }
        for preset in waiting {
            passed.push(Passed {
                reason: format!(
                    "the preset it inherits from, \"{}\", is not installed",
                    preset.inherits.clone().unwrap_or_default()
                ),
                file: preset.file,
            });
        }

        let mut imported: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        {
            let mut state = lock(&self.state);
            if state.users.dir.as_ref() != Some(&dir) {
                return Err("the data folder changed meanwhile".into());
            }
            let presets = Arc::make_mut(&mut state.users.presets);
            for preset in &ordered {
                let kind = preset.kind;
                let parent = match &preset.inherits {
                    None => None,
                    Some(parent) if presets.get(kind, parent).is_some() => Some(
                        presets
                            .resolve(kind, parent, &|up| shelves.resolve(kind, up))
                            .map_err(|e| e.to_string())?,
                    ),
                    Some(parent) => Some(shelves.resolve(kind, parent).map_err(|e| e.to_string())?),
                };
                // The id of the preset it is laid over, for the
                // reference's cloud.
                let base_id = preset
                    .inherits
                    .as_deref()
                    .and_then(|parent| match presets.get(kind, parent) {
                        Some(mine) => Some(mine.info.setting_id.clone()),
                        None => shelves
                            .resolve(kind, parent)
                            .ok()?
                            .text("setting_id")
                            .map(str::to_owned),
                    })
                    .filter(|id| !id.is_empty());
                let to_save = ToSave {
                    own: own_of(kind, &preset.settings, parent.as_ref()),
                    inherits: preset.inherits.clone(),
                    version: preset.version.clone(),
                    base_id,
                };
                presets
                    .save(kind, &preset.name, to_save)
                    .map_err(|e| e.to_string())?;
                imported
                    .entry(kind_word(kind))
                    .or_default()
                    .push(preset.name.clone());
            }
        }
        let listing = self.presets_reload();
        let count = ordered.len() as u64;
        let mut message = fill(
            &trn(
                "There is %d config imported. (Only non-system and compatible configs)",
                "There are %d configs imported. (Only non-system and compatible configs)",
                count,
            ),
            &[&count],
        );
        if count == 0 {
            message.push_str(&tr(
                "\nHint: Make sure you have added the corresponding printer before importing the configs.",
            ));
        }
        Ok(json!({
            "imported": true,
            "printer": imported.get("printer").cloned().unwrap_or_default(),
            "filament": imported.get("filament").cloned().unwrap_or_default(),
            "process": imported.get("process").cloned().unwrap_or_default(),
            "passed": passed_json(&passed),
            "removed": removed,
            "title": tr("Import result"),
            "message": message,
            "presets": listing,
        }))
    }

    /// What the person's own process presets carry (`collect_trusted`).
    fn trusted(&self, users: &UserPresets, shelves: &Shelves) -> Trusted {
        let mut trusted = Trusted::default();
        for preset in users.of(Kind::Process) {
            let Ok(settings) = users.resolve(Kind::Process, &preset.name, &|parent| {
                shelves.resolve(Kind::Process, parent)
            }) else {
                continue;
            };
            let post = joined_post_process(&settings);
            if !post.is_empty() {
                trusted.post_process.insert(post);
            }
            if let Some(format) = settings.text("filename_format") {
                trusted.filename_format.insert(format.to_owned());
            }
        }
        trusted
    }

    /// The question on untrusted settings (`ask_keep`), naming the
    /// presets that carry them.
    fn untrusted_question(&self, incoming: &[Incoming], flagged: &[Untrusted]) -> Value {
        let mut names: Vec<&str> = flagged
            .iter()
            .map(|item| incoming[item.at].name.as_str())
            .collect();
        names.dedup();
        let mut what = String::new();
        let mut seen = BTreeSet::new();
        for item in flagged {
            if !seen.insert((item.key, item.value.clone())) {
                continue;
            }
            what.push_str("\n\n");
            if item.key == "post_process" {
                what.push_str(&tr(
                    "Post-processing scripts (programs run on this computer):",
                ));
            } else {
                what.push_str(&tr("Output file name that leaves the output folder:"));
            }
            what.push('\n');
            what.push_str(&shorten(&item.value));
        }
        let message = format!(
            "{}{what}\n\n{}",
            fill(
                &tr(
                    "\"%1%\" contains settings that can run programs on this computer or write files outside the output folder. EdgeSlicer runs post-processing scripts every time G-code is exported, and for Bambu Lab printers every time a plate is sliced."
                ),
                &[&names.join(", ")],
            ),
            tr(
                "Keep them only if you trust where this file came from and know what the scripts do. If you remove them, your own settings are used instead."
            ),
        );
        json!({
            "imported": false,
            "title": tr("Untrusted settings in file"),
            "ask": message,
            "remove": tr("Remove (recommended)"),
            "keep": tr("Keep"),
            "untrusted": flagged
                .iter()
                .map(|item| json!({
                    "name": incoming[item.at].name,
                    "key": item.key,
                    "value": item.value,
                }))
                .collect::<Vec<_>>(),
        })
    }

    /// What there is to export, for each type.
    fn exportable(&self, shelves: &Shelves) -> Exportable {
        let users = self.users();
        let (library, models) = {
            let state = lock(&self.state);
            (state.library.clone(), state.store.data.models.clone())
        };
        let mine = |kind: Kind| -> Vec<String> {
            users
                .of(kind)
                .filter(|preset| preset.visible && !shelves.knows(kind, &preset.name))
                .map(|preset| preset.name.clone())
                .collect()
        };
        let user_printers = mine(Kind::Machine);
        let user_filaments = mine(Kind::Filament);
        let user_processes = mine(Kind::Process);

        // The printers shown: the system ones of the models added, then
        // the person's.
        let mut printers: Vec<BundlePrinter> = Vec::new();
        if let Some(library) = &library {
            for (vendor, added) in &models {
                let (Ok(known), Ok(shelf)) = (library.models(vendor), self.shelf(vendor)) else {
                    continue;
                };
                for model in known.iter().filter(|m| added.contains_key(&m.model)) {
                    let variants = &added[&model.model];
                    for machine in &model.machines {
                        if !variants.is_empty() && !variants.contains(&machine.variant) {
                            continue;
                        }
                        printers.push(BundlePrinter {
                            name: machine.name.clone(),
                            mine: false,
                            shelf: Some(shelf.clone()),
                            filaments: Vec::new(),
                            processes: Vec::new(),
                        });
                    }
                }
            }
        }
        for name in &user_printers {
            let base = self.system_printer(name);
            printers.push(BundlePrinter {
                name: name.clone(),
                mine: true,
                shelf: shelves.with_machine(&base).cloned(),
                filaments: Vec::new(),
                processes: Vec::new(),
            });
        }
        // Each with the person's presets that go with it; a system one
        // only when some do (`select_curr_radiobox`).
        for printer in &mut printers {
            if let Some(shelf) = &printer.shelf {
                let offered = self.offered(shelf, &printer.name);
                printer.filaments = offered.user_filaments;
                printer.processes = offered.user_processes;
            }
        }
        printers.retain(|p| p.mine || !p.filaments.is_empty() || !p.processes.is_empty());

        // The person's filament presets by the filament their system
        // preset (or the preset itself, standing alone) names, each with
        // the vendor of the printer that name gives.
        let mut filaments: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for preset in users.of(Kind::Filament) {
            if shelves.knows(Kind::Filament, &preset.name) {
                continue;
            }
            let base = match &preset.inherits {
                None => preset.name.clone(),
                Some(_) => match users.system_parent(Kind::Filament, &preset.name) {
                    Some(parent) if shelves.knows(Kind::Filament, &parent) => parent,
                    _ => continue,
                },
            };
            let vendor = bundle::vendor_name(bundle::machine_name(&base)).to_owned();
            filaments
                .entry(bundle::filament_name(&base).to_owned())
                .or_default()
                .push((vendor, preset.name.clone()));
        }
        Exportable {
            printers,
            filaments,
            user_printers,
            user_filaments,
            user_processes,
        }
    }

    /// `preset_export`: without `path`, what each type can export; with
    /// it (an existing folder), the archives of a `type` written there,
    /// of the presets (or printers, or filaments) `names` lists, every
    /// one when none is given. A zip whose name is taken is asked about
    /// first: `replace` true clears it, false adds the time to the new
    /// one's name (`initial_file_name`).
    pub(super) fn preset_export(&self, args: &Value) -> Result<Value, String> {
        self.presets_dir()?;
        let shelves = self.installed_shelves();
        let exportable = self.exportable(&shelves);
        let Some(dir) = text(args, "path") else {
            return Ok(json!({
                "types": TYPES
                    .iter()
                    .map(|(name, title, note)| json!({
                        "type": name,
                        "title": tr(title),
                        "note": note.map(|note| tr(note).replace("&&", "&")),
                    }))
                    .collect::<Vec<_>>(),
                "printer_bundle": exportable.printers.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                "filament_bundle": exportable.filaments.keys().cloned().collect::<Vec<_>>(),
                "printer": exportable.user_printers,
                "filament": exportable.user_filaments,
                "process": exportable.user_processes,
            }));
        };
        let dir = Path::new(dir);
        if !dir.is_dir() {
            return Err(format!("{} is not a folder", dir.display()));
        }
        let kind = needed(args, "type")?;
        let listed: Vec<String> = match kind {
            "printer_bundle" => exportable.printers.iter().map(|p| p.name.clone()).collect(),
            "filament_bundle" => exportable.filaments.keys().cloned().collect(),
            "printer" => exportable.user_printers.clone(),
            "filament" => exportable.user_filaments.clone(),
            "process" => exportable.user_processes.clone(),
            other => return Err(format!("there is no export of type {other}")),
        };
        let chosen: Vec<String> = match args.get("names").and_then(Value::as_array) {
            Some(names) => names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            None => listed.clone(),
        };
        if chosen.is_empty() {
            return Err(match kind {
                "process" => tr("Please select at least one process preset."),
                "filament" => tr("Please select at least one filament preset."),
                _ => tr("Please select at least one printer or filament."),
            });
        }
        if let Some(missing) = chosen.iter().find(|name| !listed.contains(name)) {
            return Err(format!("\"{missing}\" is not among what can be exported"));
        }
        let seconds = now();
        let users = self.users();
        let file_of = |kind: Kind, name: &str| -> Result<(String, Vec<u8>), String> {
            let preset = users
                .get(kind, name)
                .ok_or_else(|| format!("there is no preset named \"{name}\""))?;
            let file = preset
                .file
                .file_name()
                .map(|leaf| leaf.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("{name}.json"));
            Ok((file, bundle::file_of(preset).map_err(|e| e.to_string())?))
        };
        let whole_printer = |printer: &str, shelf: Option<&Shelf>| -> Result<Vec<u8>, String> {
            let text = match users.get(Kind::Machine, printer) {
                Some(mine) => {
                    let resolved = users
                        .resolve(Kind::Machine, printer, &|parent| {
                            shelves.resolve(Kind::Machine, parent)
                        })
                        .map_err(|e| e.to_string())?;
                    bundle::whole_printer(
                        printer,
                        &resolved,
                        mine.inherits.as_deref(),
                        "User",
                        mine.version.as_deref().unwrap_or_default(),
                    )
                }
                None => {
                    let shelf = shelf.ok_or_else(|| format!("{printer} is not installed"))?;
                    let resolved = shelf
                        .resolve(Kind::Machine, printer)
                        .map_err(|e| e.to_string())?;
                    bundle::whole_printer(
                        printer,
                        &resolved,
                        None,
                        "System",
                        &shelf.preset_version(),
                    )
                }
            };
            Ok(text.into_bytes())
        };
        let safe = |name: &str| -> Result<(), String> {
            if name.contains(['/', '\\']) || name == "." || name == ".." {
                return Err(format!("\"{name}\" cannot be a file's name"));
            }
            Ok(())
        };
        let mut written: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        match kind {
            "printer_bundle" => {
                for printer in exportable
                    .printers
                    .iter()
                    .filter(|p| chosen.contains(&p.name))
                {
                    safe(&printer.name)?;
                    let files = |kind: Kind, names: &[String]| -> Result<Vec<_>, String> {
                        names.iter().map(|name| file_of(kind, name)).collect()
                    };
                    let one = bundle::PrinterBundle {
                        printer: printer.name.clone(),
                        printer_file: whole_printer(&printer.name, printer.shelf.as_deref())?,
                        filaments: files(Kind::Filament, &printer.filaments)?,
                        processes: files(Kind::Process, &printer.processes)?,
                    };
                    written.push((
                        dir.join(format!("{}.orca_printer", printer.name)),
                        bundle::printer_bundle(&one, seconds)?,
                    ));
                }
            }
            "filament_bundle" => {
                for (filament, presets) in exportable
                    .filaments
                    .iter()
                    .filter(|(f, _)| chosen.contains(f))
                {
                    safe(filament)?;
                    let mut vendors: BTreeMap<String, Vec<(String, Vec<u8>)>> = BTreeMap::new();
                    for (vendor, name) in presets {
                        // One the name gives no printer's vendor for is
                        // left out (`if (printer_vendor.empty()) continue;`).
                        if vendor.is_empty() {
                            continue;
                        }
                        let list = vendors.entry(vendor.clone()).or_default();
                        if list.iter().any(|(had, _)| had == name) {
                            continue;
                        }
                        list.push((name.clone(), file_of(Kind::Filament, name)?.1));
                    }
                    written.push((
                        dir.join(format!("{filament}.orca_filament")),
                        bundle::filament_bundle(filament, &vendors, seconds)?,
                    ));
                }
            }
            _ => {
                let (zip_name, preset_kind) = match kind {
                    "printer" => ("Printer presets.zip", Kind::Machine),
                    "filament" => ("Filament presets.zip", Kind::Filament),
                    _ => ("Process presets.zip", Kind::Process),
                };
                let mut entries = Vec::new();
                for name in listed.iter().filter(|name| chosen.contains(name)) {
                    let bytes = if preset_kind == Kind::Machine {
                        whole_printer(name, None)?
                    } else {
                        file_of(preset_kind, name)?.1
                    };
                    entries.push(bundle::Entry {
                        name: format!("{name}.json"),
                        bytes,
                    });
                }
                let mut path = dir.join(zip_name);
                if path.exists() {
                    match args.get("replace").and_then(Value::as_bool) {
                        None => {
                            return Ok(json!({
                                "exported": false,
                                "file": shown(&path),
                                "ask": fill(
                                    &tr(
                                        "The '%s' folder already exists in the current directory. Do you want to clear it and rebuild it.\nIf not, a time suffix will be added, and you can modify the name after creation."
                                    ),
                                    &[&zip_name],
                                ),
                            }));
                        }
                        Some(true) => {}
                        Some(false) => {
                            let stem = zip_name.trim_end_matches(".zip");
                            let mut at = seconds;
                            loop {
                                let c = bundle::clock(at);
                                // "%Y_%m_%d_%H_%M_%S" (`get_curr_time`).
                                let stamp = format!(
                                    "{}_{}_{}_{}_{}_{}",
                                    &c[0..4],
                                    &c[4..6],
                                    &c[6..8],
                                    &c[8..10],
                                    &c[10..12],
                                    &c[12..14]
                                );
                                path = dir.join(format!("{stem} {stamp}.zip"));
                                if !path.exists() {
                                    break;
                                }
                                at += 1;
                            }
                        }
                    }
                }
                written.push((path, bundle::zip(&entries, seconds)?));
            }
        }
        for (path, bytes) in &written {
            user::write_whole(path, bytes).map_err(|e| e.to_string())?;
        }
        Ok(json!({
            "exported": true,
            "files": written.iter().map(|(path, _)| shown(path)).collect::<Vec<_>>(),
            "message": tr("Export successful"),
        }))
    }
}

#[cfg(test)]
mod tests;
