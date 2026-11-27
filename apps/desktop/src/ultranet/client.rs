//! The app's side of the bridge (ULTRANET_BRIDGE.md): starting it,
//! `hello` and the version checks, requests with their time-outs, the
//! events (the printer's connection and reports, the jobs), and what
//! happens when it dies: every request and job waiting ends, and the next
//! thing that needs it starts it again, at most once every 5 seconds and
//! not after 3 deaths in a minute until "Retry".
//!
//! Every text the bridge sends is cleared of the access codes this app
//! keeps before anything else sees it.

use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use serde_json::{Value, json};

#[cfg(test)]
use super::link::Link;
use super::machine::Machine;

/// The protocol this app speaks (ULTRANET_BRIDGE.md section 4).
pub const PROTOCOL: i64 = 1;

/// The methods a bridge must answer for this app.
pub const NEEDED: [&str; 6] = [
    "connect_printer",
    "disconnect_printer",
    "send_message_to_printer",
    "start_local_print",
    "start_send_gcode_to_sdcard",
    "cancel_job",
];

/// The methods of the account (ULTRANET_BRIDGE.md section 16, minor 1)
/// "Log in to Bambu Account..." needs.
pub const ACCOUNT: [&str; 4] = [
    "account_sign_in",
    "account_restore",
    "account_sign_out",
    "account_printers",
];

/// The methods of the proposed later minor (ULTRANET_BRIDGE.md section
/// 15) each part of the Bambu Device page needs; a bridge of protocol 1.0
/// offers none of them, and the part says so.
pub const CAMERA: [&str; 2] = ["camera_start", "camera_stop"];
pub const FILES: [&str; 2] = ["files_list", "files_delete"];
pub const UPDATE: [&str; 1] = ["firmware_upgrade"];

/// The largest camera frame kept (base64 of a JPEG of 4 MiB).
const LARGEST_FRAME: usize = 6 << 20;

/// The longest line read (section 3).
const LONGEST_LINE: usize = 4 << 20;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// How long each wait may be. The tests make them short.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// The answer to `hello` (the plug-in is loaded then).
    pub hello: Duration,
    /// Any other short request.
    pub ask: Duration,
    /// `local_connect` after `connect_printer`.
    pub connect: Duration,
    /// The printer taking a print it was sent (`PRINT_JOB_SENDING_TIMEOUT`, 25 s).
    pub started: Duration,
    /// Between starts of a bridge that died.
    pub restart: Duration,
    /// An `account_*` request, which calls the cloud (section 16.3: the
    /// bridge answers within 150 s).
    pub account: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            hello: Duration::from_secs(10),
            ask: Duration::from_secs(15),
            connect: Duration::from_secs(20),
            started: Duration::from_secs(25),
            restart: Duration::from_secs(5),
            account: Duration::from_secs(160),
        }
    }
}

/// How the bridge is reached.
#[derive(Clone)]
pub enum Launcher {
    /// No bridge was found.
    Absent,
    /// The bridge's executable.
    Program(PathBuf),
    /// A bridge made by a test.
    #[cfg(test)]
    Test(Arc<dyn Fn() -> Link + Send + Sync>),
}

/// What `hello` was answered.
#[derive(Clone, Debug, Default)]
pub struct Hello {
    pub protocol: i64,
    pub minor: i64,
    pub bridge_version: String,
    pub plugin_version: String,
    pub plugin_expected: String,
    pub methods: Vec<String>,
}

/// The state of a printer's LAN session.
#[derive(Clone, Debug, PartialEq)]
pub enum Session {
    Idle,
    Connecting,
    Connected,
    /// The plug-in said it failed (`msg`), or nothing came in time.
    Failed(String),
    Lost,
}

/// A printer as the bridge reports it.
#[derive(Clone, Debug)]
pub struct Remote {
    pub machine: Machine,
    pub session: Session,
    /// The last dial was refused for the access code
    /// (`lan_access_code_refused`).
    pub refused: bool,
    pub since: Instant,
}

impl Default for Remote {
    fn default() -> Self {
        Remote {
            machine: Machine::default(),
            session: Session::Idle,
            refused: false,
            since: Instant::now(),
        }
    }
}

/// What the bridge said of one of its jobs.
#[derive(Clone, Debug, Default)]
pub struct JobState {
    /// `job_status`: (stage, code, info), in order.
    pub statuses: Vec<(i64, i64, String)>,
    /// `job_done`'s `ret`.
    pub done: Option<i64>,
    /// Why it ended without `job_done` (the bridge died).
    pub lost: Option<String>,
}

/// What is shared with the thread that reads the bridge.
#[derive(Default)]
struct Shared {
    printers: Mutex<HashMap<String, Remote>>,
    /// The latest camera frame of each printer (`camera_frame`): its
    /// number and the JPEG as base64.
    frames: Mutex<HashMap<String, (u64, String)>>,
    jobs: Mutex<HashMap<u64, JobState>>,
    codes: Mutex<Vec<String>>,
    /// `account_expired` came (section 16.2), not yet seen by the session.
    expired: AtomicBool,
    /// Woken on every event.
    changed: Condvar,
    changed_lock: Mutex<u64>,
}

impl Shared {
    fn touch(&self) {
        *lock(&self.changed_lock) += 1;
        self.changed.notify_all();
    }

    /// Waits until `ready` holds or `deadline` passes.
    fn wait_for(&self, deadline: Instant, mut ready: impl FnMut() -> bool) -> bool {
        let mut seen = lock(&self.changed_lock);
        loop {
            drop(seen);
            if ready() {
                return true;
            }
            seen = lock(&self.changed_lock);
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let wait = (deadline - now).min(Duration::from_millis(100));
            seen = self
                .changed
                .wait_timeout(seen, wait)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }

    fn scrub(&self, text: &str) -> String {
        scrub(text, &lock(&self.codes))
    }
}

/// `text` without any of `codes` in it.
pub fn scrub(text: &str, codes: &[String]) -> String {
    let mut out = text.to_owned();
    for code in codes.iter().filter(|code| code.len() >= 4) {
        if out.contains(code.as_str()) {
            out = out.replace(code.as_str(), "********");
        }
    }
    out
}

/// An answer to a request: its result, or why not.
type Answer = Result<Value, String>;

/// One running bridge.
struct Conn {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Answer>>>,
    next: AtomicU64,
    alive: AtomicBool,
    stop: Mutex<Box<dyn FnMut() + Send>>,
    late: AtomicU32,
}

const STOPPED: &str = "The UltraNet bridge stopped.";

impl Conn {
    fn write(&self, message: &Value) -> Result<(), String> {
        let mut line = message.to_string();
        line.push('\n');
        let mut writer = lock(&self.writer);
        writer
            .write_all(line.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|_| STOPPED.to_owned())
    }

    /// A request whose answer is not waited for.
    fn tell(&self, method: &str, params: Value) {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let _ =
            self.write(&json!({ "type": "request", "id": id, "method": method, "params": params }));
    }

    fn ask(&self, method: &str, params: Value, timeout: Duration) -> Answer {
        self.ask_counted(method, params, timeout, true)
    }

    /// `late`: whether a time-out counts towards the bridge taken for
    /// dead (not for the account's long calls, section 16.3).
    fn ask_counted(&self, method: &str, params: Value, timeout: Duration, late: bool) -> Answer {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(STOPPED.into());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (send, receive) = mpsc::channel();
        lock(&self.pending).insert(id, send);
        if let Err(error) =
            self.write(&json!({ "type": "request", "id": id, "method": method, "params": params }))
        {
            lock(&self.pending).remove(&id);
            return Err(error);
        }
        match receive.recv_timeout(timeout) {
            Ok(answer) => {
                self.late.store(0, Ordering::SeqCst);
                answer
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.pending).remove(&id);
                // Twice in a row: the bridge is taken for dead (section 3).
                if late && self.late.fetch_add(1, Ordering::SeqCst) >= 1 {
                    self.kill();
                }
                Err("The UltraNet bridge did not answer in time.".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(STOPPED.into()),
        }
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        lock(&self.pending).clear();
        (lock(&self.stop))();
    }
}

/// The bridge, as far as this app is concerned.
#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// No bridge was found.
    Absent,
    /// Found, not started yet (or stopped by this app).
    Idle,
    Running,
    /// Started, but refused: too old, too new, no plug-in, a plug-in it
    /// cannot use. The sentence says which.
    Refused(String),
    /// It died, or would not start.
    Failed(String),
    /// It died 3 times in a minute: not started again until "Retry".
    GaveUp(String),
}

struct Life {
    state: State,
    hello: Option<Hello>,
    deaths: Vec<Instant>,
    last_start: Option<Instant>,
}

pub struct Client {
    launcher: Launcher,
    pub timeouts: Timeouts,
    shared: Arc<Shared>,
    conn: Mutex<Option<Arc<Conn>>>,
    life: Mutex<Life>,
    /// `hello`'s directories.
    data_dir: Option<PathBuf>,
}

impl Client {
    pub fn new(launcher: Launcher, data_dir: Option<PathBuf>, timeouts: Timeouts) -> Self {
        let state = match launcher {
            Launcher::Absent => State::Absent,
            _ => State::Idle,
        };
        Client {
            launcher,
            timeouts,
            shared: Arc::default(),
            conn: Mutex::new(None),
            life: Mutex::new(Life {
                state,
                hello: None,
                deaths: Vec::new(),
                last_start: None,
            }),
            data_dir,
        }
    }

    /// Where the bridge was found (for the page: its file name only).
    pub fn found(&self) -> Option<String> {
        match &self.launcher {
            Launcher::Program(path) => Some(path.to_string_lossy().into_owned()),
            Launcher::Absent => None,
            #[cfg(test)]
            Launcher::Test(_) => Some("(test)".into()),
        }
    }

    pub fn state(&self) -> State {
        self.check_alive();
        lock(&self.life).state.clone()
    }

    pub fn hello(&self) -> Option<Hello> {
        lock(&self.life).hello.clone()
    }

    /// Codes to take out of what the bridge says: every code known in
    /// this run stays on the list, a forgotten one too.
    pub fn set_codes(&self, codes: Vec<String>) {
        let mut known = lock(&self.shared.codes);
        for code in codes {
            if !known.contains(&code) {
                known.push(code);
            }
        }
    }

    pub fn scrub(&self, text: &str) -> String {
        self.shared.scrub(text)
    }

    /// "Retry": starts again after it gave up.
    pub fn retry(&self) {
        let mut life = lock(&self.life);
        if matches!(
            life.state,
            State::GaveUp(_) | State::Failed(_) | State::Refused(_)
        ) {
            life.state = State::Idle;
            life.deaths.clear();
            life.last_start = None;
        }
    }

    /// Notes a bridge that died since it was last looked at.
    fn check_alive(&self) {
        let dead = {
            let conn = lock(&self.conn);
            conn.as_ref()
                .is_some_and(|c| !c.alive.load(Ordering::SeqCst))
        };
        if dead {
            lock(&self.conn).take();
            let mut life = lock(&self.life);
            if life.state == State::Running {
                let now = Instant::now();
                life.deaths
                    .retain(|at| now.duration_since(*at) < Duration::from_secs(60));
                life.deaths.push(now);
                life.state = if life.deaths.len() >= 3 {
                    State::GaveUp("The UltraNet bridge keeps stopping.".into())
                } else {
                    State::Failed(STOPPED.into())
                };
            }
        }
    }

    /// The running bridge, started if need be.
    fn conn(&self) -> Result<Arc<Conn>, String> {
        self.check_alive();
        if let Some(conn) = lock(&self.conn).as_ref() {
            return Ok(conn.clone());
        }
        let mut life = lock(&self.life);
        match &life.state {
            State::Absent => return Err(absent()),
            State::GaveUp(why) | State::Refused(why) => return Err(why.clone()),
            State::Failed(_) | State::Idle | State::Running => {}
        }
        if let Some(at) = life.last_start
            && at.elapsed() < self.timeouts.restart
            && matches!(life.state, State::Failed(_))
        {
            return Err(STOPPED.into());
        }
        life.last_start = Some(Instant::now());
        let link = match &self.launcher {
            Launcher::Absent => return Err(absent()),
            Launcher::Program(path) => match super::link::spawn(path) {
                Ok(link) => link,
                Err(why) => {
                    life.state = State::Failed(why.clone());
                    return Err(why);
                }
            },
            #[cfg(test)]
            Launcher::Test(make) => make(),
        };
        let conn = Arc::new(Conn {
            writer: Mutex::new(link.writer),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            stop: Mutex::new(link.stop),
            late: AtomicU32::new(0),
        });
        {
            let conn = conn.clone();
            let shared = self.shared.clone();
            let reader = link.reader;
            let started = self.timeouts.started;
            std::thread::spawn(move || read_bridge(reader, conn, shared, started));
        }
        let dir = |leaf: &str| {
            self.data_dir
                .as_ref()
                .map(|dir| {
                    dir.join("ultranet")
                        .join(leaf)
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_default()
        };
        let answer = conn.ask(
            "hello",
            json!({
                "protocol": PROTOCOL,
                "client": "EdgeSlicerRS",
                "client_version": env!("CARGO_PKG_VERSION"),
                "log_dir": dir("log"),
                "config_dir": dir("config"),
            }),
            self.timeouts.hello,
        );
        let refuse = |life: &mut Life, why: String| {
            conn.tell("bye", json!({}));
            conn.kill();
            life.state = State::Refused(why.clone());
            Err(why)
        };
        let result = match answer {
            Ok(result) => result,
            Err(error) => {
                // A bridge that cannot speak protocol 1 answers -101.
                if let Some(theirs) = error.strip_prefix("-101 ") {
                    return refuse(&mut life, too_old_or_new(theirs.trim().parse().ok()));
                }
                conn.kill();
                let why = self.shared.scrub(&error);
                life.state = State::Failed(why.clone());
                return Err(why);
            }
        };
        let hello = Hello {
            protocol: result["protocol"].as_i64().unwrap_or(0),
            minor: result["minor"].as_i64().unwrap_or(0),
            bridge_version: result["bridge_version"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            plugin_version: result["plugin_version"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            plugin_expected: result["plugin_expected"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            methods: result["methods"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str().map(str::to_owned))
                .collect(),
        };
        life.hello = Some(hello.clone());
        if hello.protocol != PROTOCOL {
            return refuse(&mut life, too_old_or_new(Some(hello.protocol)));
        }
        if let Some(missing) = NEEDED
            .iter()
            .find(|m| !hello.methods.iter().any(|h| h == *m))
        {
            return refuse(
                &mut life,
                format!(
                    "The UltraNet bridge is too old for this app: it does not offer {missing}."
                ),
            );
        }
        if result["plugin_loaded"] != json!(true) {
            let why = self
                .shared
                .scrub(result["plugin_error"].as_str().unwrap_or_default());
            return refuse(
                &mut life,
                format!(
                    "{}{}",
                    crate::api::i18n::tr("Network plug-in missing"),
                    if why.is_empty() {
                        String::new()
                    } else {
                        format!(": {why}")
                    }
                ),
            );
        }
        if result["plugin_compatible"] != json!(true) {
            return refuse(
                &mut life,
                format!(
                    "The UltraNet bridge cannot use its network plug-in (plug-in version {}, needed {}).",
                    hello.plugin_version, hello.plugin_expected
                ),
            );
        }
        life.state = State::Running;
        *lock(&self.conn) = Some(conn.clone());
        Ok(conn)
    }

    /// Whether the running bridge offers all of `methods` (section 15's
    /// later minor); false before `hello`.
    pub fn offers(&self, methods: &[&str]) -> bool {
        lock(&self.life)
            .hello
            .as_ref()
            .is_some_and(|hello| methods.iter().all(|m| hello.methods.iter().any(|h| h == m)))
    }

    /// The latest camera frame of a printer: its number and the JPEG as
    /// base64.
    pub fn frame(&self, dev_id: &str) -> Option<(u64, String)> {
        lock(&self.shared.frames).get(dev_id).cloned()
    }

    pub fn forget_frame(&self, dev_id: &str) {
        lock(&self.shared.frames).remove(dev_id);
    }

    /// Starts the bridge if need be (Device, the send dialog).
    pub fn start(&self) -> Result<(), String> {
        self.conn().map(|_| ())
    }

    /// A short request, its result.
    pub fn ask(&self, method: &str, params: Value) -> Result<Value, String> {
        let conn = self.conn()?;
        let answer = conn.ask(method, params, self.timeouts.ask);
        self.check_alive();
        answer.map_err(|error| self.shared.scrub(&error))
    }

    /// An `account_*` request (section 16.3): the long wait, and when it
    /// is not answered in time `account_sign_out`, so the bridge keeps no
    /// session this app does not know of.
    pub fn ask_account(&self, method: &str, params: Value) -> Result<Value, String> {
        let conn = self.conn()?;
        let answer = conn.ask_counted(method, params, self.timeouts.account, false);
        if answer.is_err() && conn.alive.load(Ordering::SeqCst) {
            conn.tell("account_sign_out", json!({}));
        }
        self.check_alive();
        answer.map_err(|error| self.shared.scrub(&error))
    }

    /// Whether `account_expired` came since this was last asked.
    pub fn take_expired(&self) -> bool {
        self.shared.expired.swap(false, Ordering::SeqCst)
    }

    /// Whether a bridge is running now (nothing is started to find out).
    pub fn running(&self) -> bool {
        self.check_alive();
        lock(&self.conn).is_some()
    }

    /// Stops the bridge (`bye`).
    pub fn shutdown(&self) {
        if let Some(conn) = lock(&self.conn).take() {
            let _ = conn.ask("bye", json!({}), Duration::from_secs(3));
            conn.kill();
        }
        let mut life = lock(&self.life);
        if life.state == State::Running {
            life.state = State::Idle;
        }
    }

    /// The printer as last reported.
    pub fn remote(&self, dev_id: &str) -> Remote {
        lock(&self.shared.printers)
            .get(dev_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Forgets what is known of a printer.
    pub fn forget(&self, dev_id: &str) {
        lock(&self.shared.printers).remove(dev_id);
    }

    /// `MachineObject::connect`: dials the printer and waits for the
    /// plug-in's answer. The one session another printer had ends.
    pub fn connect(&self, dev_id: &str, ip: &str, code: &str) -> Result<Session, String> {
        {
            let mut printers = lock(&self.shared.printers);
            for (id, remote) in printers.iter_mut() {
                if id != dev_id
                    && matches!(remote.session, Session::Connected | Session::Connecting)
                {
                    remote.session = Session::Idle;
                }
            }
            let remote = printers.entry(dev_id.to_owned()).or_default();
            remote.session = Session::Connecting;
            remote.since = Instant::now();
        }
        let answer = self.ask(
            "connect_printer",
            json!({ "dev_id": dev_id, "dev_ip": ip, "username": "bblp", "password": code, "use_ssl": true }),
        );
        let set = |session: Session| {
            lock(&self.shared.printers)
                .entry(dev_id.to_owned())
                .or_default()
                .session = session.clone();
            self.shared.touch();
            session
        };
        match answer {
            Err(error) => {
                set(Session::Failed(error.clone()));
                return Err(error);
            }
            Ok(result) if result["ret"].as_i64().unwrap_or(-1) != 0 => {
                let why = format!("connect_printer: {}", result["ret"]);
                return Ok(set(Session::Failed(why)));
            }
            Ok(_) => {}
        }
        let deadline = Instant::now() + self.timeouts.connect;
        let came = self.shared.wait_for(deadline, || {
            !matches!(
                lock(&self.shared.printers).get(dev_id).map(|r| &r.session),
                Some(Session::Connecting)
            )
        });
        if !came {
            return Ok(set(Session::Failed(
                "Synchronizing device information time out".into(),
            )));
        }
        Ok(self.remote(dev_id).session)
    }

    /// `send_message_to_printer`.
    pub fn publish(&self, dev_id: &str, message: &Value, qos: i64) -> Result<i64, String> {
        let result = self.ask(
            "send_message_to_printer",
            json!({ "dev_id": dev_id, "json": message.to_string(), "qos": qos, "flag": 0 }),
        )?;
        Ok(result["ret"].as_i64().unwrap_or(-1))
    }

    /// Starts one of the four `start_*` jobs; its number.
    pub fn start_job(&self, method: &str, params: Value) -> Result<u64, String> {
        let result = self.ask(method, params)?;
        let job = result["job"]
            .as_u64()
            .ok_or("The UltraNet bridge gave no job number.")?;
        lock(&self.shared.jobs).entry(job).or_default();
        Ok(job)
    }

    pub fn cancel_job(&self, job: u64) {
        if let Some(conn) = lock(&self.conn).as_ref() {
            conn.tell("cancel_job", json!({ "job": job }));
        }
    }

    pub fn job(&self, job: u64) -> JobState {
        lock(&self.shared.jobs)
            .get(&job)
            .cloned()
            .unwrap_or_default()
    }

    /// Waits for a job to end, telling `progress` of each status; ends
    /// with `job_done`'s `ret`, or why there was none. `cancelled` is
    /// asked as it goes and, once true, cancels the job.
    pub fn wait_job(
        &self,
        job: u64,
        mut progress: impl FnMut(i64, i64, &str),
        cancelled: impl Fn() -> bool,
    ) -> Result<i64, String> {
        let mut seen = 0;
        let mut told_cancel = false;
        loop {
            let state = self.job(job);
            for (stage, code, info) in state.statuses.iter().skip(seen) {
                progress(*stage, *code, info);
            }
            seen = state.statuses.len();
            if let Some(ret) = state.done {
                lock(&self.shared.jobs).remove(&job);
                return Ok(ret);
            }
            if let Some(why) = state.lost {
                lock(&self.shared.jobs).remove(&job);
                return Err(why);
            }
            if !told_cancel && cancelled() {
                told_cancel = true;
                self.cancel_job(job);
            }
            self.check_alive();
            let deadline = Instant::now() + Duration::from_millis(200);
            let before = seen;
            self.shared.wait_for(deadline, || {
                let state = self.job(job);
                state.statuses.len() != before || state.done.is_some() || state.lost.is_some()
            });
        }
    }

    /// Waits until `ready` holds for the printer, or `within` passes.
    pub fn wait_printer(
        &self,
        dev_id: &str,
        within: Duration,
        ready: impl Fn(&Remote) -> bool,
    ) -> bool {
        self.shared
            .wait_for(Instant::now() + within, || ready(&self.remote(dev_id)))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn absent() -> String {
    crate::api::i18n::tr("Network plug-in missing")
}

fn too_old_or_new(theirs: Option<i64>) -> String {
    match theirs {
        Some(n) if n > PROTOCOL => format!(
            "The UltraNet bridge is too new for this app (protocol {n}, this app speaks {PROTOCOL})."
        ),
        Some(n) => format!(
            "The UltraNet bridge is too old for this app (protocol {n}, this app needs {PROTOCOL})."
        ),
        None => format!("The UltraNet bridge cannot speak protocol {PROTOCOL}."),
    }
}

/// Reads the bridge until it ends: answers to the requests waiting, and
/// the events.
fn read_bridge(
    mut reader: Box<dyn std::io::BufRead + Send>,
    conn: Arc<Conn>,
    shared: Arc<Shared>,
    started: Duration,
) {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = {
            use std::io::{BufRead, Read};
            (&mut reader)
                .take(LONGEST_LINE as u64 + 1)
                .read_until(b'\n', &mut line)
        };
        match read {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() > LONGEST_LINE => break,
            Ok(_) => {}
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        match message["type"].as_str() {
            Some("response") => {
                let Some(id) = message["id"].as_u64() else {
                    continue;
                };
                let Some(waiting) = lock(&conn.pending).remove(&id) else {
                    continue;
                };
                let answer = match message.get("error") {
                    // -101 carries the bridge's protocol (section 4).
                    Some(error) if error["code"] == -101 => Err(format!(
                        "-101 {}",
                        error["data"]["protocol"].as_i64().unwrap_or(0)
                    )),
                    Some(error) => Err(format!(
                        "{} {}",
                        error["code"].as_i64().unwrap_or(0),
                        error["message"].as_str().unwrap_or_default()
                    )),
                    None => Ok(message["result"].clone()),
                };
                let _ = waiting.send(answer);
            }
            Some("event") => event(&message, &conn, &shared, started),
            _ => {}
        }
    }
    // Dead: everything waiting ends.
    conn.alive.store(false, Ordering::SeqCst);
    lock(&conn.pending).clear();
    for job in lock(&shared.jobs).values_mut() {
        if job.done.is_none() {
            job.lost = Some(STOPPED.into());
        }
    }
    for remote in lock(&shared.printers).values_mut() {
        if matches!(remote.session, Session::Connected | Session::Connecting) {
            remote.session = Session::Lost;
        }
    }
    shared.touch();
    (lock(&conn.stop))();
}

/// The sequence number of the commands this app sends.
static SEQUENCE: AtomicU64 = AtomicU64::new(20_000);

pub fn sequence() -> String {
    SEQUENCE.fetch_add(1, Ordering::SeqCst).to_string()
}

/// `LanAccessCode::is_auth_refusal` (LanAccessCodeWatch.hpp l.67).
pub fn is_auth_refusal(message: &str) -> bool {
    if message == "5" {
        return true;
    }
    let m = message.to_lowercase();
    for code in ["rc=5", "rc=134", "rc=135"] {
        if let Some(at) = m.find(code) {
            let end = at + code.len();
            if !m[end..].starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    m.contains("not authori") || m.contains("bad user name or password")
}

fn event(message: &Value, conn: &Arc<Conn>, shared: &Arc<Shared>, started: Duration) {
    let p = &message["params"];
    let text = |key: &str| shared.scrub(p[key].as_str().unwrap_or_default());
    // An identifier is not cleared: a serial may hold eight characters
    // that happen to be a code.
    let id = |key: &str| p[key].as_str().unwrap_or_default().to_owned();
    match message["event"].as_str() {
        Some("local_connect") => {
            let dev_id = id("dev_id");
            let status = p["status"].as_i64().unwrap_or(1);
            let msg = text("msg");
            let mut printers = lock(&shared.printers);
            let remote = printers.entry(dev_id.clone()).or_default();
            remote.since = Instant::now();
            match status {
                0 => {
                    remote.session = Session::Connected;
                    remote.refused = false;
                    // GUI_App.cpp l.2681: a full report, the versions.
                    conn.tell(
                        "send_message_to_printer",
                        json!({ "dev_id": dev_id, "json": json!({ "pushing": { "sequence_id": sequence(), "command": "pushall", "version": 1, "push_target": 1 } }).to_string(), "qos": 0, "flag": 0 }),
                    );
                    conn.tell(
                        "send_message_to_printer",
                        json!({ "dev_id": dev_id, "json": json!({ "info": { "sequence_id": sequence(), "command": "get_version" } }).to_string(), "qos": 1, "flag": 0 }),
                    );
                }
                2 => remote.session = Session::Lost,
                _ => {
                    remote.refused = is_auth_refusal(&msg);
                    remote.session = Session::Failed(msg);
                }
            }
        }
        Some("local_message") => {
            let dev_id = id("dev_id");
            let msg = text("msg");
            let mut printers = lock(&shared.printers);
            let remote = printers.entry(dev_id).or_default();
            // A report only comes over a live session.
            if remote.session != Session::Connected {
                remote.session = Session::Connected;
                remote.refused = false;
            }
            remote.machine.read(&msg);
        }
        // Section 15 (proposed): a frame of the camera the app started;
        // only a JPEG (base64 of FF D8 FF begins "/9j/"), of a printer the
        // session knows, and not too large, is kept.
        Some("camera_frame") => {
            let dev_id = id("dev_id");
            let jpeg = p["jpeg"].as_str().unwrap_or_default();
            if jpeg.starts_with("/9j/")
                && jpeg.len() <= LARGEST_FRAME
                && jpeg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
                && lock(&shared.printers).contains_key(&dev_id)
            {
                lock(&shared.frames)
                    .insert(dev_id, (p["seq"].as_u64().unwrap_or(0), jpeg.to_owned()));
            }
        }
        // Section 16: the bridge ended the account's session.
        Some("account_expired") => shared.expired.store(true, Ordering::SeqCst),
        Some("job_status") => {
            let Some(job) = p["job"].as_u64() else { return };
            lock(&shared.jobs).entry(job).or_default().statuses.push((
                p["stage"].as_i64().unwrap_or(0),
                p["code"].as_i64().unwrap_or(0),
                text("info"),
            ));
        }
        Some("job_done") => {
            let Some(job) = p["job"].as_u64() else { return };
            lock(&shared.jobs).entry(job).or_default().done = Some(p["ret"].as_i64().unwrap_or(-1));
        }
        Some("job_wait") => {
            // PrintJob.cpp `wait_fn`: true once the printer reports the
            // job, or prints, or after the wait; answered from a thread
            // of its own, so this one goes on reading.
            let (Some(cb), job_info) = (p["cb"].as_u64(), text("job_info")) else {
                return;
            };
            let wanted = serde_json::from_str::<Value>(&job_info)
                .ok()
                .and_then(|info| info["job_id"].as_str().map(str::to_owned));
            let conn = conn.clone();
            let shared = shared.clone();
            std::thread::spawn(move || {
                let deadline = Instant::now() + started;
                shared.wait_for(deadline, || {
                    lock(&shared.printers).values().any(|remote| {
                        remote.machine.printing()
                            || wanted
                                .as_deref()
                                .is_some_and(|id| remote.machine.job_id == id)
                    })
                });
                conn.tell("callback_return", json!({ "cb": cb, "value": true }));
            });
        }
        _ => return,
    }
    shared.touch();
}
