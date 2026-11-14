//! The Snapmaker U1, over the Moonraker API it serves on the local
//! network.
//!
//! The U1's profiles give it the host type `octoprint`, but the fork never
//! sends to a U1 through OctoPrint: its Print button opens Snapmaker's own
//! bundled page (`WebPreprintDialog`, Plater.cpp `send_gcode_legacy`
//! l.27502, `PrintHostDevices::send_flow_for`), which uploads and starts
//! the print itself over MQTT or a websocket, in code that is not in the
//! source (a Flutter bundle). What the fork does in C++ for a U1 on the
//! local network is `GUI/SnapmakerLan.cpp`, the route its phone hub takes
//! (`RemoteSend.cpp` l.2390–2440), and that is what is ported here:
//!
//! - "Test": `GET /server/info` (an answer with `result`), and
//!   `GET /access/info` for whether it wants a login.
//! - "Upload": `POST /server/files/upload`, multipart `print=false`,
//!   `root=gcodes`, `file` (`SnapmakerLan::upload`: never `print=true`, so a
//!   print that does not start still leaves the file on the printer).
//! - "Upload and Print": first `GET /printer/objects/query?print_stats`,
//!   and a printer that is printing or paused is not sent the file
//!   (RemoteSend.cpp l.727, l.2394); then the upload; then
//!   `GET /server/files/metadata?filename=` to see the file is there whole
//!   (`file_on_printer`); then the toolhead map as the U1's own page sends
//!   it, each filament on its own toolhead ("As sliced",
//!   `SnapmakerLan::mapping_script`) with `END_UNLOAD_FILAMENT` when
//!   "Unload filaments after print" is ticked (SnapmakerTaskConfig.cpp),
//!   through `POST /printer/gcode/script?script=`; then
//!   `POST /printer/print/start?filename=`.
//! - "Storage": `GET /server/files/list?root=gcodes`,
//!   `DELETE /server/files/gcodes/<name>`, and "Print" as above
//!   (`Utils/PrinterStorage.cpp`).
//!
//! The U1 asks for no key (`/access/info` answers `login_required: false`,
//! SnapmakerLan.hpp l.18); one given is sent all the same, as `X-Api-Key`,
//! for a Moonraker that does want it. The reference sends none on this
//! route (PrintHost.cpp `script_target`), so a key-protected Moonraker
//! refuses it.

use std::{sync::atomic::Ordering, time::Duration};

use serde_json::Value;

use crate::api::i18n::tr;

use super::{
    Answer, Connection, Failure, Form, Host, Http, Progress, Status, Stored, Timeouts, Toolhead,
    Upload, http_error, moonraker,
};

/// Snapmaker's tool changers have four toolheads (SnapmakerTaskConfig.hpp
/// `TOOLHEAD_COUNT`).
pub const TOOLHEADS: usize = 4;

/// The fork's test for a U1: "Snapmaker" and "U1" both in the model's
/// name, in any case (PrintHostDevices.cpp `is_snapmaker_u1_model`).
pub fn is_u1(printer_model: &str) -> bool {
    let model = printer_model.to_lowercase();
    model.contains("snapmaker") && model.contains("u1")
}

/// What is said when a printer is busy, and nothing was sent (PrintHost.cpp
/// l.556, the reference's words for the same refusal to a QIDI printer).
pub const BUSY: &str = "The printer is busy with another print, so nothing was sent to it and this plate was not started. Send it again once that print has finished.";

pub struct SnapmakerLan {
    pub http: Http,
}

impl SnapmakerLan {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        Ok(Self {
            http: Http::new(connection, timeouts)?,
        })
    }

    /// A short GET answered with Moonraker's `result`.
    fn result(&self, path: &str) -> Result<Value, String> {
        moonraker::result(&self.http, path)
    }

    /// The printer's state (`print_stats.state`: standby, printing,
    /// paused, complete, cancelled, error).
    pub fn state(&self) -> Result<String, String> {
        let result = self.result("printer/objects/query?print_stats")?;
        Ok(result["status"]["print_stats"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// Refuses a printer that is printing or paused (SnapmakerLan.hpp
    /// `Status::printing`).
    fn idle(&self) -> Result<(), String> {
        match self.state()?.as_str() {
            "printing" | "paused" => Err(tr(BUSY)),
            _ => Ok(()),
        }
    }

    fn command(&self, path: &str, within: Duration) -> Result<(), String> {
        let answer = self.http.post_empty(path, within)?;
        if answer.ok() {
            Ok(())
        } else {
            Err(http_error(&answer))
        }
    }

    /// `printer.print.start` over HTTP.
    fn start(&self, name: &str) -> Result<(), String> {
        self.command(
            &format!("printer/print/start?filename={}", encode(name, false)),
            Duration::from_secs(30),
        )
        .map_err(|error| format!("the printer refused the print start: {error}"))
    }
}

/// The state of a Moonraker printer, with the facts of the file it is on
/// (SnapmakerLan.cpp `file_meta_for`).
pub fn moonraker_status(http: &Http) -> Result<Status, String> {
    let answer = http.get(&moonraker::status_query())?;
    if !answer.ok() {
        return Err(http_error(&answer));
    }
    let status = answer
        .json()
        .and_then(|json| json.get("result").and_then(|r| r.get("status")).cloned())
        .ok_or_else(|| "The printer's answer is not Moonraker's.".to_owned())?;
    // The reference keeps the facts it found while the job ran for after
    // it (SnapmakerLan.cpp `file_meta_for`); without that cache they are
    // asked for whenever the printer names a file.
    let file = moonraker::printing_file(&status);
    let metadata = if !file.is_empty() {
        http.get(&format!(
            "server/files/metadata?filename={}",
            encode(&file, false)
        ))
        .ok()
        .filter(Answer::ok)
        .and_then(|answer| answer.json())
        .and_then(|json| json.get("result").cloned())
    } else {
        None
    };
    Ok(moonraker::status_of(&status, metadata.as_ref()))
}

/// Percent-encodes `text` for a query (`/` encoded) or a path (`/` kept).
pub fn encode(text: &str, path: bool) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        let keep =
            byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) || (path && byte == b'/');
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The U1's own device page's "Print Preferences", as the macro takes
/// them (`SET_PRINT_PREFERENCES`; the bundled page builds the line from
/// its options `bed_level`, `flow_calibrate`, `time_lapse_camera`, each 0
/// until ticked, in that order).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Preferences {
    pub bed_level: bool,
    pub flow_calibrate: bool,
    pub time_lapse: bool,
}

/// Each filament the plate prints in on its own toolhead ("As sliced"):
/// the mapping the app sent before a toolhead map could be chosen, and
/// what a filament past the fourth has none in.
pub fn identity_mapping(filaments: &[usize], count: usize) -> Vec<i32> {
    let mut mapping = vec![-1; count.max(filaments.iter().map(|f| f + 1).max().unwrap_or(0))];
    for filament in filaments {
        if *filament < TOOLHEADS {
            mapping[*filament] = *filament as i32;
        }
    }
    mapping
}

/// The macros for one mapping, without the start (SnapmakerLan.cpp
/// `mapping_script`): `mapping[i]` the toolhead for the file's filament
/// `i`, -1 for one it does not use; the toolheads used listed once each
/// in the order the file first uses them; then the preferences, with
/// `END_UNLOAD_FILAMENT` when "Unload filaments after print" is ticked
/// (SnapmakerTaskConfig.cpp `end_unload_parameter`). The reference's own
/// LAN route always sends the three preferences as 0; its device page
/// sends what is ticked, and so does this.
pub fn mapping_script(mapping: &[i32], preferences: Preferences, unload_at_end: bool) -> String {
    let mut script = String::new();
    let mut used: Vec<i32> = Vec::new();
    for (filament, toolhead) in mapping.iter().enumerate() {
        if *toolhead < 0 {
            continue;
        }
        script +=
            &format!("SET_PRINT_EXTRUDER_MAP CONFIG_EXTRUDER={filament} MAP_EXTRUDER={toolhead}\n");
        if !used.contains(toolhead) {
            used.push(*toolhead);
        }
    }
    if script.is_empty() {
        return script;
    }
    let list: Vec<String> = used.iter().map(i32::to_string).collect();
    script += &format!("SET_PRINT_USED_EXTRUDERS EXTRUDERS={}\n", list.join(","));
    let flag = |on: bool| if on { 1 } else { 0 };
    script += &format!(
        "SET_PRINT_PREFERENCES BED_LEVEL={} FLOW_CALIBRATE={} TIME_LAPSE_CAMERA={}",
        flag(preferences.bed_level),
        flag(preferences.flow_calibrate),
        flag(preferences.time_lapse)
    );
    if unload_at_end {
        let flags: Vec<&str> = (0..TOOLHEADS as i32)
            .map(|toolhead| if used.contains(&toolhead) { "1" } else { "0" })
            .collect();
        script += &format!(" END_UNLOAD_FILAMENT=[{}]", flags.join(","));
    }
    script
}

/// One filament of the sliced file (SnapmakerLan.hpp `FileFilament`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileFilament {
    pub index: usize,
    /// `#RRGGBB`.
    pub color: String,
    pub filament_type: String,
    pub used: bool,
}

fn rgb_of(hex: &str) -> Option<[f64; 3]> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() < 6 || !hex.is_char_boundary(6) {
        return None;
    }
    let part = |i: usize| {
        u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .ok()
            .map(f64::from)
    };
    Some([part(0)?, part(1)?, part(2)?])
}

/// The "redmean" distance between two colours (SnapmakerLan.cpp
/// `color_distance`); far when either is not a colour.
fn color_distance(a: &str, b: &str) -> f64 {
    let (Some(a), Some(b)) = (rgb_of(a), rgb_of(b)) else {
        return 1e9;
    };
    let mean = (a[0] + b[0]) / 2.0;
    let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    ((2.0 + mean / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - mean) / 256.0) * db * db)
        .sqrt()
}

/// Which toolhead prints which filament, as the reference first offers it
/// (SnapmakerLan.cpp `auto_match`): each used filament the nearest free
/// loaded colour; a filament of exactly the same colour as one placed
/// shares its toolhead; anything left the first loaded toolhead (0 when
/// none is loaded). Type never decides; it only earns a warning.
pub fn auto_match(filaments: &[FileFilament], heads: &[Toolhead]) -> Vec<i32> {
    let mut out = vec![-1; filaments.len()];
    let mut taken = vec![false; heads.len()];
    for (i, filament) in filaments.iter().enumerate() {
        if !filament.used {
            continue;
        }
        let mut best = 1e9;
        let mut pick = None;
        for (h, head) in heads.iter().enumerate() {
            if !head.loaded || taken[h] {
                continue;
            }
            let distance = color_distance(&filament.color, &head.color);
            if distance < best {
                best = distance;
                pick = Some(h);
            }
        }
        if let Some(h) = pick {
            out[i] = h as i32;
            taken[h] = true;
        }
    }
    for i in 0..filaments.len() {
        if out[i] >= 0 || !filaments[i].used || filaments[i].color.is_empty() {
            continue;
        }
        let twin = (0..filaments.len()).find(|&j| {
            j != i && out[j] >= 0 && filaments[j].color.eq_ignore_ascii_case(&filaments[i].color)
        });
        if let Some(j) = twin {
            out[i] = out[j];
        }
    }
    let first_loaded = heads
        .iter()
        .position(|head| head.loaded)
        .map_or(0, |h| h as i32);
    for (i, filament) in filaments.iter().enumerate() {
        if out[i] < 0 && filament.used {
            out[i] = first_loaded;
        }
    }
    out
}

/// Why a mapping cannot be sent, as the reference refuses it before the
/// file goes up (RemoteSend.cpp `prepare_snapmaker` l.770–790): a used
/// filament without a toolhead, a toolhead the printer has not got, and,
/// unless `force`, a toolhead with nothing loaded.
pub fn mapping_problem(
    filaments: &[FileFilament],
    mapping: &[i32],
    heads: &[Toolhead],
    printer: &str,
    force: bool,
) -> Option<String> {
    for filament in filaments.iter().filter(|f| f.used) {
        let toolhead = mapping.get(filament.index).copied().unwrap_or(-1);
        if toolhead < 0 {
            return Some(format!(
                "Filament {} has no toolhead; every filament the file uses needs one.",
                filament.index + 1
            ));
        }
        let count = if heads.is_empty() {
            TOOLHEADS
        } else {
            heads.len()
        };
        if toolhead as usize >= count {
            return Some(format!(
                "{printer} has {count} toolheads, so there is no toolhead {}.",
                toolhead + 1
            ));
        }
    }
    if force {
        return None;
    }
    for filament in filaments.iter().filter(|f| f.used) {
        let toolhead = mapping[filament.index] as usize;
        if heads.get(toolhead).is_some_and(|head| !head.loaded) {
            return Some(format!(
                "Toolhead {} is empty: load it or pick another.",
                toolhead + 1
            ));
        }
    }
    None
}

/// What the person is told of a mapping without it being refused: a
/// filament type that is not the one loaded (the reference: "type never
/// decides - it only earns a warning").
pub fn mapping_warnings(
    filaments: &[FileFilament],
    mapping: &[i32],
    heads: &[Toolhead],
) -> Vec<String> {
    let mut out = Vec::new();
    for filament in filaments.iter().filter(|f| f.used) {
        let Some(head) = mapping
            .get(filament.index)
            .and_then(|t| usize::try_from(*t).ok())
            .and_then(|t| heads.get(t))
        else {
            continue;
        };
        if head.loaded
            && !head.filament_type.is_empty()
            && !filament.filament_type.is_empty()
            && !head
                .filament_type
                .eq_ignore_ascii_case(&filament.filament_type)
        {
            out.push(format!(
                "Filament {} is {} but toolhead {} holds {}.",
                filament.index + 1,
                filament.filament_type,
                head.index + 1,
                head.filament_type
            ));
        }
    }
    out
}

/// The name a file is sent under: its last part only, `.gcode` added when
/// it has not got it (RemoteSend.cpp l.743).
pub fn file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let last = if last.is_empty() || last == "." || last == ".." {
        "plate.gcode"
    } else {
        last
    };
    if last.to_lowercase().ends_with(".gcode") {
        last.to_owned()
    } else {
        format!("{last}.gcode")
    }
}

impl Host for SnapmakerLan {
    fn test(&self) -> Result<(), String> {
        self.result("server/info")?;
        // A printer that wants a login is one this app cannot reach yet.
        if let Ok(access) = self.result("access/info")
            && access["login_required"].as_bool() == Some(true)
        {
            return Err("The printer asks for a login, which this app cannot give yet.".into());
        }
        Ok(())
    }

    fn test_ok(&self) -> String {
        tr("Connection to Moonraker works correctly.")
    }

    fn test_failed(&self, reason: &str) -> String {
        // MoonRaker.cpp l.434, which adds the note on Moonraker's version.
        format!(
            "{}: {reason}\n\n{}",
            tr("Could not connect to Moonraker"),
            tr("Note: Moonraker version at least 1.1.0 is required.")
        )
    }

    fn upload(&self, upload: &Upload, progress: &Progress) -> Result<(), Failure> {
        let name = file_name(&upload.name);
        if upload.start {
            self.idle()?;
        }
        let total = upload.data.len() as u64;
        progress.total.store(total, Ordering::Relaxed);
        let form = Form::new()
            .field("print", "false")
            .field("root", "gcodes")
            .file("file", &name, upload.data.clone(), 0..upload.data.len());
        let answer = self
            .http
            .post_form("server/files/upload", &form, progress, (0, total))?;
        if !answer.ok() {
            return Err(Failure::Error(http_error(&answer)));
        }
        if !upload.start {
            return Ok(());
        }
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        progress.set_stage("Starting the print");
        let metadata = self
            .result(&format!(
                "server/files/metadata?filename={}",
                encode(&name, false)
            ))
            .map_err(|error| format!("The file is not on the printer: {error}"))?;
        if metadata["size"].as_u64().is_some_and(|size| size != total) {
            return Err(Failure::Error(
                "The file on the printer is not the file sent; the print was not started.".into(),
            ));
        }
        // Checked again just before the start: the file took a while.
        self.idle()?;
        let options = &upload.options;
        let mapping = if options.mapping.is_empty() {
            identity_mapping(&options.filaments, 0)
        } else {
            options.mapping.clone()
        };
        let preferences = Preferences {
            bed_level: options.bed_level,
            flow_calibrate: options.flow_calibrate,
            time_lapse: options.time_lapse,
        };
        let script = mapping_script(&mapping, preferences, options.unload_at_end);
        if !script.is_empty() {
            // The printer answers a script only once it has run it.
            self.command(
                &format!("printer/gcode/script?script={}", encode(&script, false)),
                Duration::from_secs(60),
            )
            .map_err(|error| format!("the printer refused the toolhead mapping: {error}"))?;
        }
        self.start(&name)?;
        Ok(())
    }

    fn files(&self) -> Option<Result<Vec<Stored>, String>> {
        Some(moonraker::list_files(&self.http))
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        moonraker::delete_file(&self.http, name)
    }

    fn toolheads(&self) -> Option<Result<Vec<Toolhead>, String>> {
        Some(
            self.result(&moonraker::toolheads_query())
                .map(|result| moonraker::toolheads_of(&result["status"])),
        )
    }

    fn status(&self) -> Option<Result<Status, String>> {
        Some(moonraker_status(&self.http))
    }

    /// A file already on the printer: refused while it is busy
    /// (PrinterStorage.cpp l.499), then started.
    fn print(&self, name: &str) -> Result<(), String> {
        self.idle()?;
        self.start(name)
    }
}
