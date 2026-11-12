//! Where each printer's connection is kept between runs.
//!
//! The reference keeps a printer's "Print Host upload" options in the
//! user's printer preset (a physical printer), and several printers of one
//! model in `<datadir>/hub/print_host_devices.json`; never in a project (a
//! saved 3MF has them taken out, PresetBundle.cpp `full_config_secure`).
//! This app has no user presets yet, so it keeps one connection for each
//! printer preset, by the preset's name, in its own data folder (the
//! folder `settings.json` is in, `store.rs`):
//!
//! - `print_hosts.json`: `{ "version": 1, "printers": [{ "printer",
//!   "host_type", "print_host", "print_host_webui", "printhost_cafile" }] }`,
//!   under the reference's own keys, so a later pass that adds user presets
//!   can take each entry over as a physical printer preset of that printer.
//! - `print_host_secrets.json`: `{ "<printer>": { "printhost_apikey" } }`,
//!   apart from the rest and readable by this user alone (mode 0600 where
//!   files have modes; on Windows the folder is the user's own
//!   `%APPDATA%`). The page is only told whether a key is set.
//!
//! Why a file and not the system's keychain: a build of this app is not
//! signed, so macOS's keychain would ask the person each time a new build
//! reads its own entry; the keychain is one per user, where the settings
//! follow `EDGE_DESKTOP_DATA` (the tests, a second copy of the app); and
//! the reference keeps the same key in plain text in the preset and in
//! `print_host_devices.json` (PrintHostDevices.cpp l.173). A keychain can
//! take this file's place behind `secret` and `set` when the app is
//! signed.
//!
//! Both are written whole or not at all; a file that cannot be read is
//! said and left as it is until something is saved, then set aside.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

const FILE: &str = "print_hosts.json";
const SECRETS: &str = "print_host_secrets.json";

/// One printer's connection, but its key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub host_type: String,
    pub print_host: String,
    pub print_host_webui: String,
    pub printhost_cafile: String,
}

/// What becomes of the key when a connection is saved.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Keep,
    Set(String),
    Clear,
}

#[derive(Debug, Default)]
pub struct Kept {
    dir: Option<PathBuf>,
    connections: BTreeMap<String, Saved>,
    keys: BTreeMap<String, String>,
    /// Why a file could not be read: said, and set aside when next saved.
    pub problem: Option<String>,
    unreadable: Vec<PathBuf>,
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

impl Kept {
    /// What is kept in `dir`; with none, for the run only.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let mut kept = Kept {
            dir: dir.clone(),
            ..Kept::default()
        };
        let Some(dir) = dir else { return kept };
        if let Some(root) = kept.read(&dir.join(FILE)) {
            for entry in root
                .get("printers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let printer = text(entry, "printer");
                if printer.is_empty() {
                    continue;
                }
                kept.connections.insert(
                    printer,
                    Saved {
                        host_type: text(entry, "host_type"),
                        print_host: text(entry, "print_host"),
                        print_host_webui: text(entry, "print_host_webui"),
                        printhost_cafile: text(entry, "printhost_cafile"),
                    },
                );
            }
        }
        if let Some(root) = kept.read(&dir.join(SECRETS)) {
            for (printer, entry) in root {
                let key = text(&entry, "printhost_apikey");
                if !key.is_empty() {
                    kept.keys.insert(printer, key);
                }
            }
        }
        kept
    }

    fn read(&mut self, file: &Path) -> Option<Map<String, Value>> {
        // The file's name only: its folder is the user's, and the reason
        // never holds what is in it.
        let named = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read_to_string(file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.problem = Some(format!("{named}: {}", error.kind()));
                self.unreadable.push(file.to_owned());
                None
            }
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(root)) => Some(root),
                _ => {
                    self.problem = Some(format!("{named} could not be read"));
                    self.unreadable.push(file.to_owned());
                    None
                }
            },
        }
    }

    pub fn get(&self, printer: &str) -> Option<&Saved> {
        self.connections.get(printer)
    }

    /// The key kept for a printer. Only the hosts are handed it.
    pub fn key(&self, printer: &str) -> Option<&str> {
        self.keys.get(printer).map(String::as_str)
    }

    /// Saves a printer's connection; none forgets it, key included.
    pub fn set(&mut self, printer: &str, saved: Option<Saved>, key: Key) -> Result<(), String> {
        let mut connections = self.connections.clone();
        let mut keys = self.keys.clone();
        match saved {
            Some(saved) => {
                connections.insert(printer.to_owned(), saved);
                match key {
                    Key::Keep => {}
                    Key::Set(key) if !key.is_empty() => {
                        keys.insert(printer.to_owned(), key);
                    }
                    Key::Set(_) | Key::Clear => {
                        keys.remove(printer);
                    }
                }
            }
            None => {
                connections.remove(printer);
                keys.remove(printer);
            }
        }
        self.write(&connections, &keys)?;
        self.connections = connections;
        self.keys = keys;
        Ok(())
    }

    fn write(
        &mut self,
        connections: &BTreeMap<String, Saved>,
        keys: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let failed = |what: &str, error: std::io::Error| {
            format!(
                "The printer connection could not be saved ({what}: {})",
                error.kind()
            )
        };
        std::fs::create_dir_all(dir).map_err(|error| failed("folder", error))?;
        for file in std::mem::take(&mut self.unreadable) {
            if file.exists() {
                let aside = file.with_extension("unreadable.json");
                std::fs::rename(&file, &aside).map_err(|error| failed(FILE, error))?;
            }
        }
        self.problem = None;
        let printers: Vec<Value> = connections
            .iter()
            .map(|(printer, saved)| {
                json!({
                    "printer": printer,
                    "host_type": saved.host_type,
                    "print_host": saved.print_host,
                    "print_host_webui": saved.print_host_webui,
                    "printhost_cafile": saved.printhost_cafile,
                })
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({ "version": 1, "printers": printers }))
            .expect("plain JSON");
        write_whole(&dir.join(FILE), text.as_bytes(), false)
            .map_err(|error| failed(FILE, error))?;
        let secrets: Map<String, Value> = keys
            .iter()
            .map(|(printer, key)| (printer.clone(), json!({ "printhost_apikey": key })))
            .collect();
        let text = serde_json::to_string_pretty(&Value::Object(secrets)).expect("plain JSON");
        write_whole(&dir.join(SECRETS), text.as_bytes(), true)
            .map_err(|error| failed(SECRETS, error))
    }
}

/// Written beside the file, then put in its place; `private` makes it
/// readable by this user alone from the moment it is made.
fn write_whole(file: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write;
    let beside = file.with_extension("json.new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut out = options.open(&beside)?;
    #[cfg(unix)]
    if private {
        // A file left from before keeps its mode: set it.
        use std::os::unix::fs::PermissionsExt;
        out.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    out.write_all(bytes)?;
    out.write_all(b"\n")?;
    out.sync_all()?;
    drop(out);
    std::fs::rename(&beside, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(host: &str) -> Saved {
        Saved {
            host_type: "octoprint".into(),
            print_host: host.into(),
            ..Saved::default()
        }
    }

    #[test]
    fn a_connection_and_its_key_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let mut kept = Kept::open(Some(dir.path().to_owned()));
        kept.set(
            "Snapmaker U1 (0.4 nozzle)",
            Some(saved("10.0.0.9")),
            Key::Set("sekrit-key".into()),
        )
        .unwrap();
        let again = Kept::open(Some(dir.path().to_owned()));
        assert_eq!(
            again.get("Snapmaker U1 (0.4 nozzle)"),
            Some(&saved("10.0.0.9"))
        );
        assert_eq!(again.key("Snapmaker U1 (0.4 nozzle)"), Some("sekrit-key"));
        let plain = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert!(!plain.contains("sekrit-key"), "{plain}");
        assert!(plain.contains("\"print_host\": \"10.0.0.9\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(SECRETS))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Kept as it is, changed, cleared, forgotten.
        let mut kept = again;
        kept.set(
            "Snapmaker U1 (0.4 nozzle)",
            Some(saved("10.0.0.10")),
            Key::Keep,
        )
        .unwrap();
        assert_eq!(kept.key("Snapmaker U1 (0.4 nozzle)"), Some("sekrit-key"));
        kept.set(
            "Snapmaker U1 (0.4 nozzle)",
            Some(saved("10.0.0.10")),
            Key::Clear,
        )
        .unwrap();
        assert_eq!(
            Kept::open(Some(dir.path().to_owned())).key("Snapmaker U1 (0.4 nozzle)"),
            None
        );
        kept.set("Snapmaker U1 (0.4 nozzle)", None, Key::Keep)
            .unwrap();
        assert_eq!(
            Kept::open(Some(dir.path().to_owned())).get("Snapmaker U1 (0.4 nozzle)"),
            None
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn an_unreadable_file_is_said_and_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), "{ nope").unwrap();
        let mut kept = Kept::open(Some(dir.path().to_owned()));
        assert!(
            kept.problem
                .as_deref()
                .is_some_and(|said| said.contains(FILE))
        );
        kept.set("P", Some(saved("h")), Key::Keep).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("print_hosts.unreadable.json")).unwrap(),
            "{ nope"
        );
        assert!(kept.problem.is_none());
    }
}
