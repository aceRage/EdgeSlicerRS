//! A Bambu Lab printer on the local network, through the UltraNet bridge
//! (ULTRANET_BRIDGE.md): the bridge's state, the printers added by
//! address ("Connect the printer using IP and access code",
//! ReleaseNote.cpp `InputIpAddressDialog`), the printer's state for Device
//! (StatusPanel.cpp), "Send print job to" (SelectMachine.cpp: its checks,
//! messages and AMS mapping) and the send (Jobs/PrintJob.cpp), pause,
//! resume and stop.
//!
//! The access code is the Rust side's alone: no answer here holds it (a
//! test searches every one), and whatever the bridge says is cleared of it
//! (`Client::scrub`). Nothing is asked of the bridge until the page asks
//! for something that needs it; with no bridge found, every answer says so
//! and nothing is started.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};

use super::{Session, lock, needed};
use crate::api::i18n::tr;
use crate::ultranet::{
    client::{Client, Launcher, Session as Link, State, Timeouts, sequence},
    machine::{Machine, time_dhm},
    mapping::{self, Filament, Mapped},
    printers::{self, Kept, Model, Options, Printer},
    send::{self, Plan, Progress},
};

/// The calls answered here.
pub(super) const JSON: [&str; 14] = [
    "bambu_bridge",
    "bambu_printers",
    "bambu_printer_add",
    "bambu_printer_remove",
    "bambu_printer_code",
    "bambu_select",
    "bambu_connect",
    "bambu_state",
    "bambu_send_prepare",
    "bambu_send",
    "bambu_task",
    "bambu_task_cancel",
    "bambu_control",
    "bambu_options",
];

/// Sends under way.
type Task = (Arc<Mutex<Progress>>, Arc<AtomicBool>);

/// What the session keeps for Bambu Lab printers.
pub(super) struct Bambu {
    pub(super) client: Arc<Client>,
    pub(super) kept: Mutex<Kept>,
    /// The Bambu Lab account's sign-in (bambu_account.rs).
    pub(super) account: Mutex<super::bambu_account::AccountState>,
    tasks: Mutex<HashMap<u64, Task>>,
    next_task: AtomicU64,
}

impl Bambu {
    /// The printers kept in `settings`, their codes in `secrets` (not read
    /// until one is needed).
    pub(super) fn new(
        launcher: Launcher,
        settings: Option<PathBuf>,
        timeouts: Timeouts,
        secrets: Box<dyn crate::secrets::Secrets>,
    ) -> Self {
        let kept = Kept::open(settings.clone(), secrets);
        let client = Arc::new(Client::new(launcher, settings, timeouts));
        Bambu {
            client,
            kept: Mutex::new(kept),
            account: Mutex::default(),
            tasks: Mutex::new(HashMap::new()),
            next_task: AtomicU64::new(1),
        }
    }

    /// The bridge for this app: found where ULTRANET_BRIDGE.md section 2
    /// says, never started here. A test build finds none.
    pub(super) fn found(settings: Option<&std::path::Path>) -> Launcher {
        if cfg!(test) {
            return Launcher::Absent;
        }
        match crate::ultranet::link::find(settings) {
            Some(path) => Launcher::Program(path),
            None => Launcher::Absent,
        }
    }
}

/// `get_stage_string`'s "Printing" and the states the Device page shows.
fn state_said(machine: &Machine) -> String {
    match machine.print_status.as_str() {
        "IDLE" => tr("Idle"),
        "PREPARE" => tr("Downloading..."),
        "SLICING" => tr("Cloud Slicing..."),
        "RUNNING" => tr("Printing"),
        "PAUSE" => tr("Paused"),
        "FINISH" => tr("Finished"),
        "FAILED" => tr("Failed"),
        "" => String::new(),
        other => other.to_owned(),
    }
}

/// `format_steel_name`.
fn steel(kind: &str) -> String {
    match kind {
        "hardened_steel" => tr("Hardened Steel"),
        "stainless_steel" => tr("Stainless Steel"),
        _ => tr("Unknown"),
    }
}

/// What "Send" asks before a plate goes to a printer. Nothing is sent
/// without one question answered: the reference's warning ("Please check
/// the following:", `on_ok_btn`) when it has lines, otherwise "Print this
/// plate on %s (%s)?" (the owner's decision: not both, the warning already
/// asks). `confirmed` is the `answer` of the question the person accepted
/// (`"warning"` or `"printer"`): a warning that appears after only the
/// plain question was answered is still shown.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SendAsk {
    /// The warning with a line that blocks: nothing to confirm.
    Blocked,
    /// "Please check the following:", then "Confirm".
    Warning,
    /// "Print this plate on %s (%s)?", then "Send".
    Printer,
    /// Asked and answered: sent.
    Send,
}

pub(super) fn send_ask(warned: bool, blocking: bool, confirmed: Option<&str>) -> SendAsk {
    match (blocking, warned, confirmed) {
        (true, _, _) => SendAsk::Blocked,
        (false, true, Some("warning")) => SendAsk::Send,
        (false, true, _) => SendAsk::Warning,
        // A warning accepted that has since gone (the tray now says what it
        // holds) was a question answered all the same.
        (false, false, Some("warning" | "printer")) => SendAsk::Send,
        (false, false, _) => SendAsk::Printer,
    }
}

/// "Print this plate on <name> (<address>)?": each `%s` once, in order,
/// whatever the name holds.
fn printer_question(name: &str, ip: &str) -> String {
    let template = tr("Print this plate on %s (%s)?");
    let mut parts = template.splitn(3, "%s");
    let (a, b, c) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    format!("{a}{name}{b}{ip}{c}")
}

/// `nozzle_info.json`'s `nozzle_hrc`.
fn nozzle_hrc(kind: &str) -> i64 {
    match kind {
        "hardened_steel" => 55,
        "stainless_steel" => 20,
        "brass" => 2,
        _ => 0,
    }
}

/// The bed as the G-code names it (`bed_type_to_gcode_string`).
fn bed_gcode(bed: &str) -> &'static str {
    match bed {
        "Supertack Plate" => "supertack_plate",
        "Cool Plate" => "cool_plate",
        "Textured Cool Plate" => "textured_cool_plate",
        "Engineering Plate" => "eng_plate",
        "High Temp Plate" => "hot_plate",
        "Textured PEI Plate" => "textured_plate",
        _ => "auto",
    }
}

/// The dialog's state: `PrintDialogStatus`, its message, whether it is a
/// warning, whether "Send" is on.
struct Status {
    key: &'static str,
    message: String,
    warning: bool,
    can_send: bool,
}

impl Status {
    fn new(key: &'static str, message: String, warning: bool, can_send: bool) -> Self {
        Status {
            key,
            message,
            warning,
            can_send,
        }
    }
}

/// The plate as the dialog reads it.
struct PlateInfo {
    filaments: Vec<Filament>,
    /// The filament presets' ids, every project filament.
    filament_ids: Vec<String>,
    project_filaments: usize,
    /// `required_nozzle_HRC` of each used filament: (type, HRC).
    hrc: Vec<(String, i64)>,
    bed: String,
    /// The printer preset's model id ("C12").
    model_id: Option<String>,
    nozzle_diameter: Option<f64>,
    plate: u32,
}

impl Session {
    pub(super) fn bambu_lan_json(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "bambu_bridge" => {
                if args.get("retry").and_then(Value::as_bool) == Some(true) {
                    self.bambu.client.retry();
                }
                if args.get("start").and_then(Value::as_bool) == Some(true) {
                    let _ = self.bambu.client.start();
                }
                Ok(self.bridge_json())
            }
            "bambu_printers" => self.bambu_printers(),
            "bambu_printer_add" => self.bambu_printer_add(args),
            "bambu_printer_remove" => {
                let dev_id = needed(args, "dev_id")?;
                lock(&self.bambu.kept).remove(dev_id)?;
                self.bambu.client.forget(dev_id);
                self.bambu.client.set_codes(lock(&self.bambu.kept).codes());
                self.bambu_printers()
            }
            "bambu_printer_code" => self.bambu_printer_code(args),
            "bambu_select" => {
                let dev_id = needed(args, "dev_id")?;
                let mut kept = lock(&self.bambu.kept);
                if kept.get(dev_id).is_none() {
                    return Err("There is no such printer.".into());
                }
                kept.select(dev_id)?;
                drop(kept);
                self.bambu_printers()
            }
            "bambu_connect" => self.bambu_connect(args),
            "bambu_state" => self.bambu_state(args),
            "bambu_send_prepare" => self.bambu_send_prepare(args),
            "bambu_send" => self.bambu_send(args),
            "bambu_task" => {
                let task = args
                    .get("task")
                    .and_then(Value::as_u64)
                    .ok_or("task was not given")?;
                let tasks = lock(&self.bambu.tasks);
                let (progress, _) = tasks.get(&task).ok_or("There is no such send.")?;
                let p = lock(progress).clone();
                Ok(json!({
                    "task": task, "percent": p.percent, "message": p.message, "done": p.done, "ok": p.ok,
                    "error": p.error, "detail": p.detail, "code": p.code, "refused": p.refused,
                    "ask_code": p.ask_code,
                }))
            }
            "bambu_task_cancel" => {
                let task = args
                    .get("task")
                    .and_then(Value::as_u64)
                    .ok_or("task was not given")?;
                if let Some((_, cancel)) = lock(&self.bambu.tasks).get(&task) {
                    cancel.store(true, Ordering::SeqCst);
                }
                Ok(json!({ "task": task }))
            }
            "bambu_control" => self.bambu_control(args),
            "bambu_options" => {
                let flag = |key: &str| args.get(key).and_then(Value::as_bool);
                let mut kept = lock(&self.bambu.kept);
                let was = kept.options.clone();
                kept.set_options(Options {
                    bed_leveling: flag("bed_leveling").or(was.bed_leveling),
                    flow_cali: flag("flow_cali").or(was.flow_cali),
                    timelapse: flag("timelapse").or(was.timelapse),
                })?;
                Ok(json!({}))
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// `{ state, said, found, bridge_version, plugin_version }`.
    fn bridge_json(&self) -> Value {
        let client = &self.bambu.client;
        let state = client.state();
        let (key, said) = match &state {
            State::Absent => (
                "absent",
                format!(
                    "{} {}",
                    tr("Network plug-in missing"),
                    "No UltraNet bridge was found, so this app cannot print to a Bambu Lab printer over the local network. Nothing is sent to one until the bridge is installed."
                ),
            ),
            State::Idle => ("idle", String::new()),
            State::Running => ("running", String::new()),
            State::Refused(why) => ("refused", why.clone()),
            State::Failed(why) => ("failed", why.clone()),
            State::GaveUp(why) => ("gave_up", why.clone()),
        };
        let hello = client.hello().unwrap_or_default();
        json!({
            "state": key,
            "said": client.scrub(&said),
            "found": client.found().is_some(),
            "bridge_version": hello.bridge_version,
            "plugin_version": hello.plugin_version,
            "protocol": [hello.protocol, hello.minor],
        })
    }

    /// The printer models the reference knows (`resources/printers`).
    fn bambu_models(&self) -> Vec<Model> {
        let dir = lock(&self.state)
            .library
            .as_ref()
            .map(|library| library.dir().join("..").join("printers"));
        dir.map(|dir| printers::models(&dir)).unwrap_or_default()
    }

    /// `{ printers [{ dev_id, name, ip, model, model_name, has_access_code,
    /// selected }], selected, models [{ id, name }], problem }`.
    fn bambu_printers(&self) -> Result<Value, String> {
        let models = self.bambu_models();
        // The account (section 16): who, and its printers without codes.
        let account = self.account_status();
        let account_printers = self.account_printers_json(&models);
        let kept = lock(&self.bambu.kept);
        let list: Vec<Value> = kept
            .printers
            .iter()
            .map(|p| {
                json!({
                    "dev_id": p.dev_id,
                    "name": p.name,
                    "ip": p.ip,
                    "model": p.model,
                    "model_name": models.iter().find(|m| m.id == p.model).map(|m| m.name.clone()),
                    "has_access_code": kept.has_code(&p.dev_id),
                    "selected": kept.selected.as_deref() == Some(p.dev_id.as_str()),
                    "in_account": kept.account_printer(&p.dev_id).is_some(),
                })
            })
            .collect();
        Ok(json!({
            "printers": list,
            "selected": kept.selected,
            "models": models.iter().map(|m| json!({ "id": m.id, "name": m.name })).collect::<Vec<_>>(),
            "problem": kept.problem,
            // Where the codes are kept, said when it is not the system's
            // store as it should be; and whether a code can be had without
            // asking the system's store (which may ask the person), so the
            // page dials a printer by itself only then.
            "secrets": {
                "notice": kept.notice,
                "at_hand": kept.codes_at_hand(),
                "kept_in": match kept.store() {
                    crate::secrets::Kind::System | crate::secrets::Kind::Unavailable => "system",
                    crate::secrets::Kind::File => "file",
                    crate::secrets::Kind::Memory => "memory",
                },
            },
            "bridge": self.bridge_json(),
            "account": account,
            "account_printers": account_printers,
        }))
    }

    /// "Connect the printer using IP and access code", "Manual Setup": the
    /// address, the access code, the serial, the model and a name, checked
    /// as the dialog checks them (`on_text`: a well-formed IP, eight
    /// letters or digits, fifteen characters of serial, a name), the serial
    /// put in upper case (`on_ok`). Kept and selected.
    fn bambu_printer_add(&self, args: &Value) -> Result<Value, String> {
        let field = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned()
        };
        let (ip, code, name, serial, model) = (
            field("ip"),
            args.get("access_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            field("name"),
            field("serial").to_uppercase(),
            field("model"),
        );
        if !printers::is_ip(&ip) {
            return Err("The IP address is not valid.".into());
        }
        // A printer of the account may be added with its address alone: the
        // code the account gave is used (section 16.5); one typed wins.
        let account_code = code.is_empty()
            && lock(&self.bambu.kept)
                .account_printer(&serial)
                .is_some_and(|p| p.has_code);
        if !account_code && !printers::is_access_code(&code) {
            return Err("The access code is eight letters or digits.".into());
        }
        if serial.chars().count() != 15 {
            return Err("The serial number is fifteen characters.".into());
        }
        if name.is_empty() {
            return Err(tr("The name is not allowed to be empty."));
        }
        if !model.is_empty() && !self.bambu_models().iter().any(|m| m.id == model) {
            return Err("That printer model is not known.".into());
        }
        {
            let mut kept = lock(&self.bambu.kept);
            kept.put(
                Printer {
                    dev_id: serial.clone(),
                    name,
                    ip,
                    model,
                },
                (!account_code).then_some(code),
            )?;
            kept.select(&serial)?;
            self.bambu.client.set_codes(kept.codes());
        }
        self.bambu.client.forget(&serial);
        self.bambu_printers()
    }

    /// "The printer refused the access code. Re-enter it": a new code for
    /// a printer added.
    fn bambu_printer_code(&self, args: &Value) -> Result<Value, String> {
        let dev_id = needed(args, "dev_id")?;
        let code = needed(args, "access_code")?;
        if !printers::is_access_code(code) {
            return Err("The access code is eight letters or digits.".into());
        }
        let mut kept = lock(&self.bambu.kept);
        let printer = kept
            .get(dev_id)
            .cloned()
            .ok_or("There is no such printer.")?;
        kept.put(printer, Some(code.to_owned()))?;
        self.bambu.client.set_codes(kept.codes());
        drop(kept);
        self.bambu.client.forget(dev_id);
        self.bambu_printers()
    }

    /// The printer named, else the one selected.
    pub(super) fn bambu_printer(&self, args: &Value) -> Result<Printer, String> {
        let kept = lock(&self.bambu.kept);
        let dev_id = args
            .get("dev_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| kept.selected.clone())
            .ok_or_else(|| tr("No available printers"))?;
        kept.get(&dev_id)
            .cloned()
            .ok_or_else(|| "There is no such printer.".into())
    }

    /// A printer's access code, read from where codes are kept the first
    /// time one is needed (which may make the system ask the person), and
    /// handed to the bridge's list of what it must never say.
    pub(super) fn bambu_code(&self, dev_id: &str) -> Result<Option<String>, String> {
        let mut kept = lock(&self.bambu.kept);
        let code = kept.code(dev_id)?;
        self.bambu.client.set_codes(kept.codes());
        Ok(code)
    }

    /// Dials the printer (`MachineObject::connect`), starting the bridge if
    /// need be; answers as `bambu_state`.
    fn bambu_connect(&self, args: &Value) -> Result<Value, String> {
        let printer = self.bambu_printer(args)?;
        let Some(code) = self.bambu_code(&printer.dev_id)? else {
            return Err("Not sent: no access code has been entered for this printer.".into());
        };
        // Started here: a bridge that is absent or refused is said, nothing tried.
        if let Err(why) = self.bambu.client.start() {
            return Ok(
                json!({ "dev_id": printer.dev_id, "bridge": self.bridge_json(), "error": why }),
            );
        }
        let _ = self
            .bambu
            .client
            .connect(&printer.dev_id, &printer.ip, &code);
        // "Incorrect password": the code is forgotten, to be entered again
        // (GUI_App.cpp l.2690, `erase_user_access_code`).
        if self.bambu.client.remote(&printer.dev_id).refused {
            let mut kept = lock(&self.bambu.kept);
            kept.forget_code(&printer.dev_id)?;
            self.bambu.client.set_codes(kept.codes());
        }
        self.bambu_state(&json!({ "dev_id": printer.dev_id }))
    }

    /// The printer for Device: `{ dev_id, name, bridge, session (idle,
    /// connecting, connected, failed, lost), refused, said, machine
    /// { state, said, file, percent, left, left_text, layer, total_layers,
    /// nozzle, bed, chamber, sdcard, secured, ams [...] } }`. Asks nothing
    /// of the bridge, but re-dials a session that was lost (the reference's
    /// reconnect, `LanReconnectLadder`) when `redial` is given.
    fn bambu_state(&self, args: &Value) -> Result<Value, String> {
        let printer = self.bambu_printer(args)?;
        let client = &self.bambu.client;
        let remote = client.remote(&printer.dev_id);
        // A session lost was dialled with the code, which is at hand.
        if args.get("redial").and_then(Value::as_bool) == Some(true)
            && remote.session == Link::Lost
            && remote.since.elapsed() > Duration::from_secs(10)
            && client.state() == State::Running
            && let Some(code) = self.bambu_code(&printer.dev_id)?
        {
            client.forget(&printer.dev_id);
            let _ = client.connect(&printer.dev_id, &printer.ip, &code);
        }
        let remote = client.remote(&printer.dev_id);
        let m = &remote.machine;
        let (session, said) = match &remote.session {
            Link::Idle => ("idle", String::new()),
            Link::Connecting => ("connecting", tr("Synchronizing device information")),
            Link::Connected => ("connected", String::new()),
            Link::Failed(msg) if remote.refused => ("failed", {
                let _ = msg;
                tr("Incorrect password")
            }),
            Link::Failed(msg) if msg == "Synchronizing device information time out" => {
                ("failed", tr("Synchronizing device information time out"))
            }
            Link::Failed(msg) => (
                "failed",
                tr("Connect %s failed! [SN:%s, code=%s]")
                    .replacen("%s", &printer.name, 1)
                    .replacen("%s", &printer.dev_id, 1)
                    .replacen("%s", &client.scrub(msg), 1),
            ),
            Link::Lost => ("lost", tr("Offline")),
        };
        let ams: Vec<Value> = m
            .ams
            .iter()
            .map(|ams| {
                json!({
                    "id": ams.id,
                    "trays": ams.trays.iter().map(|tray| {
                        let id = ams.id.parse::<i64>().unwrap_or(0) * 4 + tray.id.parse::<i64>().unwrap_or(0);
                        json!({
                            "tray": id, "name": mapping::tray_name(id), "type": tray.display_type(),
                            "color": tray.color, "exists": tray.exists, "remain": tray.remain,
                            "active": m.tray_now == id.to_string(),
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        let percent = m.printing() || m.print_status == "FINISH";
        Ok(json!({
            "dev_id": printer.dev_id,
            "name": printer.name,
            "ip": printer.ip,
            "bridge": self.bridge_json(),
            "session": session,
            "refused": remote.refused,
            "said": said,
            "machine": {
                "ready": m.ready(),
                "state": m.print_status,
                "said": state_said(m),
                "file": client.scrub(if m.subtask.is_empty() { &m.gcode_file } else { &m.subtask }),
                "percent": if percent { m.percent } else { None },
                "left": m.left,
                "left_text": m.left.filter(|_| percent).map(time_dhm),
                "layer": m.layer,
                "total_layers": m.total_layers,
                "nozzle": m.nozzle.map(|(t, target)| json!([t, target])),
                "bed": m.bed.map(|(t, target)| json!([t, target])),
                "chamber": m.chamber,
                "sdcard": m.sdcard,
                "secured": m.secured,
                "can_pause": m.print_status == "RUNNING",
                "can_resume": m.paused(),
                "can_stop": m.print_status == "RUNNING" || m.paused(),
                "ams": ams,
            },
        }))
    }

    /// The plate as "Send print job to" reads it.
    fn bambu_plate(&self, args: &Value) -> Result<PlateInfo, String> {
        let (done, plate) = self.done(args)?;
        done.gcode
            .as_ref()
            .map_err(|_| tr("Cannot send the print job for empty plate"))?;
        let used = self.plate_filaments(args, plate)?;
        let settings = &done.sliced.settings;
        let texts = |key: &str| {
            settings
                .texts(key)
                .map(<[String]>::to_vec)
                .unwrap_or_default()
        };
        let (colours, kinds, ids, vendors) = (
            texts("filament_colour"),
            texts("filament_type"),
            texts("filament_ids"),
            texts("filament_vendor"),
        );
        let hrc_of = texts("required_nozzle_HRC");
        let filaments: Vec<Filament> = used
            .iter()
            .map(|&index| Filament {
                id: index,
                color: colours.get(index).cloned().unwrap_or_default(),
                kind: kinds.get(index).cloned().unwrap_or_default(),
                filament_id: ids.get(index).cloned().unwrap_or_default(),
                brand: vendors.get(index).cloned().unwrap_or_default(),
            })
            .collect();
        let hrc = used
            .iter()
            .map(|&index| {
                (
                    kinds.get(index).cloned().unwrap_or_default(),
                    hrc_of
                        .get(index)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0),
                )
            })
            .collect();
        let model_id = {
            let state = lock(&self.state);
            state.library.as_ref().and_then(|library| {
                let model = settings.text("printer_model")?;
                library.model_id(model.trim())
            })
        };
        Ok(PlateInfo {
            project_filaments: colours
                .len()
                .max(kinds.len())
                .max(used.iter().map(|f| f + 1).max().unwrap_or(0)),
            filaments,
            filament_ids: ids,
            hrc,
            bed: settings
                .text("curr_bed_type")
                .unwrap_or_default()
                .trim()
                .to_owned(),
            model_id,
            nozzle_diameter: settings
                .texts("nozzle_diameter")
                .and_then(|all| all.first().and_then(|d| d.trim().parse().ok())),
            plate: plate.unwrap_or(1),
        })
    }

    /// The mapping shown: the one picked (a tray for each filament, -1
    /// none), else the reference's auto-match. Its validity.
    fn bambu_mapping(
        &self,
        args: &Value,
        machine: &Machine,
        plate: &PlateInfo,
    ) -> (Vec<Mapped>, bool) {
        match args.get("mapping").and_then(Value::as_array) {
            Some(picked) => {
                let mut result: Vec<Mapped> = plate
                    .filaments
                    .iter()
                    .enumerate()
                    .map(|(at, f)| {
                        mapping::picked(
                            machine,
                            f.id,
                            picked.get(at).and_then(Value::as_i64).unwrap_or(-1),
                        )
                    })
                    .collect();
                let valid = mapping::is_valid(machine, &mut result, true);
                (result, valid)
            }
            None => {
                let (_, mut result) = mapping::auto_map(machine, &plate.filaments);
                let valid = mapping::is_valid(machine, &mut result, false);
                (result, valid)
            }
        }
    }

    /// `update_show_status`, for a printer in LAN mode with one extruder.
    fn bambu_status(
        &self,
        printer: &Printer,
        plate: Option<&PlateInfo>,
        use_ams: bool,
        timelapse: bool,
        valid: bool,
    ) -> Status {
        let client = &self.bambu.client;
        let plate = match plate {
            Some(plate) => plate,
            None => {
                return Status::new(
                    "blank_plate",
                    tr("Cannot send the print job for empty plate"),
                    true,
                    false,
                );
            }
        };
        match client.state() {
            State::Running => {}
            State::Idle => {
                return Status::new(
                    "reading",
                    tr("Synchronizing device information"),
                    false,
                    false,
                );
            }
            _ => {
                return Status::new(
                    "bridge",
                    self.bridge_json()["said"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    true,
                    false,
                );
            }
        }
        let remote = client.remote(&printer.dev_id);
        match &remote.session {
            Link::Connected => {}
            Link::Failed(_) if remote.refused => {
                return Status::new(
                    "refused",
                    tr("The printer refused the access code. Re-enter it"),
                    true,
                    false,
                );
            }
            Link::Failed(_) | Link::Lost => {
                return Status::new(
                    "timeout",
                    tr("Synchronizing device information time out"),
                    true,
                    false,
                );
            }
            Link::Idle | Link::Connecting => {
                return Status::new(
                    "reading",
                    tr("Synchronizing device information"),
                    false,
                    false,
                );
            }
        }
        let m = &remote.machine;
        if !m.ready() {
            return Status::new(
                "reading",
                tr("Synchronizing device information"),
                false,
                false,
            );
        }
        // `is_blocking_printing`: the file's model against the printer's.
        let models = self.bambu_models();
        if let Some(profile) = &plate.model_id
            && !printer.model.is_empty()
            && !printers::matches(profile, &printer.model, &models)
        {
            let name_of = |id: &str| {
                models
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| m.name.replace("Bambu Lab ", ""))
                    .unwrap_or_else(|| {
                        tr("%s, no printer definition in this build").replace("%s", id)
                    })
            };
            return Status::new(
                "unsupported_printer",
                tr("The selected printer (%s) is incompatible with the chosen printer profile in the slicer (%s).")
                    .replacen("%s", &name_of(&printer.model), 1)
                    .replacen("%s", &name_of(profile), 1),
                true,
                false,
            );
        }
        if m.printing() {
            return Status::new(
                "in_printing",
                tr("The printer is busy on other print job"),
                true,
                false,
            );
        }
        if !m.without_sd && m.sdcard == Some(false) && !m.emmc {
            return Status::new(
                "no_sdcard",
                tr("An SD card needs to be inserted before printing."),
                true,
                false,
            );
        }
        if m.sdcard == Some(false) && !m.emmc {
            return Status::new(
                "lan_no_sdcard",
                tr("An SD card needs to be inserted before printing via LAN."),
                true,
                true,
            );
        }
        // `has_tips`.
        let tips = timelapse && m.sdcard == Some(false);
        if !m.has_ams() || !use_ams {
            return if tips {
                Status::new(
                    "timelapse_no_sdcard",
                    tr("An SD card needs to be inserted to record timelapse."),
                    true,
                    true,
                )
            } else {
                Status::new("ready", String::new(), false, true)
            };
        }
        if valid {
            if tips {
                return Status::new(
                    "timelapse_no_sdcard",
                    tr("An SD card needs to be inserted to record timelapse."),
                    true,
                    true,
                );
            }
            return Status::new(
                "ams_mapping_success",
                tr(
                    "Filaments to AMS slots mappings have been established. You can click a filament above to change its mapping AMS slot",
                ),
                false,
                true,
            );
        }
        Status::new(
            "ams_mapping_invalid",
            tr(
                "Please click each filament above to specify its mapping AMS slot before sending the print job",
            ),
            true,
            false,
        )
    }

    /// "Send print job to": `{ printers, account_printers (the account's
    /// not added yet, as `bambu_printers` lists them), printer, ip, bridge, plate, bed, bed_gcode,
    /// filaments [{ id, color, type, tray, tray_name, tray_color, tray_type }],
    /// trays [{ tray, name, type, color, exists }], options { bed_leveling,
    /// flow_cali, timelapse, use_ams: { shown, value } }, status { key,
    /// message, warning, can_send }, refused }`. `use_ams` and `mapping` as
    /// the page has them; else the reference's first offer.
    fn bambu_send_prepare(&self, args: &Value) -> Result<Value, String> {
        self.bambu_project(args)?;
        let printers_list = self.bambu_printers()?;
        let printer = self.bambu_printer(args).ok();
        let plate = self.bambu_plate(args).ok();
        let options = lock(&self.bambu.kept).options.clone();
        let on = |o: Option<bool>| o.unwrap_or(true);
        // The account's printers not added yet (section 16.5), beside those
        // added (SelectMachine.cpp lists the account's machines with the LAN
        // ones): the cloud gives no address, so choosing one asks for it,
        // after which it is a printer added like any other. Never a code.
        let account_printers: Vec<Value> = printers_list["account_printers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["added"] != true)
            .cloned()
            .collect();
        let Some(printer) = printer else {
            return Ok(json!({
                "printers": printers_list["printers"], "account_printers": account_printers,
                "printer": null, "bridge": self.bridge_json(),
                "status": { "key": "invalid_printer", "message": tr("No available printers"), "warning": true, "can_send": false },
            }));
        };
        // Synchronized with the printer when the dialog opens.
        if args.get("connect").and_then(Value::as_bool) == Some(true)
            && self.bambu.client.remote(&printer.dev_id).session != Link::Connected
        {
            let _ = self.bambu_connect(&json!({ "dev_id": printer.dev_id }));
        }
        let machine = self.bambu.client.remote(&printer.dev_id).machine;
        let use_ams =
            machine.has_ams() && args.get("use_ams").and_then(Value::as_bool).unwrap_or(true);
        let timelapse = args
            .get("timelapse")
            .and_then(Value::as_bool)
            .unwrap_or(on(options.timelapse));
        let (mapped, valid) = match &plate {
            Some(plate) if use_ams => self.bambu_mapping(args, &machine, plate),
            _ => (Vec::new(), false),
        };
        let status = self.bambu_status(&printer, plate.as_ref(), use_ams, timelapse, valid);
        let filaments: Vec<Value> = plate
            .as_ref()
            .map(|plate| {
                plate
                    .filaments
                    .iter()
                    .enumerate()
                    .map(|(at, f)| {
                        let m = mapped.get(at);
                        json!({
                            "id": f.id, "color": f.color, "type": f.kind,
                            "tray": m.map_or(-1, |m| m.tray_id),
                            "tray_name": mapping::tray_name(m.map_or(-1, |m| m.tray_id)),
                            "tray_color": m.map(|m| m.color.clone()),
                            "tray_type": m.map(|m| m.kind.clone()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let trays: Vec<Value> = machine
            .ams
            .iter()
            .flat_map(|ams| {
                ams.trays.iter().map(move |tray| {
                    let id = ams.id.parse::<i64>().unwrap_or(0) * 4 + tray.id.parse::<i64>().unwrap_or(0);
                    json!({ "tray": id, "name": mapping::tray_name(id), "type": tray.display_type(), "color": tray.color, "exists": tray.exists })
                })
            })
            .collect();
        let bed = plate.as_ref().map(|p| p.bed.clone()).unwrap_or_default();
        Ok(json!({
            "printers": printers_list["printers"],
            "account_printers": account_printers,
            "printer": printer.dev_id,
            "ip": printer.ip,
            "bridge": self.bridge_json(),
            "plate": plate.as_ref().map(|p| p.plate),
            "bed": bed,
            "bed_gcode": bed_gcode(&bed),
            "filaments": filaments,
            "trays": trays,
            "options": {
                "bed_leveling": { "shown": true, "value": on(options.bed_leveling) },
                "flow_cali": { "shown": true, "value": on(options.flow_cali) },
                "timelapse": { "shown": true, "value": timelapse },
                "use_ams": { "shown": machine.has_ams(), "value": use_ams },
            },
            "status": { "key": status.key, "message": status.message, "warning": status.warning, "can_send": status.can_send },
            "refused": self.bambu.client.remote(&printer.dev_id).refused,
        }))
    }

    /// The project's printer is Bambu Lab's.
    fn bambu_project(&self, args: &Value) -> Result<(), String> {
        let bambu = {
            let state = lock(&self.state);
            let project = &state.projects[state.target(args)?];
            super::export::is_bambu(&project.choices, project.file.as_deref())
        };
        if bambu {
            Ok(())
        } else {
            Err("This is for a Bambu Lab printer.".into())
        }
    }

    /// "Send": the checks of `on_ok_btn`, then one question (`{ confirm {
    /// title, lines, blocking, ok, answer } }`, `send_ask`) until
    /// `confirmed` is its `answer`, or at once a blocking one; then the
    /// sliced file written and the send started: `{ task }`. Never sent
    /// without a question answered.
    fn bambu_send(&self, args: &Value) -> Result<Value, String> {
        self.bambu_project(args)?;
        let printer = self.bambu_printer(args)?;
        let code = self
            .bambu_code(&printer.dev_id)?
            .ok_or("Not sent: no access code has been entered for this printer.")?;
        let plate = self.bambu_plate(args)?;
        let machine = self.bambu.client.remote(&printer.dev_id).machine;
        let flag = |key: &str, default: bool| args["options"][key].as_bool().unwrap_or(default);
        let options = lock(&self.bambu.kept).options.clone();
        let use_ams = machine.has_ams() && flag("use_ams", true);
        let (bed_leveling, flow_cali, timelapse) = (
            flag("bed_leveling", options.bed_leveling.unwrap_or(true)),
            flag("flow_cali", options.flow_cali.unwrap_or(true)),
            flag("timelapse", options.timelapse.unwrap_or(true)),
        );
        let (mapped, valid) = if use_ams {
            self.bambu_mapping(args, &machine, &plate)
        } else {
            (Vec::new(), false)
        };
        let status = self.bambu_status(&printer, Some(&plate), use_ams, timelapse, valid);
        if !status.can_send {
            return Err(status.message);
        }
        // `on_ok_btn`.
        let mut lines: Vec<String> = Vec::new();
        let mut blocking = false;
        if let Some(profile) = &plate.model_id
            && !printer.model.is_empty()
            && profile != &printer.model
        {
            // Orca: a P1P and a P1S are the same here.
            let p1 = |id: &str| id == "C11" || id == "C12";
            if !(p1(profile) && p1(&printer.model)) {
                lines.push(tr("The printer type selected when generating G-code is not consistent with the currently selected printer. It is recommended that you use the same printer type for slicing."));
            }
        }
        let blacklist = {
            let dir = lock(&self.state).library.as_ref().map(|l| {
                l.dir()
                    .join("..")
                    .join("printers")
                    .join("filaments_blacklist.json")
            });
            dir.and_then(|file| std::fs::read_to_string(file).ok())
                .map(|text| mapping::read_blacklist(&text))
                .unwrap_or_default()
        };
        let mut prohibited: Option<String> = None;
        let mut unknown = false;
        for (at, m) in mapped.iter().enumerate() {
            let brand = plate
                .filaments
                .get(at)
                .map(|f| f.brand.as_str())
                .unwrap_or_default();
            if let Some(entry) = mapping::blacklisted(&blacklist, brand, &m.kind.to_uppercase()) {
                if entry.action == "warning" {
                    lines.push(tr(entry.said));
                } else if entry.action == "prohibition" {
                    prohibited = Some(tr(entry.said));
                }
            }
            // A tray that does not say what it holds (`TrayType::THIRD`, `EMPTY`).
            if m.tray_id >= 0
                && machine
                    .tray(m.tray_id)
                    .is_none_or(|tray| !tray.ready() || !tray.exists)
            {
                unknown = true;
            }
        }
        if let Some(prohibited) = prohibited
            && use_ams
        {
            // `show_errors`: "Errors", the one sentence, nothing to confirm.
            return Ok(
                json!({ "confirm": { "title": tr("Errors"), "lines": [prohibited], "blocking": true } }),
            );
        }
        if unknown {
            lines.push(tr("There are some unknown filaments in the AMS mappings. Please check whether they are the required filaments. If they are okay, press \"Confirm\" to start printing."));
        }
        if let Some(kind) = machine
            .nozzle_type
            .as_deref()
            .filter(|k| !k.is_empty() && *k != "undefine")
        {
            if let (Some(sliced), Some(memorized)) =
                (plate.nozzle_diameter, machine.nozzle_diameter)
                && memorized > 0.0
                && (sliced - memorized).abs() > 1e-6
            {
                lines.push(format!(
                    "{}\n    {}\n    {}\n",
                    tr("Your nozzle diameter in sliced file is not consistent with memorized nozzle. If you changed your nozzle lately, please go to Device > Printer Parts to change settings."),
                    tr("nozzle in preset: %.1f %s").replace("%.1f", &format!("{sliced:.1}")).replace(" %s", ""),
                    tr("nozzle memorized: %.1f %s").replace("%.1f", &format!("{memorized:.1}")).replace(" %s", ""),
                ));
            }
            let printer_hrc = nozzle_hrc(kind);
            if let Some((material, _)) = plate
                .hrc
                .iter()
                .find(|(_, hrc)| hrc.abs() > printer_hrc.abs())
            {
                blocking = true;
                lines.push(
                    tr("Printing high temperature material (%s material) with %s may cause nozzle damage")
                        .replacen("%s", material, 1)
                        .replacen("%s", &steel(kind), 1),
                );
            }
        }
        let ask = send_ask(
            !lines.is_empty(),
            blocking,
            args.get("confirmed").and_then(Value::as_str),
        );
        if ask == SendAsk::Printer {
            return Ok(json!({ "confirm": {
                "title": tr("Send print job to"),
                "lines": [printer_question(&printer.name, &printer.ip)],
                "blocking": false,
                "ok": tr("Send"),
                "answer": "printer",
            } }));
        }
        if ask != SendAsk::Send {
            let mut shown = vec![tr("Please check the following:")];
            shown.extend(
                lines
                    .iter()
                    .enumerate()
                    .map(|(at, line)| format!("{}. {line}", at + 1)),
            );
            shown.push(if blocking {
                tr("Please fix the error above, otherwise printing cannot continue.")
            } else {
                tr("Please click the confirm button if you still want to proceed with printing.")
            });
            return Ok(json!({ "confirm": {
                "title": tr("Warning"),
                "lines": shown,
                "blocking": blocking,
                "ok": tr("Confirm"),
                "answer": "warning",
            } }));
        }
        // The options are remembered (`app_config` "print").
        lock(&self.bambu.kept).set_options(Options {
            bed_leveling: Some(bed_leveling),
            flow_cali: Some(flow_cali),
            timelapse: Some(timelapse),
        })?;
        // The sliced file, as "Export plate sliced file" writes it.
        let task = self.bambu.next_task.fetch_add(1, Ordering::SeqCst);
        // A folder of its own for each send of this process.
        static FOLDERS: AtomicU64 = AtomicU64::new(1);
        let folder = std::env::temp_dir()
            .join("EdgeSlicerRS")
            .join("ultranet")
            .join(format!(
                "{}-{}",
                std::process::id(),
                FOLDERS.fetch_add(1, Ordering::SeqCst)
            ));
        std::fs::create_dir_all(&folder)
            .map_err(|_| tr("Abnormal print file data. Please slice again"))?;
        let ready = self.sliced_project(args, false)?;
        let project_name: String = {
            let (file, name, _) = ready.naming();
            let stem = super::export::suggested_sliced(file, name, None, false, false);
            stem.trim_end_matches(".gcode.3mf")
                .chars()
                .take(100)
                .collect()
        };
        let path = folder.join(format!("{project_name}.gcode.3mf"));
        let saving = self.saving(&ready.choices, Some(&ready.file))?;
        ready
            .write(&path, saving, false, super::export::pictures(args)?)
            .map_err(|_| tr("Abnormal print file data. Please slice again"))?;
        let verify = folder.join("check_access_code.txt");
        std::fs::write(&verify, "just a test file")
            .map_err(|_| tr("Abnormal print file data. Please slice again"))?;
        // `get_ams_mapping_result`; with "Enable AMS" off, the first
        // filament's colour and type only (`on_send_print`).
        let (ams_mapping, ams_mapping2, ams_mapping_info) = if use_ams {
            mapping::compose(
                &mapped,
                &plate.filaments,
                plate.project_filaments,
                &plate.filament_ids,
            )
            .map(|(a, b, c, _)| (a, b, c))
            .unwrap_or_default()
        } else {
            let info = plate.filaments.first().map(|f| {
                json!([{ "sourceColor": f.color.trim_start_matches('#').chars().take(8).collect::<String>(), "filamentType": f.kind }]).to_string()
            });
            (String::new(), String::new(), info.unwrap_or_default())
        };
        let models = self.bambu_models();
        let ftp_folder = models
            .iter()
            .find(|m| m.id == printer.model)
            .map(|m| m.ftp_folder.clone())
            .unwrap_or_default();
        let params = json!({
            "dev_id": printer.dev_id,
            "task_name": "",
            "project_name": project_name,
            "preset_name": format!("{project_name}_plate_{}", plate.plate),
            "filename": path.to_string_lossy(),
            "config_filename": "",
            "plate_index": plate.plate,
            "ftp_folder": ftp_folder,
            "ftp_file": "", "ftp_file_md5": "",
            "ams_mapping": ams_mapping,
            "ams_mapping2": ams_mapping2,
            "ams_mapping_info": ams_mapping_info,
            "nozzles_info": "[]",
            "connection_type": "lan",
            "comments": "",
            "origin_profile_id": 0, "stl_design_id": 0, "origin_model_id": "",
            "print_type": "from_normal",
            "dst_file": "",
            "dev_name": printer.name,
            "use_ssl_for_ftp": true,
            "use_ssl_for_mqtt": true,
            "task_bed_leveling": bed_leveling,
            "task_flow_cali": flow_cali,
            "task_vibration_cali": false,
            "task_layer_inspect": true,
            "task_record_timelapse": timelapse,
            "task_use_ams": use_ams,
            "task_bed_type": bed_gcode(&plate.bed),
            "extra_options": "",
            "auto_bed_leveling": 0, "auto_flow_cali": 0, "auto_offset_cali": 0,
            "nozzle_mapping_request": "",
            "extruder_cali_manual_mode": 0,
        });
        // No card, and the printer prints from its internal storage: no
        // check (PrintJob.cpp l.217).
        let verify = (!(machine.sdcard == Some(false) && machine.emmc)).then_some(verify);
        let plan = Plan {
            dev_id: printer.dev_id.clone(),
            ip: printer.ip.clone(),
            code,
            params,
            verify,
            file: path,
        };
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.bambu.tasks).insert(task, (progress.clone(), cancel.clone()));
        let client = self.bambu.client.clone();
        let language = lock(&self.language).clone();
        std::thread::spawn(move || {
            crate::api::i18n::with(language, || {
                send::run(&client, plan, &progress, &cancel);
            });
            let _ = std::fs::remove_dir_all(folder);
        });
        Ok(json!({ "task": task }))
    }

    /// Pause, resume, stop (`command_task_pause`, `_resume`, `_abort`):
    /// stop only once confirmed ("Cancel print", "Are you sure you want to
    /// cancel this print?").
    fn bambu_control(&self, args: &Value) -> Result<Value, String> {
        let printer = self.bambu_printer(args)?;
        let action = needed(args, "action")?;
        let command = match action {
            "pause" => "pause",
            "resume" => "resume",
            "stop" => {
                if args.get("confirmed").and_then(Value::as_bool) != Some(true) {
                    return Ok(
                        json!({ "confirm": { "title": tr("Cancel print"), "lines": [tr("Are you sure you want to cancel this print?")], "blocking": false } }),
                    );
                }
                "stop"
            }
            other => return Err(format!("{other} is not pause, resume or stop")),
        };
        let remote = self.bambu.client.remote(&printer.dev_id);
        if remote.session != Link::Connected {
            return Err(tr("Offline"));
        }
        if remote.machine.secured == Some(true) {
            return Err(tr(
                "This printer is not in Developer Mode, so it rejects commands from EdgeSlicer. Turn on LAN Only + Developer Mode on the printer.",
            ));
        }
        let message =
            json!({ "print": { "command": command, "param": "", "sequence_id": sequence() } });
        let ret = self.bambu.client.publish(&printer.dev_id, &message, 1)?;
        if ret != 0 {
            return Err(format!("The printer was not sent the command ({ret})."));
        }
        Ok(json!({ "sent": command }))
    }
}

#[cfg(test)]
#[path = "bambu_lan_tests.rs"]
mod tests;
