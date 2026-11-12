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

use super::{
    Connection, Failure, Form, Host, Http, Progress, Stored, Timeouts, Upload, http_error,
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
        let answer = self.http.get(path)?;
        if !answer.ok() {
            return Err(http_error(&answer));
        }
        answer
            .json()
            .and_then(|json| json.get("result").cloned())
            .ok_or_else(|| "The printer's answer is not Moonraker's.".to_owned())
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
            "printing" | "paused" => Err(BUSY.into()),
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

/// The toolhead map the U1's page sends before a start: each filament the
/// plate prints in on its own toolhead, which toolheads are used, and the
/// preferences the page sends with it (SnapmakerLan.cpp `mapping_script`,
/// SnapmakerTaskConfig.cpp `end_unload_parameter`). Filaments past the
/// fourth have no toolhead and are left out.
pub fn mapping_script(filaments: &[usize], unload_at_end: bool) -> String {
    let mut used: Vec<usize> = filaments
        .iter()
        .copied()
        .filter(|filament| *filament < TOOLHEADS)
        .collect();
    used.sort_unstable();
    used.dedup();
    if used.is_empty() {
        return String::new();
    }
    let mut script = String::new();
    for filament in &used {
        script +=
            &format!("SET_PRINT_EXTRUDER_MAP CONFIG_EXTRUDER={filament} MAP_EXTRUDER={filament}\n");
    }
    let list: Vec<String> = used.iter().map(usize::to_string).collect();
    script += &format!("SET_PRINT_USED_EXTRUDERS EXTRUDERS={}\n", list.join(","));
    script += "SET_PRINT_PREFERENCES BED_LEVEL=0 FLOW_CALIBRATE=0 TIME_LAPSE_CAMERA=0";
    if unload_at_end {
        let flags: Vec<&str> = (0..TOOLHEADS)
            .map(|toolhead| if used.contains(&toolhead) { "1" } else { "0" })
            .collect();
        script += &format!(" END_UNLOAD_FILAMENT=[{}]", flags.join(","));
    }
    script
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
        "Connection to Moonraker works correctly.".into()
    }

    fn test_failed(&self, reason: &str) -> String {
        format!("Could not connect to Moonraker: {reason}")
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
        let script = mapping_script(&upload.options.filaments, upload.options.unload_at_end);
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
        Some(self.result("server/files/list?root=gcodes").map(|result| {
            let mut files: Vec<Stored> = result
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| {
                    Some(Stored {
                        name: file["path"].as_str()?.to_owned(),
                        size: file["size"].as_u64(),
                        modified: file["modified"].as_f64(),
                    })
                })
                .collect();
            // Newest first, as the printer's own list.
            files.sort_by(|a, b| {
                b.modified
                    .partial_cmp(&a.modified)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            files
        }))
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let answer = self
            .http
            .delete(&format!("server/files/gcodes/{}", encode(name, true)))?;
        if answer.ok() {
            Ok(())
        } else {
            Err(http_error(&answer))
        }
    }

    /// A file already on the printer: refused while it is busy
    /// (PrinterStorage.cpp l.499), then started.
    fn print(&self, name: &str) -> Result<(), String> {
        self.idle()?;
        self.start(name)
    }
}
