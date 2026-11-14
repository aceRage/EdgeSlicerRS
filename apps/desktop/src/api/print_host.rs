//! Sending a plate to a printer, as the page asks for it: the printer's
//! connection ("Physical Printer", PhysicalPrinterDialog.cpp), "Test",
//! "Send G-code to printer host" ("Upload", "Upload and Print"), the
//! "Print host upload queue", and the files on the printer for the Device
//! tab. The requests themselves are made by `crate::hosts`.
//!
//! A connection is kept for each printer preset (`hosts::kept`); one that
//! is not kept is the preset's own (`print_host` and the rest, which
//! forty-seven vendor presets carry). The key is never answered, only
//! whether there is one.

use std::sync::Arc;

use edge_config::Settings;
use serde_json::{Value, json};

use super::{Session, lock, needed};
use crate::api::i18n::tr;
use crate::hosts::{
    self, Connection, HOST_TYPES, Options, Toolhead, Upload, elegoo,
    kept::{Key, Saved},
    snapmaker::{self, FileFilament},
};

/// The calls answered here.
pub(super) const JSON: [&str; 13] = [
    "print_host",
    "print_host_set",
    "print_host_test",
    "print_send_name",
    "print_send",
    "print_queue",
    "print_cancel",
    "print_queue_clear",
    "printer_files",
    "printer_file_delete",
    "printer_file_print",
    "printer_toolheads",
    "printer_status",
];

/// The printer a project is on, as far as sending is concerned.
struct Printer {
    /// The preset's name (or the name a project's own printer was saved
    /// under): what its connection is kept by.
    name: String,
    settings: Arc<Settings>,
}

impl Printer {
    fn text(&self, key: &str) -> String {
        self.settings
            .text(key)
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    fn model(&self) -> String {
        self.text("printer_model")
    }

    /// The preset's own connection.
    fn preset(&self) -> Connection {
        let host_type = self.text("host_type");
        Connection {
            // The reference's default (PrintConfig.cpp, `host_type`).
            host_type: if host_type.is_empty() {
                "octoprint".into()
            } else {
                host_type
            },
            print_host: self.text("print_host"),
            print_host_webui: self.text("print_host_webui"),
            printhost_cafile: self.text("printhost_cafile"),
            apikey: Some(self.text("printhost_apikey")).filter(|key| !key.is_empty()),
            printer_model: self.model(),
        }
    }
}

impl Session {
    pub(super) fn print_host_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "print_host" => self.print_host(args),
            "print_host_set" => self.print_host_set(args),
            "print_host_test" => self.print_host_test(args),
            "print_send_name" => self.print_send_name(args),
            "print_send" => self.print_send(args),
            "print_queue" => Ok(self.hosts.queue.listing()),
            "print_cancel" => {
                let id = args
                    .get("id")
                    .and_then(Value::as_u64)
                    .ok_or("id was not given")?;
                self.hosts.queue.cancel(id as u32)?;
                Ok(self.hosts.queue.listing())
            }
            "print_queue_clear" => {
                self.hosts.queue.clear_done();
                Ok(self.hosts.queue.listing())
            }
            "printer_files" => self.printer_files(args),
            "printer_file_delete" => {
                let (host, _) = self.host(args)?;
                host.delete(needed(args, "name")?)?;
                self.printer_files(args)
            }
            "printer_toolheads" => self.printer_toolheads(args),
            "printer_status" => self.printer_status(args),
            "printer_file_print" => {
                let (host, _) = self.host(args)?;
                host.print(needed(args, "name")?)?;
                Ok(json!({ "started": true }))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    fn printer(&self, args: &Value) -> Result<Option<Printer>, String> {
        let (choices, file) = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            (project.choices.clone(), project.file.clone())
        };
        let Some((_, settings)) = self.printer_of(&choices, file.as_deref())? else {
            return Ok(None);
        };
        let name = if choices.from_presets {
            choices.machine.clone()
        } else {
            settings.text("printer_settings_id").map(str::to_owned)
        };
        Ok(name
            .filter(|name| !name.is_empty())
            .map(|name| Printer { name, settings }))
    }

    /// The connection in force for a printer: the one kept, else the
    /// preset's; and where it comes from.
    fn connection(&self, printer: &Printer) -> (Connection, &'static str) {
        let kept = lock(&self.hosts.kept);
        match kept.get(&printer.name) {
            Some(saved) => (
                Connection {
                    host_type: saved.host_type.clone(),
                    print_host: saved.print_host.clone(),
                    print_host_webui: saved.print_host_webui.clone(),
                    printhost_cafile: saved.printhost_cafile.clone(),
                    apikey: kept.key(&printer.name).map(str::to_owned),
                    printer_model: printer.model(),
                },
                "kept",
            ),
            None => (printer.preset(), "preset"),
        }
    }

    /// The host for the project's printer, and its address as shown.
    fn host(&self, args: &Value) -> Result<(Box<dyn hosts::Host>, String), String> {
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        let (connection, _) = self.connection(&printer);
        let host = hosts::make(&connection, self.hosts.timeouts)?;
        let shown = hosts::Address::parse(&connection.print_host)?
            .shown()
            .to_owned();
        Ok((host, shown))
    }

    /// `{ printer, model, from ("kept", "preset"), connection { host_type,
    /// print_host, print_host_webui, printhost_cafile, has_api_key },
    /// host_types [{ value, label, built }], built, can_send, can_start,
    /// device_url, route, offers { elegoo, unload }, bed_in_force, problem }`.
    fn print_host(&self, args: &Value) -> Result<Value, String> {
        let problem = lock(&self.hosts.kept).problem.clone();
        // A Bambu Lab printer: the print button offers "Send to Bambu
        // Connect" (MainFrame.cpp `available_print_actions`).
        let bambu = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            super::export::is_bambu(&project.choices, project.file.as_deref())
        };
        let host_types: Vec<Value> = HOST_TYPES
            .iter()
            .map(|(value, label, built)| json!({ "value": value, "label": label, "built": built }))
            .collect();
        let Some(printer) = self.printer(args)? else {
            return Ok(
                json!({ "printer": null, "host_types": host_types, "problem": problem, "bambu": bambu }),
            );
        };
        let (connection, from) = self.connection(&printer);
        let model = printer.model();
        let built = HOST_TYPES
            .iter()
            .any(|(key, _, built)| *built && *key == connection.host_type);
        let made = hosts::make(&connection, self.hosts.timeouts);
        let route = match connection.host_type.as_str() {
            "octoprint" if snapmaker::is_u1(&model) => "snapmaker",
            other => other,
        };
        // The Centauri Carbon's own options (PrintHostDialogs.cpp l.2710
        // asks for its `model_id`, which a printer preset does not carry
        // here: its `printer_model` says the same).
        let elegoo = connection.host_type == "elegoolink"
            && model.to_lowercase().starts_with("elegoo centauri")
            && !elegoo::is_cc2(&model);
        // "Unload filaments after print", for Snapmaker's tool changers
        // (Plater.cpp l.27495), ticked as the preset says.
        let unload = (route == "snapmaker").then(|| {
            json!({ "default": printer.settings.flag("unload_filaments_at_end").ok().flatten().unwrap_or(false) })
        });
        Ok(json!({
            "printer": printer.name,
            "model": model,
            "from": from,
            "connection": {
                "host_type": connection.host_type,
                "print_host": connection.print_host,
                "print_host_webui": connection.print_host_webui,
                "printhost_cafile": connection.printhost_cafile,
                "has_api_key": connection.apikey.is_some(),
            },
            "host_types": host_types,
            "built": built,
            "can_send": made.is_ok(),
            "why_not": made.as_ref().err(),
            "can_start": made.as_ref().is_ok_and(|host| host.can_start()),
            "storage": route == "snapmaker",
            // The U1's toolhead map and its state in Device.
            "toolheads": route == "snapmaker",
            "status": route == "snapmaker",
            "device_url": hosts::device_url(&connection),
            "route": route,
            "offers": { "elegoo": elegoo, "unload": unload },
            "problem": problem,
            "bambu": bambu,
        }))
    }

    /// The fields of a connection as the page sends them, over `base`.
    fn typed(args: &Value, base: &Connection) -> Connection {
        let field = |key: &str, was: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map_or_else(|| was.to_owned(), |text| text.trim().to_owned())
        };
        Connection {
            host_type: field("host_type", &base.host_type),
            print_host: field("print_host", &base.print_host),
            print_host_webui: field("print_host_webui", &base.print_host_webui),
            printhost_cafile: field("printhost_cafile", &base.printhost_cafile),
            apikey: match args.get("apikey") {
                Some(Value::String(key)) => Some(key.clone()).filter(|key| !key.is_empty()),
                Some(Value::Null) => None,
                _ => base.apikey.clone(),
            },
            printer_model: base.printer_model.clone(),
        }
    }

    /// Saves the connection of the project's printer: the fields given
    /// (each as typed), `apikey` a new key ("" or null to clear it, left
    /// out to keep it); or with `forget` goes back to the preset's.
    fn print_host_set(&self, args: &Value) -> Result<Value, String> {
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        if args.get("forget").and_then(Value::as_bool) == Some(true) {
            lock(&self.hosts.kept).set(&printer.name, None, Key::Keep)?;
            return self.print_host(args);
        }
        let (base, from) = self.connection(&printer);
        let typed = Self::typed(args, &base);
        if !HOST_TYPES.iter().any(|(key, _, _)| *key == typed.host_type) {
            return Err(format!("{} is not a host type", typed.host_type));
        }
        if !typed.print_host.is_empty() {
            hosts::Address::parse(&typed.print_host)?;
        }
        if !typed.print_host_webui.is_empty() {
            hosts::Address::parse(&typed.print_host_webui)?;
        }
        let key = match args.get("apikey") {
            Some(Value::String(key)) => Key::Set(key.clone()),
            Some(Value::Null) => Key::Clear,
            // Saved for the first time: the preset's key comes with it.
            _ if from == "preset" => typed.apikey.clone().map_or(Key::Keep, Key::Set),
            _ => Key::Keep,
        };
        let saved = Saved {
            host_type: typed.host_type,
            print_host: typed.print_host,
            print_host_webui: typed.print_host_webui,
            printhost_cafile: typed.printhost_cafile,
        };
        lock(&self.hosts.kept).set(&printer.name, Some(saved), key)?;
        self.print_host(args)
    }

    /// "Test": the connection as typed (what is not given is as saved).
    /// `{ ok, message }`, the message the reference's.
    fn print_host_test(&self, args: &Value) -> Result<Value, String> {
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        let (base, _) = self.connection(&printer);
        let connection = Self::typed(args, &base);
        let host = match hosts::make(&connection, self.hosts.timeouts) {
            Ok(host) => host,
            Err(why) => return Ok(json!({ "ok": false, "message": why })),
        };
        Ok(match host.test() {
            Ok(()) => json!({ "ok": true, "message": host.test_ok() }),
            Err(reason) => json!({ "ok": false, "message": host.test_failed(&reason) }),
        })
    }

    /// The name "Send G-code to printer host" offers: the printer's
    /// `filename_format` for the project, as the reference fills it in
    /// for this dialog (Plater.cpp `prepare_upload_filename_for_dialog`).
    fn print_send_name(&self, args: &Value) -> Result<Value, String> {
        let (done, plate) = self.done(args)?;
        let project_name = {
            let state = lock(&self.state);
            state.projects[state.target(args)?].name.clone()
        };
        let initial = self
            .plate_filaments(args, plate)?
            .first()
            .copied()
            .unwrap_or(0);
        Ok(
            json!({ "name": upload_name(&done.sliced.settings, &project_name, done.gcode.as_ref().ok().map(|g| g.summary.seconds), initial) }),
        )
    }

    /// The filaments a plate prints in, from 0.
    fn plate_filaments(&self, args: &Value, plate: Option<u32>) -> Result<Vec<usize>, String> {
        let state = lock(&self.state);
        let project = &state.projects[state.target(args)?];
        let Some(file) = project.file.as_deref() else {
            return Ok(Vec::new());
        };
        let on_plate = plate
            .and_then(|plate| file.plates.iter().find(|info| info.id == plate))
            .map(|info| info.filaments.clone())
            .unwrap_or_else(|| file.used.clone());
        Ok(on_plate
            .into_iter()
            .filter(|n| *n > 0)
            .map(|n| n - 1)
            .collect())
    }

    /// "Upload" (`start` false) or "Upload and Print": the plate's G-code,
    /// the very bytes "Export G-code file" writes, put in the queue under
    /// `name`. `{ id, queue }`.
    fn print_send(&self, args: &Value) -> Result<Value, String> {
        let name = needed(args, "name")?.trim().to_owned();
        if name.is_empty() {
            return Err("The file name is empty.".into());
        }
        let start = args.get("start").and_then(Value::as_bool).unwrap_or(false);
        let (done, plate) = self.done(args)?;
        let gcode = done.gcode.as_ref().map_err(String::clone)?;
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        let (connection, _) = self.connection(&printer);
        let host = hosts::make(&connection, self.hosts.timeouts)?;
        if start && !host.can_start() {
            return Err("This printer cannot be started from here.".into());
        }
        let shown = hosts::Address::parse(&connection.print_host)?
            .shown()
            .to_owned();
        let flag = |key: &str| args["options"][key].as_bool().unwrap_or(false);
        let filaments = self.plate_filaments(args, plate)?;
        // "Upload and Print" to a U1: the toolhead of each filament, as
        // chosen, else as the reference first offers it; checked against
        // what the printer holds before anything is sent (RemoteSend.cpp
        // `prepare_snapmaker`).
        let mapping = if start && snapmaker::is_u1(&connection.printer_model) {
            let file = self.file_filaments(&done.sliced.settings, &filaments);
            let heads = match host.toolheads() {
                Some(Ok(heads)) => heads,
                Some(Err(error)) => return Err(error),
                None => Vec::new(),
            };
            let mapping = match args["options"]["mapping"].as_array() {
                Some(chosen) => chosen
                    .iter()
                    .map(|t| t.as_i64().map_or(-1, |t| t as i32))
                    .collect(),
                None => snapmaker::auto_match(&file, &heads),
            };
            if let Some(problem) =
                snapmaker::mapping_problem(&file, &mapping, &heads, &printer.name, flag("force"))
            {
                return Err(problem);
            }
            mapping
        } else {
            Vec::new()
        };
        let upload = Upload {
            name,
            data: Arc::new(gcode.text.clone()),
            start,
            options: Options {
                timelapse: flag("timelapse"),
                leveling: flag("leveling"),
                smooth_plate: flag("smooth_plate"),
                unload_at_end: flag("unload_at_end"),
                filaments,
                mapping,
                bed_level: flag("bed_level"),
                flow_calibrate: flag("flow_calibrate"),
                time_lapse: flag("time_lapse"),
            },
        };
        let id = self.hosts.queue.add(host, &shown, &printer.name, upload);
        Ok(json!({ "id": id, "queue": self.hosts.queue.listing() }))
    }

    /// The file's filaments, each with its colour and type and whether the
    /// plate prints in it (RemoteSend.cpp `file_filaments_of`).
    fn file_filaments(&self, settings: &Settings, used: &[usize]) -> Vec<FileFilament> {
        let colours = settings.texts("filament_colour").unwrap_or_default();
        let types = settings.texts("filament_type").unwrap_or_default();
        let count = colours
            .len()
            .max(types.len())
            .max(used.iter().map(|f| f + 1).max().unwrap_or(0));
        (0..count)
            .map(|index| FileFilament {
                index,
                color: colours.get(index).cloned().unwrap_or_default(),
                filament_type: types.get(index).cloned().unwrap_or_default(),
                used: used.contains(&index),
            })
            .collect()
    }

    /// What the U1's toolheads hold, set against the plate's filaments:
    /// `{ printer, state, toolheads [{ index, type, sub_type, vendor, color,
    /// loaded, nozzle }], filaments [{ index, color, type, used }], mapping,
    /// warnings, problem }`. `mapping` given is checked; else the
    /// reference's first offer is answered.
    fn printer_toolheads(&self, args: &Value) -> Result<Value, String> {
        let (done, plate) = self.done(args)?;
        let (host, _) = self.host(args)?;
        let printer = self
            .printer(args)?
            .ok_or("No printer is chosen for the project.")?;
        let heads = host
            .toolheads()
            .ok_or("This printer does not say what its toolheads hold.")??;
        let state = match host.status() {
            Some(Ok(status)) => status.state,
            _ => String::new(),
        };
        let used = self.plate_filaments(args, plate)?;
        let file = self.file_filaments(&done.sliced.settings, &used);
        let mapping: Vec<i32> = match args.get("mapping").and_then(Value::as_array) {
            Some(chosen) => chosen
                .iter()
                .map(|t| t.as_i64().map_or(-1, |t| t as i32))
                .collect(),
            None => snapmaker::auto_match(&file, &heads),
        };
        let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
        Ok(json!({
            "printer": printer.name,
            "state": state,
            "toolheads": heads.iter().map(toolhead_json).collect::<Vec<_>>(),
            "filaments": file.iter().map(|f| json!({ "index": f.index, "color": f.color, "type": f.filament_type, "used": f.used })).collect::<Vec<_>>(),
            "mapping": mapping,
            "warnings": snapmaker::mapping_warnings(&file, &mapping, &heads),
            "problem": snapmaker::mapping_problem(&file, &mapping, &heads, &printer.name, force),
        }))
    }

    /// What the printer is doing, for the Device tab: `{ online, state,
    /// said, filename, message, percent, left, layer, total_layers, bed,
    /// nozzles }`, or `{ online: false, error }` when it does not answer.
    /// `{ known: false }` for a printer this app cannot ask.
    fn printer_status(&self, args: &Value) -> Result<Value, String> {
        let (host, _) = self.host(args)?;
        let Some(answer) = host.status() else {
            return Ok(json!({ "known": false }));
        };
        Ok(match answer {
            Ok(status) => json!({
                "known": true,
                "online": true,
                "state": status.state,
                "said": tr(state_said(&status.state)),
                "filename": status.filename,
                "message": status.message,
                "percent": status.percent,
                "left": status.left,
                "layer": status.layer,
                "total_layers": status.total_layers,
                "bed": status.bed.map(|(t, target)| json!([t, target])),
                "nozzles": status.nozzles.iter().map(|(t, target)| json!([t, target])).collect::<Vec<_>>(),
            }),
            Err(error) => {
                json!({ "known": true, "online": false, "said": tr("Offline"), "error": error })
            }
        })
    }

    /// The files on the printer: `{ files: [{ name, size, modified }] }`.
    fn printer_files(&self, args: &Value) -> Result<Value, String> {
        let (host, _) = self.host(args)?;
        let files = host
            .files()
            .ok_or("The files on this printer cannot be listed from here yet.")??;
        Ok(json!({
            "files": files
                .iter()
                .map(|file| json!({ "name": file.name, "size": file.size, "modified": file.modified }))
                .collect::<Vec<_>>(),
        }))
    }
}

impl Session {
    /// The origins of the printers' pages the page may frame: every
    /// connection kept, and the address each open project's printer
    /// preset gives.
    pub fn frame_origins(&self) -> Vec<String> {
        let ids: Vec<u32> = lock(&self.state)
            .projects
            .iter()
            .map(|project| project.id)
            .collect();
        let mut origins: Vec<String> = lock(&self.hosts.kept)
            .all()
            .filter_map(|saved| {
                hosts::device_url(&Connection {
                    print_host: saved.print_host.clone(),
                    print_host_webui: saved.print_host_webui.clone(),
                    ..Connection::default()
                })
            })
            .collect();
        for id in ids {
            if let Ok(Some(printer)) = self.printer(&json!({ "project": id })) {
                origins.extend(hosts::device_url(&self.connection(&printer).0));
            }
        }
        let mut origins: Vec<String> = origins.iter().filter_map(|url| origin(url)).collect();
        origins.sort();
        origins.dedup();
        origins
    }
}

fn toolhead_json(head: &Toolhead) -> Value {
    json!({
        "index": head.index,
        "type": head.filament_type,
        "sub_type": head.sub_type,
        "vendor": head.vendor,
        "color": head.color,
        "loaded": head.loaded,
        "official": head.official,
        "nozzle": head.nozzle,
    })
}

/// A Moonraker printer's state in words (`print_stats.state`).
fn state_said(state: &str) -> &'static str {
    match state {
        "standby" => "Idle",
        "printing" => "Printing",
        "paused" => "Paused",
        "complete" => "Finished",
        "cancelled" => "Canceled",
        "error" => "Error",
        _ => "Unknown",
    }
}

/// `http://host:port` of `http://host:port/path/`.
fn origin(url: &str) -> Option<String> {
    let after = url.find("://")? + 3;
    let end = url[after..].find('/').map_or(url.len(), |at| after + at);
    Some(url[..end].to_owned())
}

/// `filename_format` filled in for a plate sent to a printer. The
/// placeholders the reference's default and the vendors' own use are
/// filled in: `input_filename_base` (the project's name),
/// `filament_type[n]` and `filament_type[initial_tool]`, `print_time`,
/// `normal_print_time`, `silent_print_time` (no silent mode here: the
/// normal time), and the `.gcode` added when there is no extension
/// (PrintBase.cpp `output_filename`). Anything else in the format (a
/// condition, another variable) gives the project's name and `.gcode`, as
/// "Export G-code file" offers. The name is then folded to ASCII
/// (`fold_utf8_to_ascii`, Plater.cpp l.27382).
pub fn upload_name(
    settings: &Settings,
    project: &str,
    seconds: Option<f64>,
    initial: usize,
) -> String {
    let base = project.trim_end_matches(".3mf").trim_end_matches(".stl");
    let fallback = || fold_ascii(&format!("{base}.gcode"));
    let Some(format) = settings
        .text("filename_format")
        .filter(|format| !format.trim().is_empty())
    else {
        return fallback();
    };
    let types = settings.texts("filament_type").unwrap_or_default();
    let time = seconds.map(short_time).unwrap_or_default();
    let mut out = String::new();
    let mut rest = format;
    while let Some(open) = rest.find(['{', '[']) {
        out.push_str(&rest[..open]);
        let close_char = if rest[open..].starts_with('{') {
            '}'
        } else {
            ']'
        };
        let Some(close) = rest[open..].find(close_char) else {
            return fallback();
        };
        let inner = rest[open + 1..open + close].trim();
        let value = match inner {
            "input_filename_base" => base.to_owned(),
            "print_time" | "normal_print_time" | "silent_print_time" => time.clone(),
            _ => match inner
                .strip_prefix("filament_type[")
                .and_then(|index| index.strip_suffix(']'))
            {
                Some("initial_tool" | "initial_extruder") => {
                    types.get(initial).cloned().unwrap_or_default()
                }
                Some(index) => match index.parse::<usize>().ok().and_then(|n| types.get(n)) {
                    Some(kind) => kind.clone(),
                    None => return fallback(),
                },
                None => return fallback(),
            },
        };
        out.push_str(&value);
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    let name = if std::path::Path::new(&out).extension().is_none() {
        format!("{out}.gcode")
    } else {
        out
    };
    fold_ascii(&name)
}

/// `short_time(get_time_dhms(seconds))` (libslic3r/Utils.hpp l.581).
pub fn short_time(seconds: f64) -> String {
    let total = seconds.max(0.0);
    let mut days = (total / 86400.0) as u64;
    let mut hours = ((total % 86400.0) / 3600.0) as u64;
    let mut minutes = ((total % 3600.0) / 60.0) as u64;
    let secs = (total % 60.0) as u64;
    if days + hours > 0 && secs >= 30 {
        minutes += 1;
        if minutes == 60 {
            minutes = 0;
            hours += 1;
            if hours == 24 {
                hours = 0;
                days += 1;
            }
        }
    }
    if days > 0 {
        format!("{days}d{hours}h{minutes}m")
    } else if hours > 0 {
        format!("{hours}h{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m{secs}s")
    } else if secs >= 1 {
        format!("{secs}s")
    } else if total > 0.0 {
        "<1s".into()
    } else {
        "0s".into()
    }
}

/// Letters with accents to their plain letter, anything else that is not
/// ASCII to `_` (Utils/ASCIIFolding.cpp keeps more: its whole table).
pub fn fold_ascii(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                return c.to_string();
            }
            let plain = match c {
                'à'..='å' => "a",
                'À'..='Å' => "A",
                'æ' => "ae",
                'Æ' => "AE",
                'ç' => "c",
                'Ç' => "C",
                'è'..='ë' => "e",
                'È'..='Ë' => "E",
                'ì'..='ï' => "i",
                'Ì'..='Ï' => "I",
                'ñ' => "n",
                'Ñ' => "N",
                'ò'..='ö' | 'ø' => "o",
                'Ò'..='Ö' | 'Ø' => "O",
                'ù'..='ü' => "u",
                'Ù'..='Ü' => "U",
                'ý' | 'ÿ' => "y",
                'Ý' => "Y",
                'ß' => "ss",
                _ => "_",
            };
            plain.to_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_offered_is_the_printers_format_filled_in() {
        let with = |format: &str| {
            Settings::from_json(
                &json!({ "filename_format": format, "filament_type": ["PLA", "PETG"] }).to_string(),
            )
            .unwrap()
        };
        let settings =
            with("{input_filename_base}_{filament_type[initial_tool]}_{print_time}.gcode");
        assert_eq!(
            upload_name(&settings, "cube", Some(3725.0), 1),
            "cube_PETG_1h2m.gcode"
        );
        assert_eq!(
            upload_name(&settings, "Würfel", Some(59.0), 0),
            "Wurfel_PLA_59s.gcode"
        );
        assert_eq!(
            upload_name(&with("{input_filename_base}"), "cube", None, 0),
            "cube.gcode"
        );
        assert_eq!(
            upload_name(&with("{if x}a{endif}.gcode"), "cube", None, 0),
            "cube.gcode"
        );
        assert_eq!(
            upload_name(
                &with("[input_filename_base]-{filament_type[0]}"),
                "cube",
                None,
                0
            ),
            "cube-PLA.gcode"
        );
        assert_eq!(
            upload_name(&Settings::default(), "cube", None, 0),
            "cube.gcode"
        );
    }

    #[test]
    fn times_are_short_as_the_reference_writes_them() {
        assert_eq!(short_time(0.0), "0s");
        assert_eq!(short_time(0.5), "<1s");
        assert_eq!(short_time(42.0), "42s");
        assert_eq!(short_time(125.0), "2m5s");
        assert_eq!(short_time(3600.0 + 29.0 * 60.0 + 31.0), "1h30m");
        assert_eq!(short_time(86400.0 + 3600.0), "1d1h0m");
    }
}
