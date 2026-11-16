//! A bridge for the tests, in the test's own process: it speaks
//! ULTRANET_BRIDGE.md over a pair of pipes in memory and plays a Bambu Lab
//! printer on the LAN (a P1S with an AMS) as the reference reads one. No
//! process is started and nothing leaves the test.

use std::{
    io::{BufRead, BufReader, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::{Value, json};

use super::{
    client::Launcher,
    link::{Link, Pipe, pipe},
};

/// How the mock behaves.
#[derive(Clone, Debug)]
pub struct Setup {
    pub protocol: i64,
    pub methods: Vec<&'static str>,
    pub plugin_loaded: bool,
    pub plugin_compatible: bool,
    /// The printer's access code.
    pub code: String,
    pub serial: String,
    /// `gcode_state` at first.
    pub state: String,
    pub sdcard: bool,
    /// (tray, type, colour "RRGGBBAA") in AMS 0.
    pub trays: Vec<(u32, &'static str, &'static str)>,
    /// Developer Mode off: the print is refused, unsigned.
    pub secured: bool,
    /// The bridge dies once the upload is half done.
    pub die_in_upload: bool,
    /// The bridge echoes the access code in its texts (a bad bridge).
    pub echo_code: bool,
    pub nozzle_diameter: f64,
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            protocol: 1,
            methods: vec![
                "hello",
                "bye",
                "connect_printer",
                "disconnect_printer",
                "send_message_to_printer",
                "start_local_print",
                "start_local_print_with_record",
                "start_send_gcode_to_sdcard",
                "start_sdcard_print",
                "cancel_job",
                "callback_return",
            ],
            plugin_loaded: true,
            plugin_compatible: true,
            code: "12345678".into(),
            serial: "01P09C470100142".into(),
            state: "IDLE".into(),
            sdcard: true,
            trays: vec![
                (0, "PLA", "FF0000FF"),
                (1, "PLA", "00FF00FF"),
                (2, "PETG", "0000FFFF"),
            ],
            secured: false,
            die_in_upload: false,
            echo_code: false,
            nozzle_diameter: 0.4,
        }
    }
}

/// What the mock was asked, and what the printer is doing.
#[derive(Debug, Default)]
pub struct Seen {
    /// Every request: (method, params).
    pub requests: Vec<(String, Value)>,
    /// How many bridges were started.
    pub starts: u32,
    pub state: String,
    pub job_id: u64,
    pub percent: i64,
    pub layer: i64,
    pub subtask: String,
    /// What the Device page set: targets, fans (0..255), speed level,
    /// lamp, homed axes (`home_flag`), and the HMS list.
    pub nozzle_target: f64,
    pub bed_target: f64,
    pub fans: [i64; 3],
    pub speed: i64,
    pub light: bool,
    pub home_flag: i64,
    pub hms: Vec<(u64, u64)>,
    /// Every G-code line it was sent (`gcode_line`).
    pub gcode: Vec<String>,
}

#[derive(Clone)]
pub struct Mock {
    pub setup: Arc<Mutex<Setup>>,
    pub seen: Arc<Mutex<Seen>>,
    writer: Arc<Mutex<Option<Out>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl Mock {
    pub fn new(setup: Setup) -> Self {
        let seen = Seen {
            state: setup.state.clone(),
            nozzle_target: 220.0,
            bed_target: 55.0,
            speed: 2,
            home_flag: 0,
            ..Seen::default()
        };
        Mock {
            setup: Arc::new(Mutex::new(setup)),
            seen: Arc::new(Mutex::new(seen)),
            writer: Arc::new(Mutex::new(None)),
        }
    }

    /// The launcher a client starts this mock with.
    pub fn launcher(&self) -> Launcher {
        let mock = self.clone();
        Launcher::Test(Arc::new(move || mock.start()))
    }

    fn start(&self) -> Link {
        let (to_bridge, from_app) = pipe();
        let (to_app, from_bridge) = pipe();
        let out: Out = Arc::new(Mutex::new(Some(to_app)));
        *lock(&self.writer) = Some(out.clone());
        lock(&self.seen).starts += 1;
        let mock = self.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        std::thread::spawn(move || mock.serve(BufReader::new(from_app), out, flag));
        let input = Arc::new(Mutex::new(Some(to_bridge)));
        let closer = input.clone();
        let writer = Shared(input);
        Link {
            reader: Box::new(BufReader::new(from_bridge)),
            writer: Box::new(writer),
            stop: Box::new(move || {
                stopped.store(true, Ordering::SeqCst);
                lock(&closer).take();
            }),
        }
    }

    /// Sends a report of the printer's, as the printer would push it.
    pub fn report(&self, print: Value) {
        let serial = lock(&self.setup).serial.clone();
        if let Some(out) = lock(&self.writer).clone() {
            let mut body = print;
            body["command"] = json!("push_status");
            send(
                &out,
                &event(
                    "local_message",
                    json!({ "dev_id": serial, "msg": json!({ "print": body }).to_string() }),
                ),
            );
        }
    }

    /// Ends the running bridge as a crash would: its output closes.
    pub fn die(&self) {
        if let Some(out) = lock(&self.writer).take() {
            lock(&out).take();
        }
    }

    pub fn requests(&self, method: &str) -> Vec<Value> {
        lock(&self.seen)
            .requests
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }

    fn full_report(&self) -> Value {
        let setup = lock(&self.setup).clone();
        let seen = lock(&self.seen);
        let trays: Vec<Value> = setup
            .trays
            .iter()
            .map(|(slot, kind, colour)| {
                json!({ "id": slot.to_string(), "tray_type": kind, "tray_info_idx": format!("GF{}00", &kind[..1]), "tray_color": colour, "remain": 80, "tray_sub_brands": "" })
            })
            .collect();
        let tray_bits: u32 = setup.trays.iter().map(|(slot, _, _)| 1 << slot).sum();
        json!({
            "gcode_state": seen.state,
            "mc_percent": seen.percent,
            "mc_remaining_time": if seen.state == "RUNNING" { 42 } else { 0 },
            "layer_num": seen.layer,
            "total_layer_num": 120,
            "subtask_name": seen.subtask,
            "gcode_file": "",
            "job_id": seen.job_id.to_string(),
            "nozzle_temper": 219.5, "nozzle_target_temper": seen.nozzle_target,
            "bed_temper": 54.8, "bed_target_temper": seen.bed_target,
            // Classic 0..15 steps (BambuFans.hpp); the P1S has the aux and
            // chamber fans.
            "cooling_fan_speed": fan_step(seen.fans[0]),
            "big_fan1_speed": fan_step(seen.fans[1]),
            "big_fan2_speed": fan_step(seen.fans[2]),
            "support_aux_fan": true,
            "support_chamber_fan": true,
            "spd_lvl": seen.speed,
            "lights_report": [{ "node": "chamber_light", "mode": if seen.light { "on" } else { "off" } }],
            "home_flag": seen.home_flag,
            "hms": seen.hms.iter().map(|(attr, code)| json!({ "attr": attr, "code": code })).collect::<Vec<_>>(),
            "chamber_temper": 31,
            "sdcard": setup.sdcard,
            "fun": if setup.secured { "20000000" } else { "0" },
            "nozzle_diameter": setup.nozzle_diameter.to_string(),
            "nozzle_type": "stainless_steel",
            "ams": {
                "ams_exist_bits": if trays.is_empty() { "0" } else { "1" },
                "tray_exist_bits": format!("{tray_bits:x}"),
                "tray_now": "255",
                "ams": if trays.is_empty() { json!([]) } else { json!([{ "id": "0", "info": "1001", "tray": trays }]) },
            },
        })
    }

    fn serve(
        self,
        reader: BufReader<crate::ultranet::link::PipeReader>,
        out: Out,
        stopped: Arc<AtomicBool>,
    ) {
        let jobs: Arc<Mutex<std::collections::HashMap<u64, Arc<AtomicBool>>>> = Arc::default();
        let mut next_job = 0u64;
        let mut said_hello = false;
        for line in reader.lines() {
            if stopped.load(Ordering::SeqCst) || lock(&out).is_none() {
                break;
            }
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = message["id"].clone();
            let method = message["method"].as_str().unwrap_or_default().to_owned();
            let p = message["params"].clone();
            lock(&self.seen).requests.push((method.clone(), p.clone()));
            let setup = lock(&self.setup).clone();
            let answer = |result: Value| {
                send(
                    &out,
                    &json!({ "type": "response", "id": id, "result": result }),
                )
            };
            let refuse = |code: i64, text: &str| {
                send(
                    &out,
                    &json!({ "type": "response", "id": id, "error": { "code": code, "message": text } }),
                )
            };
            if method != "hello" && !said_hello {
                refuse(-100, "hello first");
                continue;
            }
            if method != "hello" && !setup.methods.contains(&method.as_str()) {
                refuse(-32601, "no such method");
                continue;
            }
            match method.as_str() {
                "hello" => {
                    said_hello = true;
                    if p["protocol"] != json!(setup.protocol) && setup.protocol < 1 {
                        send(
                            &out,
                            &json!({ "type": "response", "id": id, "error": { "code": -101, "message": "protocol", "data": { "protocol": setup.protocol } } }),
                        );
                        continue;
                    }
                    answer(json!({
                        "protocol": setup.protocol, "minor": 0, "bridge": "ultranet-bridge", "bridge_version": "0.9.0-mock",
                        "plugin_loaded": setup.plugin_loaded,
                        "plugin_version": if setup.plugin_compatible { "02.01.01.52" } else { "01.10.01.01" },
                        "plugin_expected": "02.01.01.52",
                        "plugin_compatible": setup.plugin_compatible,
                        "plugin_error": if setup.plugin_loaded { "" } else { "libbambu_networking.dylib is not beside the bridge" },
                        "methods": setup.methods, "events": ["local_connect", "local_message", "job_status", "job_wait", "job_done"],
                    }));
                }
                "bye" => {
                    answer(json!({}));
                    break;
                }
                "connect_printer" => {
                    answer(json!({ "ret": 0 }));
                    let ok =
                        p["password"] == json!(setup.code) && p["dev_id"] == json!(setup.serial);
                    let msg = if ok {
                        String::new()
                    } else if setup.echo_code {
                        format!(
                            "rc=5 bad user name or password for {}",
                            p["password"].as_str().unwrap_or_default()
                        )
                    } else {
                        "5".into()
                    };
                    send(
                        &out,
                        &event(
                            "local_connect",
                            json!({ "status": if ok { 0 } else { 1 }, "dev_id": p["dev_id"], "msg": msg }),
                        ),
                    );
                }
                "disconnect_printer" => answer(json!({ "ret": 0 })),
                "send_message_to_printer" => {
                    let sent: Value = serde_json::from_str(p["json"].as_str().unwrap_or("{}"))
                        .unwrap_or_default();
                    // Taken before it is answered, so a test sees it once
                    // the app has its answer.
                    let changed = self.device_command(&sent);
                    answer(json!({ "ret": 0 }));
                    if sent["pushing"]["command"] == "pushall" {
                        self.report(self.full_report());
                    }
                    if sent["info"]["command"] == "get_version" {
                        let serial = setup.serial.clone();
                        let reply = json!({ "info": { "command": "get_version", "module": [
                            { "name": "ota", "sw_ver": "01.08.02.00", "hw_ver": "", "sn": serial },
                            { "name": "ams/0", "sw_ver": "00.00.06.49", "hw_ver": "AMS08", "sn": "00600A000000001" },
                        ] } });
                        send(
                            &out,
                            &event(
                                "local_message",
                                json!({ "dev_id": serial, "msg": reply.to_string() }),
                            ),
                        );
                    }
                    if changed {
                        self.report(self.full_report());
                    }
                    let command = sent["print"]["command"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    let new_state = match command.as_str() {
                        "pause" => Some("PAUSE"),
                        "resume" => Some("RUNNING"),
                        "stop" => Some("FAILED"),
                        _ => None,
                    };
                    if let Some(state) = new_state {
                        lock(&self.seen).state = state.into();
                        self.report(json!({ "gcode_state": state }));
                    }
                }
                "start_send_gcode_to_sdcard"
                | "start_local_print"
                | "start_sdcard_print"
                | "start_local_print_with_record" => {
                    next_job += 1;
                    let job = next_job;
                    let cancelled = Arc::new(AtomicBool::new(false));
                    lock(&jobs).insert(job, cancelled.clone());
                    answer(json!({ "job": job }));
                    let mock = self.clone();
                    let out = out.clone();
                    let params = p["params"].clone();
                    let method = method.clone();
                    std::thread::spawn(move || {
                        mock.run_job(&out, job, &method, &params, &cancelled)
                    });
                }
                "cancel_job" => {
                    if let Some(flag) = lock(&jobs).get(&p["job"].as_u64().unwrap_or(0)) {
                        flag.store(true, Ordering::SeqCst);
                    }
                    answer(json!({}));
                }
                "callback_return" => answer(json!({})),
                _ => refuse(-32601, "no such method"),
            }
        }
    }

    /// A Device page's command, as a P1S takes it: the targets of
    /// `M104` / `M140`, `M106 P<n> S<v>`, `G28` (all homed), `print_speed`,
    /// `ledctrl`. Answers whether anything changed.
    fn device_command(&self, sent: &Value) -> bool {
        let mut seen = lock(&self.seen);
        if let Some(gcode) = sent["print"]["param"].as_str()
            && sent["print"]["command"] == "gcode_line"
        {
            seen.gcode.push(gcode.to_owned());
            let word = |line: &str, letter: char| {
                line.split_whitespace()
                    .find_map(|w| w.strip_prefix(letter).and_then(|v| v.parse::<f64>().ok()))
            };
            for line in gcode.lines() {
                if line.starts_with("M104") {
                    seen.nozzle_target = word(line, 'S').unwrap_or(0.0);
                } else if line.starts_with("M140") {
                    seen.bed_target = word(line, 'S').unwrap_or(0.0);
                } else if line.starts_with("M106") {
                    let index = word(line, 'P').unwrap_or(1.0) as usize;
                    if (1..=3).contains(&index) {
                        seen.fans[index - 1] = word(line, 'S').unwrap_or(0.0) as i64;
                    }
                } else if line.starts_with("G28") {
                    seen.home_flag = 7;
                }
            }
            return true;
        }
        if sent["print"]["command"] == "print_speed" {
            seen.speed = sent["print"]["param"]
                .as_str()
                .and_then(|p| p.parse().ok())
                .unwrap_or(2);
            return true;
        }
        if sent["system"]["command"] == "ledctrl" {
            seen.light = sent["system"]["led_mode"] == "on";
            return true;
        }
        false
    }

    fn run_job(&self, out: &Out, job: u64, method: &str, params: &Value, cancelled: &AtomicBool) {
        let setup = lock(&self.setup).clone();
        let status = |stage: i64, code: i64, info: &str| {
            send(
                out,
                &event(
                    "job_status",
                    json!({ "job": job, "stage": stage, "code": code, "info": info }),
                ),
            );
        };
        let done = |ret: i64| send(out, &event("job_done", json!({ "job": job, "ret": ret })));
        let pause = || std::thread::sleep(std::time::Duration::from_millis(5));
        if params["password"] != json!(setup.code) {
            // The FTPS login is refused.
            let info = if setup.echo_code {
                format!(
                    "ftp login failed with {}",
                    params["password"].as_str().unwrap_or_default()
                )
            } else {
                "ftp login failed".into()
            };
            status(7, -2130, &info);
            done(if method == "start_send_gcode_to_sdcard" {
                -5010
            } else {
                -4020
            });
            return;
        }
        if method == "start_send_gcode_to_sdcard" {
            status(1, 100, "100%");
            done(0);
            return;
        }
        status(0, 0, "");
        for percent in [0, 25, 50, 75, 100] {
            if cancelled.load(Ordering::SeqCst) {
                done(-18);
                return;
            }
            if percent == 50 && setup.die_in_upload {
                self.die();
                return;
            }
            status(1, percent, &format!("{percent}%"));
            pause();
        }
        status(3, 0, "");
        if setup.secured {
            // The printer answers the unsigned project_file.
            let serial = setup.serial.clone();
            let reply = json!({ "print": { "command": "project_file", "result": "failed", "reason": "mqtt message verify failed", "err_code": 84033543 } });
            send(
                out,
                &event(
                    "local_message",
                    json!({ "dev_id": serial, "msg": reply.to_string() }),
                ),
            );
            done(0);
            return;
        }
        {
            let mut seen = lock(&self.seen);
            seen.job_id += 1;
            seen.state = "RUNNING".into();
            seen.percent = 0;
            seen.layer = 1;
            seen.subtask = params["project_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
        }
        done(0);
        self.report(self.full_report());
    }
}

/// The app's end of the bridge's input, which `stop` closes.
struct Shared(Arc<Mutex<Option<Pipe>>>);

impl Write for Shared {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match lock(&self.0).as_mut() {
            Some(pipe) => pipe.write(bytes),
            None => Err(std::io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A fan's 0..255 as the printer's 0..15 steps, rounded so the
/// reference's reading (`classic_fan_byte`) gives the tens back.
fn fan_step(byte: i64) -> String {
    ((byte as f64 / 25.5).round() * 1.5)
        .ceil()
        .min(15.0)
        .to_string()
}

fn event(name: &str, params: Value) -> Value {
    json!({ "type": "event", "event": name, "params": params })
}

/// The bridge's output; `None` once it died.
type Out = Arc<Mutex<Option<Pipe>>>;

fn send(out: &Out, message: &Value) {
    let mut line = message.to_string();
    line.push('\n');
    if let Some(pipe) = lock(out).as_mut() {
        let _ = pipe.write_all(line.as_bytes());
    }
}
