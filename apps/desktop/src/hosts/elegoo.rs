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
    Connection, Failure, Form, Host, Http, Progress, TIMED_OUT, Timeouts, Upload, http_error,
    octoprint, snapmaker::BUSY, uuid,
};

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
    timeouts: Timeouts,
    /// For the tests: the websocket's port.
    pub ws_port: u16,
}

impl ElegooLink {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        let http = Http::new(connection, timeouts)?;
        Ok(Self {
            ws_host: http.address.host.clone(),
            http,
            cc2: is_cc2(&connection.printer_model),
            timeouts,
            ws_port: WEBSOCKET_PORT,
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
        "Connection to ElegooLink is working correctly.".into()
    }

    fn test_failed(&self, reason: &str) -> String {
        format!("Could not connect to ElegooLink: {reason}")
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
                return Err(Failure::Error(BUSY.into()));
            }
            let _ = socket.close(None);
        }
        self.upload_parts(upload, progress)?;
        if !upload.start {
            return Ok(());
        }
        progress.set_stage("Starting the print");
        let mut socket = self.connect().map_err(|_| {
            Failure::Error("The file has been transferred, but some unknown errors occurred. Please check the device page for the file and try to start printing again.".into())
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
    /// The printer's websocket, every read timed.
    fn connect(&self) -> Result<WebSocket<TcpStream>, String> {
        let address = (self.ws_host.as_str(), self.ws_port)
            .to_socket_addrs()
            .map_err(|_| {
                "The Hostname/IP/URL could not be parsed, please check it and try again.".to_owned()
            })?
            .next()
            .ok_or("The Hostname/IP/URL could not be parsed, please check it and try again.")?;
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
            tungstenite::HandshakeError::Interrupted(_) => TIMED_OUT.to_owned(),
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

/// An SDCP request (ElegooLink.cpp l.940): command `cmd`, no data.
fn request(cmd: u32) -> String {
    json!({
        "Id": "",
        "Data": { "Cmd": cmd, "Data": {}, "RequestID": uuid(), "MainboardID": "", "TimeStamp": now_ms() as u64, "From": 1 }
    })
    .to_string()
}

/// Command 128, laid out as `ElegooStartPrint::build_payload` lays it out
/// (its keys in that order). No `slot_map`: the Canvas is not built.
pub fn start_payload(
    name: &str,
    options: &super::Options,
    request_id: &str,
    timestamp: u128,
) -> String {
    let file = serde_json::to_string(&format!("/local/{name}")).expect("a string");
    format!(
        "{{\"Id\":\"\",\"Data\":{{\"Cmd\":128,\"Data\":{{\"Filename\":{file},\"StartLayer\":0,\"Calibration_switch\":{},\"PrintPlatformType\":{},\"Tlp_Switch\":{}}},\"RequestID\":\"{request_id}\",\"MainboardID\":\"\",\"TimeStamp\":{timestamp},\"From\":1}}}}",
        u8::from(options.leveling),
        u8::from(options.smooth_plate),
        u8::from(options.timelapse),
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
    Err(Failure::Error(TIMED_OUT.into()))
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
            return Err(Failure::Error("Start print timeout".into()));
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
    Err(Failure::Error("Start print timeout".into()))
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
    format!("{text} Error code: {ack}")
}

fn ack_result(ack: Option<i64>) -> Result<(), Failure> {
    match ack {
        Some(0) => Ok(()),
        Some(ack) => Err(Failure::Error(ack_said(ack))),
        None => Err(Failure::Error("Error code not found".into())),
    }
}
