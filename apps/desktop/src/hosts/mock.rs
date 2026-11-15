//! Printers that are not there: an HTTP server and a websocket on
//! 127.0.0.1, each on a port the system picks, answering as a script
//! says and keeping what they were sent. For the tests only; nothing here
//! reaches the network.

use std::{
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::Value;
use tungstenite::Message;

/// One request as the mock received it.
#[derive(Clone, Debug)]
pub struct Got {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Got {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The parts of a multipart form: name, file name, bytes.
    pub fn form(&self) -> Vec<(String, Option<String>, Vec<u8>)> {
        let kind = self.header("Content-Type").unwrap_or_default();
        let boundary = kind.split("boundary=").nth(1).expect("a multipart form");
        let marker = format!("--{boundary}");
        let mut parts = Vec::new();
        let body = &self.body;
        let mut at = find(body, marker.as_bytes(), 0).expect("a first boundary");
        loop {
            at += marker.len();
            if body[at..].starts_with(b"--") {
                break;
            }
            at += 2; // \r\n
            let head_end = find(body, b"\r\n\r\n", at).expect("a part's head");
            let head = String::from_utf8_lossy(&body[at..head_end]).into_owned();
            let start = head_end + 4;
            let next = find(body, format!("\r\n{marker}").as_bytes(), start).expect("a part's end");
            let quoted = |key: &str| {
                head.split(&format!("{key}=\""))
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .map(str::to_owned)
            };
            parts.push((
                quoted(" name")
                    .or_else(|| quoted("name"))
                    .unwrap_or_default(),
                quoted("filename"),
                body[start..next].to_vec(),
            ));
            at = next + 2;
        }
        parts
    }

    /// A form's field as text.
    pub fn field(&self, name: &str) -> Option<String> {
        self.form()
            .into_iter()
            .find(|(key, _, _)| key == name)
            .map(|(_, _, bytes)| String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// What the mock answers: status and body.
pub type Script = dyn Fn(&Got, &[Got]) -> (u16, String) + Send + Sync;

/// An HTTP printer on 127.0.0.1.
pub struct Printer {
    pub port: u16,
    pub got: Arc<Mutex<Vec<Got>>>,
    stop: Arc<AtomicBool>,
}

impl Printer {
    /// `script` is given each request and those before it. `slow` waits
    /// that long before reading a request's body (to see a cancel).
    pub fn start(script: Box<Script>, slow: Option<Duration>) -> Self {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = server.server_addr().to_ip().expect("an IP address").port();
        let got = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (kept, stopped) = (got.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok(Some(mut request)) = server.recv_timeout(Duration::from_millis(50)) else {
                    continue;
                };
                if let Some(wait) = slow {
                    std::thread::sleep(wait);
                }
                let mut body = Vec::new();
                let _ = request.as_reader().read_to_end(&mut body);
                let one = Got {
                    method: request.method().to_string(),
                    url: request.url().to_owned(),
                    headers: request
                        .headers()
                        .iter()
                        .map(|header| (header.field.to_string(), header.value.to_string()))
                        .collect(),
                    body,
                };
                let before = kept.lock().unwrap().clone();
                let (status, text) = script(&one, &before);
                kept.lock().unwrap().push(one);
                let _ = request
                    .respond(tiny_http::Response::from_string(text).with_status_code(status));
            }
        });
        Self { port, got, stop }
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    pub fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    /// The paths asked for, in order, with their methods.
    pub fn asked(&self) -> Vec<String> {
        self.got()
            .iter()
            .map(|got| format!("{} {}", got.method, got.url))
            .collect()
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A port that takes a connection and never answers: a printer that
/// hangs.
pub struct Silent {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Silent {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                if let Ok((stream, _)) = listener.accept() {
                    held.push(stream);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Self { port, stop }
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the websocket answers each message with: none for silence.
pub type Says = dyn Fn(&Value, usize) -> Option<Value> + Send + Sync;

/// The Centauri Carbon's websocket on 127.0.0.1: one connection at a
/// time, each message read as JSON and answered as `says` says (given the
/// message and how many came before it, on every connection).
pub struct Socket {
    pub port: u16,
    pub got: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
}

impl Socket {
    pub fn start(says: Box<Says>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port on 127.0.0.1");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (kept, stopped) = (got.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let Ok(mut socket) = tungstenite::accept(stream) else {
                    continue;
                };
                socket
                    .get_ref()
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .unwrap();
                while !stopped.load(Ordering::Relaxed) {
                    match socket.read() {
                        Ok(Message::Text(text)) => {
                            let message: Value =
                                serde_json::from_str(text.as_str()).unwrap_or_default();
                            let count = kept.lock().unwrap().len();
                            kept.lock().unwrap().push(message.clone());
                            if let Some(answer) = says(&message, count) {
                                let _ = socket.send(Message::text(answer.to_string()));
                            }
                        }
                        Ok(Message::Close(_)) => break,
                        Ok(_) => {}
                        Err(tungstenite::Error::Io(io))
                            if matches!(
                                io.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) => {}
                        Err(_) => break,
                    }
                }
            }
        });
        Self { port, got, stop }
    }

    pub fn got(&self) -> Vec<Value> {
        self.got.lock().unwrap().clone()
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A G-code file of `size` bytes, every byte telling where it is.
pub fn gcode(size: usize) -> Arc<Vec<u8>> {
    let mut text = Vec::with_capacity(size);
    let mut line = 0u64;
    while text.len() < size {
        text.extend_from_slice(format!("G1 X{} Y{} ; {line}\n", line % 200, line % 190).as_bytes());
        line += 1;
    }
    text.truncate(size);
    Arc::new(text)
}

/// How the mock U1 behaves.
#[derive(Clone, Debug)]
pub struct U1Setup {
    /// `print_stats.state` before anything is started.
    pub state: &'static str,
    /// What each toolhead holds: type, `RRGGBBAA`, loaded.
    pub toolheads: Vec<(&'static str, &'static str, bool)>,
    /// The key it wants (`X-Api-Key`), if any.
    pub key: Option<&'static str>,
    pub login_required: bool,
    /// Refuse the toolhead map, or the start.
    pub refuse_script: bool,
    pub refuse_start: bool,
    /// Status queries a started print takes to finish.
    pub steps: u64,
    /// `printer/objects/list` (a U1's, unless a test gives another
    /// Klipper's) and its webcam: (service, stream URL).
    pub objects: Vec<&'static str>,
    pub webcam: Option<(&'static str, &'static str)>,
}

/// A U1's objects, as the reference queries them (SnapmakerLan.cpp l.803).
pub const U1_OBJECTS: [&str; 15] = [
    "print_stats",
    "display_status",
    "virtual_sdcard",
    "heater_bed",
    "extruder",
    "extruder1",
    "extruder2",
    "extruder3",
    "print_task_config",
    "gcode_move",
    "toolhead",
    "fan",
    "led cavity_led",
    "fan_generic cavity_fan",
    "configfile",
];

impl Default for U1Setup {
    fn default() -> Self {
        Self {
            state: "standby",
            toolheads: vec![
                ("PLA", "FF0000FF", true),
                ("PETG", "00FF00FF", true),
                ("PLA", "0000FFFF", true),
                ("", "", false),
            ],
            key: None,
            login_required: false,
            refuse_script: false,
            refuse_start: false,
            steps: 4,
            objects: U1_OBJECTS.to_vec(),
            webcam: Some(("webrtc-camerastreamer", "/webcam/webrtc")),
        }
    }
}

/// What the mock U1 has been through.
#[derive(Debug, Default)]
pub struct U1State {
    pub state: String,
    /// The file it holds and prints, with its size.
    pub files: Vec<(String, usize)>,
    pub printing: Option<String>,
    /// Status queries since the start.
    pub polls: u64,
    pub scripts: Vec<String>,
    /// What the Device page set: each heater's target, each fan (0..1),
    /// the speed factor, the LED, the homed axes; and the print commands.
    pub targets: std::collections::HashMap<String, f64>,
    pub fans: std::collections::HashMap<String, f64>,
    pub speed_factor: f64,
    pub led: bool,
    pub homed: String,
    pub commands: Vec<String>,
}

/// What a script the Device page sends does to the mock's state.
fn run_script(u1: &mut U1State, script: &str) {
    let word = |line: &str, key: &str| {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .map(str::to_owned)
    };
    for line in script.lines() {
        let number = |key: &str| {
            word(line, key)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
        };
        if line.starts_with("SET_HEATER_TEMPERATURE") {
            let heater = word(line, "HEATER=").unwrap_or_default();
            let object = if heater == "heater_bed" || heater.starts_with("extruder") {
                heater
            } else {
                format!("heater_generic {heater}")
            };
            u1.targets.insert(object, number("TARGET="));
        } else if line.starts_with("M106") {
            u1.fans.insert("fan".into(), number("S") / 255.0);
        } else if line.starts_with("SET_FAN_SPEED") {
            let fan = word(line, "FAN=").unwrap_or_default();
            u1.fans
                .insert(format!("fan_generic {fan}"), number("SPEED="));
        } else if line.starts_with("M220") {
            u1.speed_factor = number("S") / 100.0;
        } else if line.starts_with("SET_LED") {
            u1.led = number("WHITE=") > 0.0;
        } else if line.starts_with("G28") {
            u1.homed = "xyz".into();
        }
    }
}

/// A Snapmaker U1 as its Moonraker answers over HTTP, from the
/// reference's own requests (SnapmakerLan.cpp, PrinterStorage.cpp,
/// MoonrakerProgress.hpp): `/server/info`, `/access/info`, the object
/// queries (`print_stats`, `print_task_config`, `extruder*`, `heater_bed`,
/// `virtual_sdcard`, `display_status`), `/server/files/upload`,
/// `/server/files/metadata`, `/server/files/list`, a delete, the macros
/// through `/printer/gcode/script` and `/printer/print/start`. A print
/// started runs through `steps` status queries to "complete".
pub fn u1(setup: U1Setup) -> (Printer, Arc<Mutex<U1State>>) {
    let shared = Arc::new(Mutex::new(U1State {
        state: setup.state.to_owned(),
        speed_factor: 1.0,
        ..U1State::default()
    }));
    let kept = shared.clone();
    let printer = Printer::start(
        Box::new(move |got: &Got, _: &[Got]| {
            let unauthorised = || {
                (
                    401,
                    serde_json::json!({ "error": { "code": 401, "message": "Unauthorized" } })
                        .to_string(),
                )
            };
            if let Some(key) = setup.key
                && got.header("X-Api-Key") != Some(key)
            {
                return unauthorised();
            }
            let mut u1 = kept.lock().unwrap();
            let url = got.url.as_str();
            let (path, query) = url.split_once('?').unwrap_or((url, ""));
            let arg = |name: &str| {
                query.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == name).then(|| decode(value))
                })
            };
            let ok = |result: Value| (200, serde_json::json!({ "result": result }).to_string());
            match (got.method.as_str(), path) {
                ("GET", "/server/info") => ok(serde_json::json!({ "klippy_state": "ready" })),
                ("GET", "/access/info") => {
                    ok(serde_json::json!({ "login_required": setup.login_required, "trusted": true }))
                }
                ("GET", "/printer/objects/query") => {
                    let mut status = serde_json::Map::new();
                    let total = u1
                        .printing
                        .as_ref()
                        .and_then(|name| u1.files.iter().find(|(n, _)| n == name))
                        .map_or(0, |(_, size)| *size as u64);
                    if query.contains("virtual_sdcard") && u1.printing.is_some() && u1.state == "printing" {
                        u1.polls += 1;
                        if u1.polls >= setup.steps {
                            u1.state = "complete".into();
                        }
                    }
                    let position = if u1.state == "complete" {
                        total + 1
                    } else {
                        total * u1.polls / setup.steps.max(1)
                    };
                    let printing = u1.printing.clone().unwrap_or_default();
                    for object in query.split('&') {
                        let object = decode(object.split('=').next().unwrap_or_default());
                        let object = object.as_str();
                        let value = match object {
                            "print_stats" => serde_json::json!({
                                "state": u1.state, "filename": printing, "print_duration": 60.0 * u1.polls as f64,
                                "message": "", "info": { "current_layer": u1.polls * 10, "total_layer": setup.steps * 10 },
                            }),
                            "virtual_sdcard" => serde_json::json!({ "file_position": position, "file_path": format!("/home/lava/printer_data/gcodes/{printing}") }),
                            "display_status" => serde_json::json!({ "progress": 0.0 }),
                            "heater_bed" => {
                                let target = u1.targets.get("heater_bed").copied().unwrap_or(60.0);
                                serde_json::json!({ "temperature": target.max(22.0), "target": target })
                            }
                            "print_task_config" => serde_json::json!({
                                "filament_type": setup.toolheads.iter().map(|t| t.0).collect::<Vec<_>>(),
                                "filament_color_rgba": setup.toolheads.iter().map(|t| t.1).collect::<Vec<_>>(),
                                "filament_exist": setup.toolheads.iter().map(|t| t.2).collect::<Vec<_>>(),
                            }),
                            extruder if extruder.starts_with("extruder") => {
                                let target = u1.targets.get(extruder).copied().unwrap_or(0.0);
                                serde_json::json!({
                                    "temperature": if target > 0.0 { target } else { 25.0 }, "target": target, "nozzle_diameter": 0.4,
                                })
                            }
                            generic if generic.starts_with("heater_generic ") => {
                                let target = u1.targets.get(generic).copied().unwrap_or(0.0);
                                serde_json::json!({ "temperature": if target > 0.0 { target } else { 24.0 }, "target": target })
                            }
                            "fan" => serde_json::json!({ "speed": u1.fans.get("fan").copied().unwrap_or(0.0) }),
                            generic if generic.starts_with("fan_generic ") => serde_json::json!({ "speed": u1.fans.get(generic).copied().unwrap_or(0.0) }),
                            led if led.starts_with("led ") => {
                                let v = if u1.led { 1.0 } else { 0.0 };
                                serde_json::json!({ "color_data": [[0.0, 0.0, 0.0, v]] })
                            }
                            "gcode_move" => serde_json::json!({ "speed_factor": u1.speed_factor }),
                            "toolhead" => serde_json::json!({ "homed_axes": u1.homed }),
                            "configfile" => serde_json::json!({ "settings": {
                                "extruder": { "max_temp": 285.0 }, "heater_bed": { "max_temp": 110.0 },
                                "heater_generic chamber": { "max_temp": 65.0 },
                            } }),
                            _ => continue,
                        };
                        status.insert(object.to_owned(), value);
                    }
                    ok(serde_json::json!({ "status": status }))
                }
                ("POST", "/server/files/upload") => {
                    let form = got.form();
                    let Some((_, Some(name), bytes)) =
                        form.into_iter().find(|(field, _, _)| field == "file")
                    else {
                        return (400, "{}".into());
                    };
                    u1.files.retain(|(n, _)| *n != name);
                    u1.files.push((name.clone(), bytes.len()));
                    (201, serde_json::json!({ "result": { "item": { "path": name } } }).to_string())
                }
                ("GET", "/server/files/metadata") => {
                    let name = arg("filename").unwrap_or_default();
                    match u1.files.iter().find(|(n, _)| *n == name) {
                        Some((_, size)) => ok(serde_json::json!({
                            "size": size, "gcode_start_byte": 0, "gcode_end_byte": size, "estimated_time": 1200.0,
                        })),
                        None => (404, serde_json::json!({ "error": { "code": 404, "message": "Metadata not available" } }).to_string()),
                    }
                }
                ("GET", "/server/files/list") => ok(Value::Array(
                    u1.files
                        .iter()
                        .enumerate()
                        .map(|(i, (name, size))| serde_json::json!({ "path": name, "size": size, "modified": 100.0 + i as f64 }))
                        .collect(),
                )),
                ("DELETE", _) if path.starts_with("/server/files/gcodes/") => {
                    let name = decode(&path["/server/files/gcodes/".len()..]);
                    let before = u1.files.len();
                    u1.files.retain(|(n, _)| *n != name);
                    if u1.files.len() == before {
                        (404, serde_json::json!({ "error": { "code": 404, "message": "File does not exist" } }).to_string())
                    } else {
                        ok(serde_json::json!({ "item": { "path": name } }))
                    }
                }
                ("POST", "/printer/gcode/script") => {
                    if setup.refuse_script {
                        return (400, serde_json::json!({ "error": { "code": 400, "message": "Unknown command" } }).to_string());
                    }
                    let script = arg("script").unwrap_or_default();
                    run_script(&mut u1, &script);
                    u1.scripts.push(script);
                    ok(Value::from("ok"))
                }
                ("GET", "/printer/objects/list") => ok(serde_json::json!({ "objects": setup.objects })),
                ("GET", "/server/webcams/list") => ok(serde_json::json!({ "webcams": setup.webcam.iter().map(|(service, url)| serde_json::json!({
                    "name": "cam", "enabled": true, "service": service, "stream_url": url, "snapshot_url": "/webcam/?action=snapshot",
                })).collect::<Vec<_>>() })),
                ("GET", "/webcam/") => (200, "mjpeg".into()),
                ("POST", "/printer/print/pause" | "/printer/print/resume" | "/printer/print/cancel") => {
                    let action = path.rsplit('/').next().unwrap_or_default().to_owned();
                    let (from, to) = match action.as_str() {
                        "pause" => ("printing", "paused"),
                        "resume" => ("paused", "printing"),
                        _ => ("", "cancelled"),
                    };
                    if (!from.is_empty() && u1.state != from) || (from.is_empty() && !matches!(u1.state.as_str(), "printing" | "paused")) {
                        return (400, serde_json::json!({ "error": { "code": 400, "message": format!("Cannot {action}: not printing") } }).to_string());
                    }
                    u1.state = to.into();
                    u1.commands.push(action);
                    ok(Value::from("ok"))
                }
                ("POST", "/printer/print/start") => {
                    let name = arg("filename").unwrap_or_default();
                    if setup.refuse_start || !u1.files.iter().any(|(n, _)| *n == name) {
                        return (400, serde_json::json!({ "error": { "code": 400, "message": "Unable to start print" } }).to_string());
                    }
                    if u1.state == "printing" || u1.state == "paused" {
                        return (400, serde_json::json!({ "error": { "code": 400, "message": "Printer is busy" } }).to_string());
                    }
                    u1.state = "printing".into();
                    u1.printing = Some(name);
                    u1.polls = 0;
                    ok(Value::from("ok"))
                }
                _ => (404, serde_json::json!({ "error": { "code": 404, "message": "Not Found" } }).to_string()),
            }
        }),
        None,
    );
    (printer, shared)
}

/// Percent-decoding of a query or path part.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
