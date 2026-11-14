//! The Bambu Lab printers added for LAN printing, kept between runs, and
//! the printer models the reference knows (`resources/printers/*.json`).
//!
//! The reference keeps a LAN printer in `app_config` (`DeviceManager::
//! update_local_machine`: its serial, name, address, model) and its access
//! code in `app_config`'s `access_code` / `user_access_code` sections, by
//! serial (DeviceManager.cpp l.509-541), in the same plain file. Here the
//! printers are in `bambu_printers.json` of this app's data folder and the
//! codes apart in `bambu_printers_secrets.json`, readable by this user
//! alone (as `hosts::kept` keeps a print host's key). The code never leaves
//! the Rust side: the page is told whether there is one.
//!
//! The print options of "Send print job to" are remembered here too, as the
//! reference keeps them in `app_config`'s `print` section ("bed_leveling",
//! "flow_cali", "timelapse": SelectMachine.cpp l.902-929).

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

use crate::hosts::kept::write_whole;

const FILE: &str = "bambu_printers.json";
const SECRETS: &str = "bambu_printers_secrets.json";

/// A printer added by its address (`BBLocalMachine`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Printer {
    /// The serial, upper case (`dev_id`).
    pub dev_id: String,
    pub name: String,
    pub ip: String,
    /// The model's id ("C12"): `printer_type`.
    pub model: String,
}

/// The remembered print options; `None` is the reference's default (on).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    pub bed_leveling: Option<bool>,
    pub flow_cali: Option<bool>,
    pub timelapse: Option<bool>,
}

#[derive(Debug, Default)]
pub struct Kept {
    dir: Option<PathBuf>,
    pub printers: Vec<Printer>,
    codes: BTreeMap<String, String>,
    pub selected: Option<String>,
    pub options: Options,
    /// Why a file could not be read.
    pub problem: Option<String>,
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
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
            for entry in root["printers"].as_array().into_iter().flatten() {
                let printer = Printer {
                    dev_id: text(entry, "dev_id"),
                    name: text(entry, "dev_name"),
                    ip: text(entry, "dev_ip"),
                    model: text(entry, "printer_type"),
                };
                if !printer.dev_id.is_empty() {
                    kept.printers.push(printer);
                }
            }
            kept.selected = root["selected"].as_str().map(str::to_owned);
            let option = |key: &str| root["print"][key].as_bool();
            kept.options = Options {
                bed_leveling: option("bed_leveling"),
                flow_cali: option("flow_cali"),
                timelapse: option("timelapse"),
            };
        }
        if let Some(root) = kept.read(&dir.join(SECRETS))
            && let Some(map) = root.as_object()
        {
            for (dev_id, entry) in map {
                let code = text(entry, "access_code");
                if !code.is_empty() {
                    kept.codes.insert(dev_id.clone(), code);
                }
            }
        }
        kept
    }

    fn read(&mut self, file: &Path) -> Option<Value> {
        let named = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read_to_string(file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.problem = Some(format!("{named}: {}", error.kind()));
                None
            }
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(root @ Value::Object(_)) => Some(root),
                _ => {
                    self.problem = Some(format!("{named} could not be read"));
                    None
                }
            },
        }
    }

    pub fn get(&self, dev_id: &str) -> Option<&Printer> {
        self.printers
            .iter()
            .find(|printer| printer.dev_id == dev_id)
    }

    /// The access code of a printer. Handed to the bridge only.
    pub fn code(&self, dev_id: &str) -> Option<&str> {
        self.codes.get(dev_id).map(String::as_str)
    }

    /// Every code kept: what is taken out of anything the bridge says.
    pub fn codes(&self) -> Vec<String> {
        self.codes.values().cloned().collect()
    }

    /// Adds a printer, or changes the one of that serial; `code` replaces
    /// its access code when given.
    pub fn put(&mut self, printer: Printer, code: Option<String>) -> Result<(), String> {
        let mut printers = self.printers.clone();
        let mut codes = self.codes.clone();
        match printers.iter_mut().find(|p| p.dev_id == printer.dev_id) {
            Some(was) => *was = printer.clone(),
            None => printers.push(printer.clone()),
        }
        if let Some(code) = code {
            codes.insert(printer.dev_id.clone(), code);
        }
        self.write(&printers, &codes, self.selected.as_deref(), &self.options)?;
        self.printers = printers;
        self.codes = codes;
        Ok(())
    }

    /// Forgets a printer, its code with it.
    pub fn remove(&mut self, dev_id: &str) -> Result<(), String> {
        let printers: Vec<Printer> = self
            .printers
            .iter()
            .filter(|p| p.dev_id != dev_id)
            .cloned()
            .collect();
        let mut codes = self.codes.clone();
        codes.remove(dev_id);
        let selected = self.selected.clone().filter(|s| s != dev_id);
        self.write(&printers, &codes, selected.as_deref(), &self.options)?;
        self.printers = printers;
        self.codes = codes;
        self.selected = selected;
        Ok(())
    }

    /// Forgets a printer's code (the printer refused it: `erase_user_access_code`).
    pub fn forget_code(&mut self, dev_id: &str) -> Result<(), String> {
        let mut codes = self.codes.clone();
        codes.remove(dev_id);
        self.write(
            &self.printers,
            &codes,
            self.selected.as_deref(),
            &self.options,
        )?;
        self.codes = codes;
        Ok(())
    }

    pub fn select(&mut self, dev_id: &str) -> Result<(), String> {
        if self.selected.as_deref() == Some(dev_id) {
            return Ok(());
        }
        self.write(&self.printers, &self.codes, Some(dev_id), &self.options)?;
        self.selected = Some(dev_id.to_owned());
        Ok(())
    }

    pub fn set_options(&mut self, options: Options) -> Result<(), String> {
        self.write(
            &self.printers,
            &self.codes,
            self.selected.as_deref(),
            &options,
        )?;
        self.options = options;
        Ok(())
    }

    fn write(
        &self,
        printers: &[Printer],
        codes: &BTreeMap<String, String>,
        selected: Option<&str>,
        options: &Options,
    ) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let failed = |what: &str, error: std::io::Error| {
            format!("The printer could not be saved ({what}: {})", error.kind())
        };
        std::fs::create_dir_all(dir).map_err(|error| failed("folder", error))?;
        let list: Vec<Value> = printers
            .iter()
            .map(|p| {
                json!({ "dev_id": p.dev_id, "dev_name": p.name, "dev_ip": p.ip, "printer_type": p.model })
            })
            .collect();
        let mut print = Map::new();
        for (key, value) in [
            ("bed_leveling", options.bed_leveling),
            ("flow_cali", options.flow_cali),
            ("timelapse", options.timelapse),
        ] {
            if let Some(value) = value {
                print.insert(key.into(), json!(value));
            }
        }
        let root = json!({ "version": 1, "selected": selected, "print": print, "printers": list });
        let text = serde_json::to_string_pretty(&root).expect("plain JSON");
        write_whole(&dir.join(FILE), text.as_bytes(), false)
            .map_err(|error| failed(FILE, error))?;
        let secrets: Map<String, Value> = codes
            .iter()
            .map(|(dev_id, code)| (dev_id.clone(), json!({ "access_code": code })))
            .collect();
        let text = serde_json::to_string_pretty(&Value::Object(secrets)).expect("plain JSON");
        write_whole(&dir.join(SECRETS), text.as_bytes(), true)
            .map_err(|error| failed(SECRETS, error))
    }
}

/// A printer model of the reference's (`resources/printers/<id>.json`,
/// its "00.00.00.00" section).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub ftp_folder: String,
    /// `compatible_machine`: models a file sliced for this one may print on.
    pub compatible: Vec<String>,
    /// "core_xy" or "i3".
    pub arch: String,
}

/// The models in `dir` (`DeviceManager::get_all_model_id_with_name`), by
/// name.
pub fn models(dir: &Path) -> Vec<Model> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(root) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let model = &root["00.00.00.00"];
        let (id, name) = (text_of(model, "model_id"), text_of(model, "display_name"));
        if id.is_empty() || name.is_empty() {
            continue;
        }
        out.push(Model {
            id,
            name,
            ftp_folder: text_of(model, "ftp_folder"),
            compatible: model["compatible_machine"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            arch: text_of(model, "printer_arch"),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn text_of(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

/// `device_matches_profile_model` (DeviceModelCode.cpp l.184): a file
/// sliced for `profile` may go to a printer of `device`.
pub fn matches(profile: &str, device: &str, models: &[Model]) -> bool {
    profile == device
        || models
            .iter()
            .find(|model| model.id == device)
            .is_some_and(|model| model.compatible.iter().any(|id| id == profile))
}

/// `InputIpAddressDialog::isIp`: four numbers 0..255 with dots.
pub fn is_ip(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 3
                && part.bytes().all(|b| b.is_ascii_digit())
                && part.parse::<u16>().is_ok_and(|n| n <= 255)
        })
}

/// The reference's rule for an access code (`on_text`): eight letters or
/// digits.
pub fn is_access_code(text: &str) -> bool {
    text.len() == 8 && text.bytes().all(|b| b.is_ascii_alphanumeric())
}
