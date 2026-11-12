//! Presets brought in from files and sent out in archives, in the
//! reference's formats, so what one application exports the other imports.
//!
//! What is read (PresetBundle.cpp `import_presets`, `import_json_presets`;
//! MainFrame.cpp `load_config_file`, "Import Configs"): a preset's `.json`,
//! or a zip of them under any of the names the reference writes (`.zip`,
//! `.orca_printer`, `.orca_filament`). Of an archive only each entry's own
//! name counts, never its folders, and `bundle_structure.json` is passed
//! over; every other entry is read as a preset. A preset's kind is the id
//! it carries (`printer_settings_id`, else `print_settings_id`, else
//! `filament_settings_id`), its name is its `name`, and one without a
//! version is passed over.
//!
//! What is written (CreatePresetsDialog.cpp `ExportConfigsDialog`):
//! - "Printer config bundle(.orca_printer)"
//!   (`archive_preset_bundle_to_file`): `<printer>.orca_printer`, holding
//!   `printer/<printer>.json`, `filament/<file>.json` and
//!   `process/<file>.json` for the person's presets that go with the
//!   printer, and `bundle_structure.json` listing them.
//! - "Filament bundle(.orca_filament)" (`archive_filament_bundle_to_file`):
//!   `<filament>.orca_filament`, holding `<vendor>/<preset>.json` for each
//!   of the person's presets of one filament, and `bundle_structure.json`.
//! - "Printer presets(.zip)", "Filament presets(.zip)", "Process
//!   presets(.zip)" (`archive_printer_preset_to_file`,
//!   `archive_filament_preset_to_file`, `archive_process_preset_to_file`,
//!   `save_presets_to_zip`; ProcessPresetExport.cpp
//!   `preset_export_entries`): `Printer presets.zip` and so on, holding
//!   `<preset>.json` for each preset chosen, side by side.
//!
//! A process or filament preset goes in as its file is on the disk: what
//! it sets over the preset it inherits from. A printer preset goes in
//! whole, everything it inherits written out, without the print host's
//! address and log-in (`earse_preset_fields_for_safe`, which saves a copy
//! with `Preset::save(nullptr)`).
//!
//! What differs from the reference, on purpose:
//! - The time a bundle's id carries, and every entry's time, is the one
//!   the caller gives ([`clock`] turns it into the reference's
//!   "%Y%m%d%H%M%S"), in UTC; the reference takes the local time, and each
//!   file's own time on the disk. Given the same time the same archive
//!   comes out byte for byte.
//! - `bundle_structure.json` lists the vendors of a filament bundle in
//!   order of name; the reference lists them as an unordered map holds
//!   them.
//! - The name a filament bundle goes by is a preset's name up to its "@",
//!   with the spaces before it taken off. The reference cuts one character
//!   before the "@" whatever it is (`get_filament_name`), so "PLA@U1"
//!   becomes "PL".
//! - Entries a Mac adds to an archive it zips (`__MACOSX/`, `._<name>`)
//!   are passed over: they are not presets.
//! - An entry larger than [`MAX_ENTRY_BYTES`] once unpacked is not read.

use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};

use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::{
    ConfigError, Kind, Settings, Value,
    user::{self, UserPreset},
};

/// The entry of a bundle that says what is in it
/// (PresetBundle.hpp `BUNDLE_STRUCTURE_JSON_NAME`).
pub const BUNDLE_STRUCTURE: &str = "bundle_structure.json";

/// What a printer preset is exported without: the print host's address
/// and log-in (`earse_preset_fields_for_safe`).
pub const PRINT_HOST_KEYS: [&str; 7] = [
    "print_host",
    "print_host_webui",
    "printhost_apikey",
    "printhost_cafile",
    "printhost_user",
    "printhost_password",
    "printhost_port",
];

/// The largest entry read from an archive, once unpacked. A preset is a
/// few kilobytes; a few bytes of zip can otherwise unpack without bound.
pub const MAX_ENTRY_BYTES: u64 = 16 << 20;

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

/// A preset read from a file, to be imported.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub kind: Kind,
    pub name: String,
    /// What the file sets, without its name, `inherits`, the kind's id
    /// and what says what the file is.
    pub settings: Settings,
    /// The preset it is laid over; none for one that stands alone.
    pub inherits: Option<String>,
    pub version: String,
    /// The file it came from: the path, or the archive's entry.
    pub file: String,
}

/// A file that was not taken, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passed {
    pub file: String,
    pub reason: String,
}

/// What a file or an archive holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub presets: Vec<Incoming>,
    /// Files passed over as the reference passes over them: no version,
    /// no kind, a name that cannot be a file's, an entry with an unsafe
    /// name, a second preset of a name.
    pub passed: Vec<Passed>,
    /// Files that are not preset JSON at all.
    pub broken: Vec<Passed>,
}

/// Whether a file's version reads as one (`Semver::parse`): numbers
/// joined by dots, with whatever follows a "-" or "+".
fn is_version(text: &str) -> bool {
    let core = text.split(['-', '+']).next().unwrap_or_default();
    !core.is_empty()
        && core
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// The kind a preset's file is, by the id it carries, in the reference's
/// order (`import_json_presets`).
fn kind_of(settings: &Settings) -> Option<Kind> {
    [
        ("printer_settings_id", Kind::Machine),
        ("print_settings_id", Kind::Process),
        ("filament_settings_id", Kind::Filament),
    ]
    .into_iter()
    .find(|(key, _)| settings.get(key).is_some())
    .map(|(_, kind)| kind)
}

impl Found {
    /// Takes one file's bytes: a preset, or the reason it is not one.
    fn take(&mut self, bytes: &[u8], file: &str) {
        let passed = |reason: String| Passed {
            file: file.to_owned(),
            reason,
        };
        let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
        let mut settings = match Settings::from_json(&text) {
            Ok(settings) => settings,
            Err(reason) => {
                self.broken.push(passed(reason));
                return;
            }
        };
        let name = settings.text("name").unwrap_or_default().to_owned();
        let version = settings.text("version").unwrap_or_default().to_owned();
        if !is_version(&version) {
            self.passed.push(passed("it has no version".to_owned()));
            return;
        }
        let Some(kind) = kind_of(&settings) else {
            self.passed.push(passed(
                "it is not a printer, filament or process preset".to_owned(),
            ));
            return;
        };
        // The name becomes the file's: it may not reach out of the
        // folder (`is_relative_name_within_root`), nor be one a preset
        // may not have.
        if user::check_name(&name).is_err() || name == "." || name == ".." {
            self.passed
                .push(passed(format!("\"{name}\" cannot be a preset's name")));
            return;
        }
        if self
            .presets
            .iter()
            .any(|other| other.kind == kind && other.name == name)
        {
            self.passed.push(passed(format!(
                "another file holds a {} preset named \"{name}\"",
                user::folder(kind)
            )));
            return;
        }
        let inherits = settings
            .text("inherits")
            .filter(|parent| !parent.is_empty())
            .map(str::to_owned);
        for key in HEADER_KEYS {
            settings.remove(key);
        }
        settings.remove(user::id_key(kind));
        self.presets.push(Incoming {
            kind,
            name,
            settings,
            inherits,
            version,
            file: file.to_owned(),
        });
    }
}

/// How an archive's entry name stands (UntrustedInput.cpp
/// `normalize_archive_entry_path`, `archive_entry_leaf`).
enum EntryName {
    /// Its own name, the folders dropped.
    Leaf(String),
    Skip,
    Unsafe,
}

fn entry_name(raw: &str) -> EntryName {
    if raw.is_empty() || raw.len() > 4096 {
        return EntryName::Unsafe;
    }
    let path = raw.replace('\\', "/");
    if path.starts_with('/') {
        return EntryName::Unsafe;
    }
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            ".." => return EntryName::Unsafe,
            "" | "." => {}
            other => segments.push(other),
        }
    }
    let Some(leaf) = segments.last() else {
        return EntryName::Skip;
    };
    let unsafe_segment = |segment: &&str| {
        segment.chars().all(|c| c == '.' || c == ' ')
            || segment.chars().any(|c| c.is_control() || c == ':')
    };
    if segments.iter().any(unsafe_segment) {
        return EntryName::Unsafe;
    }
    if segments[0] == "__MACOSX" || leaf.starts_with("._") {
        return EntryName::Skip;
    }
    EntryName::Leaf((*leaf).to_owned())
}

/// The presets an archive holds.
pub fn read_archive(bytes: &[u8]) -> Result<Found, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut read = Found::default();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let raw = String::from_utf8_lossy(entry.name_raw()).into_owned();
        if entry.is_dir() {
            continue;
        }
        let leaf = match entry_name(&raw) {
            EntryName::Leaf(leaf) => leaf,
            EntryName::Skip => continue,
            EntryName::Unsafe => {
                read.passed.push(Passed {
                    file: raw,
                    reason: "its name is not safe to unpack".to_owned(),
                });
                continue;
            }
        };
        if leaf == BUNDLE_STRUCTURE {
            continue;
        }
        let mut bytes = Vec::new();
        (&mut entry)
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("{raw}: {e}"))?;
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            read.broken.push(Passed {
                file: raw,
                reason: format!("larger than {} MiB", MAX_ENTRY_BYTES >> 20),
            });
            continue;
        }
        read.take(&bytes, &raw);
    }
    Ok(read)
}

/// The preset a `.json` file holds.
pub fn read_json(bytes: &[u8], file: &str) -> Found {
    let mut read = Found::default();
    read.take(bytes, file);
    read
}

/// Whether the reference reads a file of this name as an archive of
/// presets, or as a preset; none for neither (`import_presets`).
pub fn is_archive(path: &Path) -> Option<bool> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    if name.ends_with(".json") {
        Some(false)
    } else if [".zip", ".orca_printer", ".orca_filament"]
        .iter()
        .any(|end| name.ends_with(end))
    {
        Some(true)
    } else {
        None
    }
}

/// The presets a file holds, by its name: a preset's `.json`, or an
/// archive of them.
pub fn read_path(path: &Path) -> Result<Found, ConfigError> {
    let not = |reason: String| ConfigError::Json {
        path: path.to_owned(),
        reason,
    };
    let archive = is_archive(path).ok_or_else(|| {
        not("it is not a .json, .zip, .orca_printer or .orca_filament file".to_owned())
    })?;
    let bytes = fs::read(path).map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    if archive {
        read_archive(&bytes).map_err(not)
    } else {
        Ok(read_json(&bytes, &path.to_string_lossy()))
    }
}

/// A time in seconds since 1970 as the reference stamps a bundle's id
/// (`get_curr_timestmp`, "%Y%m%d%H%M%S"), in UTC.
pub fn clock(seconds: i64) -> String {
    let (year, month, day, hour, minute, second) = civil(seconds);
    format!("{year:04}{month:02}{day:02}{hour:02}{minute:02}{second:02}")
}

/// The date and time of seconds since 1970, in UTC (Howard Hinnant's
/// `civil_from_days`).
fn civil(seconds: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

/// One file of an archive: its name there and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// The archive of these entries, in this order, each stamped with the
/// time given and deflated as the reference's `MZ_DEFAULT_COMPRESSION`
/// deflates.
pub fn zip(entries: &[Entry], seconds: i64) -> Result<Vec<u8>, String> {
    let (year, month, day, hour, minute, second) = civil(seconds);
    let narrow =
        |value: i64, low: i64, high: i64| u8::try_from(value.clamp(low, high)).unwrap_or(0);
    let time = DateTime::from_date_and_time(
        u16::try_from(year.clamp(1980, 2107)).unwrap_or(1980),
        narrow(month, 1, 12),
        narrow(day, 1, 31),
        narrow(hour, 0, 23),
        narrow(minute, 0, 59),
        narrow(second, 0, 59),
    )
    .unwrap_or_default();
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(time)
        .unix_permissions(0o644);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for entry in entries {
        writer
            .start_file(entry.name.as_str(), options)
            .map_err(|e| e.to_string())?;
        writer.write_all(&entry.bytes).map_err(|e| e.to_string())?;
    }
    Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
}

/// The bytes a preset is exported as: its file as it is on the disk.
pub fn file_of(preset: &UserPreset) -> Result<Vec<u8>, ConfigError> {
    fs::read(&preset.file).map_err(|source| ConfigError::Io {
        path: preset.file.clone(),
        source,
    })
}

/// A printer preset written out whole, for export
/// (`earse_preset_fields_for_safe`, `Preset::save(nullptr)`,
/// `ConfigBase::save_to_json`): every setting it has with what it
/// inherits, its `inherits` as it is, without the print host's address
/// and log-in. `from` is "User" for one of the person's, "System" for a
/// vendor's.
pub fn whole_printer(
    name: &str,
    resolved: &Settings,
    inherits: Option<&str>,
    from: &str,
    version: &str,
) -> String {
    let mut values: BTreeMap<String, Value> = resolved
        .iter()
        .filter(|(key, _)| crate::reference_knows(key) && !PRINT_HOST_KEYS.contains(key))
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect();
    values.insert("name".into(), Value::One(name.to_owned()));
    values.insert("from".into(), Value::One(from.to_owned()));
    values.insert("is_custom_defined".into(), Value::One("0".into()));
    values.insert("version".into(), Value::One(version.to_owned()));
    values.insert(
        "inherits".into(),
        Value::One(inherits.unwrap_or_default().to_owned()),
    );
    values.insert(
        user::id_key(Kind::Machine).into(),
        Value::One(name.to_owned()),
    );
    user::to_json(&values)
}

/// A bundle's `bundle_structure.json` as the reference's JSON library
/// writes it (`json::dump()`): on one line, the keys in order of name.
fn structure(values: BTreeMap<&str, serde_json::Value>) -> Vec<u8> {
    let object: serde_json::Map<String, serde_json::Value> = values
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
    serde_json::Value::Object(object).to_string().into_bytes()
}

/// A printer's presets for "Printer config bundle(.orca_printer)"
/// (`archive_preset_bundle_to_file`): its own file, and the files of the
/// filament and process presets that go with it.
#[derive(Clone, Debug, Default)]
pub struct PrinterBundle {
    /// The printer preset's name, which the bundle is named by.
    pub printer: String,
    /// The printer preset written out whole ([`whole_printer`]).
    pub printer_file: Vec<u8>,
    /// Each filament and process preset's file name (`<name>.json`) and
    /// its file as on the disk.
    pub filaments: Vec<(String, Vec<u8>)>,
    pub processes: Vec<(String, Vec<u8>)>,
}

/// The archive of a printer bundle, with its id stamped with `seconds`.
pub fn printer_bundle(bundle: &PrinterBundle, seconds: i64) -> Result<Vec<u8>, String> {
    let printer_entry = format!("printer/{}.json", bundle.printer);
    let mut entries = vec![Entry {
        name: printer_entry.clone(),
        bytes: bundle.printer_file.clone(),
    }];
    let mut listed = |folder: &str, files: &[(String, Vec<u8>)]| -> Vec<String> {
        files
            .iter()
            .map(|(file, bytes)| {
                let name = format!("{folder}/{file}");
                entries.push(Entry {
                    name: name.clone(),
                    bytes: bytes.clone(),
                });
                name
            })
            .collect()
    };
    let filaments = listed("filament", &bundle.filaments);
    let processes = listed("process", &bundle.processes);
    // Without an account, as here (`agent` none): "offline_", no version.
    let mut values = BTreeMap::new();
    values.insert("version", "".into());
    values.insert(
        "bundle_id",
        format!("offline_{}_{}", bundle.printer, clock(seconds)).into(),
    );
    values.insert("bundle_type", "printer config bundle".into());
    values.insert("printer_preset_name", bundle.printer.clone().into());
    values.insert("printer_config", vec![printer_entry].into());
    values.insert("filament_config", filaments.into());
    values.insert("process_config", processes.into());
    entries.push(Entry {
        name: BUNDLE_STRUCTURE.to_owned(),
        bytes: structure(values),
    });
    zip(&entries, seconds)
}

/// The archive of a filament bundle (`archive_filament_bundle_to_file`):
/// the filament's name, and for each printer vendor the presets of it,
/// by name, with their files as on the disk.
pub fn filament_bundle(
    filament: &str,
    vendors: &BTreeMap<String, Vec<(String, Vec<u8>)>>,
    seconds: i64,
) -> Result<Vec<u8>, String> {
    let mut entries = Vec::new();
    let mut listed = Vec::new();
    for (vendor, presets) in vendors {
        let mut paths = Vec::new();
        for (name, bytes) in presets {
            let path = format!("{vendor}/{name}.json");
            entries.push(Entry {
                name: path.clone(),
                bytes: bytes.clone(),
            });
            paths.push(serde_json::Value::from(path));
        }
        let mut one = serde_json::Map::new();
        one.insert("filament_path".into(), paths.into());
        one.insert("vendor".into(), vendor.clone().into());
        listed.push(serde_json::Value::Object(one));
    }
    let mut values = BTreeMap::new();
    values.insert("version", "".into());
    values.insert(
        "bundle_id",
        format!("offline_{filament}_{}", clock(seconds)).into(),
    );
    values.insert("bundle_type", "filament config bundle".into());
    values.insert("filament_name", filament.into());
    // Absent when nothing went in, as `push_back` was never called.
    if !listed.is_empty() {
        values.insert("printer_vendor", listed.into());
    }
    entries.push(Entry {
        name: BUNDLE_STRUCTURE.to_owned(),
        bytes: structure(values),
    });
    zip(&entries, seconds)
}

/// The printer a preset's name says it is for: what follows its last "@"
/// (`get_machine_name`), none without one.
pub fn machine_name(preset: &str) -> &str {
    preset.rfind('@').map_or("", |at| &preset[at + 1..])
}

/// The filament a preset's name says it is: what comes before its last
/// "@", the spaces before that taken off (`get_filament_name`).
pub fn filament_name(preset: &str) -> &str {
    match preset.rfind('@') {
        Some(at) => preset[..at].trim_end(),
        None => preset,
    }
}

/// The vendor a printer's name begins with: its first word
/// (`get_vendor_name`).
pub fn vendor_name(machine: &str) -> &str {
    machine.split_whitespace().next().unwrap_or_default()
}

#[cfg(test)]
mod tests;
