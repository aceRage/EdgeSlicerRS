//! The Device page through `Session::call`, for each family against a mock
//! of its printer on this machine (the mock bridge in the test's own
//! process, mock printers on 127.0.0.1): every reading, every command
//! asked first and sent only confirmed, the states that refuse a command,
//! a printer that does not answer, a wrong key or code (never in an
//! answer). Nothing here reaches a real printer.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::Session;
use crate::ultranet::{
    client::Timeouts,
    mock::{Mock, Setup},
};

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const P1S: &str = "Bambu Lab P1S 0.4 nozzle";
const CODE: &str = "12345678";

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// A session and every answer it gave, to search for secrets.
struct Probe {
    session: Arc<Session>,
    answers: Mutex<Vec<String>>,
    _dir: tempfile::TempDir,
}

impl Probe {
    fn bambu(mock: &Mock) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let quick = Timeouts {
            hello: Duration::from_secs(5),
            ask: Duration::from_secs(5),
            connect: Duration::from_secs(5),
            started: Duration::from_secs(5),
            restart: Duration::from_millis(0),
        };
        let session = Session::with_bridge(Some(dir.path().to_owned()), mock.launcher(), quick);
        Probe {
            session: Arc::new(session),
            answers: Mutex::new(Vec::new()),
            _dir: dir,
        }
    }

    fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        let answer = self.session.call(name, &args);
        let text = match &answer {
            Ok(value) => value.to_string(),
            Err(error) => error.clone(),
        };
        self.answers.lock().unwrap().push(format!("{name}: {text}"));
        answer
    }

    fn ask(&self, name: &str, args: Value) -> Value {
        self.call(name, args.clone())
            .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
    }

    fn refused(&self, name: &str, args: Value) -> String {
        self.call(name, args).expect_err("should be refused")
    }

    fn never_said(&self, secret: &str) {
        let answers = self.answers.lock().unwrap();
        assert!(!answers.is_empty());
        for answer in answers.iter() {
            assert!(!answer.contains(secret), "{answer}");
        }
    }

    /// A project on `vendor`'s `machine`.
    fn project(&self, vendor: &str, machine: &str) -> u64 {
        let id = self.ask("project_open", json!({ "path": reference(SMALL) }))["id"]
            .as_u64()
            .unwrap();
        let presets = self.ask("presets", json!({ "vendor": vendor, "machine": machine }));
        self.ask(
            "project_set",
            json!({
                "project": id, "source": "presets", "vendor": vendor, "machine": machine,
                "process": presets["processes"][0], "filaments": [presets["filaments"][0]], "initial": true,
            }),
        );
        id
    }

    fn until<T>(&self, what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(found) = ready() {
                return found;
            }
            assert!(Instant::now() < deadline, "waited too long for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Asked, then sent confirmed: the question.
    fn confirmed(&self, args: Value) -> Value {
        let asked = self.ask("device_command", args.clone());
        let confirm = asked["confirm"].clone();
        assert!(confirm.is_object(), "{asked}");
        let mut args = args;
        args["confirmed"] = json!(true);
        let sent = self.ask("device_command", args);
        assert!(sent["sent"].is_string(), "{sent}");
        confirm
    }

    fn read(&self) -> Value {
        self.ask("device_read", json!({}))
    }
}

// ---- Bambu Lab, through the mock bridge ----

fn bambu_connected(mock: &Mock) -> Probe {
    let probe = Probe::bambu(mock);
    probe.project("BBL", P1S);
    probe.ask(
        "bambu_printer_add",
        json!({ "name": "Workshop P1S", "ip": "192.168.1.20", "access_code": CODE, "serial": "01P09C470100142", "model": "C12" }),
    );
    let state = probe.ask("bambu_connect", json!({}));
    assert_eq!(state["session"], "connected", "{state}");
    probe.until("the first report and the versions", || {
        let read = probe.ask("device_read", json!({ "refresh": true }));
        (read["reading"]["versions"]
            .as_array()
            .is_some_and(|v| !v.is_empty()))
        .then_some(())
    });
    probe
}

#[test]
fn a_bambu_printer_is_drawn_from_what_it_reports() {
    let mock = Mock::new(Setup::default());
    let probe = bambu_connected(&mock);
    assert_eq!(probe.ask("device_info", json!({}))["family"], "bambu");
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "bambu");
    let heaters: Vec<&str> = caps["heaters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(heaters, ["nozzle", "bed", "chamber"]);
    // The chamber is read only: the P1S does not say it can be set.
    assert_eq!(caps["heaters"][2]["settable"], false);
    assert_eq!(
        (
            caps["heaters"][0]["max"].clone(),
            caps["heaters"][1]["max"].clone()
        ),
        (json!(300.0), json!(120.0))
    );
    let fans: Vec<&str> = caps["fans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert_eq!(fans, ["Part", "Aux", "Cham"]);
    let levels: Vec<&str> = caps["speed"]["levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["label"].as_str().unwrap())
        .collect();
    assert_eq!(levels, ["Silent", "Standard", "Sport", "Ludicrous"]);
    assert_eq!(caps["light"], true);
    assert_eq!(caps["jog"]["z_label"], "Bed");
    assert_eq!(
        (
            caps["slots"].clone(),
            caps["hms"].clone(),
            caps["versions"].clone(),
            caps["camera"].clone()
        ),
        (json!("ams"), json!(true), json!(true), json!(false))
    );
    let r = &read["reading"];
    assert_eq!(
        (r["state"].clone(), r["said"].clone()),
        (json!("idle"), json!("Idle"))
    );
    assert_eq!(
        r["heaters"][0],
        json!({ "id": "nozzle", "now": 219.5, "target": 220.0 })
    );
    assert_eq!(r["heaters"][1]["target"], 55.0);
    assert_eq!(r["speed"], 2);
    assert_eq!(r["light"], false);
    assert_eq!(r["homed"], "");
    assert_eq!(r["fans"].as_array().unwrap().len(), 3);
    assert_eq!(r["slots"][0]["name"], "AMS 1");
    assert_eq!(r["slots"][0]["slots"][2]["type"], "PETG");
    assert_eq!(r["slots"][0]["slots"][0]["color"], "#FF0000");
    assert_eq!(r["versions"][0]["sw"], "01.08.02.00");
    assert_eq!(r["hms"], json!([]));
    // An HMS message, with the reference's own text for the P1 series.
    mock.seen.lock().unwrap().hms = vec![(0x0702_2000, 0x0002_0004)];
    mock.report(json!({ "hms": [{ "attr": 0x0702_2000u64, "code": 0x0002_0004u64 }] }));
    let hms = probe.until("the HMS message", || {
        let r = probe.read()["reading"].clone();
        r["hms"]
            .as_array()
            .is_some_and(|l| !l.is_empty())
            .then_some(r["hms"][0].clone())
    });
    assert_eq!(hms["code"], "0702200000020004");
    assert_eq!(hms["level"], "Serious");
    assert_eq!(hms["module"], "AMS");
    assert_eq!(
        hms["text"],
        "AMS C Slot 1 filament may be broken in the tool head."
    );
    probe.never_said(CODE);
}

#[test]
fn every_bambu_command_is_asked_first_and_sent_as_the_reference_sends_it() {
    let mock = Mock::new(Setup::default());
    let probe = bambu_connected(&mock);
    let gcode = || mock.seen.lock().unwrap().gcode.clone();
    // Asked, not sent.
    let asked = probe.ask(
        "device_command",
        json!({ "action": "temperature", "heater": "nozzle", "target": 230 }),
    );
    assert_eq!(
        asked["confirm"]["lines"][0],
        "Set Nozzle to 230 °C on Workshop P1S?"
    );
    assert!(gcode().is_empty(), "nothing is sent before it is confirmed");
    // Temperatures: M104 / M140 as `gcode_line`, above the limit the limit.
    probe.confirmed(json!({ "action": "temperature", "heater": "nozzle", "target": 230 }));
    let confirm =
        probe.confirmed(json!({ "action": "temperature", "heater": "bed", "target": 400 }));
    assert_eq!(confirm["lines"][0], "Set Bed to 120 °C on Workshop P1S?");
    assert_eq!(gcode(), ["M104 S230\n", "M140 S120\n"]);
    probe.until("the new targets", || {
        let r = probe.read()["reading"].clone();
        (r["heaters"][0]["target"] == 230.0 && r["heaters"][1]["target"] == 120.0).then_some(())
    });
    // The chamber is not settable on this printer.
    let said = probe.refused(
        "device_command",
        json!({ "action": "temperature", "heater": "chamber", "target": 40 }),
    );
    assert_eq!(said, "This printer cannot do that from here.");
    // Fans: M106 P<n> S<floor(tens * 25.5)>.
    probe.confirmed(json!({ "action": "fan", "fan": "part", "percent": 50 }));
    probe.confirmed(json!({ "action": "fan", "fan": "chamber", "percent": 100 }));
    assert_eq!(&gcode()[2..], ["M106 P1 S127 \n", "M106 P3 S255 \n"]);
    let fans = probe.until("the fans", || {
        let r = probe.read()["reading"].clone();
        (r["fans"][0]["percent"] == 50 && r["fans"][2]["percent"] == 100)
            .then_some(r["fans"].clone())
    });
    assert_eq!(fans[1]["percent"], 0);
    // Speed and lamp.
    let confirm = probe.confirmed(json!({ "action": "speed", "value": 3 }));
    assert_eq!(confirm["lines"][0], "Set Speed to Sport on Workshop P1S?");
    probe.confirmed(json!({ "action": "light", "on": true }));
    probe.until("speed and lamp", || {
        let r = probe.read()["reading"].clone();
        (r["speed"] == 3 && r["light"] == true).then_some(())
    });
    let speeds = probe.refused("device_command", json!({ "action": "speed", "value": 7 }));
    assert_eq!(speeds, "This printer cannot do that from here.");
    // Jog: the reference's G-code, with the home warning (not homed).
    let confirm = probe.confirmed(json!({ "action": "jog", "axis": "x", "mm": 10 }));
    assert!(
        confirm["lines"][1]
            .as_str()
            .unwrap()
            .starts_with("Please home all axes (click "),
        "{confirm}"
    );
    assert_eq!(
        gcode().last().unwrap(),
        "M211 S \nM211 X1 Y1 Z1\nM1002 push_ref_mode\nG91 \nG1 X10.0 F3000\nM1002 pop_ref_mode\nM211 R\n"
    );
    probe.confirmed(json!({ "action": "jog", "axis": "z", "mm": -1 }));
    assert!(gcode().last().unwrap().contains("G1 Z-1.0 F900"));
    probe.confirmed(json!({ "action": "home" }));
    assert_eq!(gcode().last().unwrap(), "G28 \n");
    probe.until("homed", || {
        (probe.read()["reading"]["homed"] == "xyz").then_some(())
    });
    let confirm = probe.ask(
        "device_command",
        json!({ "action": "jog", "axis": "x", "mm": 1 }),
    );
    assert_eq!(
        confirm["confirm"]["lines"].as_array().unwrap().len(),
        2,
        "homed: no warning"
    );
    // Only 1 and 10 mm.
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "jog", "axis": "x", "mm": 50 })
        ),
        "The axes move by 1 or 10 mm."
    );
    // The extruder: hot, plainly; cold, the reference's warning.
    let confirm = probe.confirmed(json!({ "action": "extrude", "mm": 10 }));
    assert_eq!(confirm["title"], "Extruder");
    assert_eq!(gcode().last().unwrap(), "M83 \nG0 E10.0 F900\n");
    mock.report(json!({ "nozzle_temper": 25.0 }));
    let cold = probe.until("a cold nozzle", || {
        let asked = probe.ask("device_command", json!({ "action": "extrude", "mm": -10 }));
        (asked["confirm"]["title"] == "Warning").then_some(asked)
    });
    assert_eq!(
        cold["confirm"]["lines"][0],
        "Please heat the nozzle to above 170°C before loading or unloading filament."
    );
    assert_eq!(cold["confirm"]["ok"], "Still unload");
    probe.never_said(CODE);
}

#[test]
fn a_bambu_print_is_paused_resumed_and_stopped_only_when_it_can_be() {
    let mock = Mock::new(Setup::default());
    let probe = bambu_connected(&mock);
    // Idle: nothing to pause, resume or stop.
    assert_eq!(
        probe.refused("device_command", json!({ "action": "pause" })),
        "The printer is not printing, so there is nothing to pause."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "resume" })),
        "The printer has no paused print to resume."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "stop" })),
        "The printer is not printing, so there is nothing to stop."
    );
    mock.seen.lock().unwrap().state = "RUNNING".into();
    mock.report(json!({ "gcode_state": "RUNNING", "mc_percent": 40, "mc_remaining_time": 42, "layer_num": 12, "total_layer_num": 120, "subtask_name": "cube", "stg_cur": 0 }));
    let r = probe.until("printing", || {
        let r = probe.read()["reading"].clone();
        (r["state"] == "printing").then_some(r)
    });
    assert_eq!(
        (
            r["percent"].clone(),
            r["layer"].clone(),
            r["total_layers"].clone(),
            r["left_text"].clone(),
            r["file"].clone()
        ),
        (
            json!(40),
            json!(12),
            json!(120),
            json!("42m"),
            json!("cube")
        )
    );
    // A print runs: the axes do not move.
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "jog", "axis": "y", "mm": 1 })
        ),
        "The printer is printing: the axes and the extruder cannot be moved now."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "home" })),
        "The printer is printing: the axes and the extruder cannot be moved now."
    );
    // The stage the reference names.
    mock.report(json!({ "stg_cur": 2 }));
    probe.until("the stage", || {
        (probe.read()["reading"]["said"] == "Heatbed preheating").then_some(())
    });
    let confirm = probe.confirmed(json!({ "action": "pause" }));
    assert_eq!(confirm["lines"][0], "Pause the print on Workshop P1S?");
    probe.until("paused", || {
        (probe.read()["reading"]["state"] == "paused").then_some(())
    });
    probe.confirmed(json!({ "action": "resume" }));
    probe.until("running", || {
        (probe.read()["reading"]["state"] == "printing").then_some(())
    });
    let confirm = probe.confirmed(json!({ "action": "stop" }));
    assert_eq!(
        (confirm["title"].clone(), confirm["lines"][0].clone()),
        (
            json!("Cancel print"),
            json!("Are you sure you want to cancel this print?")
        )
    );
    probe.until("stopped", || {
        (probe.read()["reading"]["state"] == "cancelled").then_some(())
    });
    let sent: Vec<String> = mock
        .requests("send_message_to_printer")
        .iter()
        .filter_map(|p| serde_json::from_str::<Value>(p["json"].as_str()?).ok())
        .filter_map(|m| m["print"]["command"].as_str().map(str::to_owned))
        .filter(|c| ["pause", "resume", "stop"].contains(&c.as_str()))
        .collect();
    assert_eq!(sent, ["pause", "resume", "stop"]);
    probe.never_said(CODE);
}

#[test]
fn a_bambu_printer_not_in_developer_mode_or_not_connected_is_sent_nothing() {
    let mock = Mock::new(Setup {
        secured: true,
        ..Setup::default()
    });
    let probe = bambu_connected(&mock);
    let before = mock.requests("send_message_to_printer").len();
    let asked = probe.ask("device_command", json!({ "action": "light", "on": true }));
    assert!(asked["confirm"].is_object());
    let said = probe.refused(
        "device_command",
        json!({ "action": "light", "on": true, "confirmed": true }),
    );
    assert!(
        said.starts_with("This printer is not in Developer Mode"),
        "{said}"
    );
    assert_eq!(
        mock.requests("send_message_to_printer").len(),
        before,
        "nothing sent"
    );
    // A printer added but not connected: offline, nothing sent.
    let mock = Mock::new(Setup::default());
    let probe = Probe::bambu(&mock);
    probe.project("BBL", P1S);
    probe.ask("bambu_printer_add", json!({ "name": "Other", "ip": "192.168.1.21", "access_code": CODE, "serial": "01P09C470100143", "model": "C12" }));
    let read = probe.read();
    assert_eq!(
        (read["online"].clone(), read["said"].clone()),
        (json!(false), json!("Offline"))
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "home" })),
        "Offline"
    );
    assert!(mock.requests("send_message_to_printer").is_empty());
    // A wrong code: refused, and never said.
    let mock = Mock::new(Setup::default());
    let probe = Probe::bambu(&mock);
    probe.project("BBL", P1S);
    probe.ask("bambu_printer_add", json!({ "name": "Workshop P1S", "ip": "192.168.1.20", "access_code": "87654321", "serial": "01P09C470100142", "model": "C12" }));
    let state = probe.ask("bambu_connect", json!({}));
    assert_eq!(state["session"], "failed");
    assert_eq!(probe.read()["online"], false);
    probe.never_said("87654321");
}
