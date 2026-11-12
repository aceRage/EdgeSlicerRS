//! A person's own presets, kept as the reference keeps them, so a folder
//! copied from the C++ application's data folder, or into it, is read the
//! same by both.
//!
//! The layout (PresetBundle.cpp `load_user_presets`, Preset.cpp
//! `PresetCollection::load_presets`, `path_from_name`):
//!
//! ```text
//! <data folder>/user/<user id, "default" without an account>/
//!     machine/<name>.json   machine/<name>.info
//!     filament/<name>.json  filament/<name>.info
//!     process/<name>.json   process/<name>.info
//!     <kind>/base/<name>.json   a preset that inherits from nothing
//! ```
//!
//! A preset's JSON file holds only what differs from the preset it
//! inherits from (`Preset::save` with the parent's config), the name of
//! that preset (`inherits`), its own name twice (`name` and the kind's
//! `print_settings_id`, `filament_settings_id` or `printer_settings_id`),
//! `from: "User"`, `is_custom_defined` and `version`; every value is text
//! or a list of texts, the keys in order, four spaces a level, as the
//! reference's JSON library writes them (Config.cpp
//! `ConfigBase::save_to_json`). The `.info` beside it is a few lines of
//! `key = value` (`Preset::save_info`): `sync_info`, `user_id`,
//! `setting_id`, `base_id` (the `setting_id` of the system preset it was
//! made from) and `updated_time` (seconds since 1970).
//!
//! What differs from the reference, on purpose:
//! - A file that cannot be read is left where it is and listed
//!   ([`UserPresets::unread`]); the reference moves it aside into a
//!   quarantine folder (upstream deleted it).
//! - A file with no `version`, or one that does not read as a version, is
//!   read all the same; the reference passes over it without a word
//!   (Preset.cpp l.1764 `if (!version) continue;`), so the preset is gone
//!   from its lists with nothing said.
//! - A new preset's `.info` gets the time it was saved; the reference
//!   writes 0 and puts the time in when it next reads the file
//!   (`Preset::load_info`, "workaround for current info file convert").
//! - A chain of presets of the person's own (a preset made from one made
//!   from a system preset) is followed to its end wherever the reference
//!   looks only one step up (`is_compatible_with_parent_printer`,
//!   "FIXME only check the parent now").
//!
//! Every file is written whole or not at all: beside its place first,
//! then renamed over it.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{ConfigError, Kind, Settings, Value};

/// The folder under the data folder that holds every account's presets.
pub const USER_FOLDER: &str = "user";
/// The account folder used without an account (GUI_App.cpp
/// `DEFAULT_USER_FOLDER_NAME`).
pub const DEFAULT_USER: &str = "default";
/// What a preset's name may not hold, on every system alike
/// (Preset.cpp `preset_name_illegal_characters`).
pub const ILLEGAL_CHARACTERS: &str = "<>[]:/\\|?*\"";
/// What the reference puts after a preset's name in its lists while it
/// has unsaved changes (Preset.cpp `g_suffix_modified`); a name may not
/// hold it.
pub const SUFFIX_MODIFIED: &str = " (modified)";
/// The names of the reference's default presets, which a preset may not
/// take.
pub const RESERVED: [&str; 3] = ["Default Setting", "Default Filament", "Default Printer"];
/// The kinds a person keeps presets of.
pub const KINDS: [Kind; 3] = [Kind::Machine, Kind::Filament, Kind::Process];

/// Keys of a preset's file that say what the file is rather than set
/// anything (Config.cpp `load_from_json` keeps them apart).
const HEADER_KEYS: [&str; 9] = [
    "name",
    "from",
    "version",
    "is_custom_defined",
    "description",
    "type",
    "url",
    "instantiation",
    "inherits",
];

/// The folder of a kind of preset (Preset.hpp `PRESET_PRINTER_NAME`,
/// `PRESET_FILAMENT_NAME`, `PRESET_PRINT_NAME`).
pub fn folder(kind: Kind) -> &'static str {
    match kind {
        Kind::Machine | Kind::MachineModel => "machine",
        Kind::Filament => "filament",
        Kind::Process => "process",
    }
}

/// The setting a preset of this kind names itself by.
pub fn id_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Machine | Kind::MachineModel => "printer_settings_id",
        Kind::Filament => "filament_settings_id",
        Kind::Process => "print_settings_id",
    }
}

/// The kind a file's `type` says it is: either the folder's word or the
/// reference's other one (`Preset::get_type_from_string`).
fn kind_of_type(text: &str) -> Option<Kind> {
    match text {
        "machine" | "printer" => Some(Kind::Machine),
        "filament" => Some(Kind::Filament),
        "process" | "print" => Some(Kind::Process),
        _ => None,
    }
}

/// Seconds since 1970, as the reference stamps a preset.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// What the reference keeps beside a preset for its cloud (`.info`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// "create", "update", "delete", "hold" or nothing.
    pub sync_info: String,
    pub user_id: String,
    /// The preset's own id in the cloud; none for one never sent there.
    pub setting_id: String,
    /// The `setting_id` of the system preset it was made from.
    pub base_id: String,
    pub updated_time: i64,
}

impl Info {
    /// Reads the `key = value` lines of an `.info` (`Preset::load_info`):
    /// "null" stands for nothing; a line it does not know is passed over.
    pub fn parse(text: &str) -> Self {
        let mut info = Info::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().to_owned();
            let value_or_none = |value: String| {
                if value == "null" {
                    String::new()
                } else {
                    value
                }
            };
            match key.trim() {
                "sync_info" => info.sync_info = value,
                "user_id" => info.user_id = value,
                "setting_id" => info.setting_id = value_or_none(value),
                "base_id" => info.base_id = value_or_none(value),
                "updated_time" => info.updated_time = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        info
    }

    /// The file as the reference writes it (`Preset::save_info`): "hold"
    /// is not written.
    pub fn text(&self) -> String {
        let sync = if self.sync_info == "hold" {
            ""
        } else {
            &self.sync_info
        };
        format!(
            "sync_info = {sync}\nuser_id = {}\nsetting_id = {}\nbase_id = {}\nupdated_time = {}\n",
            self.user_id, self.setting_id, self.base_id, self.updated_time
        )
    }
}

/// One of a person's presets, as its file has it.
#[derive(Clone, Debug, PartialEq)]
pub struct UserPreset {
    pub kind: Kind,
    pub name: String,
    pub file: PathBuf,
    /// The settings the file sets, without what says what the file is.
    pub own: Settings,
    /// The preset it is laid over; none for one that stands alone (kept
    /// in `base/`).
    pub inherits: Option<String>,
    pub version: Option<String>,
    pub info: Info,
    /// Whether it is offered in the lists (`instantiation` not "false").
    pub visible: bool,
}

/// A file in the folder that could not be taken as a preset, and why. It
/// is left as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    pub file: PathBuf,
    pub reason: String,
}

/// What is saved: the settings that differ from the preset it is laid
/// over, and that preset.
#[derive(Clone, Debug, Default)]
pub struct ToSave {
    /// The settings to write, without the name, `inherits` or the
    /// kind's id, which are put in by the save.
    pub own: Settings,
    pub inherits: Option<String>,
    /// The version written, the vendor's as the reference writes it
    /// ("2.3.3.14").
    pub version: String,
    /// The `setting_id` of the system preset it comes from.
    pub base_id: Option<String>,
}

/// A person's presets of every kind, read from one account's folder.
#[derive(Clone, Debug, Default)]
pub struct UserPresets {
    dir: PathBuf,
    presets: Vec<UserPreset>,
    unread: Vec<Unread>,
}

/// The JSON text of a preset, as the reference's library writes it
/// (`json::dump(4)`): the keys in order, four spaces a level, a new line
/// at the end.
pub fn to_json(values: &BTreeMap<String, Value>) -> String {
    let quoted = |text: &str| serde_json::to_string(text).expect("text always serialises");
    let mut out = String::from("{");
    for (at, (key, value)) in values.iter().enumerate() {
        out.push_str(if at == 0 { "\n    " } else { ",\n    " });
        out.push_str(&quoted(key));
        out.push_str(": ");
        match value {
            Value::One(text) => out.push_str(&quoted(text)),
            Value::Many(items) if items.is_empty() => out.push_str("[]"),
            Value::Many(items) => {
                out.push('[');
                for (at, item) in items.iter().enumerate() {
                    out.push_str(if at == 0 { "\n        " } else { ",\n        " });
                    out.push_str(&quoted(item));
                }
                out.push_str("\n    ]");
            }
        }
    }
    if !values.is_empty() {
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// Writes a file whole or not at all: beside its place, then renamed over
/// it.
pub fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let io = |source| ConfigError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io)?;
    }
    let leaf = path
        .file_name()
        .map_or_else(String::new, |leaf| leaf.to_string_lossy().into_owned());
    let beside = path.with_file_name(format!(".{leaf}.{}.part", std::process::id()));
    if let Err(error) = fs::write(&beside, bytes) {
        let _ = fs::remove_file(&beside);
        return Err(io(error));
    }
    fs::rename(&beside, path).map_err(|error| {
        let _ = fs::remove_file(&beside);
        io(error)
    })
}

/// Why a name cannot be a preset's, as far as the name alone says
/// (Preset.cpp `validate_preset_name`, the checks that need no list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameProblem {
    IllegalCharacters,
    IllegalSuffix,
    Reserved,
    Empty,
    LeadingSpace,
    TrailingSpace,
}

/// Checks a name as the reference does, in its order.
pub fn check_name(name: &str) -> Result<(), NameProblem> {
    if name.contains(|c| ILLEGAL_CHARACTERS.contains(c)) {
        return Err(NameProblem::IllegalCharacters);
    }
    if name.contains(SUFFIX_MODIFIED) {
        return Err(NameProblem::IllegalSuffix);
    }
    if RESERVED.contains(&name) {
        return Err(NameProblem::Reserved);
    }
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if name.starts_with(' ') {
        return Err(NameProblem::LeadingSpace);
    }
    if name.ends_with(' ') {
        return Err(NameProblem::TrailingSpace);
    }
    Ok(())
}

/// A text with its C-style escapes taken out, for comparing a value
/// written either way.
fn unescaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Whether two texts of a setting mean the same: the same text, the same
/// number however written ("0.20" and "0.2"), or the same text once
/// escapes are taken out.
fn same_text(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let number = |text: &str| -> Option<(f64, bool)> {
        let text = text.trim();
        let (digits, percent) = match text.strip_suffix('%') {
            Some(digits) => (digits, true),
            None => (text, false),
        };
        digits.parse::<f64>().ok().map(|n| (n, percent))
    };
    if let (Some((x, xp)), Some((y, yp))) = (number(a), number(b)) {
        return xp == yp && (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0);
    }
    unescaped(a) == unescaped(b)
}

/// Whether two values of a setting mean the same, as the reference
/// compares two options of one type: a single value and a list of one
/// are the same.
pub fn same(a: &Value, b: &Value) -> bool {
    let items = |value: &Value| -> Vec<String> {
        match value {
            Value::One(text) => vec![text.clone()],
            Value::Many(items) => items.clone(),
        }
    };
    let (a, b) = (items(a), items(b));
    a.len() == b.len() && a.iter().zip(&b).all(|(x, y)| same_text(x, y))
}

/// What of `full` a preset laid over `parent` writes (Preset.cpp
/// `Preset::save` with the parent's config): every setting the reference
/// knows whose value differs from the parent's, and one the parent lacks
/// whose value differs from the reference's default; with no parent,
/// every setting. A flow-variant list of more than one entry is always
/// written (`keep_flow_support_in_differential_save`). Names and ids are
/// left out: the save puts them in.
pub fn differences(full: &Settings, parent: Option<&Settings>, kind: Kind) -> Settings {
    let defaults = crate::reference_defaults();
    let flow_support = match kind {
        Kind::Filament => "filament_flow_support",
        Kind::Process => "process_flow_support",
        _ => "printer_flow_support",
    };
    let mut own = Settings::default();
    for (key, value) in full.iter() {
        if HEADER_KEYS.contains(&key) || key == id_key(kind) || !crate::reference_knows(key) {
            continue;
        }
        let differs = match parent {
            None => true,
            Some(parent) => match parent.get(key) {
                Some(theirs) => !same(value, theirs),
                None => defaults
                    .get(key)
                    .is_none_or(|default| !same(value, default)),
            },
        };
        let flows = key == flow_support && matches!(value, Value::Many(items) if items.len() > 1);
        if differs || flows {
            own.insert(key, value.clone());
        }
    }
    own
}

impl UserPresets {
    /// The folder of an account under a data folder.
    pub fn account_dir(data_dir: &Path, user: &str) -> PathBuf {
        data_dir.join(USER_FOLDER).join(user)
    }

    /// None at all, kept in `dir` once one is saved.
    pub fn empty(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            ..Self::default()
        }
    }

    /// Reads every preset under an account's folder. What cannot be read
    /// is listed in [`UserPresets::unread`] and left as it is; a folder
    /// that is not there holds none.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        let mut presets = Self::empty(dir);
        for kind in KINDS {
            let kind_dir = presets.dir.join(folder(kind));
            // Presets that stand alone first, as the reference reads them.
            for sub in [kind_dir.join("base"), kind_dir.clone()] {
                let Ok(entries) = fs::read_dir(&sub) else {
                    continue;
                };
                let mut files: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.is_file()
                            && path
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
                    })
                    .collect();
                files.sort();
                for file in files {
                    presets.read_one(kind, &file);
                }
            }
        }
        presets
            .presets
            .sort_by(|a, b| (folder(a.kind), &a.name).cmp(&(folder(b.kind), &b.name)));
        presets
    }

    fn read_one(&mut self, kind: Kind, file: &Path) {
        let unread = |reason: String| Unread {
            file: file.to_owned(),
            reason,
        };
        let text = match fs::read(file) {
            Ok(bytes) => {
                String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                    .into_owned()
            }
            Err(error) => {
                self.unread.push(unread(error.to_string()));
                return;
            }
        };
        let mut settings = match Settings::from_json(&text) {
            Ok(settings) => settings,
            Err(reason) => {
                self.unread.push(unread(reason));
                return;
            }
        };
        if let Some(said) = settings.text("type").and_then(kind_of_type)
            && said != kind
        {
            self.unread.push(unread(format!(
                "it is a {} preset in the {} folder",
                folder(said),
                folder(kind)
            )));
            return;
        }
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = settings
            .text("name")
            .filter(|name| !name.is_empty())
            .map_or(stem, str::to_owned);
        if self.get(kind, &name).is_some() {
            self.unread.push(unread(format!(
                "another file holds a {} preset named \"{name}\"",
                folder(kind)
            )));
            return;
        }
        let inherits = settings
            .text("inherits")
            .filter(|parent| !parent.is_empty())
            .map(str::to_owned);
        let version = settings.text("version").map(str::to_owned);
        let visible = settings.text("instantiation") != Some("false");
        for key in HEADER_KEYS {
            settings.remove(key);
        }
        let info = fs::read_to_string(file.with_extension("info"))
            .map(|text| Info::parse(&text))
            .unwrap_or_default();
        self.presets.push(UserPreset {
            kind,
            name,
            file: file.to_owned(),
            own: settings,
            inherits,
            version,
            info,
            visible,
        });
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The files that could not be taken as presets, and why.
    pub fn unread(&self) -> &[Unread] {
        &self.unread
    }

    /// A kind's presets, in order of name.
    pub fn of(&self, kind: Kind) -> impl Iterator<Item = &UserPreset> {
        self.presets
            .iter()
            .filter(move |preset| preset.kind == kind)
    }

    pub fn get(&self, kind: Kind, name: &str) -> Option<&UserPreset> {
        self.presets
            .iter()
            .find(|preset| preset.kind == kind && preset.name == name)
    }

    /// The presets laid over this one.
    pub fn children(&self, kind: Kind, name: &str) -> Vec<&str> {
        self.of(kind)
            .filter(|preset| preset.inherits.as_deref() == Some(name))
            .map(|preset| preset.name.as_str())
            .collect()
    }

    /// The first preset up the chain that is not one of these: the system
    /// preset it was made from. None for one that stands alone.
    pub fn system_parent(&self, kind: Kind, name: &str) -> Option<String> {
        let mut seen = vec![name.to_owned()];
        let mut at = self.get(kind, name)?;
        loop {
            let parent = at.inherits.clone()?;
            match self.get(kind, &parent) {
                Some(next) if !seen.contains(&parent) => {
                    seen.push(parent);
                    at = next;
                }
                Some(_) => return None,
                None => return Some(parent),
            }
        }
    }

    /// Flattens a preset: its own settings, then each preset's up the
    /// chain for whatever is still unset, into the system preset it
    /// comes from (`system` finds that by name). The name, `inherits` and
    /// the kind's id stay the preset's own.
    pub fn resolve(
        &self,
        kind: Kind,
        name: &str,
        system: &dyn Fn(&str) -> Result<Settings, ConfigError>,
    ) -> Result<Settings, ConfigError> {
        let leaf = self
            .get(kind, name)
            .ok_or_else(|| ConfigError::UnknownPreset {
                vendor: "user".to_owned(),
                kind,
                name: name.to_owned(),
            })?;
        let identity = |key: &str| {
            matches!(
                key,
                "name" | "inherits" | "instantiation" | "setting_id" | "type" | "from"
            ) || key == id_key(kind)
        };
        let mut resolved = leaf.own.clone();
        resolved.insert("name", Value::One(leaf.name.clone()));
        if let Some(parent) = &leaf.inherits {
            resolved.insert("inherits", Value::One(parent.clone()));
        }
        let id = match kind {
            Kind::Filament => Value::Many(vec![leaf.name.clone()]),
            _ => Value::One(leaf.name.clone()),
        };
        resolved.insert(id_key(kind), id);
        let mut chain = vec![leaf.name.clone()];
        let mut parent = leaf.inherits.clone();
        while let Some(wanted) = parent {
            if chain.contains(&wanted) {
                chain.push(wanted);
                return Err(ConfigError::InheritanceCycle {
                    name: name.to_owned(),
                    chain: chain.join(" -> "),
                });
            }
            match self.get(kind, &wanted) {
                Some(user) => {
                    resolved.inherit(&user.own, identity);
                    chain.push(wanted);
                    parent = user.inherits.clone();
                }
                None => {
                    resolved.inherit(&system(&wanted)?, identity);
                    parent = None;
                }
            }
        }
        Ok(resolved)
    }

    /// Where a preset of this name is kept: `base/` for one that stands
    /// alone (`PresetCollection::path_for_preset`).
    pub fn path_for(&self, kind: Kind, name: &str, stands_alone: bool) -> PathBuf {
        let kind_dir = self.dir.join(folder(kind));
        let dir = if stands_alone {
            kind_dir.join("base")
        } else {
            kind_dir
        };
        dir.join(format!("{name}.json"))
    }

    /// The JSON a preset is written as.
    fn file_text(kind: Kind, name: &str, preset: &ToSave) -> String {
        let mut values: BTreeMap<String, Value> = preset
            .own
            .iter()
            .filter(|(key, _)| !HEADER_KEYS.contains(key) && *key != id_key(kind))
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        values.insert("name".into(), Value::One(name.to_owned()));
        values.insert("from".into(), Value::One("User".into()));
        values.insert("is_custom_defined".into(), Value::One("0".into()));
        values.insert("version".into(), Value::One(preset.version.clone()));
        values.insert(
            "inherits".into(),
            Value::One(preset.inherits.clone().unwrap_or_default()),
        );
        let id = match kind {
            Kind::Filament => Value::Many(vec![name.to_owned()]),
            _ => Value::One(name.to_owned()),
        };
        values.insert(id_key(kind).into(), id);
        to_json(&values)
    }

    /// Saves a preset under a name: a new one, or over one of these of
    /// that name (the person was asked). The file is written whole, then
    /// its `.info`. A preset that stands alone goes to `base/`.
    pub fn save(&mut self, kind: Kind, name: &str, preset: ToSave) -> Result<(), ConfigError> {
        let existing = self.get(kind, name).cloned();
        let file = match &existing {
            Some(old) if old.inherits.is_some() == preset.inherits.is_some() => old.file.clone(),
            _ => self.path_for(kind, name, preset.inherits.is_none()),
        };
        let mut info = existing
            .as_ref()
            .map(|old| old.info.clone())
            .unwrap_or_default();
        info.sync_info = if existing.is_some() {
            "update"
        } else {
            "create"
        }
        .to_owned();
        if info.base_id.is_empty()
            && let Some(base) = &preset.base_id
        {
            info.base_id.clone_from(base);
        }
        info.updated_time = now();
        write_whole(&file, Self::file_text(kind, name, &preset).as_bytes())?;
        write_whole(&file.with_extension("info"), info.text().as_bytes())?;
        // A preset moved between `base/` and its kind's folder leaves its
        // old place.
        if let Some(old) = &existing
            && old.file != file
        {
            remove_files(&old.file)?;
        }
        let saved = UserPreset {
            kind,
            name: name.to_owned(),
            file,
            own: preset.own,
            inherits: preset.inherits,
            version: Some(preset.version),
            info,
            visible: true,
        };
        self.presets.retain(|p| !(p.kind == kind && p.name == name));
        self.presets.push(saved);
        self.presets
            .sort_by(|a, b| (folder(a.kind), &a.name).cmp(&(folder(b.kind), &b.name)));
        Ok(())
    }

    /// Gives a preset another name (Preset.cpp `rename_user_preset`): it
    /// is written under the new name before the old files go, and every
    /// preset laid over it is written again naming it so.
    pub fn rename(&mut self, kind: Kind, old: &str, new: &str) -> Result<(), ConfigError> {
        let preset = self
            .get(kind, old)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownPreset {
                vendor: "user".to_owned(),
                kind,
                name: old.to_owned(),
            })?;
        let file = self.path_for(kind, new, preset.inherits.is_none());
        let mut info = preset.info.clone();
        info.updated_time = now();
        if !info.setting_id.is_empty() && info.sync_info.is_empty() {
            info.sync_info = "update".to_owned();
        }
        let to_save = ToSave {
            own: preset.own.clone(),
            inherits: preset.inherits.clone(),
            version: preset.version.clone().unwrap_or_default(),
            base_id: None,
        };
        let new_text = Self::file_text(kind, new, &to_save);
        // Only the case of a letter changed: on a disk where that is the
        // same file, the old one goes first, and comes back if the new
        // one cannot be written.
        let same_file =
            file.to_string_lossy().to_lowercase() == preset.file.to_string_lossy().to_lowercase();
        if same_file {
            let old_text = fs::read(&preset.file).map_err(|source| ConfigError::Io {
                path: preset.file.clone(),
                source,
            })?;
            let old_info = fs::read(preset.file.with_extension("info")).ok();
            remove_files(&preset.file)?;
            if let Err(error) = write_whole(&file, new_text.as_bytes()) {
                write_whole(&preset.file, &old_text)?;
                if let Some(old_info) = old_info {
                    write_whole(&preset.file.with_extension("info"), &old_info)?;
                }
                return Err(error);
            }
            write_whole(&file.with_extension("info"), info.text().as_bytes())?;
        } else {
            write_whole(&file, new_text.as_bytes())?;
            write_whole(&file.with_extension("info"), info.text().as_bytes())?;
            remove_files(&preset.file)?;
        }
        let at = self
            .presets
            .iter()
            .position(|p| p.kind == kind && p.name == old)
            .expect("found above");
        self.presets[at] = UserPreset {
            name: new.to_owned(),
            file,
            info,
            ..preset
        };
        let children: Vec<String> = self
            .children(kind, old)
            .iter()
            .map(|c| (*c).to_owned())
            .collect();
        for child in children {
            let mut preset = self.get(kind, &child).cloned().expect("listed");
            preset.inherits = Some(new.to_owned());
            let to_save = ToSave {
                own: preset.own.clone(),
                inherits: preset.inherits.clone(),
                version: preset.version.clone().unwrap_or_default(),
                base_id: None,
            };
            write_whole(
                &preset.file,
                Self::file_text(kind, &child, &to_save).as_bytes(),
            )?;
            let at = self
                .presets
                .iter()
                .position(|p| p.kind == kind && p.name == child)
                .expect("listed");
            self.presets[at] = preset;
        }
        self.presets
            .sort_by(|a, b| (folder(a.kind), &a.name).cmp(&(folder(b.kind), &b.name)));
        Ok(())
    }

    /// Takes a preset away, its file and its `.info` (`Preset::remove_files`).
    /// The session refuses one that others are laid over before it gets
    /// here.
    pub fn delete(&mut self, kind: Kind, name: &str) -> Result<(), ConfigError> {
        let preset = self
            .get(kind, name)
            .cloned()
            .ok_or_else(|| ConfigError::UnknownPreset {
                vendor: "user".to_owned(),
                kind,
                name: name.to_owned(),
            })?;
        remove_files(&preset.file)?;
        self.presets.retain(|p| !(p.kind == kind && p.name == name));
        Ok(())
    }
}

/// A preset's file and its `.info`.
fn remove_files(file: &Path) -> Result<(), ConfigError> {
    for path in [file.to_owned(), file.with_extension("info")] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(ConfigError::Io { path, source }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
