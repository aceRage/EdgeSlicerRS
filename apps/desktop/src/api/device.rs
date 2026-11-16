//! The Device page as the page asks for it (`crate::device`): which family
//! the project's printer is and what its page has (`device_info`), what it
//! shows now (`device_read`), and a command, asked first and then sent
//! (`device_command`).
//!
//! A Bambu Lab printer is the one picked among those added (UltraNet,
//! `bambu_lan`); any other is the project's printer through its connection
//! (`print_host`). Nothing is asked of a printer but when the page asks,
//! while Device is looked at. No answer holds a key or an access code.

use std::{collections::HashMap, sync::Mutex};

use serde_json::{Value, json};

use super::{Session, lock};
use crate::api::i18n::tr;
use crate::device::{self, Caps, Device};
use crate::hosts::{self, Connection, Http, snapmaker};

/// The calls answered here.
pub(super) const JSON: [&str; 6] = [
    "device_info",
    "device_read",
    "device_command",
    "device_camera",
    "device_files",
    "device_file_delete",
];

/// What each printer's page has, kept while Device is looked at (asked
/// again with `refresh`), and whether an "Octo/Klipper" address is a
/// Moonraker or an OctoPrint.
#[derive(Default)]
pub(super) struct Devices {
    caps: Mutex<HashMap<String, Caps>>,
    kinds: Mutex<HashMap<String, &'static str>>,
    /// "Storage"'s starts the person was asked about: the advice they
    /// read, by printer and file; one confirmation, one start
    /// (ElegooSdcpFiles.cpp `m_confirmed_plan`).
    pub(super) confirmed: Mutex<HashMap<String, String>>,
}

/// A printer's page: its adapter, the key its facts are kept under, its
/// name for the questions.
struct Found {
    device: Box<dyn Device>,
    key: String,
    name: String,
}

/// The HMS files' name for a language (`de_DE` "de", `pt_BR` "pt-BR",
/// `zh_CN` "zh-cn").
fn hms_language(code: &str) -> String {
    match code {
        "pt_BR" => "pt-BR".into(),
        "pt_PT" => "pt-PT".into(),
        "zh_CN" => "zh-cn".into(),
        "zh_TW" => "zh-tw".into(),
        other => other.chars().take(2).collect(),
    }
}

impl Session {
    pub(super) fn device_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "device_info" => self.device_info(args),
            "device_read" => self.device_read(args),
            "device_command" => self.device_command(args),
            "device_camera" => {
                let found = self.found(args)?.ok_or_else(|| tr(device::NOT_HERE))?;
                found.device.camera(super::needed(args, "action")?)
            }
            "device_files" => self.device_files(args),
            "device_file_delete" => self.device_file_delete(args),
            other => Err(format!("there is no call named {other}")),
        }
    }

    fn is_bambu_project(&self, args: &Value) -> Result<bool, String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        Ok(super::export::is_bambu(
            &project.choices,
            project.file.as_deref(),
        ))
    }

    /// Which kind of host an "Octo/Klipper" address is: a Moonraker answers
    /// `server/info`; an OctoPrint `api/version` with "OctoPrint". Asked
    /// once for an address.
    fn octo_kind(&self, connection: &Connection) -> Result<&'static str, String> {
        let address = connection.print_host.trim().to_owned();
        if let Some(kind) = lock(&self.devices.kinds).get(&address) {
            return Ok(kind);
        }
        let http = Http::new(connection, self.hosts.timeouts)?;
        let moonraker = http.get("server/info")?;
        let kind = if moonraker.ok() && moonraker.json().is_some_and(|j| j.get("result").is_some())
        {
            "moonraker"
        } else if moonraker.status == 401 || moonraker.status == 403 {
            // A Moonraker that wants a key it was not given, or OctoPrint
            // refusing the key: OctoPrint answers `api/version` 403 too.
            match http.get("api/version") {
                Ok(answer) if answer.json().is_some_and(|j| j.get("api").is_some()) => "octoprint",
                _ => return Err(hosts::http_error(&moonraker)),
            }
        } else {
            hosts::octoprint::version(&http)?;
            "octoprint"
        };
        lock(&self.devices.kinds).insert(address, kind);
        Ok(kind)
    }

    /// The page of the project's printer, or why it has none (`Ok(None)`:
    /// a family without a native page; the printer's own page is shown).
    fn found(&self, args: &Value) -> Result<Option<Found>, String> {
        if self.is_bambu_project(args)? {
            let printer = self.bambu_printer(args)?;
            let hms_dir = lock(&self.state)
                .library
                .as_ref()
                .map(|library| library.dir().join("..").join("hms"));
            return Ok(Some(Found {
                device: Box::new(device::bambu::Bambu {
                    client: self.bambu.client.clone(),
                    dev_id: printer.dev_id.clone(),
                    hms_dir,
                    language: hms_language(super::i18n::current().code),
                }),
                key: format!("bambu:{}", printer.dev_id),
                name: printer.name,
            }));
        }
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        let (connection, _) = self.connection(&printer);
        if connection.print_host.trim().is_empty() {
            return Err(tr(
                "Please set up your printer connection to view the device.",
            ));
        }
        let key = format!("{}|{}", connection.host_type, connection.print_host.trim());
        let timeouts = self.hosts.timeouts;
        let device: Box<dyn Device> = match connection.host_type.as_str() {
            "octoprint" if snapmaker::is_u1(&connection.printer_model) => {
                Box::new(device::klipper::Klipper {
                    http: Http::new(&connection, timeouts)?,
                    u1: true,
                })
            }
            "octoprint" => match self.octo_kind(&connection)? {
                "moonraker" => Box::new(device::klipper::Klipper {
                    http: Http::new(&connection, timeouts)?,
                    u1: false,
                }),
                _ => Box::new(device::octo::Octo {
                    http: Http::new(&connection, timeouts)?,
                }),
            },
            "elegoolink" if !hosts::elegoo::is_cc2(&connection.printer_model) => {
                Box::new(device::sdcp::Sdcp {
                    link: hosts::elegoo::ElegooLink::new(&connection, timeouts)?,
                })
            }
            "prusalink" | "prusaconnect" => Box::new(device::prusalink::PrusaLink {
                http: Http::new(&connection, timeouts)?,
            }),
            _ => return Ok(None),
        };
        Ok(Some(Found {
            device,
            key,
            name: printer.name,
        }))
    }

    /// Which page the project's printer has: `{ native, family, printer }`;
    /// `native` false for a family drawn by the printer's own page alone.
    /// Asks the printer nothing, but which kind an "Octo/Klipper" host is.
    fn device_info(&self, args: &Value) -> Result<Value, String> {
        if self.is_bambu_project(args)? {
            return Ok(json!({ "native": true, "family": "bambu" }));
        }
        let Some(printer) = self.printer(args)? else {
            return Ok(json!({ "native": false, "family": null }));
        };
        let (connection, _) = self.connection(&printer);
        let family = match connection.host_type.as_str() {
            "octoprint" if snapmaker::is_u1(&connection.printer_model) => "u1",
            "octoprint" => "octo_klipper",
            "elegoolink" if !hosts::elegoo::is_cc2(&connection.printer_model) => "elegoo",
            "prusalink" | "prusaconnect" => "prusalink",
            _ => {
                return Ok(
                    json!({ "native": false, "family": connection.host_type, "printer": printer.name }),
                );
            }
        };
        Ok(json!({ "native": true, "family": family, "printer": printer.name }))
    }

    /// `{ caps, reading }`, or `{ online: false, error }` when the printer
    /// does not answer (the page keeps the last caps then). `refresh` asks
    /// what the printer has again.
    fn device_read(&self, args: &Value) -> Result<Value, String> {
        let found = match self.found(args) {
            Ok(Some(found)) => found,
            Ok(None) => return Ok(json!({ "native": false })),
            Err(error) => {
                return Ok(json!({ "online": false, "said": tr("Offline"), "error": error }));
            }
        };
        let refresh = args.get("refresh").and_then(Value::as_bool) == Some(true);
        let kept = if refresh {
            None
        } else {
            lock(&self.devices.caps).get(&found.key).cloned()
        };
        let caps = match kept {
            Some(caps) => caps,
            None => match found.device.caps() {
                Ok(caps) => {
                    lock(&self.devices.caps).insert(found.key.clone(), caps.clone());
                    caps
                }
                Err(error) => {
                    return Ok(json!({ "online": false, "said": tr("Offline"), "error": error }));
                }
            },
        };
        match found.device.read(&caps) {
            Ok(reading) => Ok(json!({
                "printer": found.name,
                "caps": device::caps_json(&caps),
                "reading": device::reading_json(&reading),
            })),
            Err(error) => Ok(json!({
                "printer": found.name,
                "caps": device::caps_json(&caps),
                "online": false, "said": tr("Offline"), "error": error,
            })),
        }
    }

    /// A command: without `confirmed` the question to ask (`{ confirm }`),
    /// or why it cannot be done (an error); with it, checked again against
    /// what the printer does now and sent (`{ sent }`).
    fn device_command(&self, args: &Value) -> Result<Value, String> {
        let mut command = device::command_of(args)?;
        let found = self.found(args)?.ok_or_else(|| tr(device::NOT_HERE))?;
        let caps = match lock(&self.devices.caps).get(&found.key).cloned() {
            Some(caps) => caps,
            None => found.device.caps()?,
        };
        let reading = found.device.read(&caps)?;
        let confirm = device::gate(&mut command, &caps, &reading, &found.name)?;
        if args.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return Ok(json!({ "confirm": confirm.json() }));
        }
        found.device.command(&caps, &command)?;
        Ok(json!({ "sent": args["action"] }))
    }

    /// The printer's files of a `kind` (Bambu's "Storage", section 15 of
    /// ULTRANET_BRIDGE.md): `{ files: [{ name, path, time, size }] }`.
    fn device_files(&self, args: &Value) -> Result<Value, String> {
        let found = self.found(args)?.ok_or_else(|| tr(device::NOT_HERE))?;
        let files = found.device.files(super::needed(args, "kind")?)?;
        Ok(json!({
            "files": files
                .iter()
                .map(|f| json!({ "name": f.name, "path": f.path, "time": f.time, "size": f.size }))
                .collect::<Vec<_>>(),
        }))
    }

    /// "Delete" in Bambu's "Storage": without `confirmed` the reference's
    /// question (MediaFilePanel.cpp l.580); with it, the file deleted.
    fn device_file_delete(&self, args: &Value) -> Result<Value, String> {
        let path = super::needed(args, "path")?;
        let found = self.found(args)?.ok_or_else(|| tr(device::NOT_HERE))?;
        if args.get("confirmed").and_then(Value::as_bool) != Some(true) {
            let name = path.rsplit('/').next().unwrap_or(path);
            let confirm = device::Confirm {
                title: tr("Delete file"),
                lines: vec![crate::api::i18n::fill(
                    &tr("Do you want to delete the file '%s' from printer?"),
                    &[&name],
                )],
                ok: tr("Delete"),
            };
            return Ok(json!({ "confirm": confirm.json() }));
        }
        found.device.delete_file(path)?;
        Ok(json!({ "deleted": path }))
    }
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
