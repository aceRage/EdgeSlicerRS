//! Sending a sliced plate to a printer: the print hosts of the reference
//! (`src/slic3r/Utils/PrintHost.cpp` and the classes it makes), behind one
//! trait, with the upload queue the page watches.
//!
//! This is the desktop app's, not the engine's: engine crates take no
//! network dependency. Nothing here runs unless the person asks for it
//! ("Test", "Upload", "Upload and Print", the Device tab's "Storage"), and
//! nothing looks for printers on the network by itself.
//!
//! What is built, by host type (`host_type`, PrintConfig.cpp
//! `s_keys_map_PrintHostType`):
//!
//! - `octoprint` ("Octo/Klipper"): OctoPrint's API (`OctoPrint.cpp`), which
//!   Moonraker also answers. For a Snapmaker U1, which the profiles give
//!   this type, the fork does not use it: see [`snapmaker`].
//! - `elegoolink` ("Elegoo Link"): the Centauri Carbon's own protocol
//!   (`ElegooLink.cpp`), see [`elegoo`].
//!
//! Every other type is "not yet" (`HOST_TYPES` says which exist).
//!
//! What is never done: a key or a password is never put into a URL, a log
//! or an error; an address typed with `user:password@` in it is shown and
//! said without them; a redirect is never followed (it would carry the
//! key to another host).

use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::Value;

use crate::api::i18n::tr;

pub mod elegoo;
pub mod kept;
#[cfg(test)]
pub mod mock;
pub mod moonraker;
pub mod octoprint;
pub mod queue;
pub mod snapmaker;
#[cfg(test)]
mod tests;

/// The host types of the reference, in the order of its "Host Type" list
/// (PrintConfig.cpp, `host_type`), with its labels, and whether this app
/// can send to one yet.
pub const HOST_TYPES: [(&str, &str, bool); 14] = [
    ("prusalink", "PrusaLink", false),
    ("prusaconnect", "PrusaConnect", false),
    ("octoprint", "Octo/Klipper", true),
    ("duet", "Duet", false),
    ("flashair", "FlashAir", false),
    ("astrobox", "AstroBox", false),
    ("repetier", "Repetier", false),
    ("mks", "MKS", false),
    ("esp3d", "ESP3D", false),
    ("crealityprint", "CrealityPrint", false),
    ("obico", "Obico", false),
    ("flashforge", "Flashforge", false),
    ("simplyprint", "SimplyPrint", false),
    ("elegoolink", "Elegoo Link", true),
];

/// The label of a host type, as the reference's list shows it.
pub fn label_of(host_type: &str) -> &str {
    HOST_TYPES
        .iter()
        .find(|(key, _, _)| *key == host_type)
        .map_or(host_type, |(_, label, _)| label)
}

/// Where a printer is and how it is reached: the reference's "Print Host
/// upload" options, by their keys. The two secrets are here only on the
/// Rust side; the page is told whether each is set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Connection {
    pub host_type: String,
    pub print_host: String,
    pub print_host_webui: String,
    pub printhost_cafile: String,
    /// "API Key / Password".
    pub apikey: Option<String>,
    /// The printer preset's `printer_model`: the U1 and the Centauri
    /// Carbon 2 are told apart by it, as the reference tells them.
    pub printer_model: String,
}

/// How long each kind of request may take. The tests make them short.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// Reaching the printer at all.
    pub connect: Duration,
    /// A short question ("Test", a state, the files): all of it.
    pub ask: Duration,
    /// Sending the file: each write, then the answer once it is sent.
    pub write: Duration,
    pub answer: Duration,
    /// Waiting for a printer to start a print it was sent (the Centauri
    /// Carbon checks the file first).
    pub start: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            ask: Duration::from_secs(15),
            write: Duration::from_secs(60),
            answer: Duration::from_secs(120),
            start: Duration::from_secs(60),
        }
    }
}

/// A file to send.
#[derive(Clone, Debug, Default)]
pub struct Upload {
    /// The name it gets on the printer.
    pub name: String,
    /// Exactly the bytes "Export G-code file" writes.
    pub data: Arc<Vec<u8>>,
    /// "Upload and Print".
    pub start: bool,
    pub options: Options,
}

/// What some printers are told with "Upload and Print".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// Elegoo Centauri Carbon: "Time-lapse", "Heated Bed Leveling", and
    /// "Smooth Build Plate (Side B)" (else "Textured Build Plate (Side A)").
    pub timelapse: bool,
    pub leveling: bool,
    pub smooth_plate: bool,
    /// Snapmaker U1: "Unload filaments after print".
    pub unload_at_end: bool,
    /// The filaments the plate prints in, from 0: the U1 is told to print
    /// each with its own toolhead ("As sliced") when `mapping` is empty.
    pub filaments: Vec<usize>,
    /// Snapmaker U1: the toolhead (from 0) that prints each of the file's
    /// filaments, by the filament's index; -1 for one the file does not
    /// use (SnapmakerLan.hpp `start_print_mapped`).
    pub mapping: Vec<i32>,
    /// Snapmaker U1, the bundled device page's "Print Preferences":
    /// "Auto Leveling", "Extrusion Flow Calibration", "Time-lapse Camera"
    /// (`BED_LEVEL`, `FLOW_CALIBRATE`, `TIME_LAPSE_CAMERA`), all off unless
    /// ticked, as that page starts them.
    pub bed_level: bool,
    pub flow_calibrate: bool,
    pub time_lapse: bool,
}

/// One of a printer's toolheads and what is loaded in it (SnapmakerLan.hpp
/// `Toolhead`, read from `print_task_config`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Toolhead {
    pub index: usize,
    pub filament_type: String,
    pub sub_type: String,
    pub vendor: String,
    /// `#RRGGBB`, or empty when the printer gives none.
    pub color: String,
    /// `filament_exist`.
    pub loaded: bool,
    pub official: bool,
    /// `extruder<N>.nozzle_diameter`, 0 when not given.
    pub nozzle: f64,
}

/// What a printer is doing, as the Device tab shows it (SnapmakerLan.hpp
/// `Status`, MoonrakerProgress.hpp).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// `print_stats.state`: standby, printing, paused, complete,
    /// cancelled, error.
    pub state: String,
    pub filename: String,
    /// The printer's own message (`print_stats.message`).
    pub message: String,
    /// 0 to 100.
    pub percent: u32,
    /// Seconds left, 0 when unknown.
    pub left: u32,
    pub layer: u32,
    pub total_layers: u32,
    /// (temperature, target).
    pub bed: Option<(f64, f64)>,
    pub nozzles: Vec<(f64, f64)>,
}

/// How an upload is going, shared with whoever watches it; and the way to
/// call it off.
#[derive(Debug, Default)]
pub struct Progress {
    pub sent: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    /// What it is doing after the file is sent ("Starting the print").
    pub stage: Mutex<String>,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn set_stage(&self, text: &str) {
        *self.stage.lock().unwrap_or_else(|e| e.into_inner()) = text.to_owned();
    }
}

/// Why something was not done.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// The person called it off.
    Cancelled,
    /// A sentence to show.
    Error(String),
}

impl From<String> for Failure {
    fn from(text: String) -> Self {
        Failure::Error(text)
    }
}

/// A file on the printer, for the Device tab's "Storage".
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    /// Its path under the printer's G-code folder, `/` between folders.
    pub name: String,
    pub size: Option<u64>,
    /// Seconds since 1970.
    pub modified: Option<f64>,
    /// G-code the printer can run (`.gcode`, `.g`, `.gco`); `.bgcode` and
    /// `.3mf` are listed but not started (PrinterStorage.cpp
    /// `is_printable_name`).
    pub printable: bool,
    /// What the slicer wrote into it, when the printer says
    /// (`server/files/metadata`): seconds, millimetres, grams, types.
    pub print_time: Option<u32>,
    pub filament_mm: Option<f64>,
    pub filament_g: Option<f64>,
    pub filament_type: String,
}

/// One kind of print host.
pub trait Host: Send + Sync {
    /// "Test": whether the printer answers as this kind of host.
    fn test(&self) -> Result<(), String>;
    /// What "Test" says when it works, and when it does not (`reason`).
    fn test_ok(&self) -> String;
    fn test_failed(&self, reason: &str) -> String;
    /// Whether "Upload and Print" is offered.
    fn can_start(&self) -> bool {
        true
    }
    /// Sends the file, and starts it when asked.
    fn upload(&self, upload: &Upload, progress: &Progress) -> Result<(), Failure>;
    /// The files on the printer, when this host lists them.
    fn files(&self) -> Option<Result<Vec<Stored>, String>> {
        None
    }
    fn delete(&self, _name: &str) -> Result<(), String> {
        Err("this printer's files cannot be deleted from here".into())
    }
    fn print(&self, _name: &str) -> Result<(), String> {
        Err("this printer's files cannot be printed from here".into())
    }
    /// The printer's toolheads and what each holds, when this host says.
    fn toolheads(&self) -> Option<Result<Vec<Toolhead>, String>> {
        None
    }
    /// What the printer is doing, when this host says. Asks only; never
    /// tells the printer anything.
    fn status(&self) -> Option<Result<Status, String>> {
        None
    }
}

/// The host for a connection, or why there is none.
pub fn make(connection: &Connection, timeouts: Timeouts) -> Result<Box<dyn Host>, String> {
    if connection.print_host.trim().is_empty() {
        return Err("Please set up your printer connection to view the device.".into());
    }
    match connection.host_type.as_str() {
        // The fork sends to a U1 with Snapmaker's own page, not through
        // OctoPrint (PrintHostDevices.cpp `send_flow_for`): see `snapmaker`.
        "octoprint" if snapmaker::is_u1(&connection.printer_model) => Ok(Box::new(
            snapmaker::SnapmakerLan::new(connection, timeouts)?,
        )),
        "octoprint" => Ok(Box::new(octoprint::OctoPrint::new(connection, timeouts)?)),
        "elegoolink" => Ok(Box::new(elegoo::ElegooLink::new(connection, timeouts)?)),
        other => Err(format!(
            "This app cannot send to a {} host yet.",
            label_of(other)
        )),
    }
}

/// The printer's address, taken apart: what requests go to, and what is
/// shown. The reference puts the address straight into its URLs and its
/// log (OctoPrint.cpp l.246), with any `user:password@` in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Address {
    /// `http://host:port/prefix/`, always ending in `/`, without user and
    /// password.
    pub base: String,
    /// The host alone, without port or brackets.
    pub host: String,
    /// The port typed, if any.
    pub port: Option<u16>,
    pub https: bool,
    /// "Basic ..." for an address typed as `https://user:password@host/`
    /// (the `print_host` tooltip offers it, for a host behind a proxy).
    basic: Option<String>,
}

impl Address {
    /// As OctoPrint.cpp `make_url` reads it: `http://` when no scheme is
    /// given.
    pub fn parse(typed: &str) -> Result<Self, String> {
        let unparsable = || tr(UNPARSABLE);
        let typed = typed.trim();
        let (https, rest) = if let Some(rest) = typed.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = typed.strip_prefix("http://") {
            (false, rest)
        } else if typed.contains("://") {
            return Err(unparsable());
        } else {
            (false, typed)
        };
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(at) => (Some(&authority[..at]), &authority[at + 1..]),
            None => (None, authority),
        };
        let (host, port) = if let Some(inner) = hostport.strip_prefix('[') {
            let end = inner.find(']').ok_or_else(unparsable)?;
            let after = &inner[end + 1..];
            let port = match after.strip_prefix(':') {
                Some(port) => Some(port.parse::<u16>().map_err(|_| unparsable())?),
                None if after.is_empty() => None,
                None => return Err(unparsable()),
            };
            (format!("[{}]", &inner[..end]), port)
        } else {
            match hostport.rsplit_once(':') {
                Some((host, port)) => (
                    host.to_owned(),
                    Some(port.parse::<u16>().map_err(|_| unparsable())?),
                ),
                None => (hostport.to_owned(), None),
            }
        };
        let clean = |c: char| c.is_ascii_alphanumeric() || "-._[]:%".contains(c);
        if host.is_empty() || !host.chars().all(clean) {
            return Err(unparsable());
        }
        if path
            .chars()
            .any(|c| c.is_whitespace() || c == '?' || c == '#')
        {
            return Err(unparsable());
        }
        let basic = userinfo.map(|info| format!("Basic {}", base64(info.as_bytes())));
        let mut base = format!(
            "{}://{}{}{}",
            if https { "https" } else { "http" },
            host,
            port.map(|port| format!(":{port}")).unwrap_or_default(),
            path
        );
        if !base.ends_with('/') {
            base.push('/');
        }
        Ok(Self {
            base,
            host: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port,
            https,
            basic,
        })
    }

    /// A path under the address (`api/version`).
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path.trim_start_matches('/'))
    }

    /// The address as it may be shown: without user and password.
    pub fn shown(&self) -> &str {
        &self.base
    }
}

/// The page's own address for a printer: `print_host_webui` if set, else
/// `print_host`, with `http://` in front when it has no scheme (Plater.cpp
/// l.4304; MainFrame.cpp `load_printer_url`). Without user and password.
pub fn device_url(connection: &Connection) -> Option<String> {
    let typed = if connection.print_host_webui.trim().is_empty() {
        connection.print_host.trim()
    } else {
        connection.print_host_webui.trim()
    };
    if typed.is_empty() {
        return None;
    }
    Address::parse(typed).ok().map(|address| address.base)
}

/// Standard base64, for "Basic" authorisation.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | u32::from(*byte) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A random UUID (version 4), from the operating system's seed, as the
/// standard library's hashers take it: for the Centauri Carbon's upload
/// and its requests. Nothing secret depends on it.
pub fn uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let word = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    let (a, b) = (word(), word());
    let a = (a & !0xf000) | 0x4000; // version 4
    let b = (b & !(0xc000 << 48)) | (0x8000 << 48); // variant 1
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        a >> 32,
        (a >> 16) & 0xffff,
        a & 0xffff,
        b >> 48,
        b & 0xffff_ffff_ffff
    )
}

// ---- HTTP ----

/// The requests to one printer.
pub struct Http {
    pub address: Address,
    api_key: Option<String>,
    tls: ureq::tls::TlsConfig,
    pub timeouts: Timeouts,
}

/// An answer: the status and the body as text.
pub struct Answer {
    pub status: u16,
    pub body: String,
}

impl Answer {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }
}

impl Http {
    pub fn new(connection: &Connection, timeouts: Timeouts) -> Result<Self, String> {
        Ok(Self {
            address: Address::parse(&connection.print_host)?,
            api_key: connection.apikey.clone().filter(|key| !key.is_empty()),
            tls: tls_config(&connection.printhost_cafile)?,
            timeouts,
        })
    }

    /// An agent for one request: a whole question within `within`, or a
    /// file sent with its writes and the answer timed apart.
    fn agent(&self, within: Option<Duration>) -> ureq::Agent {
        let builder = ureq::Agent::config_builder()
            .http_status_as_error(false)
            // A redirect would carry the key to another host.
            .max_redirects(0)
            .user_agent("EdgeSlicerRS")
            .tls_config(self.tls.clone())
            .timeout_connect(Some(self.timeouts.connect));
        let builder = match within {
            Some(within) => builder.timeout_global(Some(within)),
            None => builder
                .timeout_per_write(Some(self.timeouts.write))
                .timeout_recv_response(Some(self.timeouts.answer))
                .timeout_recv_body(Some(self.timeouts.ask)),
        };
        builder.build().new_agent()
    }

    fn headers<B>(&self, mut request: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
        // OctoPrint.cpp `set_auth`: the key goes in a header, never the URL.
        if let Some(key) = &self.api_key {
            request = request.header("X-Api-Key", key);
        }
        if let Some(basic) = &self.address.basic {
            request = request.header("Authorization", basic);
        }
        request
    }

    pub fn get(&self, path: &str) -> Result<Answer, String> {
        let agent = self.agent(Some(self.timeouts.ask));
        read_answer(self.headers(agent.get(self.address.url(path))).call())
    }

    pub fn delete(&self, path: &str) -> Result<Answer, String> {
        let agent = self.agent(Some(self.timeouts.ask));
        read_answer(self.headers(agent.delete(self.address.url(path))).call())
    }

    /// A POST with a JSON body of `{}` (Moonraker's commands take their
    /// arguments in the query), answered within `within`.
    pub fn post_empty(&self, path: &str, within: Duration) -> Result<Answer, String> {
        let agent = self.agent(Some(within));
        let request = self
            .headers(agent.post(self.address.url(path)))
            .header("Content-Type", "application/json");
        read_answer(request.send("{}"))
    }

    /// A multipart form, the file last, its bytes counted into `progress`
    /// as they go and the sending stopped when it is called off. `counted`
    /// is the bytes of the file sent before this request, and in it.
    pub fn post_form(
        &self,
        path: &str,
        form: &Form,
        progress: &Progress,
        counted: (u64, u64),
    ) -> Result<Answer, Failure> {
        let (body, content_type) = form.body();
        let length = body.len() as u64;
        let mut reader = Counted {
            inner: std::io::Cursor::new(body),
            progress,
            before: counted.0,
            file_bytes: counted.1,
            body_bytes: length,
            read: 0,
        };
        let agent = self.agent(None);
        let request = self
            .headers(agent.post(self.address.url(path)))
            .header("Content-Type", &content_type)
            .header("Content-Length", &length.to_string());
        let sent = request.send(ureq::SendBody::from_reader(&mut reader));
        if progress.cancelled() {
            return Err(Failure::Cancelled);
        }
        read_answer(sent).map_err(Failure::Error)
    }
}

/// The TLS settings: the CA file's certificates when one is given ("HTTPS
/// CA File"), else Mozilla's. The reference accepts any certificate from a
/// printer without a CA file (PrintHost.cpp `apply_tls`,
/// `tls_policy_without_ca`); here a printer behind HTTPS with a
/// certificate of its own needs that file.
fn tls_config(cafile: &str) -> Result<ureq::tls::TlsConfig, String> {
    let builder = ureq::tls::TlsConfig::builder();
    if cafile.trim().is_empty() {
        return Ok(builder.build());
    }
    let pem = std::fs::read(cafile.trim())
        .map_err(|error| format!("The HTTPS CA file could not be read ({error})."))?;
    let certificates: Vec<_> = ureq::tls::parse_pem(&pem)
        .filter_map(|item| match item {
            Ok(ureq::tls::PemItem::Certificate(certificate)) => Some(certificate),
            _ => None,
        })
        .collect();
    if certificates.is_empty() {
        return Err("The HTTPS CA file holds no certificate.".into());
    }
    Ok(builder
        .root_certs(ureq::tls::RootCerts::new_with_certs(&certificates))
        .build())
}

/// A request's answer read whole, or what went wrong said as the
/// reference says it (PrintHost.cpp `format_error`). ureq's own text is
/// not shown: it can hold the URL.
fn read_answer(
    sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Answer, String> {
    match sent {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let body = response
                .body_mut()
                .with_config()
                .limit(16 * 1024 * 1024)
                .read_to_string()
                .map_err(said)?;
            Ok(Answer { status, body })
        }
        Err(error) => Err(said(error)),
    }
}

/// An error of the connection in the reference's words.
pub fn said(error: ureq::Error) -> String {
    use ureq::Error as E;
    match error {
        E::Timeout(_) => tr(TIMED_OUT),
        E::HostNotFound | E::BadUri(_) => tr(UNPARSABLE),
        E::Io(io) => io_said(&io),
        E::ConnectionFailed => "Could not connect to the printer.".into(),
        E::Tls(_) | E::Pem(_) | E::Rustls(_) => {
            "The printer's HTTPS certificate is not trusted. Give its CA file under \"HTTPS CA File\".".into()
        }
        _ => "The printer's answer could not be read.".into(),
    }
}

/// The reference's words (PrintHost.cpp `format_error`, l.251–255).
pub const UNPARSABLE: &str =
    "The Hostname/IP/URL could not be parsed, please check it and try again.";
pub const TIMED_OUT: &str = "Connection timed out. Please check if the printer and computer network are functioning properly, and confirm that they are on the same network.";

pub(crate) fn io_said(io: &std::io::Error) -> String {
    use std::io::ErrorKind as K;
    match io.kind() {
        K::TimedOut | K::WouldBlock => tr(TIMED_OUT),
        K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof => tr(
            "File/data transfer interrupted. Please check the printer and network, then try it again.",
        ),
        K::ConnectionRefused => {
            "Could not connect to the printer: the connection was refused.".into()
        }
        _ => format!("Could not connect to the printer ({}).", io.kind()),
    }
}

/// An HTTP error as the reference words it: "HTTP 401: <reason or body>",
/// with Moonraker's own reason when it gave one (PrintHost.cpp
/// `moonraker_error_reason`, which this reads more simply: the message).
pub fn http_error(answer: &Answer) -> String {
    let reason = answer
        .json()
        .and_then(|json| {
            json.get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            let body = answer.body.trim();
            if body.len() > 300 || body.starts_with('<') {
                String::new()
            } else {
                body.to_owned()
            }
        });
    format!("HTTP {}: {}", answer.status, reason)
        .trim_end()
        .trim_end_matches(':')
        .to_owned()
}

/// A form's file: its field, its name, the bytes and the part of them sent.
type FilePart = (String, String, Arc<Vec<u8>>, std::ops::Range<usize>);

/// A multipart form.
pub struct Form {
    fields: Vec<(String, String)>,
    file: Option<FilePart>,
}

impl Form {
    pub fn new() -> Self {
        Self {
            fields: Vec::new(),
            file: None,
        }
    }

    pub fn field(mut self, name: &str, value: &str) -> Self {
        self.fields.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The file part: `name` the field, `file_name` what the printer calls
    /// it, `range` the part of `data` sent.
    pub fn file(
        mut self,
        name: &str,
        file_name: &str,
        data: Arc<Vec<u8>>,
        range: std::ops::Range<usize>,
    ) -> Self {
        self.file = Some((name.to_owned(), file_name.to_owned(), data, range));
        self
    }

    /// The body and its content type.
    fn body(&self) -> (Vec<u8>, String) {
        let boundary = format!("------------------------{}", uuid().replace('-', ""));
        let mut body = Vec::new();
        for (name, value) in &self.fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{value}\r\n",
                    quoted(name)
                )
                .as_bytes(),
            );
        }
        if let Some((name, file_name, data, range)) = &self.file {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                    quoted(name),
                    quoted(file_name)
                )
                .as_bytes(),
            );
            body.extend_from_slice(&data[range.clone()]);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={boundary}"))
    }
}

/// A name in a form's header: a quote or a line break in it would end
/// the header early (the reference hands curl the name as it is).
fn quoted(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' => '\'',
            '\r' | '\n' => ' ',
            c => c,
        })
        .collect()
}

/// A body read out to the connection, counted as it goes: the share of
/// the file it stands for is added to `progress`.
struct Counted<'a> {
    inner: std::io::Cursor<Vec<u8>>,
    progress: &'a Progress,
    /// Bytes of the file sent before this request, and in it.
    before: u64,
    file_bytes: u64,
    body_bytes: u64,
    read: u64,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.progress.cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        // Small pieces, so a cancel is seen soon and progress moves.
        let at_most = buf.len().min(64 * 1024);
        let read = self.inner.read(&mut buf[..at_most])?;
        self.read += read as u64;
        let share = (self.file_bytes * self.read)
            .checked_div(self.body_bytes)
            .unwrap_or(0);
        self.progress
            .sent
            .store(self.before + share, Ordering::Relaxed);
        Ok(read)
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn addresses_are_read_as_the_reference_reads_them() {
        let plain = Address::parse("192.168.1.20").unwrap();
        assert_eq!(plain.base, "http://192.168.1.20/");
        assert_eq!(plain.url("api/version"), "http://192.168.1.20/api/version");
        let port = Address::parse(" http://printer.local:7125 ").unwrap();
        assert_eq!(port.base, "http://printer.local:7125/");
        assert_eq!(
            (port.host.as_str(), port.port),
            ("printer.local", Some(7125))
        );
        let prefix = Address::parse("https://octo.example/octoprint").unwrap();
        assert_eq!(
            prefix.url("api/version"),
            "https://octo.example/octoprint/api/version"
        );
        assert!(prefix.https);
        // A user and password are taken out of what is shown and sent as
        // a header instead.
        let secret = Address::parse("https://me:s3cret@octo.example/").unwrap();
        assert_eq!(secret.shown(), "https://octo.example/");
        assert_eq!(secret.basic.as_deref(), Some("Basic bWU6czNjcmV0"));
        assert!(
            !format!("{secret:?}")
                .replace("bWU6czNjcmV0", "")
                .contains("s3cret")
        );
        let v6 = Address::parse("http://[fe80::1]:80/").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("fe80::1", Some(80)));
        for bad in ["", "ftp://x", "http://", "host:port", "a b", "http://h/x?y"] {
            assert!(Address::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_device_page_is_the_webui_else_the_host() {
        let mut connection = Connection {
            print_host: "10.0.0.5".into(),
            ..Connection::default()
        };
        assert_eq!(device_url(&connection).as_deref(), Some("http://10.0.0.5/"));
        connection.print_host_webui = "http://10.0.0.5:81".into();
        assert_eq!(
            device_url(&connection).as_deref(),
            Some("http://10.0.0.5:81/")
        );
        connection.print_host_webui = "http://u:p@10.0.0.5:81".into();
        assert_eq!(
            device_url(&connection).as_deref(),
            Some("http://10.0.0.5:81/")
        );
        assert_eq!(device_url(&Connection::default()), None);
    }

    #[test]
    fn base64_and_uuids() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(uuid(), id);
    }
}
