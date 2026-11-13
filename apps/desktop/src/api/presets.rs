//! A person's own presets in the session: where they are read from, how a
//! name a project chooses is found among them and the vendor's, which of
//! them a printer is offered, and saving, renaming and deleting them as
//! the reference does (Tab.cpp `save_preset`, `delete_preset`,
//! SavePresetDialog.cpp, Preset.cpp `save_current_preset`,
//! `rename_user_preset`, `validate_preset_name`).
//!
//! They are kept under this app's own data folder (`EDGE_DESKTOP_DATA`, or
//! the folder `settings.json` is in), in `user/default/`, in the
//! reference's layout and format (`edge_config::user`): a folder copied
//! from the C++ application's data folder, or into it, works.
//!
//! A preset is found by name: the vendor's first, as the reference loads
//! its system presets first and passes over a user preset of the same
//! name, then the person's.

use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use edge_config::{
    Kind, Settings, UserPresets,
    user::{self, NameProblem, ToSave},
};
use edge_engine::{Chosen, Shelf};
use serde_json::{Value, json};

use super::{
    Choices, Session, State,
    i18n::{fill, tr, trc, trn},
    lock, needed, printers,
    settings::{Book, kept},
    text,
};

/// The presets read, and the folder they are kept in; none without a
/// data folder (a test's session), when nothing can be saved.
pub(super) struct Users {
    pub(super) dir: Option<PathBuf>,
    pub(super) presets: Arc<UserPresets>,
}

impl Users {
    /// The presets of the account folder under a data folder.
    pub(super) fn open(data: Option<&std::path::Path>) -> Self {
        match data {
            Some(data) => {
                let dir = UserPresets::account_dir(data, user::DEFAULT_USER);
                Self {
                    presets: Arc::new(UserPresets::open(&dir)),
                    dir: Some(dir),
                }
            }
            None => Self {
                dir: None,
                presets: Arc::new(UserPresets::default()),
            },
        }
    }
}

/// The lists a printer is offered: the vendor's presets made for it and
/// the person's that go with it.
pub(super) struct Offered {
    pub(super) processes: Vec<String>,
    pub(super) filaments: Vec<String>,
    pub(super) user_processes: Vec<String>,
    pub(super) user_filaments: Vec<String>,
}

impl Offered {
    /// Every process offered, the person's first as the lists show them.
    pub(super) fn all_processes(&self) -> Vec<&str> {
        self.user_processes
            .iter()
            .chain(&self.processes)
            .map(String::as_str)
            .collect()
    }

    pub(super) fn all_filaments(&self) -> Vec<&str> {
        self.user_filaments
            .iter()
            .chain(&self.filaments)
            .map(String::as_str)
            .collect()
    }
}

/// How a name the person typed stands (`validate_preset_name`): fine, to
/// be warned about (it saves over a preset of theirs), or not to be had.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    Ok,
    Warn,
    Invalid,
}

impl Level {
    fn word(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Invalid => "invalid",
        }
    }
}

/// The title of a kind's settings tab, as "Save current %s" names it
/// (Tab.hpp: `TabPrint` "Process", `TabFilament` "Filament",
/// `TabPrinter` "Machine").
fn title(book: Book) -> &'static str {
    match book {
        Book::Process => "Process",
        Book::Filament => "Filament",
        Book::Printer => "Machine",
    }
}

fn kind_of(args: &Value) -> Result<Book, String> {
    match text(args, "kind") {
        Some("process") => Ok(Book::Process),
        Some("filament") => Ok(Book::Filament),
        Some("printer") => Ok(Book::Printer),
        Some(other) => Err(format!("there are no {other} presets")),
        None => Err("kind was not given".into()),
    }
}

impl Session {
    /// The person's presets as they stand.
    pub(super) fn users(&self) -> Arc<UserPresets> {
        lock(&self.state).users.presets.clone()
    }

    /// A preset by name, with everything it inherits: the vendor's, else
    /// the person's laid over the vendor's.
    pub(super) fn resolve_named(
        &self,
        shelf: &Shelf,
        kind: Kind,
        name: &str,
    ) -> Result<Settings, String> {
        let users = self.users();
        if shelf.knows(kind, name) || users.get(kind, name).is_none() {
            return shelf.resolve(kind, name).map_err(|e| e.to_string());
        }
        users
            .resolve(kind, name, &|parent| shelf.resolve(kind, parent))
            .map_err(|e| e.to_string())
    }

    /// The presets a project's choices name, found among the vendor's and
    /// the person's.
    pub(super) fn choose_presets(
        &self,
        vendor: &str,
        machine: Option<&str>,
        process: Option<&str>,
        filaments: &[&str],
    ) -> Result<Chosen, String> {
        let shelf = self.shelf(vendor)?;
        let found = |kind, name: &str| -> Result<(String, Settings), String> {
            Ok((name.to_owned(), self.resolve_named(&shelf, kind, name)?))
        };
        Ok(Chosen {
            machine: machine.map(|name| found(Kind::Machine, name)).transpose()?,
            process: process.map(|name| found(Kind::Process, name)).transpose()?,
            filaments: filaments
                .iter()
                .map(|name| found(Kind::Filament, name))
                .collect::<Result<_, _>>()?,
        })
    }

    /// Whether a printer preset is one of the person's.
    pub(super) fn is_users(&self, kind: Kind, name: &str) -> bool {
        self.users().get(kind, name).is_some()
    }

    /// The system printer a printer preset is made from; the printer
    /// itself when it is the vendor's.
    pub(super) fn system_printer(&self, machine: &str) -> String {
        let users = self.users();
        match users.get(Kind::Machine, machine) {
            Some(_) => users
                .system_parent(Kind::Machine, machine)
                .unwrap_or_else(|| machine.to_owned()),
            None => machine.to_owned(),
        }
    }

    /// A printer preset and every preset of the person's up its chain,
    /// down to the system one: what a preset's `compatible_printers` may
    /// name to go with it (Preset.cpp `is_compatible_with_parent_printer`,
    /// which looks one step up only).
    fn printer_chain(&self, machine: &str) -> Vec<String> {
        let users = self.users();
        let mut chain = vec![machine.to_owned()];
        let mut at = machine.to_owned();
        while let Some(parent) = users
            .get(Kind::Machine, &at)
            .and_then(|p| p.inherits.clone())
        {
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent.clone());
            at = parent;
        }
        chain
    }

    /// Whether one of the person's process or filament presets goes with a
    /// printer (Preset.cpp `is_compatible_with_printer`): its
    /// `compatible_printers` name the printer, or one it is made from;
    /// with none named and no condition, every printer. A condition is
    /// not worked out here: such a preset goes with the printers the
    /// system preset it is made from is offered for, as the vendor's own
    /// are listed (by `compatible_printers` alone).
    pub(super) fn fits(&self, shelf: &Shelf, kind: Kind, name: &str, machine: &str) -> bool {
        let Ok(settings) = self.resolve_named(shelf, kind, name) else {
            return false;
        };
        let chain = self.printer_chain(machine);
        let named = settings.texts("compatible_printers").unwrap_or_default();
        if named.iter().any(|printer| !printer.is_empty()) {
            return named.iter().any(|printer| chain.contains(printer));
        }
        let condition = settings
            .text("compatible_printers_condition")
            .unwrap_or_default();
        if condition.trim().is_empty() {
            return true;
        }
        let base = self.system_printer(machine);
        self.users()
            .system_parent(kind, name)
            .is_some_and(|parent| match kind {
                Kind::Process => shelf.processes_for(&base).contains(&parent.as_str()),
                _ => shelf.filaments_for(&base).contains(&parent.as_str()),
            })
    }

    /// What a printer is offered: the vendor's process and filament
    /// presets made for it (for one of the person's, those made for the
    /// system printer it is made from), and the person's that go with it.
    pub(super) fn offered(&self, shelf: &Shelf, machine: &str) -> Offered {
        let base = self.system_printer(machine);
        let users = self.users();
        // The vendor's filaments shown: those enabled ("Filament Selection",
        // the Filament manager), and any an open project is on, as the
        // reference shows a preset its project names (Preset.cpp
        // `load_external_preset` makes it visible). The person's own are
        // always shown.
        let shown = self.shown_filaments();
        let mine = |kind: Kind| -> Vec<String> {
            users
                .of(kind)
                .filter(|preset| preset.visible && !shelf.knows(kind, &preset.name))
                .filter(|preset| self.fits(shelf, kind, &preset.name, machine))
                .map(|preset| preset.name.clone())
                .collect()
        };
        Offered {
            processes: shelf
                .processes_for(&base)
                .into_iter()
                .map(str::to_owned)
                .collect(),
            filaments: shelf
                .filaments_for(&base)
                .into_iter()
                .filter(|name| shown.as_ref().is_none_or(|shown| shown.contains(*name)))
                .map(str::to_owned)
                .collect(),
            user_processes: mine(Kind::Process),
            user_filaments: mine(Kind::Filament),
        }
    }

    /// The `presets` call: what a printer is offered, the person's apart.
    pub(super) fn presets_for(&self, args: &Value) -> Result<Value, String> {
        let shelf = self.shelf(needed(args, "vendor")?)?;
        let machine = needed(args, "machine")?;
        let offered = self.offered(&shelf, machine);
        let (process, filament) = shelf.usual_for(&self.system_printer(machine));
        Ok(json!({
            "processes": offered.processes,
            "filaments": offered.filaments,
            "user_processes": offered.user_processes,
            "user_filaments": offered.user_filaments,
            "favorite_filaments": self.favorite_filaments(&offered.filaments, &offered.user_filaments),
            // What the printer's own profile usually prints with.
            "usual_process": process,
            "usual_filament": filament,
        }))
    }

    /// The person's printer presets that can be picked: those made from a
    /// printer among those added, each with its model and nozzle.
    pub(super) fn user_printers(&self, installed: &[edge_engine::PrinterModel]) -> Vec<Value> {
        let users = self.users();
        let kept = lock(&self.state).store.data.printers.clone();
        users
            .of(Kind::Machine)
            .filter(|preset| preset.visible)
            .filter_map(|preset| {
                if preset.inherits.is_none() {
                    // One that stands alone ("Create printer"): its own
                    // model; the vendor of the printer it was made from,
                    // else the first added, for the lists.
                    let vendor = kept
                        .get(&preset.name)
                        .and_then(|kept| kept.vendor.clone())
                        .or_else(|| installed.first().map(|model| model.vendor.clone()))?;
                    return Some(json!({
                        "name": preset.name,
                        "vendor": vendor,
                        "model": preset.own.text("printer_model"),
                        "variant": preset.own.text("printer_variant"),
                        "base": preset.name,
                        "alone": true,
                    }));
                }
                let base = users.system_parent(Kind::Machine, &preset.name)?;
                let (model, system) = installed.iter().find_map(|model| {
                    let found = model.machines.iter().find(|machine| machine.name == base)?;
                    Some((model, found))
                })?;
                Some(json!({
                    "name": preset.name,
                    "vendor": model.vendor,
                    "model": model.model,
                    "variant": system.variant,
                    "base": base,
                }))
            })
            .collect()
    }

    /// The name a project's preset of a kind goes by, and whether
    /// anything is set over it.
    fn selected(choices: &Choices, book: Book, filament: usize) -> (Option<String>, bool) {
        match book {
            Book::Process => (choices.process.clone(), !choices.over.is_empty()),
            Book::Printer => (choices.machine.clone(), !choices.printer_over.is_empty()),
            Book::Filament => (
                choices
                    .filaments
                    .get(filament)
                    .or(choices.filaments.last())
                    .cloned(),
                choices
                    .filament_over
                    .get(filament)
                    .is_some_and(|own| !own.is_empty()),
            ),
        }
    }

    /// Checks a name for a preset of a kind, as the reference's dialogs do
    /// while it is typed (`validate_preset_name`, `preset_name_check_message`):
    /// a rename refuses a name that is taken, a save warns that it will
    /// save over it. `current` is the preset being saved or renamed.
    pub(super) fn check_name(
        &self,
        shelf: &Shelf,
        book: Book,
        name: &str,
        current: Option<&str>,
        rename: bool,
        machine: Option<&str>,
    ) -> (Level, Option<String>) {
        let kind = kept::kind(book);
        let invalid = |message: String| (Level::Invalid, Some(message));
        match user::check_name(name) {
            Err(NameProblem::IllegalCharacters) => {
                return invalid(format!(
                    "{}\n{} {}",
                    tr("Name is invalid;"),
                    tr("illegal characters:"),
                    user::ILLEGAL_CHARACTERS
                ));
            }
            Err(NameProblem::IllegalSuffix) => {
                return invalid(format!(
                    "{}\n{}\n\t{}",
                    tr("Name is invalid;"),
                    tr("illegal suffix:"),
                    user::SUFFIX_MODIFIED
                ));
            }
            Err(NameProblem::Reserved) => return invalid(tr("Name is unavailable.")),
            _ => {}
        }
        if shelf.knows(kind, name) {
            return invalid(tr("Overwriting a system profile is not allowed."));
        }
        if self.is_users(kind, name) && Some(name) != current {
            let fits = machine.is_none_or(|machine| match book {
                Book::Printer => true,
                _ => self.fits(shelf, kind, name, machine),
            });
            let mut message = if fits {
                fill(&tr("Preset \"%1%\" already exists."), &[&name])
            } else {
                fill(
                    &tr(
                        "Preset \"%1%\" already exists and is incompatible with the current printer.",
                    ),
                    &[&name],
                )
            };
            if rename {
                return invalid(message);
            }
            message.push('\n');
            message.push_str(&tr("Please note that saving will overwrite this preset."));
            return (Level::Warn, Some(message));
        }
        match user::check_name(name) {
            Err(NameProblem::Empty) => invalid(tr("The name is not allowed to be empty.")),
            Err(NameProblem::LeadingSpace) => {
                invalid(tr("The name is not allowed to start with space character."))
            }
            Err(NameProblem::TrailingSpace) => {
                invalid(tr("The name is not allowed to end with space character."))
            }
            _ => (Level::Ok, None),
        }
    }

    /// The project's vendor and printer, for a call about its presets.
    fn on_presets(choices: &Choices) -> Result<(String, String), String> {
        match (choices.from_presets, &choices.vendor, &choices.machine) {
            (true, Some(vendor), Some(machine)) => Ok((vendor.clone(), machine.clone())),
            // This app's own words: the reference makes a preset of a
            // project's own settings, which are a whole here (section 2 of
            // UI_PARITY.md).
            _ => Err(
                "The project is sliced with its own settings: there is no preset of it to save."
                    .to_owned(),
            ),
        }
    }

    /// `preset_check`: what the save or rename dialog says of a name as it
    /// is typed, and the name it offers to start with ("<name> - Copy" for
    /// a system preset, `SavePresetDialog::Item`).
    pub(super) fn preset_check(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let (choices, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.choices.clone(), project.filament_count())
        };
        let (vendor, machine) = Self::on_presets(&choices)?;
        let shelf = self.shelf(&vendor)?;
        let filament = filament_of(args, book, filaments)?;
        let rename = args.get("rename").and_then(Value::as_bool) == Some(true);
        let current = match text(args, "preset") {
            Some(named) => Some(named.to_owned()),
            None => Self::selected(&choices, book, filament).0,
        };
        let kind = kept::kind(book);
        let suggested = current.as_ref().map(|current| {
            if shelf.knows(kind, current) && !self.is_users(kind, current) {
                format!("{current} - {}", trc("PresetName", "Copy"))
            } else {
                current.clone()
            }
        });
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| suggested.clone().unwrap_or_default(), str::to_owned);
        let (level, message) = self.check_name(
            &shelf,
            book,
            &name,
            current.as_deref(),
            rename,
            Some(&machine),
        );
        Ok(json!({
            "kind": book_name(book),
            "name": name,
            "suggested": suggested,
            "current": current,
            "level": level.word(),
            "message": message,
            "title": tr(title(book)),
        }))
    }

    /// `preset_save`: the project's preset of a kind with what is set over
    /// it, saved as one of the person's under `name`, and chosen for the
    /// project in its place with nothing set over it (Tab.cpp
    /// `save_preset`). A name that is taken by one of theirs is asked
    /// about first (`accept` saves over it). A filament made from a system
    /// preset that names no printer is made for the printer it was saved
    /// on (Orca's rule in `save_current_preset`).
    pub(super) fn preset_save(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let name = needed(args, "name").map_err(|_| tr("The name is not allowed to be empty."))?;
        let (id, choices, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (
                project.id,
                project.choices.clone(),
                project.filament_count(),
            )
        };
        let (vendor, machine) = Self::on_presets(&choices)?;
        let users_dir = lock(&self.state)
            .users
            .dir
            .clone()
            .ok_or("there is no data folder to keep presets in")?;
        let shelf = self.shelf(&vendor)?;
        let filament = filament_of(args, book, filaments)?;
        let kind = kept::kind(book);
        let (current, _) = Self::selected(&choices, book, filament);
        let current = current.ok_or("the project has no preset of this kind chosen")?;
        let (level, message) =
            self.check_name(&shelf, book, name, Some(&current), false, Some(&machine));
        match level {
            Level::Invalid => return Err(message.unwrap_or_default()),
            Level::Warn if args.get("accept").and_then(Value::as_bool) != Some(true) => {
                return Ok(json!({ "saved": false, "ask": message }));
            }
            _ => {}
        }
        let users = self.users();
        // The preset as it stands with what is set over it.
        let preset = self.resolve_named(&shelf, kind, &current)?;
        let own = match book {
            Book::Process => choices.over.clone(),
            Book::Printer => choices.printer_over.clone(),
            Book::Filament => choices
                .filament_over
                .get(filament)
                .cloned()
                .unwrap_or_default(),
        };
        let full = kept::with_edits(book, &preset, &own);
        // What it is laid over: a preset of theirs keeps its parent; one
        // that stands alone is the parent of a copy of it, and is saved
        // whole under its own name; a system preset is the parent.
        let current_user = users.get(kind, &current).cloned();
        let inherits = match &current_user {
            Some(mine) if mine.inherits.is_some() => mine.inherits.clone(),
            Some(_) if name == current => None,
            _ => Some(current.clone()),
        };
        let parent = inherits
            .as_deref()
            .map(|parent| self.resolve_named(&shelf, kind, parent))
            .transpose()?;
        let mut own_values = user::differences(&full, parent.as_ref(), kind);
        let keep: Vec<String> = own_values
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| !kept::is_of(book, key))
            .collect();
        for key in keep {
            own_values.remove(&key);
        }
        let new = users.get(kind, name).is_none();
        if book == Book::Filament
            && new
            && current_user.is_none()
            && full
                .texts("compatible_printers")
                .is_none_or(|printers| printers.iter().all(String::is_empty))
        {
            own_values.insert(
                "compatible_printers",
                edge_config::Value::Many(vec![self.system_printer(&machine)]),
            );
        }
        // The id of the preset it is made from, for the reference's cloud.
        let base_id = match inherits.as_deref() {
            Some(parent) => match users.get(kind, parent) {
                Some(mine) => Some(mine.info.setting_id.clone()),
                None => parent_setting_id(&shelf, kind, parent),
            },
            None => None,
        }
        .filter(|id| !id.is_empty());
        let version = current_user
            .as_ref()
            .and_then(|mine| mine.version.clone())
            .unwrap_or_else(|| shelf.preset_version());
        let to_save = ToSave {
            own: own_values,
            inherits,
            version,
            base_id,
        };
        {
            let mut state = lock(&self.state);
            if state.users.dir.as_ref() != Some(&users_dir) {
                return Err("the data folder changed meanwhile".into());
            }
            let presets = Arc::make_mut(&mut state.users.presets);
            presets
                .save(kind, name, to_save)
                .map_err(|e| e.to_string())?;
        }
        // The project goes on the saved preset with nothing over it; the
        // other filaments that were on the same preset, with nothing of
        // their own set, go with it (`update_presets_from_to`).
        let mut state = lock(&self.state);
        let at = state
            .index_of(id)
            .ok_or_else(|| format!("there is no project {id}"))?;
        let project = &mut state.projects[at];
        let mut choices = project.choices.clone();
        match book {
            Book::Process => {
                choices.process = Some(name.to_owned());
                choices.over.clear();
            }
            Book::Printer => {
                choices.machine = Some(name.to_owned());
                choices.printer_over.clear();
            }
            Book::Filament => {
                if choices.filaments.len() <= filament {
                    let last = choices.filaments.last().cloned().unwrap_or_default();
                    choices.filaments.resize(filament + 1, last);
                }
                for (slot, chosen) in choices.filaments.iter_mut().enumerate() {
                    let free = choices
                        .filament_over
                        .get(slot)
                        .is_none_or(BTreeMap::is_empty);
                    if slot == filament || (*chosen == current && free && new) {
                        *chosen = name.to_owned();
                    }
                }
                if let Some(own) = choices.filament_over.get_mut(filament) {
                    own.clear();
                }
                while choices.filament_over.last().is_some_and(BTreeMap::is_empty) {
                    choices.filament_over.pop();
                }
            }
        }
        if !choices.settle_as(&project.choices) {
            project.forget();
        }
        project.choices = choices;
        let detail = project.detail();
        printers::remember(
            &mut state,
            at,
            &json!({ "process": book == Book::Process, "filaments": book == Book::Filament }),
        );
        if book == Book::Printer {
            remember_printer(&mut state, &vendor, name);
        }
        let mut answer = detail;
        if let Some(object) = answer.as_object_mut() {
            object.insert("saved".into(), json!(true));
            object.insert("preset".into(), json!(name));
        }
        Ok(answer)
    }

    /// `preset_rename`: one of the person's presets under another name,
    /// with every open project, and what is remembered with each printer,
    /// naming it so (`rename_user_preset`, `update_presets_from_to`). What
    /// is set over it in a project stays set over it.
    pub(super) fn preset_rename(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let kind = kept::kind(book);
        let old = needed(args, "preset")?.to_owned();
        let new = text(args, "name").unwrap_or_default().to_owned();
        let choices = {
            let state = lock(&self.state);
            state.projects[state.target(args)?].choices.clone()
        };
        let (vendor, machine) = Self::on_presets(&choices)?;
        let shelf = self.shelf(&vendor)?;
        if !self.is_users(kind, &old) {
            return Err(tr("System presets cannot be renamed."));
        }
        let (level, message) =
            self.check_name(&shelf, book, &new, Some(&old), true, Some(&machine));
        if level == Level::Invalid {
            return Err(message.unwrap_or_default());
        }
        if new == old {
            return self.after_change(args);
        }
        self.apply_rename(book, &old, &new)?;
        self.after_change(args)
    }

    /// One of the person's presets renamed, checked already: its files,
    /// every open project naming it, what is remembered with each printer
    /// and its favourite star follow (`rename_user_preset`,
    /// `update_presets_from_to`, `FavoriteFilamentsUI::rename_key`).
    pub(super) fn apply_rename(&self, book: Book, old: &str, new: &str) -> Result<(), String> {
        let kind = kept::kind(book);
        let (old, new) = (old.to_owned(), new.to_owned());
        let mut state = lock(&self.state);
        Arc::make_mut(&mut state.users.presets)
            .rename(kind, &old, &new)
            .map_err(|e| e.to_string())?;
        let renamed = |name: &mut String| {
            if *name == old {
                name.clone_from(&new);
            }
        };
        for project in &mut state.projects {
            let choices = &mut project.choices;
            for choices in [choices, &mut project.baseline] {
                match book {
                    Book::Process => choices.process.iter_mut().for_each(renamed),
                    Book::Printer => choices.machine.iter_mut().for_each(renamed),
                    Book::Filament => choices.filaments.iter_mut().for_each(renamed),
                }
            }
        }
        let (old_name, new_name) = (old.clone(), new.clone());
        if book == Book::Filament {
            drop(state);
            self.favorite_renamed(&old, Some(&new));
            state = lock(&self.state);
        }
        printers::keep(&mut state, |data| {
            if book == Book::Printer {
                if let Some(kept) = data.printers.remove(&old_name) {
                    data.printers.insert(new_name.clone(), kept);
                }
                if let Some((_, machine)) = &mut data.last
                    && *machine == old_name
                {
                    machine.clone_from(&new_name);
                }
            }
            for kept in data.printers.values_mut() {
                match book {
                    Book::Process => kept.process.iter_mut().for_each(|name| {
                        if *name == old_name {
                            name.clone_from(&new_name);
                        }
                    }),
                    Book::Filament => kept.filaments.iter_mut().for_each(|name| {
                        if *name == old_name {
                            name.clone_from(&new_name);
                        }
                    }),
                    Book::Printer => {}
                }
            }
        });
        Ok(())
    }

    /// `preset_delete`: one of the person's presets taken away, its file
    /// and its `.info` (Tab.cpp `delete_preset`). Refused for a system
    /// preset, and for one that other presets are laid over ("Presets
    /// inherited by other presets cannot be deleted!"). Without `confirm`
    /// the answer is the question the reference asks, with the open
    /// projects that use it; with it, the preset goes and every project
    /// that was on it goes to another of its printer's, what was set over
    /// it let go of.
    pub(super) fn preset_delete(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let kind = kept::kind(book);
        let name = needed(args, "preset")?.to_owned();
        let users = self.users();
        let Some(preset) = users.get(kind, &name) else {
            return Err(tr("System presets cannot be deleted."));
        };
        let children = users.children(kind, &name);
        let action = tr("Delete");
        let title = fill(&tr("%1% Preset"), &[&action]);
        if !children.is_empty() {
            // The reference passes the two sentences to its plural the
            // wrong way round, so one preset reads "The following presets
            // inherit this preset."; the count picks the sentence that
            // fits here.
            let sentence = if children.len() == 1 {
                trn(
                    "The following presets inherit this preset.",
                    "The following preset inherits this preset.",
                    2,
                )
            } else {
                trn(
                    "The following presets inherit this preset.",
                    "The following preset inherits this preset.",
                    1,
                )
            };
            let mut message = format!(
                "{}\n{sentence}",
                tr("Presets inherited by other presets cannot be deleted!")
            );
            for child in &children {
                message.push_str("\n - ");
                message.push_str(child);
            }
            return Err(message);
        }
        // A printer of one's own that stands alone ("Create printer") goes
        // with the presets made for it (Tab.cpp `delete_preset` l.10043,
        // `select_preset` l.9136, l.9348). The reference counts and deletes
        // every filament and process preset compatible with it, so the
        // vendor's that name no printer (or whose condition fits) go from
        // the run's lists too; here only the person's presets that belong
        // to it: those whose `compatible_printers` name it and no other,
        // as "Create Printer/Nozzle" makes them
        // (`clone_presets_for_printer`) and "Create Filament" too
        // (`clone_presets_for_filament`). See UI_PARITY.md section 8.
        let attached = if book == Book::Printer && preset.inherits.is_none() {
            self.attached_to(&name)
        } else {
            Vec::new()
        };
        let held: Vec<String> = attached
            .iter()
            .flat_map(|(kind, preset)| {
                users
                    .children(*kind, preset)
                    .into_iter()
                    .filter(|child| !attached.iter().any(|(_, other)| other == child))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect();
        if !held.is_empty() {
            let mut message = format!(
                "{}\n{}",
                tr("Presets inherited by other presets cannot be deleted!"),
                trn(
                    "The following presets inherit this preset.",
                    "The following preset inherits this preset.",
                    if held.len() == 1 { 2 } else { 1 },
                )
            );
            for child in &held {
                message.push_str("\n - ");
                message.push_str(child);
            }
            return Err(message);
        }
        if args.get("confirm").and_then(Value::as_bool) != Some(true) {
            let stands_alone = preset.inherits.is_none();
            if book == Book::Printer && stands_alone {
                let count = |kind: Kind| attached.iter().filter(|(k, _)| *k == kind).count();
                // Its own question, which stands for the usual one
                // (`confirm_delete_third_party_printer`).
                let message = fill(
                    &tr(
                        "%d Filament Preset and %d Process Preset is attached to this printer. Those presets would be deleted if the printer is deleted.",
                    ),
                    &[&count(Kind::Filament), &count(Kind::Process)],
                );
                return Ok(json!({
                    "deleted": false,
                    "title": tr("Delete"),
                    "ask": message,
                    "attached": attached.iter().map(|(_, name)| name).collect::<Vec<_>>(),
                }));
            }
            let mut message = if stands_alone && book == Book::Filament {
                tr(
                    "Are you sure to delete the selected preset? \nIf the preset corresponds to a filament currently in use on your printer, please reset the filament information for that slot.",
                )
            } else {
                fill(&tr("Are you sure to %1% the selected preset?"), &[&action])
            };
            let used = self.used_by(book, &name, args)?;
            if !used.is_empty() {
                message.push_str("\n\n");
                message.push_str(&tr("Used by:"));
                for line in used {
                    message.push_str("\n - ");
                    message.push_str(&line);
                }
            }
            return Ok(json!({ "deleted": false, "title": title, "ask": message }));
        }
        // Where its projects go, known before it is gone: the system
        // printer it is made from, else (one that stands alone) the
        // printer a project with none is given.
        let printer_to = (book == Book::Printer).then(|| self.system_printer(&name));
        {
            let mut state = lock(&self.state);
            let presets = Arc::make_mut(&mut state.users.presets);
            for (kind, attached) in &attached {
                presets.delete(*kind, attached).map_err(|e| e.to_string())?;
            }
            presets.delete(kind, &name).map_err(|e| e.to_string())?;
        }
        for (kind, attached) in &attached {
            let book = match kind {
                Kind::Process => Book::Process,
                _ => Book::Filament,
            };
            self.move_off(book, attached, None)?;
        }
        self.move_off(book, &name, printer_to.as_deref())?;
        if book == Book::Filament {
            self.favorite_renamed(&name, None);
        }
        for (kind, attached) in &attached {
            if *kind == Kind::Filament {
                self.favorite_renamed(attached, None);
            }
        }
        let mut answer = self.after_change(args)?;
        if let Some(object) = answer.as_object_mut() {
            object.insert("deleted".into(), json!(true));
        }
        Ok(answer)
    }

    /// The person's filament and process presets that belong to a printer
    /// of theirs: those whose `compatible_printers` name it and no other.
    pub(super) fn attached_to(&self, printer: &str) -> Vec<(Kind, String)> {
        let users = self.users();
        let mut attached = Vec::new();
        for kind in [Kind::Filament, Kind::Process] {
            for preset in users.of(kind) {
                let named: Vec<&String> = preset
                    .own
                    .texts("compatible_printers")
                    .unwrap_or_default()
                    .iter()
                    .filter(|named| !named.is_empty())
                    .collect();
                if !named.is_empty() && named.iter().all(|named| *named == printer) {
                    attached.push((kind, preset.name.clone()));
                }
            }
        }
        attached
    }

    /// The lines "Used by:" lists: where the open projects use a preset.
    pub(super) fn used_by(
        &self,
        book: Book,
        name: &str,
        args: &Value,
    ) -> Result<Vec<String>, String> {
        let state = lock(&self.state);
        let shown = state.projects[state.target(args)?].id;
        let mut lines = Vec::new();
        for project in &state.projects {
            let choices = &project.choices;
            if !choices.from_presets {
                continue;
            }
            let mut here = Vec::new();
            match book {
                Book::Process if choices.process.as_deref() == Some(name) => {
                    here.push(tr("The plate's process"));
                }
                Book::Printer if choices.machine.as_deref() == Some(name) => {
                    here.push(tr("Printer"));
                }
                Book::Filament => {
                    for slot in 0..project.filament_count() {
                        let chosen = choices.filaments.get(slot).or(choices.filaments.last());
                        if chosen.map(String::as_str) == Some(name) {
                            here.push(fill(
                                &tr("Filament slot %d of this project"),
                                &[&(slot + 1)],
                            ));
                        }
                    }
                }
                _ => {}
            }
            for line in here {
                if project.id == shown {
                    lines.push(line);
                } else {
                    lines.push(format!("{}: {line}", project.name));
                }
            }
        }
        Ok(lines)
    }

    /// Every open project on a preset that is gone goes to another: a
    /// process or a filament to the one its printer would be given, a
    /// printer to the system preset it was made from; what was set over
    /// the preset is let go of (`delete_user_preset_by_name`,
    /// `discard_current_changes`).
    fn move_off(&self, book: Book, name: &str, printer_to: Option<&str>) -> Result<(), String> {
        let on: Vec<(u32, Choices, usize)> = lock(&self.state)
            .projects
            .iter()
            .filter(|project| project.choices.from_presets)
            .map(|project| {
                (
                    project.id,
                    project.choices.clone(),
                    project.filament_count(),
                )
            })
            .collect();
        for (id, choices, count) in on {
            let (Some(vendor), Some(machine)) = (choices.vendor.clone(), choices.machine.clone())
            else {
                continue;
            };
            if book == Book::Printer {
                if machine == name {
                    // The system printer it was made from; one that stood
                    // alone gives way to the printer a project with none
                    // is given (the reference selects the next visible
                    // printer, Tab.cpp `select_preset` with `delete_current`).
                    let to = match printer_to.filter(|base| *base != name) {
                        Some(base) => Some((vendor.clone(), base.to_owned())),
                        None => self.usual_printer(&self.installed()),
                    };
                    {
                        let mut state = lock(&self.state);
                        if let Some(at) = state.index_of(id) {
                            state.projects[at].choices.printer_over.clear();
                        }
                    }
                    if let Some((vendor, base)) = to {
                        self.put_on(id, &vendor, &base, false, false)?;
                    }
                }
                continue;
            }
            let shelf = self.shelf(&vendor)?;
            let offered = self.offered(&shelf, &machine);
            let base = self.system_printer(&machine);
            let (usual_process, usual_filament) = shelf.usual_for(&base);
            let kind = kept::kind(book);
            let fits: Vec<&str> = match book {
                Book::Process => offered.all_processes(),
                _ => offered.all_filaments(),
            };
            let usual = match book {
                Book::Process => usual_process.or_else(|| offered.processes.first().cloned()),
                _ => usual_filament.or_else(|| offered.filaments.first().cloned()),
            };
            let replacement = self
                .next_after(&shelf, kind, name)
                .filter(|next| fits.contains(&next.as_str()))
                .or(usual)
                .unwrap_or_default();
            let mut state = lock(&self.state);
            let Some(at) = state.index_of(id) else {
                continue;
            };
            let project = &mut state.projects[at];
            let mut next = project.choices.clone();
            match book {
                Book::Process if next.process.as_deref() == Some(name) => {
                    next.process = Some(replacement);
                    next.over.clear();
                }
                Book::Filament => {
                    for slot in 0..count.max(next.filaments.len()) {
                        if next.filaments.get(slot).map(String::as_str) == Some(name) {
                            next.filaments[slot].clone_from(&replacement);
                            if let Some(own) = next.filament_over.get_mut(slot) {
                                own.clear();
                            }
                        }
                    }
                    while next.filament_over.last().is_some_and(BTreeMap::is_empty) {
                        next.filament_over.pop();
                    }
                }
                _ => {}
            }
            if !next.settle_as(&project.choices) {
                project.forget();
            }
            project.choices = next;
        }
        // What was remembered with a printer names it no longer.
        let gone = name.to_owned();
        let mut state = lock(&self.state);
        printers::keep(&mut state, |data| match book {
            Book::Process => {
                for kept in data.printers.values_mut() {
                    if kept.process.as_deref() == Some(gone.as_str()) {
                        kept.process = None;
                    }
                }
            }
            Book::Filament => {
                for kept in data.printers.values_mut() {
                    kept.filaments.retain(|filament| *filament != gone);
                }
            }
            Book::Printer => {
                data.printers.remove(&gone);
                if data
                    .last
                    .as_ref()
                    .is_some_and(|(_, machine)| *machine == gone)
                {
                    data.last = None;
                }
            }
        });
        Ok(())
    }

    /// The preset the reference goes to when one is deleted: the next of
    /// its kind by name (the vendor's and the person's in one list, as a
    /// `PresetCollection` keeps them), else the one before (Tab.cpp
    /// `select_preset` with `delete_current`, l.8978). The caller takes it
    /// when it goes with the printer; else the printer's usual one, as the
    /// reference's `update_compatible` then picks its preferred preset.
    /// A vendor's filament counts only while it is shown (enabled, or on
    /// an open project), as the reference passes over one not visible.
    fn next_after(&self, shelf: &Shelf, kind: Kind, name: &str) -> Option<String> {
        let users = self.users();
        let shown = match kind {
            Kind::Filament => self.shown_filaments(),
            _ => None,
        };
        let mut names: Vec<String> = shelf.all_of(kind);
        names.retain(|other| shown.as_ref().is_none_or(|shown| shown.contains(other)));
        names.extend(
            users
                .of(kind)
                .filter(|preset| preset.visible && preset.name != name)
                .map(|preset| preset.name.clone()),
        );
        names.sort();
        names.dedup();
        names.retain(|other| other != name);
        let at = names.partition_point(|other| other.as_str() < name);
        names
            .get(at)
            .or_else(|| at.checked_sub(1).and_then(|before| names.get(before)))
            .cloned()
    }

    /// The project shown (or named) as it is after a change to the
    /// presets.
    fn after_change(&self, args: &Value) -> Result<Value, String> {
        let state = lock(&self.state);
        Ok(state.projects[state.target(args)?].detail())
    }

    /// `preset_compare`: two presets of a kind side by side, the options
    /// they differ in by page and group (DiffPresetDialog.cpp). Either is
    /// the vendor's or the person's; the vendor is the project's unless
    /// named. Without `left` and `right`, only the presets to choose from:
    /// those offered for the project's printer, or with `all` every one
    /// of the kind ("Show all presets (including incompatible)").
    pub(super) fn preset_compare(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let kind = kept::kind(book);
        let choices = {
            let state = lock(&self.state);
            state.projects[state.target(args)?].choices.clone()
        };
        let vendor = match text(args, "vendor") {
            Some(vendor) => vendor.to_owned(),
            None => Self::on_presets(&choices)?.0,
        };
        let shelf = self.shelf(&vendor)?;
        let all = args.get("all").and_then(Value::as_bool) == Some(true);
        let users = self.users();
        let (system, mine): (Vec<String>, Vec<String>) = match (&choices.machine, all, book) {
            (Some(machine), false, Book::Process | Book::Filament) => {
                let offered = self.offered(&shelf, machine);
                match book {
                    Book::Process => (offered.processes, offered.user_processes),
                    _ => (offered.filaments, offered.user_filaments),
                }
            }
            _ => {
                let system = match book {
                    Book::Printer => shelf.machines().to_vec(),
                    Book::Process => shelf.all_of(Kind::Process),
                    Book::Filament => shelf.all_of(Kind::Filament),
                };
                let mine = users
                    .of(kind)
                    .filter(|preset| preset.visible && !shelf.knows(kind, &preset.name))
                    .map(|preset| preset.name.clone())
                    .collect();
                (system, mine)
            }
        };
        let mut answer = json!({
            "kind": book_name(book),
            "vendor": vendor,
            "system": system,
            "user": mine,
        });
        if let (Some(left), Some(right)) = (text(args, "left"), text(args, "right")) {
            let left_settings = self.resolve_named(&shelf, kind, left)?;
            let right_settings = self.resolve_named(&shelf, kind, right)?;
            let listed = kept::differences(book, &left_settings, &right_settings);
            if let Some(object) = answer.as_object_mut() {
                object.insert("left".into(), json!(left));
                object.insert("right".into(), json!(right));
                object.insert("count".into(), json!(kept::count(&listed)));
                object.insert("pages".into(), listed);
            }
        }
        Ok(answer)
    }

    /// `preset_transfer`: "Compare presets" with "Transfer values from
    /// left to right" (DiffPresetDialog.cpp l.2037–2090, `Tab::transfer_options`
    /// l.9797): the options named (`keys`) take the left preset's (`from`)
    /// values, set over the right one (`to`), which the project's preset of
    /// the kind (and filament) must be: the page selects it first, as the
    /// reference's tab does. Each value goes through the pages' own checks
    /// (`settings_set`); the first one refused stops the transfer, with
    /// what came before it kept.
    pub(super) fn preset_transfer(&self, args: &Value) -> Result<Value, String> {
        let book = kind_of(args)?;
        let kind = kept::kind(book);
        let from = needed(args, "from")?;
        let to = needed(args, "to")?;
        let (choices, filaments) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.choices.clone(), project.filament_count())
        };
        let (vendor, _) = Self::on_presets(&choices)?;
        let filament = filament_of(args, book, filaments)?;
        let (current, modified) = Self::selected(&choices, book, filament);
        if current.as_deref() != Some(to) {
            return Err(if modified {
                tr("You can only transfer to current active profile because it has been modified.")
            } else {
                format!("the project is not on \"{to}\"")
            });
        }
        let shelf = self.shelf(&vendor)?;
        let left = self.resolve_named(&shelf, kind, from)?;
        let right = self.resolve_named(&shelf, kind, to)?;
        let mut values = BTreeMap::new();
        let listed = kept::differences(book, &left, &right);
        for page in listed.as_array().into_iter().flatten() {
            for group in page["groups"].as_array().into_iter().flatten() {
                for row in group["rows"].as_array().into_iter().flatten() {
                    if let (Some(key), Some(old)) = (row["key"].as_str(), row["old"].as_str()) {
                        values.insert(key.to_owned(), old.to_owned());
                    }
                }
            }
        }
        let keys: Vec<&str> = args
            .get("keys")
            .and_then(Value::as_array)
            .ok_or("keys were not given")?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let mut set = 0;
        for key in keys {
            let Some(value) = values.get(key) else {
                continue;
            };
            let mut one = json!({ "key": key, "value": value });
            if let Some(project) = args.get("project") {
                one["project"] = project.clone();
            }
            if book != Book::Process {
                one["kind"] = json!(book_name(book));
            }
            if book == Book::Filament {
                one["filament"] = json!(filament + 1);
            }
            self.settings_set(&one, book)?;
            set += 1;
        }
        let mut answer = self.after_change(args)?;
        if let Some(object) = answer.as_object_mut() {
            object.insert("transferred".into(), json!(set));
        }
        Ok(answer)
    }

    /// Reads the person's presets again from their folder, as they now
    /// are on the disk.
    pub(super) fn presets_reload(&self) -> Value {
        let mut state = lock(&self.state);
        if let Some(dir) = state.users.dir.clone() {
            state.users.presets = Arc::new(UserPresets::open(&dir));
        }
        users_json(&state)
    }

    /// `user_presets`: the person's presets of every kind, where they are
    /// kept, and the files there that could not be read.
    pub(super) fn user_presets(&self) -> Value {
        users_json(&lock(&self.state))
    }
}

/// The person's presets, for the page.
fn users_json(state: &State) -> Value {
    let presets = &state.users.presets;
    let listed = |kind: Kind| -> Vec<Value> {
        presets
            .of(kind)
            .map(|preset| json!({ "name": preset.name, "inherits": preset.inherits }))
            .collect()
    };
    json!({
        "dir": state.users.dir.as_deref().map(super::shown),
        "printer": listed(Kind::Machine),
        "process": listed(Kind::Process),
        "filament": listed(Kind::Filament),
        "unread": presets
            .unread()
            .iter()
            .map(|unread| json!({ "file": super::shown(&unread.file), "reason": unread.reason }))
            .collect::<Vec<_>>(),
    })
}

/// The `setting_id` of a vendor's preset, as its own file gives it.
fn parent_setting_id(shelf: &Shelf, kind: Kind, name: &str) -> Option<String> {
    shelf
        .resolve(kind, name)
        .ok()?
        .text("setting_id")
        .map(str::to_owned)
}

fn book_name(book: Book) -> &'static str {
    match book {
        Book::Process => "process",
        Book::Filament => "filament",
        Book::Printer => "printer",
    }
}

/// The filament a call is about (`filament`, from 1), from 0; 0 for the
/// other kinds.
fn filament_of(args: &Value, book: Book, filaments: usize) -> Result<usize, String> {
    if book != Book::Filament {
        return Ok(0);
    }
    let number = args
        .get("filament")
        .and_then(Value::as_u64)
        .ok_or("filament was not given")?;
    let number = usize::try_from(number).unwrap_or(usize::MAX);
    if number == 0 || number > filaments {
        return Err(format!(
            "the project has {filaments} filament(s); there is no filament {number}"
        ));
    }
    Ok(number - 1)
}

/// A printer saved and chosen is the one last selected, as the reference
/// keeps its selection.
fn remember_printer(state: &mut State, vendor: &str, machine: &str) {
    let (vendor, machine) = (vendor.to_owned(), machine.to_owned());
    printers::keep(state, |data| {
        data.last = Some((vendor, machine));
    });
}

#[cfg(test)]
mod tests;
