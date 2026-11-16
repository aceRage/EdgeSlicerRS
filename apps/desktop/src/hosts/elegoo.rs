//! "Elegoo Link": the Elegoo Centauri Carbon's own protocol, ported from
//! `Utils/ElegooLink.cpp` (the CC1 path: `elegoo_test`, `loopUpload`,
//! `uploadPart`, `checkResult`, `print`) and `Utils/ElegooStartPrint.hpp`
//! (`build_payload`).
//!
//! - "Test": OctoPrint's `api/version` first (an Elegoo printer with
//!   OctoPrint on it is sent to as OctoPrint, as the reference does); else
//!   `GET /` at the address typed, which must answer with "ELEGOO" in it.
//! - "Upload": `POST /uploadFile/upload`, the file in parts of 1 MiB, each
//!   a multipart form `Check=1`, `S-File-MD5` (of the whole file),
//!   `Offset`, `Uuid` (one for the whole file), `TotalSize`, `File`; each
//!   answered `{"code": "000000"}`.
//! - "Upload and Print": the printer's state is asked first over its
//!   websocket (`ws://<host>:3030/websocket`, SDCP command 0), and a busy
//!   printer is not sent the file; after the upload, the reference's wait
//!   while the printer checks the file (`CurrentStatus` 8), then command 128
//!   with the file, "Time-lapse", "Heated Bed Leveling" and the plate side,
//!   and its `Ack` read back.
//!
//! The Centauri Carbon has no key or password; one given is sent as
//! `X-Api-Key`, as the reference does. Nothing is found on the network by
//! itself: the reference has no discovery for it either
//! (`ElegooLink::has_auto_discovery` is false).
//!
//! Where this differs from the reference, on purpose:
//! - The reference does not ask whether the printer is busy before it
//!   sends the file (`checkResult` waits only on "file checking"), so a
//!   printing printer is sent all of it and then answers `Ack` 1. Here a
//!   busy printer is refused first, with the reference's words for that
//!   refusal to another printer.
//! - The reference's websocket has no read time-out
//!   (`WebSocketClient::receive` ignores its argument and blocks), so a
//!   printer that goes quiet leaves the job hanging; its 30 s and 60 s
//!   limits are only checked when a message arrives. Here every read is
//!   timed and the limits hold.
//! - The reference turns an `Ack` that arrives after 30 s into a failure
//!   even when it is 0 (ElegooLink.cpp l.909); here an `Ack` is read as it
//!   is, whenever it comes within the limit.
//! - The reference logs every websocket message whole (l.836, l.962),
//!   the printer's mainboard serial with it; nothing here is logged.

use std::{
    net::{TcpStream, ToSocketAddrs},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use md5::{Digest, Md5};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use super::{
    Connection, Failure, Form, Host, Http, Options, Progress, StartCheck, Stored, TIMED_OUT,
    Timeouts, UNPARSABLE, Upload, http_error, octoprint, sdcp_files, snapmaker::BUSY, uuid,
};
use crate::api::i18n::{fill, tr};

/// `MAX_UPLOAD_PACKAGE_LENGTH` (ElegooLink.cpp l.39).
pub const PART: usize = 1024 * 1024;

/// The port of the printer's websocket (ElegooLink.cpp l.602).
pub const WEBSOCKET_PORT: u16 = 3030;

/// The Centauri Carbon 2 is another protocol (`ElegooCc2`), not built:
/// the reference's test for it (ElegooLink.cpp `config_is_cc2`,
/// ElegooCc2.cpp `is_cc2_model`).
pub fn is_cc2(printer_model: &str) -> bool {
    let model = printer_model.to_lowercase();
    model.contains("centauri carbon 2") || model.contains("centauri 2")
}

pub struct ElegooLink {
    http: Http,
    /// Where the websocket is: the address's host, always port 3030
    /// (`get_host_from_url_no_port`).
    ws_host: String,
    cc2: bool,
    /// The Centauri Carbon 1, whose files "Storage" reaches
    /// (`sdcp_files::is_cc1`).
    cc1: bool,
    timeouts: Timeouts,
    /// For the tests: the websocket's port.
    pub ws_port: u16,
}

/// The tests' websocket port for every `ElegooLink` made while it is not
/// 0 (the session makes its own); a test that sets it holds `TEST_LOCK`.
#[cfg(test)]
pub(crate) static TEST_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl ElegooLink {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        let http = Http::new(connection, timeouts)?;
        Ok(Self {
            ws_host: http.address.host.clone(),
            http,
            cc2: is_cc2(&connection.printer_model),
            cc1: sdcp_files::is_cc1(&connection.printer_model),
            timeouts,
            #[cfg(not(test))]
            ws_port: WEBSOCKET_PORT,
            #[cfg(test)]
            ws_port: match TEST_PORT.load(Ordering::SeqCst) {
                0 => WEBSOCKET_PORT,
                port => port,
            },
        })
    }

    fn not_cc2(&self) -> Result<(), String> {
        if self.cc2 {
            Err("This app cannot send to an Elegoo Centauri Carbon 2 yet.".into())
        } else {
            Ok(())
        }
    }

    /// `elegoo_test`: the page at the address names Elegoo.
    fn elegoo_test(&self) -> Result<(), String> {
        let answer = self.http.get("")?;
        if !answer.ok() {
            return Err(http_error(&answer));
        }
        if answer.body.to_lowercase().contains("elegoo") {
            Ok(())
        } else {
            Err("ElegooLink not detected".into())
        }
    }

    /// `uploadPart` for each part (`loopUpload`).
    fn upload_parts(&self, upload: &Upload, progress: &Progress) -> Result<(), Failure> {
        let data = &upload.data;
        let total = data.len();
        progress.total.store(total as u64, Ordering::Relaxed);
        if total == 0 {
            return Err(Failure::Error("The file to send is empty.".into()));
        }
        let md5 = format!("{:x}", Md5::digest(&data[..]));
        let id = uuid();
        let name = upload.name.rsplit('/').next().unwrap_or_default();
        let mut offset = 0;
        while offset < total {
            let end = (offset + PART).min(total);
            let form = Form::new()
                .field("Check", "1")
                .field("S-File-MD5", &md5)
                .field("Offset", &offset.to_string())
                .field("Uuid", &id)
                .field("TotalSize", &total.to_string())
                .file("File", name, data.clone(), offset..end);
            let answer = self.http.post_form(
                "uploadFile/upload",
                &form,
                progress,
                (offset as u64, (end - offset) as u64),
            )?;
            if answer.status != 200 {
                return Err(Failure::Error(http_error(&answer)));
            }
            let Some(json) = answer.json() else {
                return Err(Failure::Error("Error parsing response".into()));
            };
            if json["code"].as_str() != Some("000000") {
                let mut said = format!("ErrorCode : {}", json["code"].as_str().unwrap_or("?"));
                for message in json["messages"].as_array().into_iter().flatten() {
                    said += &format!(
                        "\n{}:{}",
                        message["field"].as_str().unwrap_or_default(),
                        message["message"].as_str().unwrap_or_default()
                    );
                }
                return Err(Failure::Error(said));
            }
            offset = end;
        }
        Ok(())
    }
}

/// "Storage" of a Centauri Carbon 1 (`sdcp_files`, ElegooSdcpFiles.cpp
/// `Cc1Files`).
impl ElegooLink {
    fn storage(&self) -> Result<(), String> {
        if self.cc1 {
            Ok(())
        } else {
            Err("The Storage tab speaks Elegoo Link (SDCP) only to the Centauri Carbon; this printer's model is not one.".into())
        }
    }

    /// Command 0's whole frame (its `Status`), or why not.
    fn status_frame(&self) -> Result<Value, String> {
        self.exchange(&envelope(0, json!({})), 0, self.timeouts.ask)
            .map_err(|error| format!("The printer did not say what it is doing ({error})."))
    }

    /// Command 258: the files of the internal storage's root.
    fn list_raw(&self) -> Result<Vec<Stored>, String> {
        let frame = self
            .exchange(
                &envelope(sdcp_files::FILE_LIST, sdcp_files::list_request()),
                sdcp_files::FILE_LIST,
                self.timeouts.ask,
            )
            .map_err(|error| {
                format!("The printer did not answer the file list request ({error}).")
            })?;
        let ack = sdcp_files::ack_of(&frame).unwrap_or(-1);
        match sdcp_files::list_of(&frame) {
            _ if ack > 0 => Err(format!(
                "The printer refused to list its files (Ack {ack})."
            )),
            Some(files) => Ok(files),
            None => {
                Err("This printer's answer to the file list request carries no file list.".into())
            }
        }
    }

    /// What the printer is doing, and the plan of a start when it is free
    /// (`start_check`, `plan_start`): every `CurrentStatus` idle; a CANVAS
    /// asked (command 324) unless command 0 says none is attached.
    fn start_plan(&self, options: &Options) -> Result<(StartCheck, sdcp_files::Plan), String> {
        let frame = self.status_frame()?;
        let status = &frame["Status"];
        let mut state = crate::device::sdcp::state_of(status).to_owned();
        let machine = crate::device::sdcp::machine(status);
        let current = status["PrintInfo"]["Filename"]
            .as_str()
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        // On purpose stricter than the reference (sdcp_files.rs): any
        // `CurrentStatus` but idle is busy.
        let busy = matches!(state.as_str(), "printing" | "paused" | "busy")
            || !machine.iter().all(|s| *s == IDLE);
        if busy && !matches!(state.as_str(), "printing" | "paused") {
            state = "busy".into();
        }
        let canvas = if busy || sdcp_files::canvas_attached(&frame) == Some(false) {
            None
        } else {
            self.exchange(
                &envelope(sdcp_files::CANVAS, json!({})),
                sdcp_files::CANVAS,
                self.timeouts.ask.min(Duration::from_secs(6)),
            )
            .ok()
            .as_ref()
            .and_then(sdcp_files::canvas_loaded)
        };
        let plan = sdcp_files::plan(options, canvas);
        Ok((
            StartCheck {
                state,
                current,
                busy,
                advice: if busy {
                    String::new()
                } else {
                    plan.advice.clone()
                },
            },
            plan,
        ))
    }
}

impl Host for ElegooLink {
    /// `ElegooLink::test`: OctoPrint first, then Elegoo's own page.
    fn test(&self) -> Result<(), String> {
        self.not_cc2()?;
        if octoprint::version(&self.http).is_ok() {
            return Ok(());
        }
        self.elegoo_test()
    }

    fn test_ok(&self) -> String {
        tr("Connection to ElegooLink is working correctly.")
    }

    fn test_failed(&self, reason: &str) -> String {
        format!("{}: {reason}", tr("Could not connect to ElegooLink"))
    }

    fn files(&self) -> Option<Result<Vec<Stored>, String>> {
        Some(self.storage().and_then(|()| self.list_raw()))
    }

    /// Command 259 (`Cc1Files::remove`): never the file a job is on; the
    /// printer's `ErrData` read, and the list asked again, so a delete the
    /// printer only acknowledged is said.
    fn delete(&self, name: &str) -> Result<(), String> {
        self.storage()?;
        if !super::moonraker::valid_path(name) {
            return Err(format!("Not a file on the printer: {name}"));
        }
        let frame = self.status_frame()?;
        let status = &frame["Status"];
        let state = crate::device::sdcp::state_of(status);
        if matches!(state, "printing" | "paused" | "busy")
            && sdcp_files::same_file(
                status["PrintInfo"]["Filename"].as_str().unwrap_or_default(),
                name,
            )
        {
            return Err(
                "The printer is printing this file. Finish or cancel the print first.".into(),
            );
        }
        let answer = self
            .exchange(
                &envelope(sdcp_files::DELETE_FILES, sdcp_files::delete_request(name)),
                sdcp_files::DELETE_FILES,
                self.timeouts.ask,
            )
            .map_err(|error| format!("The printer did not answer the delete request ({error})."))?;
        if let Some(ack) = sdcp_files::ack_of(&answer).filter(|ack| *ack > 0) {
            return Err(format!(
                "The printer refused to delete the file (Ack {ack})."
            ));
        }
        if sdcp_files::not_deleted(&answer)
            .iter()
            .any(|failed| sdcp_files::same_file(failed, name))
        {
            return Err("The printer could not delete the file.".into());
        }
        // The Ack alone is not trusted (unverified on this firmware).
        match self.list_raw() {
            Ok(files) if files.iter().any(|file| file.name == name) => {
                Err("The printer acknowledged the delete, but it still lists the file.".into())
            }
            _ => Ok(()),
        }
    }

    fn start_check(&self, _name: &str, options: &Options) -> Result<StartCheck, String> {
        self.storage()?;
        Ok(self.start_plan(options)?.0)
    }

    /// Command 128 for a file already on the printer (`start_print`):
    /// the printer read again just before, refused when it is busy or when
    /// the plan is not the one the person confirmed.
    fn print_confirmed(&self, name: &str, options: &Options, shown: &str) -> Result<(), String> {
        self.storage()?;
        if !super::moonraker::valid_path(name) || !super::moonraker::printable(name) {
            return Err(format!("Not a G-code file the printer can print: {name}"));
        }
        let (check, plan) = self.start_plan(options)?;
        if check.busy {
            return Err(format!(
                "The printer is {}, so the print was not started. Finish or cancel that job first.",
                if check.state == "paused" {
                    "paused mid-print"
                } else {
                    "printing"
                }
            ));
        }
        if check.advice != shown {
            return Err("The printer changed since you confirmed (its CANVAS trays or the send choices). Press Print again to see what it will do now.".into());
        }
        let text = start_payload_with(name, options, plan.canvas, &uuid(), now_ms());
        let answer = self.exchange(&text, 128, self.timeouts.ask).map_err(|_| {
            "The printer did not answer the start request in time; it may still start. Check the printer.".to_owned()
        })?;
        match sdcp_files::ack_of(&answer) {
            Some(ack) if ack > 0 => Err(sdcp_files::start_ack_text(ack)),
            _ => Ok(()),
        }
    }

    fn upload(&self, upload: &Upload, progress: &Progress) -> Result<(), Failure> {
        self.not_cc2()?;
        // An Elegoo printer that answers as OctoPrint is sent to as one
        // (ElegooLink.cpp l.426).
        if octoprint::version(&self.http).is_ok() {
            return octoprint::upload_file(&self.http, upload, progress);
        }
        self.elegoo_test()?;
        if upload.start {
            let mut socket = self.connect()?;
            let status = request_status(&mut socket, self.timeouts.ask, progress)?;
            if !status.iter().all(|state| *state == IDLE) {
                return Err(Failure::Error(tr(BUSY)));
            }
            let _ = socket.close(None);
        }
        self.upload_parts(upload, progress)?;
        if !upload.start {
            return Ok(());
        }
        progress.set_stage("Starting the print");
        let mut socket = self.connect().map_err(|_| {
            Failure::Error(tr("The file has been transferred, but some unknown errors occurred. Please check the device page for the file and try to start printing again."))
        })?;
        pause(Duration::from_secs(1), progress)?;
        wait_checked(&mut socket, self.timeouts.start, progress)?;
        pause(Duration::from_secs(1), progress)?;
        let name = upload.name.rsplit('/').next().unwrap_or_default();
        socket
            .send(Message::text(start_payload(
                name,
                &upload.options,
                &uuid(),
                now_ms(),
            )))
            .map_err(|error| ws_said(&error))?;
        let ack = read_ack(
            &mut socket,
            self.timeouts.start.min(Duration::from_secs(30)),
            progress,
        )?;
        let _ = socket.close(None);
        ack_result(ack)
    }
}

impl ElegooLink {
    /// One SDCP request on a connection of its own (the Device page's):
    /// command `cmd` with `data`, and the printer's answer: for command 0
    /// its status frame (`Status`), for any other the response to that
    /// command (`Data.Data`, with its `Ack`). Within the time for a question.
    pub fn ask(&self, cmd: u32, data: Value) -> Result<Value, String> {
        let frame = self.exchange(&envelope(cmd, data), cmd, self.timeouts.ask)?;
        Ok(if cmd == 0 {
            frame["Status"].clone()
        } else {
            frame["Data"]["Data"].clone()
        })
    }

    /// One request `text` (an SDCP envelope) on a connection of its own,
    /// and the whole frame that answers it: for command 0 the first one
    /// with a `Status`, for any other the one whose `Data.Cmd` is `cmd`;
    /// within `within`.
    fn exchange(&self, text: &str, cmd: u32, within: Duration) -> Result<Value, String> {
        if self.cc2 {
            return Err("This app cannot reach an Elegoo Centauri Carbon 2 yet.".into());
        }
        let mut socket = self.connect()?;
        socket
            .send(Message::text(text))
            .map_err(|error| ws_said(&error))?;
        let until = Instant::now() + within;
        while Instant::now() < until {
            let message = match next(&mut socket) {
                Ok(Some(message)) => message,
                Ok(None) => continue,
                Err(Failure::Error(error)) => return Err(error),
                Err(Failure::Cancelled) => return Err("cancelled".into()),
            };
            let answers = if cmd == 0 {
                message.get("Status").is_some()
            } else {
                message["Data"]["Cmd"].as_u64() == Some(u64::from(cmd))
            };
            if answers {
                let _ = socket.close(None);
                return Ok(message);
            }
        }
        Err(tr(TIMED_OUT))
    }

    /// The printer's websocket, every read timed.
    fn connect(&self) -> Result<WebSocket<TcpStream>, String> {
        let address = (self.ws_host.as_str(), self.ws_port)
            .to_socket_addrs()
            .map_err(|_| tr(UNPARSABLE))?
            .next()
            .ok_or_else(|| tr(UNPARSABLE))?;
        let stream = TcpStream::connect_timeout(&address, self.timeouts.connect)
            .map_err(|error| super::io_said(&error))?;
        stream
            .set_read_timeout(Some(self.timeouts.connect))
            .and_then(|_| stream.set_write_timeout(Some(self.timeouts.connect)))
            .map_err(|error| super::io_said(&error))?;
        let host = if self.ws_host.contains(':') {
            format!("[{}]", self.ws_host)
        } else {
            self.ws_host.clone()
        };
        let url = format!("ws://{host}:{}/websocket", self.ws_port);
        let (socket, _) = tungstenite::client(url, stream).map_err(|error| match error {
            tungstenite::HandshakeError::Failure(error) => ws_said(&error),
            tungstenite::HandshakeError::Interrupted(_) => tr(TIMED_OUT),
        })?;
        // Short reads from now on, so a cancel and the limits are seen.
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(200)))
            .map_err(|error| super::io_said(&error))?;
        Ok(socket)
    }
}

/// SDCP's `CurrentStatus` for a printer doing nothing, and checking a file
/// it was sent (ElegooSdcp.hpp l.44).
const IDLE: i64 = 0;
const FILE_CHECKING: i64 = 8;

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or_default()
}

/// An SDCP request with `data`, as the Device page sends it.
fn envelope(cmd: u32, data: Value) -> String {
    json!({
        "Id": "",
        "Data": { "Cmd": cmd, "Data": data, "RequestID": uuid(), "MainboardID": "", "TimeStamp": now_ms() as u64, "From": 1 }
    })
    .to_string()
}

/// An SDCP request (ElegooLink.cpp l.940): command `cmd`, no data.
fn request(cmd: u32) -> String {
    json!({
        "Id": "",
        "Data": { "Cmd": cmd, "Data": {}, "RequestID": uuid(), "MainboardID": "", "TimeStamp": now_ms() as u64, "From": 1 }
    })
    .to_string()
}

/// Command 128, laid out as `ElegooStartPrint::build_payload` lays it out
/// (its keys in that order). "Upload and Print" sends no `slot_map` (the
/// CANVAS mapping of the send dialog is not built); a start from "Storage"
/// with a CANVAS that answered sends an empty one, as the reference's
/// (`canvas`).
pub fn start_payload(
    name: &str,
    options: &super::Options,
    request_id: &str,
    timestamp: u128,
) -> String {
    start_payload_with(name, options, false, request_id, timestamp)
}

pub fn start_payload_with(
    name: &str,
    options: &super::Options,
    canvas: bool,
    request_id: &str,
    timestamp: u128,
) -> String {
    let file = serde_json::to_string(&format!("/local/{name}")).expect("a string");
    format!(
        "{{\"Id\":\"\",\"Data\":{{\"Cmd\":128,\"Data\":{{\"Filename\":{file},\"StartLayer\":0,\"Calibration_switch\":{},\"PrintPlatformType\":{},\"Tlp_Switch\":{}{}}},\"RequestID\":\"{request_id}\",\"MainboardID\":\"\",\"TimeStamp\":{timestamp},\"From\":1}}}}",
        u8::from(options.leveling),
        u8::from(options.smooth_plate),
        u8::from(options.timelapse),
        if canvas { ",\"slot_map\":[]" } else { "" },
    )
}

fn ws_said(error: &tungstenite::Error) -> String {
    match error {
        tungstenite::Error::Io(io) => super::io_said(io),
        tungstenite::Error::Http(_) => {
            "The printer declined the connection to its websocket.".into()
        }
        _ => "The printer's websocket could not be read.".into(),
    }
}

/// Waits `time`, unless called off.
fn pause(time: Duration, progress: &Progress) -> Result<(), Failure> {
    let until = Instant::now() + time;
    while Instant::now() < until {
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// The next message as JSON, or none when nothing came for a while.
fn next(socket: &mut WebSocket<TcpStream>) -> Result<Option<Value>, Failure> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(serde_json::from_str(text.as_str()).ok()),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(io))
            if matches!(
                io.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(Failure::Error(ws_said(&error))),
    }
}

fn current_status(message: &Value) -> Option<Vec<i64>> {
    let status = message.get("Status")?;
    Some(match &status["CurrentStatus"] {
        Value::Array(states) => states.iter().filter_map(Value::as_i64).collect(),
        Value::Number(state) => state.as_i64().into_iter().collect(),
        _ => Vec::new(),
    })
}

/// Command 0, and the `CurrentStatus` the printer answers with.
fn request_status(
    socket: &mut WebSocket<TcpStream>,
    within: Duration,
    progress: &Progress,
) -> Result<Vec<i64>, Failure> {
    socket
        .send(Message::text(request(0)))
        .map_err(|error| ws_said(&error))?;
    let until = Instant::now() + within;
    while Instant::now() < until {
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        if let Some(status) = next(socket)?.as_ref().and_then(current_status) {
            return Ok(status);
        }
    }
    Err(Failure::Error(tr(TIMED_OUT)))
}

/// `checkResult`: asks for the state each second while the printer checks
/// the file; done when it is anything else. A printer that has taken up
/// another print meanwhile is busy.
fn wait_checked(
    socket: &mut WebSocket<TcpStream>,
    within: Duration,
    progress: &Progress,
) -> Result<(), Failure> {
    let until = Instant::now() + within;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Failure::Error(tr("Start print timeout")));
        }
        let status = request_status(socket, left.min(Duration::from_secs(5)), progress)?;
        if !status.contains(&FILE_CHECKING) {
            if status.contains(&1) {
                return Err(Failure::Error(ack_said(1)));
            }
            return Ok(());
        }
        pause(Duration::from_secs(1), progress)?;
    }
}

/// The `Ack` of the answer to command 128.
fn read_ack(
    socket: &mut WebSocket<TcpStream>,
    within: Duration,
    progress: &Progress,
) -> Result<Option<i64>, Failure> {
    let until = Instant::now() + within;
    while Instant::now() < until {
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        if let Some(message) = next(socket)?
            && message["Data"]["Cmd"].as_i64() == Some(128)
        {
            return Ok(message["Data"]["Data"]["Ack"].as_i64());
        }
    }
    Err(Failure::Error(tr("Start print timeout")))
}

/// The reference's words for each `Ack` (ElegooLink.cpp l.52, l.860).
pub fn ack_said(ack: i64) -> String {
    let text = match ack {
        1 => {
            "The printer is busy, Please check the device page for the file and try to start printing again."
        }
        2 => "The file is lost, please check and try again.",
        3 => "The file is corrupted, please check and try again.",
        4..=6 => "Transmission abnormality, please check and try again.",
        7 => "The file does not match the printer, please check and try again.",
        _ => "Unknown error",
    };
    format!("{} {}", tr(text), fill(&tr("Error code: %d"), &[&ack]))
}

fn ack_result(ack: Option<i64>) -> Result<(), Failure> {
    match ack {
        Some(0) => Ok(()),
        Some(ack) => Err(Failure::Error(ack_said(ack))),
        None => Err(Failure::Error(tr("Error code not found"))),
    }
}
