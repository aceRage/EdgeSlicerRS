//! The Bambu Lab printers added for LAN printing, kept between runs, and
//! the printer models the reference knows (`resources/printers/*.json`).
//!
//! The reference keeps a LAN printer in `app_config` (`DeviceManager::
//! update_local_machine`: its serial, name, address, model) and its access
//! code in `app_config`'s `access_code` / `user_access_code` sections, by
//! serial (DeviceManager.cpp l.509-541), in the same plain file. Here the
//! printers are in `bambu_printers.json` of this app's data folder, with
//! whether each has a code but not the code, and the codes in the
//! system's store (`crate::secrets`: the macOS Keychain, Windows Credential
//! Manager, the Secret Service), else in `bambu_printers_secrets.json`,
//! readable by this user alone. The store is read only when a code is
//! needed (connect, send) or changed, never when the app starts or a page
//! is looked at, and what it gave is kept for the run. The code never
//! leaves the Rust side: the page is told whether there is one.
//!
//! The print options of "Send print job to" are remembered here too, as the
//! reference keeps them in `app_config`'s `print` section ("bed_leveling",
//! "flow_cali", "timelapse": SelectMachine.cpp l.902-929).
//!
//! And the Bambu Lab account (ULTRANET_BRIDGE.md section 16): who is
//! signed in and the account's printers, without their codes, in
//! `bambu_printers.json` (so Device shows them without reading the store);
//! the tokens and the account's codes in the store, in the field of their
//! own (`secrets::ACCOUNT_ID`), read only when the account is used. A code
//! typed for a printer wins over the one the account gives
//! (`MachineObject::get_access_code`, DeviceManager.cpp l.502-507).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

use super::account::{AccountPrinter, AccountSecret, User};
use crate::hosts::kept::write_whole;
use crate::secrets::{
    self, ACCOUNT_ID, FILE as SECRETS, FileStore, Kind, Memory, Refused, Secrets,
};

const FILE: &str = "bambu_printers.json";

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

/// The account as Device shows it: who, and the account's printers
/// (`userMachineList`), never a token or a code.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Account {
    pub user: User,
    pub printers: Vec<AccountPrinter>,
}

pub struct Kept {
    dir: Option<PathBuf>,
    /// The account signed in, as last known (kept between runs).
    pub account: Option<Account>,
    /// Its secrets, once read this run (`Some(None)`: read, none there).
    account_secret: Option<Option<AccountSecret>>,
    pub printers: Vec<Printer>,
    /// The printers whose code is kept (in the store, not here).
    coded: BTreeSet<String>,
    /// The codes read or set this run: handed to the bridge, and taken out
    /// of anything it says.
    known: BTreeMap<String, String>,
    secrets: Box<dyn Secrets>,
    /// The system's store has been asked something this run.
    asked: bool,
    /// The codes of the old file have been looked at this run.
    moved: bool,
    pub selected: Option<String>,
    pub options: Options,
    /// Why a file could not be read.
    pub problem: Option<String>,
    /// Where the codes are kept when not where they should be, or what
    /// went wrong moving them: said in the Device panel.
    pub notice: Option<String>,
}

impl std::fmt::Debug for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the codes.
        f.debug_struct("Kept")
            .field("printers", &self.printers)
            .field("coded", &self.coded)
            .field("store", &self.secrets.kind())
            .finish_non_exhaustive()
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

impl Kept {
    /// What is kept in `dir`; with none, for the run only. The codes are
    /// in `secrets`, not read here.
    pub fn open(dir: Option<PathBuf>, secrets: Box<dyn Secrets>) -> Self {
        let mut kept = Kept {
            dir: dir.clone(),
            account: None,
            account_secret: None,
            printers: Vec::new(),
            coded: BTreeSet::new(),
            known: BTreeMap::new(),
            secrets,
            asked: false,
            moved: false,
            selected: None,
            options: Options::default(),
            problem: None,
            notice: None,
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
                if printer.dev_id.is_empty() {
                    continue;
                }
                if entry["access_code_kept"].as_bool() == Some(true) {
                    kept.coded.insert(printer.dev_id.clone());
                }
                kept.printers.push(printer);
            }
            kept.selected = root["selected"].as_str().map(str::to_owned);
            if root["account"].is_object() {
                let account = &root["account"];
                kept.account = Some(Account {
                    user: User::from(&account["user"]),
                    printers: account["printers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|p| AccountPrinter {
                            dev_id: text(p, "dev_id"),
                            name: text(p, "dev_name"),
                            model: text(p, "printer_type"),
                            online: p["online"].as_bool().unwrap_or(false),
                            task_status: text(p, "task_status"),
                            has_code: p["access_code_kept"].as_bool().unwrap_or(false),
                        })
                        .filter(|p| !p.dev_id.is_empty())
                        .collect(),
                });
            }
            let option = |key: &str| root["print"][key].as_bool();
            kept.options = Options {
                bed_leveling: option("bed_leveling"),
                flow_cali: option("flow_cali"),
                timelapse: option("timelapse"),
            };
        }
        // The codes' file: which printers it has a code for (a list kept
        // before the system's store said nothing of them). Its codes are
        // not kept from here.
        match secrets::read_file(&dir.join(SECRETS)) {
            Ok(codes) => kept
                .coded
                .extend(codes.into_keys().filter(|id| !id.starts_with('@'))),
            Err(Refused(why)) => kept.problem = Some(why),
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

    /// Whether a code is kept for the printer: said without reading it.
    /// The account's code counts too.
    pub fn has_code(&self, dev_id: &str) -> bool {
        self.coded.contains(dev_id) || self.account_has_code(dev_id)
    }

    fn account_has_code(&self, dev_id: &str) -> bool {
        self.account_printer(dev_id).is_some_and(|p| p.has_code)
    }

    /// A printer of the account.
    pub fn account_printer(&self, dev_id: &str) -> Option<&AccountPrinter> {
        self.account
            .as_ref()
            .and_then(|a| a.printers.iter().find(|p| p.dev_id == dev_id))
    }

    /// Whether a code can be had without asking the system's store (which
    /// may ask the person): it is not the store, or it was asked this run.
    pub fn codes_at_hand(&self) -> bool {
        !self.secrets.kind().is_system() || self.asked
    }

    /// Which store the codes are in now.
    pub fn store(&self) -> Kind {
        self.secrets.kind()
    }

    /// The access code of a printer, read from the store the first time
    /// one is needed. Handed to the bridge only.
    pub fn code(&mut self, dev_id: &str) -> Result<Option<String>, String> {
        if !self.coded.contains(dev_id) {
            // The one the account gave, where none was typed.
            if !self.account_has_code(dev_id) {
                return Ok(None);
            }
            return Ok(self
                .account_secret()?
                .and_then(|secret| secret.codes.get(dev_id).cloned()));
        }
        if let Some(code) = self.known.get(dev_id) {
            return Ok(Some(code.clone()));
        }
        self.move_old_codes();
        let code = self.with_store(|store| store.get(dev_id))?;
        if let Some(code) = &code {
            self.known.insert(dev_id.to_owned(), code.clone());
        }
        Ok(code)
    }

    /// Every code read or set this run: what is taken out of anything the
    /// bridge says (it has been handed no other).
    pub fn codes(&self) -> Vec<String> {
        let mut out: Vec<String> = self.known.values().cloned().collect();
        if let Some(Some(secret)) = &self.account_secret {
            out.extend(secret.secrets());
        }
        out
    }

    /// The account's secrets (its tokens and codes), read from the store
    /// the first time they are needed this run (which may make the system
    /// ask the person). `None` when none are kept.
    pub fn account_secret(&mut self) -> Result<Option<AccountSecret>, String> {
        if let Some(read) = &self.account_secret {
            return Ok(read.clone());
        }
        self.move_old_codes();
        let text = self.with_store(|store| store.get(ACCOUNT_ID))?;
        let secret = text.as_deref().and_then(AccountSecret::parse);
        self.account_secret = Some(secret.clone());
        Ok(secret)
    }

    /// Signed in, or the session or the list changed: the secrets in the
    /// store (written only when they changed), who and the printers in
    /// the list. `printers` None keeps the list as it was.
    pub fn set_account(
        &mut self,
        secret: AccountSecret,
        printers: Option<Vec<AccountPrinter>>,
    ) -> Result<(), String> {
        if self.account_secret.as_ref() != Some(&Some(secret.clone())) {
            self.move_old_codes();
            let text = secret.text();
            self.with_store(|store| store.set(ACCOUNT_ID, &text))
                .map_err(|_| "The Bambu Lab sign-in could not be kept.".to_owned())?;
            self.account_secret = Some(Some(secret.clone()));
        }
        let was = self.account.clone();
        let printers = printers
            .or_else(|| was.as_ref().map(|a| a.printers.clone()))
            .unwrap_or_default();
        self.account = Some(Account {
            user: secret.user.clone(),
            printers,
        });
        if let Err(error) = self.write(
            &self.printers,
            &self.coded,
            self.selected.as_deref(),
            &self.options,
        ) {
            self.account = was;
            return Err(error);
        }
        Ok(())
    }

    /// Signed out (`request_user_logout`, `clean_user_info`): the account's
    /// secrets out of the store, its printers out of the list. A printer
    /// also added by hand stays, with its own address and code.
    pub fn clear_account(&mut self) -> Result<(), String> {
        let kept = self.account_secret.as_ref().is_none_or(Option::is_some);
        if kept {
            self.move_old_codes();
            self.with_store(|store| store.delete(ACCOUNT_ID))
                .map_err(|_| "The Bambu Lab sign-in could not be removed.".to_owned())?;
        }
        self.account_secret = Some(None);
        let was = self.account.take();
        if let Err(error) = self.write(
            &self.printers,
            &self.coded,
            self.selected.as_deref(),
            &self.options,
        ) {
            self.account = was;
            return Err(error);
        }
        Ok(())
    }

    /// Does `what` on the store; where the system's store refuses, the
    /// codes are kept in the file from then on, and that is said.
    fn with_store<T>(
        &mut self,
        what: impl Fn(&mut dyn Secrets) -> Result<T, Refused>,
    ) -> Result<T, String> {
        let system = self.secrets.kind().is_system();
        if system {
            self.asked = true;
        }
        match what(self.secrets.as_mut()) {
            Ok(done) => Ok(done),
            Err(Refused(why)) if system => {
                self.fall_back(&why);
                what(self.secrets.as_mut())
                    .map_err(|Refused(why)| format!("The access code could not be saved ({why})"))
            }
            Err(Refused(why)) => Err(format!("The access code could not be saved ({why})")),
        }
    }

    /// The file in place of the system's store, for the rest of the run.
    fn fall_back(&mut self, why: &str) {
        self.secrets = match &self.dir {
            Some(dir) => Box::new(FileStore::new(Some(dir.join(SECRETS)))),
            None => Box::new(Memory::default()),
        };
        let kept = if self.dir.is_some() {
            "Access codes are kept in a file on this computer instead, readable by your user account only."
        } else {
            "Access codes are kept for this run only."
        };
        self.notice = Some(format!(
            "{} could not be used: {why}. {kept}",
            first_upper(secrets::system_name())
        ));
    }

    /// The first time the system's store is used: the codes of the file
    /// they were kept in before go into the store, and the file is removed
    /// once the store gave the same back. Where it does not, the file stays
    /// as it is, is used for the run, and that is said.
    fn move_old_codes(&mut self) {
        if self.moved || !self.secrets.kind().is_system() {
            return;
        }
        self.moved = true;
        let Some(dir) = self.dir.clone() else { return };
        let file = dir.join(SECRETS);
        let old = match secrets::read_file(&file) {
            Ok(old) => old,
            Err(Refused(why)) => {
                self.notice = Some(format!(
                    "The access codes kept in a file could not be moved to {}: {why}. The file is left as it is.",
                    secrets::system_name()
                ));
                return;
            }
        };
        if old.is_empty() {
            return;
        }
        self.asked = true;
        let checked = self.secrets.set_many(&old).and_then(|()| {
            self.secrets.reread();
            for (id, code) in &old {
                if self.secrets.get(id)?.as_deref() != Some(code.as_str()) {
                    return Err(Refused("it did not give back what was written".into()));
                }
            }
            Ok(())
        });
        match checked {
            Ok(()) => {
                if let Err(error) = std::fs::remove_file(&file) {
                    self.notice = Some(format!(
                        "The access codes were moved to {}, but the file they were kept in could not be removed ({}).",
                        secrets::system_name(),
                        error.kind()
                    ));
                }
            }
            Err(Refused(why)) => {
                self.fall_back(&why);
                self.notice = Some(format!(
                    "The access codes kept in a file could not be moved to {}: {why}. They stay in the file on this computer, readable by your user account only.",
                    secrets::system_name()
                ));
            }
        }
    }

    /// Adds a printer, or changes the one of that serial; `code` replaces
    /// its access code when given (the store is written only when it is a
    /// new code).
    pub fn put(&mut self, printer: Printer, code: Option<String>) -> Result<(), String> {
        let mut printers = self.printers.clone();
        match printers.iter_mut().find(|p| p.dev_id == printer.dev_id) {
            Some(was) => *was = printer.clone(),
            None => printers.push(printer.clone()),
        }
        let mut coded = self.coded.clone();
        if let Some(code) = code
            && self.known.get(&printer.dev_id) != Some(&code)
        {
            self.move_old_codes();
            let dev_id = printer.dev_id.clone();
            self.with_store(|store| store.set(&dev_id, &code))?;
            self.known.insert(dev_id.clone(), code);
            coded.insert(dev_id);
        }
        self.write(&printers, &coded, self.selected.as_deref(), &self.options)?;
        self.printers = printers;
        self.coded = coded;
        Ok(())
    }

    /// Takes a printer's code out of the store, where one is kept.
    fn delete_code(&mut self, dev_id: &str) -> Result<(), String> {
        if !self.coded.contains(dev_id) {
            return Ok(());
        }
        self.move_old_codes();
        self.with_store(|store| store.delete(dev_id))?;
        self.known.remove(dev_id);
        Ok(())
    }

    /// Forgets a printer, its code with it.
    pub fn remove(&mut self, dev_id: &str) -> Result<(), String> {
        self.delete_code(dev_id)?;
        let printers: Vec<Printer> = self
            .printers
            .iter()
            .filter(|p| p.dev_id != dev_id)
            .cloned()
            .collect();
        let mut coded = self.coded.clone();
        coded.remove(dev_id);
        let selected = self.selected.clone().filter(|s| s != dev_id);
        self.write(&printers, &coded, selected.as_deref(), &self.options)?;
        self.printers = printers;
        self.coded = coded;
        self.selected = selected;
        Ok(())
    }

    /// Forgets a printer's code (the printer refused it: `erase_user_access_code`).
    pub fn forget_code(&mut self, dev_id: &str) -> Result<(), String> {
        self.delete_code(dev_id)?;
        let mut coded = self.coded.clone();
        coded.remove(dev_id);
        self.write(
            &self.printers,
            &coded,
            self.selected.as_deref(),
            &self.options,
        )?;
        self.coded = coded;
        Ok(())
    }

    pub fn select(&mut self, dev_id: &str) -> Result<(), String> {
        if self.selected.as_deref() == Some(dev_id) {
            return Ok(());
        }
        self.write(&self.printers, &self.coded, Some(dev_id), &self.options)?;
        self.selected = Some(dev_id.to_owned());
        Ok(())
    }

    pub fn set_options(&mut self, options: Options) -> Result<(), String> {
        self.write(
            &self.printers,
            &self.coded,
            self.selected.as_deref(),
            &options,
        )?;
        self.options = options;
        Ok(())
    }

    /// The list of printers, with whether each has a code, never the code.
    fn write(
        &self,
        printers: &[Printer],
        coded: &BTreeSet<String>,
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
                json!({
                    "dev_id": p.dev_id, "dev_name": p.name, "dev_ip": p.ip, "printer_type": p.model,
                    "access_code_kept": coded.contains(&p.dev_id),
                })
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
        let mut root =
            json!({ "version": 1, "selected": selected, "print": print, "printers": list });
        if let Some(account) = &self.account {
            root["account"] = json!({
                "user": account.user.json(),
                "printers": account.printers.iter().map(|p| json!({
                    "dev_id": p.dev_id, "dev_name": p.name, "printer_type": p.model, "online": p.online,
                    "task_status": p.task_status, "access_code_kept": p.has_code,
                })).collect::<Vec<_>>(),
            });
        }
        let text = serde_json::to_string_pretty(&root).expect("plain JSON");
        write_whole(&dir.join(FILE), text.as_bytes(), false).map_err(|error| failed(FILE, error))
    }
}

fn first_upper(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
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
