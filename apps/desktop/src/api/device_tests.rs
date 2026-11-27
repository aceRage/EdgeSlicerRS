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
const KEY: &str = "the-right-key";

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
            account: Duration::from_secs(5),
        };
        let session = Session::with_bridge(Some(dir.path().to_owned()), mock.launcher(), quick);
        Probe {
            session: Arc::new(session),
            answers: Mutex::new(Vec::new()),
            _dir: dir,
        }
    }

    /// A session whose print hosts answer quickly.
    fn plain() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut session = Session::with_settings(None, Some(dir.path().to_owned()));
        session.hosts.timeouts = crate::hosts::Timeouts {
            connect: Duration::from_secs(2),
            ask: Duration::from_millis(800),
            write: Duration::from_secs(2),
            answer: Duration::from_millis(800),
            start: Duration::from_secs(3),
        };
        Probe {
            session: Arc::new(session),
            answers: Mutex::new(Vec::new()),
            _dir: dir,
        }
    }

    /// A project on a Snapmaker `machine` whose "Octo/Klipper" connection
    /// is `address`, with `key`.
    fn on_host(machine: &str, address: &str, key: Option<&str>) -> Self {
        let probe = Probe::plain();
        probe.project("Snapmaker", machine);
        let mut typed = json!({ "host_type": "octoprint", "print_host": address });
        if let Some(key) = key {
            typed["apikey"] = json!(key);
        }
        probe.ask("print_host_set", typed);
        probe
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

/// What the page was answered and the bridge was asked, searched for the
/// methods and commands of section 15.
fn section_15_asked(mock: &Mock) -> Vec<String> {
    let mut asked: Vec<String> = mock
        .seen
        .lock()
        .unwrap()
        .requests
        .iter()
        .map(|(method, _)| method.clone())
        .filter(|m| {
            crate::ultranet::client::CAMERA.contains(&m.as_str())
                || crate::ultranet::client::FILES.contains(&m.as_str())
                || crate::ultranet::client::UPDATE.contains(&m.as_str())
        })
        .collect();
    asked.dedup();
    asked
}

#[test]
fn a_bridge_of_protocol_1_0_shows_bambu_s_camera_storage_and_update_unavailable_with_why() {
    let mock = Mock::new(Setup {
        minor: 0,
        ..Setup::default()
    });
    let probe = bambu_connected(&mock);
    let caps = probe.ask("device_read", json!({ "refresh": true }))["caps"].clone();
    assert_eq!(
        (
            caps["camera_frames"].clone(),
            caps["storage"].clone(),
            caps["update"].clone()
        ),
        (json!(false), json!(false), json!(false))
    );
    for (part, method) in [
        ("camera", "camera_start"),
        ("storage", "files_list"),
        ("update", "firmware_upgrade"),
    ] {
        let why = caps["unavailable"][part].as_str().unwrap();
        assert!(
            why.contains("protocol 1.0") && why.contains(method),
            "{why}"
        );
    }
    let why = caps["unavailable"]["camera"].as_str().unwrap().to_owned();
    assert_eq!(
        probe.refused("device_camera", json!({ "action": "start" })),
        why
    );
    assert!(
        probe
            .refused("device_files", json!({ "kind": "model" }))
            .contains("files_list")
    );
    assert!(
        probe
            .refused(
                "device_command",
                json!({ "action": "update", "version": "01.09.00.00" })
            )
            .contains("firmware_upgrade")
    );
    // Nothing of section 15 was asked of the bridge, and no `upgrade`
    // went through `send_message_to_printer`.
    assert!(section_15_asked(&mock).is_empty());
    assert!(
        mock.seen
            .lock()
            .unwrap()
            .messages
            .iter()
            .all(|m| m.get("upgrade").is_none())
    );
    probe.never_said(CODE);
}

#[test]
fn a_bridge_of_the_later_minor_gives_bambu_s_camera_storage_and_firmware_update() {
    let mock = Mock::new(Setup::later());
    let probe = bambu_connected(&mock);
    let caps = probe.ask("device_read", json!({ "refresh": true }))["caps"].clone();
    assert_eq!(
        (
            caps["camera_frames"].clone(),
            caps["storage"].clone(),
            caps["update"].clone(),
            caps["unavailable"].clone()
        ),
        (json!(true), json!(true), json!(true), json!({}))
    );
    // The camera: frames handed over as data, a frame that is not a JPEG
    // never; stopped, none.
    assert_eq!(
        probe.ask("device_camera", json!({ "action": "start" }))["kind"],
        "jpeg"
    );
    let frame = probe.until("a frame", || {
        let f = probe.ask("device_camera", json!({ "action": "frame" }));
        (f["seq"] == 2).then_some(f)
    });
    assert_eq!(
        frame["picture"],
        "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQ=="
    );
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        probe.ask("device_camera", json!({ "action": "frame" }))["seq"],
        2
    );
    probe.ask("device_camera", json!({ "action": "stop" }));
    assert_eq!(
        probe.ask("device_camera", json!({ "action": "frame" }))["picture"],
        Value::Null
    );
    // Storage: the card's models; Delete asked with the reference's words,
    // sent only confirmed.
    let files = probe.ask("device_files", json!({ "kind": "model" }))["files"].clone();
    assert_eq!(ids(&files, "name"), ["cube.gcode.3mf", "boat.gcode.3mf"]);
    assert_eq!(files[0]["size"], 300_000);
    let asked = probe.ask("device_file_delete", json!({ "path": "/cube.gcode.3mf" }));
    assert_eq!(
        asked["confirm"]["lines"][0],
        "Do you want to delete the file 'cube.gcode.3mf' from printer?"
    );
    assert!(mock.requests("files_delete").is_empty());
    probe.ask(
        "device_file_delete",
        json!({ "path": "/cube.gcode.3mf", "confirmed": true }),
    );
    assert_eq!(
        mock.requests("files_delete")[0]["paths"],
        json!(["/cube.gcode.3mf"])
    );
    let files = probe.ask("device_files", json!({ "kind": "model" }))["files"].clone();
    assert_eq!(ids(&files, "name"), ["boat.gcode.3mf"]);
    // Update: nothing offered, nothing done.
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "update", "version": "01.09.00.00" })
        ),
        "The printer has found no firmware update to install."
    );
    {
        let mut seen = mock.seen.lock().unwrap();
        seen.upgrade = 1;
        seen.upgrade_version = "01.09.00.00".into();
    }
    mock.report(json!({ "upgrade_state": { "dis_state": 1, "new_version_state": 1, "status": "IDLE", "progress": "0",
        "new_ver_list": [{ "name": "ota", "cur_ver": "01.08.02.00", "new_ver": "01.09.00.00" }] } }));
    let update = probe.until("the update offered", || {
        let r = probe.read()["reading"].clone();
        (r["update"]["state"] == 1).then_some(r["update"].clone())
    });
    assert_eq!(update["version"], "01.09.00.00");
    // Another version than the one shown: refused.
    assert!(
        probe
            .refused(
                "device_command",
                json!({ "action": "update", "version": "01.10.00.00" })
            )
            .starts_with("The firmware the printer offers changed")
    );
    let asked = probe.ask(
        "device_command",
        json!({ "action": "update", "version": "01.09.00.00" }),
    );
    assert_eq!(asked["confirm"]["title"], "Update firmware");
    assert_eq!(
        asked["confirm"]["lines"],
        json!([
            "ota: 01.08.02.00 → 01.09.00.00",
            "Are you sure you want to update? This will take about 10 minutes. Do not turn off the power while the printer is updating."
        ])
    );
    assert!(
        mock.requests("firmware_upgrade").is_empty(),
        "never without the person"
    );
    // Printing: refused, whatever the printer offers.
    mock.seen.lock().unwrap().state = "RUNNING".into();
    mock.report(json!({ "gcode_state": "RUNNING" }));
    probe.until("printing", || {
        (probe.read()["reading"]["state"] == "printing").then_some(())
    });
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "update", "version": "01.09.00.00", "confirmed": true })
        ),
        "The printer is printing: its firmware cannot be updated now."
    );
    assert!(mock.requests("firmware_upgrade").is_empty());
    mock.seen.lock().unwrap().state = "IDLE".into();
    mock.report(json!({ "gcode_state": "IDLE" }));
    probe.until("idle", || {
        (probe.read()["reading"]["state"] == "idle").then_some(())
    });
    probe.confirmed(json!({ "action": "update", "version": "01.09.00.00" }));
    assert_eq!(
        mock.requests("firmware_upgrade"),
        [json!({ "dev_id": "01P09C470100142", "module": "ota", "version": "01.09.00.00" })]
    );
    let update = probe.until("updating", || {
        let r = probe.read()["reading"].clone();
        (r["update"]["state"] == 2).then_some(r["update"].clone())
    });
    assert_eq!(
        (update["status"].clone(), update["progress"].clone()),
        (json!("DOWNLOADING"), json!("3"))
    );
    // Never through `send_message_to_printer`.
    assert!(
        mock.seen
            .lock()
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.get("upgrade").is_some())
            .count()
            == 1
    );
    assert!(
        mock.requests("send_message_to_printer")
            .iter()
            .all(|p| !p["json"].as_str().unwrap().contains("upgrade"))
    );
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

// ---- Moonraker: the Snapmaker U1, and any other Klipper ----

const U1: &str = "Snapmaker U1 (0.4 nozzle)";
const OTHER: &str = "Snapmaker Artisan (0.4 nozzle)";

use crate::hosts::mock::{self as hosts_mock, U1Setup};

fn ids(list: &Value, key: &str) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|item| item[key].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_u1_s_storage_has_the_details_and_pictures_the_reference_shows() {
    let (printer, shared) = hosts_mock::u1(U1Setup {
        key: Some(KEY),
        ..U1Setup::default()
    });
    shared
        .lock()
        .unwrap()
        .files
        .push(("sub/boat.gcode".into(), 5000));
    let probe = Probe::on_host(U1, &printer.address(), Some(KEY));
    let files = probe.ask("printer_files", json!({}))["files"].clone();
    let file = &files[0];
    assert_eq!(file["name"], "sub/boat.gcode");
    // `pick_thumbnail`: the smallest at least 300 wide, relative to the
    // file's own folder.
    assert_eq!(file["thumbnail"], "sub/.thumbs/boat-300x300.png");
    assert_eq!(
        (
            file["layer_count"].clone(),
            file["layer_height"].clone(),
            file["object_height"].clone(),
            file["nozzle_diameter"].clone(),
            file["slicer"].clone(),
            file["filament_type"].clone()
        ),
        (
            json!(122),
            json!(0.2),
            json!(24.5),
            json!(0.4),
            json!("EdgeSlicer 2.4.9"),
            json!("PLA;PETG;PLA")
        )
    );
    // The picture is fetched here, with the key, and handed over as data.
    let picture = probe.ask(
        "printer_file_thumbnail",
        json!({ "path": "sub/.thumbs/boat-300x300.png" }),
    )["picture"]
        .clone();
    assert!(
        picture
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,iVBORw0KGgo"),
        "{picture}"
    );
    let asked = printer
        .got()
        .into_iter()
        .find(|got| got.url == "/server/files/gcodes/sub/.thumbs/boat-300x300.png")
        .unwrap();
    assert_eq!(asked.header("X-Api-Key"), Some(KEY));
    // What is not a PNG or a JPEG is not handed over; a path out of the
    // G-code folder is not asked for.
    assert_eq!(
        probe.refused(
            "printer_file_thumbnail",
            json!({ "path": "sub/.thumbs/boat-32x32.png" })
        ),
        "The printer's preview is not a PNG or JPEG picture."
    );
    let before = printer.got().len();
    assert!(
        probe
            .refused(
                "printer_file_thumbnail",
                json!({ "path": "../config/printer.cfg" })
            )
            .starts_with("Not a picture on the printer")
    );
    assert_eq!(printer.got().len(), before);
    probe.never_said(KEY);
}

#[test]
fn a_u1_is_drawn_with_its_four_toolheads_and_what_they_hold() {
    let (printer, _) = hosts_mock::u1(U1Setup::default());
    let probe = Probe::on_host(U1, &printer.address(), None);
    assert_eq!(probe.ask("device_info", json!({}))["family"], "u1");
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "u1");
    assert_eq!(
        ids(&caps["heaters"], "id"),
        [
            "extruder",
            "extruder1",
            "extruder2",
            "extruder3",
            "heater_bed"
        ]
    );
    assert_eq!(
        ids(&caps["heaters"], "label"),
        ["T1", "T2", "T3", "T4", "Bed"]
    );
    // The fork's limits for a U1, not its configfile's.
    assert_eq!(
        (
            caps["heaters"][0]["max"].clone(),
            caps["heaters"][4]["max"].clone()
        ),
        (json!(300.0), json!(100.0))
    );
    assert_eq!(ids(&caps["fans"], "label"), ["Part", "Cham"]);
    assert_eq!(
        caps["speed"],
        json!({ "kind": "percent", "min": 50, "max": 150 })
    );
    assert_eq!(
        (
            caps["light"].clone(),
            caps["slots"].clone(),
            caps["jog"]["z_label"].clone()
        ),
        (json!(true), json!("toolheads"), json!("Z"))
    );
    // Its webcam is WebRTC: not shown.
    assert_eq!(caps["camera"], false);
    let r = &read["reading"];
    assert_eq!(
        (r["state"].clone(), r["said"].clone()),
        (json!("idle"), json!("Idle"))
    );
    assert_eq!(r["slots"][0]["name"], "Toolheads");
    let heads = &r["slots"][0]["slots"];
    assert_eq!(
        (
            heads[1]["name"].clone(),
            heads[1]["type"].clone(),
            heads[1]["color"].clone(),
            heads[1]["loaded"].clone()
        ),
        (json!("T2"), json!("PETG"), json!("#00FF00"), json!(true))
    );
    assert_eq!(heads[3]["loaded"], false);
    assert_eq!(
        (r["speed"].clone(), r["light"].clone(), r["homed"].clone()),
        (json!(100), json!(false), json!(""))
    );
    assert_eq!(
        r["heaters"][4],
        json!({ "id": "heater_bed", "now": 60.0, "target": 60.0 })
    );
}

#[test]
fn every_u1_command_is_asked_first_and_sent_as_the_fork_sends_it() {
    let (printer, shared) = hosts_mock::u1(U1Setup::default());
    let probe = Probe::on_host(U1, &printer.address(), None);
    let scripts = || shared.lock().unwrap().scripts.clone();
    let asked = probe.ask(
        "device_command",
        json!({ "action": "temperature", "heater": "extruder1", "target": 210 }),
    );
    assert_eq!(
        asked["confirm"]["lines"][0],
        format!("Set T2 to 210 °C on {U1}?")
    );
    assert!(
        scripts().is_empty(),
        "nothing is sent before it is confirmed"
    );
    probe.confirmed(json!({ "action": "temperature", "heater": "extruder1", "target": 210 }));
    let confirm =
        probe.confirmed(json!({ "action": "temperature", "heater": "heater_bed", "target": 150 }));
    assert_eq!(confirm["lines"][0], format!("Set Bed to 100 °C on {U1}?"));
    probe.confirmed(json!({ "action": "fan", "fan": "fan", "percent": 50 }));
    probe.confirmed(json!({ "action": "fan", "fan": "fan_generic cavity_fan", "percent": 30 }));
    let confirm = probe.confirmed(json!({ "action": "speed", "value": 200 }));
    assert_eq!(confirm["lines"][0], format!("Set Speed to 150 % on {U1}?"));
    probe.confirmed(json!({ "action": "light", "on": true }));
    probe.confirmed(json!({ "action": "jog", "axis": "x", "mm": 10 }));
    probe.confirmed(json!({ "action": "jog", "axis": "z", "mm": -1 }));
    probe.confirmed(json!({ "action": "home" }));
    // T1 is cold (25 °C): the reference's warning first.
    let asked = probe.ask("device_command", json!({ "action": "extrude", "mm": 10 }));
    assert_eq!(
        (
            asked["confirm"]["title"].clone(),
            asked["confirm"]["ok"].clone()
        ),
        (json!("Warning"), json!("Still load"))
    );
    probe.confirmed(json!({ "action": "extrude", "mm": 10 }));
    assert_eq!(
        scripts(),
        [
            "SET_HEATER_TEMPERATURE HEATER=extruder1 TARGET=210",
            "SET_HEATER_TEMPERATURE HEATER=heater_bed TARGET=100",
            "M106 S128",
            "SET_FAN_SPEED FAN=cavity_fan SPEED=0.30",
            "M220 S150",
            "SET_LED LED=cavity_led WHITE=1",
            "G91\nG1 X10 F3600\nG90",
            "G91\nG1 Z-1 F1800\nG90",
            "G28",
            "M83\nG1 E10 F300",
        ]
    );
    let r = probe.read()["reading"].clone();
    assert_eq!(
        (
            r["heaters"][1]["target"].clone(),
            r["heaters"][4]["target"].clone()
        ),
        (json!(210.0), json!(100.0))
    );
    assert_eq!(
        r["fans"],
        json!([{ "id": "fan", "percent": 50 }, { "id": "fan_generic cavity_fan", "percent": 30 }])
    );
    assert_eq!(
        (r["speed"].clone(), r["light"].clone(), r["homed"].clone()),
        (json!(150), json!(true), json!("xyz"))
    );
    // What it has not got.
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "temperature", "heater": "chamber", "target": 40 })
        ),
        "This printer cannot do that from here."
    );
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "fan", "fan": "fan_generic exhaust", "percent": 40 })
        ),
        "This printer cannot do that from here."
    );
}

#[test]
fn a_u1_print_is_paused_resumed_and_cancelled_only_when_it_can_be() {
    let (printer, shared) = hosts_mock::u1(U1Setup {
        steps: 1000,
        ..U1Setup::default()
    });
    let probe = Probe::on_host(U1, &printer.address(), None);
    assert_eq!(
        probe.refused("device_command", json!({ "action": "pause" })),
        "The printer is not printing, so there is nothing to pause."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "stop" })),
        "The printer is not printing, so there is nothing to stop."
    );
    {
        let mut u1 = shared.lock().unwrap();
        u1.files.push(("cube.gcode".into(), 1000));
        u1.printing = Some("cube.gcode".into());
        u1.state = "printing".into();
    }
    let r = probe.read()["reading"].clone();
    assert_eq!(
        (r["state"].clone(), r["file"].clone()),
        (json!("printing"), json!("cube.gcode"))
    );
    assert!(r["percent"].is_u64() && r["total_layers"] == 10000, "{r}");
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "jog", "axis": "x", "mm": 1 })
        ),
        "The printer is printing: the axes and the extruder cannot be moved now."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "resume" })),
        "The printer has no paused print to resume."
    );
    probe.confirmed(json!({ "action": "pause" }));
    assert_eq!(probe.read()["reading"]["state"], "paused");
    // Paused: the axes may move again (the reference turns them on).
    assert!(probe.ask("device_command", json!({ "action": "home" }))["confirm"].is_object());
    probe.confirmed(json!({ "action": "resume" }));
    let confirm = probe.confirmed(json!({ "action": "stop" }));
    assert_eq!(
        confirm["lines"][0],
        "Are you sure you want to cancel this print?"
    );
    assert_eq!(probe.read()["reading"]["state"], "cancelled");
    assert_eq!(
        shared.lock().unwrap().commands,
        ["pause", "resume", "cancel"]
    );
    let asked: Vec<String> = printer
        .asked()
        .into_iter()
        .filter(|a| a.starts_with("POST /printer/print/"))
        .collect();
    assert_eq!(
        asked,
        [
            "POST /printer/print/pause",
            "POST /printer/print/resume",
            "POST /printer/print/cancel"
        ]
    );
}

#[test]
fn a_klipper_printer_is_drawn_from_its_own_objects_with_its_camera() {
    let setup = U1Setup {
        objects: vec![
            "print_stats",
            "display_status",
            "virtual_sdcard",
            "heater_bed",
            "extruder",
            "heater_generic chamber",
            "fan",
            "fan_generic exhaust",
            "led status",
            "gcode_move",
            "toolhead",
            "configfile",
        ],
        webcam: Some(("mjpegstreamer-adaptive", "/webcam/?action=stream")),
        ..U1Setup::default()
    };
    let (printer, shared) = hosts_mock::u1(setup);
    let probe = Probe::on_host(OTHER, &printer.address(), None);
    assert_eq!(
        probe.ask("device_info", json!({}))["family"],
        "octo_klipper"
    );
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "klipper");
    assert_eq!(ids(&caps["heaters"], "label"), ["Nozzle", "Bed", "Chamber"]);
    // Its own limits (configfile).
    let max: Vec<f64> = caps["heaters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["max"].as_f64().unwrap())
        .collect();
    assert_eq!(max, [285.0, 110.0, 65.0]);
    assert_eq!(ids(&caps["fans"], "label"), ["Part", "exhaust"]);
    assert_eq!(
        caps["speed"],
        json!({ "kind": "percent", "min": 10, "max": 300 })
    );
    assert_eq!(
        (caps["slots"].clone(), caps["camera"].clone()),
        (Value::Null, json!(true))
    );
    assert_eq!(
        read["reading"]["camera"],
        format!("http://{}/webcam/?action=stream", printer.address())
    );
    probe.confirmed(
        json!({ "action": "temperature", "heater": "heater_generic chamber", "target": 50 }),
    );
    probe.confirmed(json!({ "action": "fan", "fan": "fan_generic exhaust", "percent": 100 }));
    probe.confirmed(json!({ "action": "light", "on": false }));
    assert_eq!(
        shared.lock().unwrap().scripts,
        [
            "SET_HEATER_TEMPERATURE HEATER=chamber TARGET=50",
            "SET_FAN_SPEED FAN=exhaust SPEED=1.00",
            "SET_LED LED=status WHITE=0"
        ]
    );
    let r = probe.read()["reading"].clone();
    assert_eq!(
        r["heaters"][2],
        json!({ "id": "heater_generic chamber", "now": 50.0, "target": 50.0 })
    );
}

#[test]
fn a_moonraker_that_does_not_answer_or_wants_another_key_is_offline_without_the_key() {
    // Nothing on the port.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap().to_string();
    drop(closed);
    let probe = Probe::on_host(U1, &address, None);
    let read = probe.read();
    assert_eq!(
        (read["online"].clone(), read["said"].clone()),
        (json!(false), json!("Offline"))
    );
    assert!(
        probe
            .refused("device_command", json!({ "action": "home" }))
            .contains("Could not connect")
    );
    // A printer that hangs.
    let silent = hosts_mock::Silent::start();
    let probe = Probe::on_host(U1, &format!("127.0.0.1:{}", silent.port), None);
    assert_eq!(probe.read()["online"], false);
    // A key the printer does not take: 401, and the key in no answer.
    let (printer, shared) = hosts_mock::u1(U1Setup {
        key: Some(KEY),
        ..U1Setup::default()
    });
    let probe = Probe::on_host(U1, &printer.address(), Some("a-wrong-key"));
    let read = probe.read();
    assert_eq!(read["online"], false);
    assert!(
        read["error"].as_str().unwrap().starts_with("HTTP 401"),
        "{read}"
    );
    assert!(
        probe
            .refused(
                "device_command",
                json!({ "action": "light", "on": true, "confirmed": true })
            )
            .starts_with("HTTP 401")
    );
    assert!(shared.lock().unwrap().scripts.is_empty());
    probe.never_said("a-wrong-key");
    // The right key: it answers, and the key is in no answer either.
    let probe = Probe::on_host(U1, &printer.address(), Some(KEY));
    assert!(probe.read()["reading"].is_object());
    probe.confirmed(json!({ "action": "light", "on": true }));
    probe.never_said(KEY);
}

// ---- OctoPrint ----

#[test]
fn an_octoprint_is_drawn_from_its_own_api_with_its_camera() {
    let (printer, shared) = hosts_mock::octoprint(KEY);
    let probe = Probe::on_host(OTHER, &printer.address(), Some(KEY));
    assert_eq!(
        probe.ask("device_info", json!({}))["family"],
        "octo_klipper"
    );
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "octoprint");
    assert_eq!(ids(&caps["heaters"], "id"), ["tool0", "bed"]);
    assert_eq!(ids(&caps["heaters"], "label"), ["Nozzle", "Bed"]);
    assert_eq!(ids(&caps["fans"], "label"), ["Part"]);
    assert_eq!(
        caps["speed"],
        json!({ "kind": "percent", "min": 50, "max": 200 })
    );
    assert_eq!(
        (
            caps["light"].clone(),
            caps["slots"].clone(),
            caps["camera"].clone()
        ),
        (json!(false), Value::Null, json!(true))
    );
    let r = &read["reading"];
    assert_eq!(
        (r["state"].clone(), r["said"].clone(), r["message"].clone()),
        (json!("idle"), json!("Idle"), json!("Operational"))
    );
    assert_eq!(
        r["camera"],
        format!("http://{}/webcam/?action=stream", printer.address())
    );
    // OctoPrint reports no fan speed and no speed factor: shown as not known.
    assert_eq!(
        (r["fans"].clone(), r["speed"].clone()),
        (json!([]), Value::Null)
    );
    // Every command, asked first, as OctoPrint's API takes it.
    let asked = probe.ask(
        "device_command",
        json!({ "action": "temperature", "heater": "tool0", "target": 215 }),
    );
    assert!(asked["confirm"].is_object());
    assert!(
        shared.lock().unwrap().posts.is_empty(),
        "nothing is sent before it is confirmed"
    );
    probe.confirmed(json!({ "action": "temperature", "heater": "tool0", "target": 215 }));
    probe.confirmed(json!({ "action": "temperature", "heater": "bed", "target": 60 }));
    probe.confirmed(json!({ "action": "fan", "fan": "fan", "percent": 50 }));
    probe.confirmed(json!({ "action": "speed", "value": 150 }));
    probe.confirmed(json!({ "action": "jog", "axis": "y", "mm": -10 }));
    probe.confirmed(json!({ "action": "home" }));
    probe.confirmed(json!({ "action": "extrude", "mm": -10 }));
    let posts = shared.lock().unwrap().posts.clone();
    assert_eq!(
        posts,
        [
            (
                "/api/printer/tool".to_owned(),
                json!({ "command": "target", "targets": { "tool0": 215.0 } })
            ),
            (
                "/api/printer/bed".to_owned(),
                json!({ "command": "target", "target": 60.0 })
            ),
            (
                "/api/printer/command".to_owned(),
                json!({ "commands": ["M106 S128"] })
            ),
            (
                "/api/printer/printhead".to_owned(),
                json!({ "command": "feedrate", "factor": 150 })
            ),
            (
                "/api/printer/printhead".to_owned(),
                json!({ "command": "jog", "y": -10.0 })
            ),
            (
                "/api/printer/printhead".to_owned(),
                json!({ "command": "home", "axes": ["x", "y", "z"] })
            ),
            (
                "/api/printer/tool".to_owned(),
                json!({ "command": "extrude", "amount": -10.0 })
            ),
        ]
    );
    assert_eq!(
        probe.read()["reading"]["heaters"][0],
        json!({ "id": "tool0", "now": 215.0, "target": 215.0 })
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "light", "on": true })),
        "This printer cannot do that from here."
    );
    probe.never_said(KEY);
}

#[test]
fn an_octoprint_print_is_paused_resumed_and_cancelled_and_its_files_are_its_own() {
    let (printer, shared) = hosts_mock::octoprint(KEY);
    let probe = Probe::on_host(OTHER, &printer.address(), Some(KEY));
    assert_eq!(
        probe.refused("device_command", json!({ "action": "pause" })),
        "The printer is not printing, so there is nothing to pause."
    );
    // Storage: OctoPrint's own file list, as it is not a Moonraker.
    let files = probe.ask("printer_files", json!({}));
    let names: Vec<&str> = files["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["cube.gcode", "sub/vase.gcode"]);
    assert_eq!(
        (
            files["files"][0]["print_time"].clone(),
            files["files"][0]["filament_mm"].clone()
        ),
        (json!(1234), json!(2500.0))
    );
    assert_eq!(
        probe.ask("printer_file_delete", json!({ "name": "sub/vase.gcode" }))["gone"],
        false
    );
    assert_eq!(
        probe.ask("printer_file_delete", json!({ "name": "sub/vase.gcode" }))["gone"],
        true
    );
    assert_eq!(
        probe.ask("printer_file_check", json!({ "name": "cube.gcode" }))["busy"],
        false
    );
    probe.ask("printer_file_print", json!({ "name": "cube.gcode" }));
    let r = probe.read()["reading"].clone();
    assert_eq!(
        (
            r["state"].clone(),
            r["file"].clone(),
            r["percent"].clone(),
            r["left"].clone()
        ),
        (
            json!("printing"),
            json!("cube.gcode"),
            json!(43),
            json!(1800)
        )
    );
    // Printing: Storage's Print is refused (the check says busy and keeps
    // no confirmation; one confirmation was one start), the axes too.
    assert_eq!(
        probe.refused("printer_file_print", json!({ "name": "cube.gcode" })),
        "Confirm the print first."
    );
    assert_eq!(
        probe.ask("printer_file_check", json!({ "name": "cube.gcode" }))["busy"],
        true
    );
    assert_eq!(
        probe.refused("printer_file_print", json!({ "name": "cube.gcode" })),
        "Confirm the print first."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "home" })),
        "The printer is printing: the axes and the extruder cannot be moved now."
    );
    probe.confirmed(json!({ "action": "pause" }));
    assert_eq!(probe.read()["reading"]["state"], "paused");
    probe.confirmed(json!({ "action": "resume" }));
    probe.confirmed(json!({ "action": "stop" }));
    assert_eq!(probe.read()["reading"]["state"], "idle");
    let jobs: Vec<Value> = shared
        .lock()
        .unwrap()
        .posts
        .iter()
        .filter(|(p, _)| p == "/api/job")
        .map(|(_, b)| b.clone())
        .collect();
    assert_eq!(
        jobs,
        [
            json!({ "command": "pause", "action": "pause" }),
            json!({ "command": "pause", "action": "resume" }),
            json!({ "command": "cancel" })
        ]
    );
    probe.never_said(KEY);
}

#[test]
fn an_octoprint_without_its_printer_or_with_a_wrong_key_says_so_without_the_key() {
    let (printer, shared) = hosts_mock::octoprint(KEY);
    shared.lock().unwrap().disconnected = true;
    let probe = Probe::on_host(OTHER, &printer.address(), Some(KEY));
    let read = probe.read();
    assert_eq!(
        (
            read["reading"]["state"].clone(),
            read["reading"]["said"].clone()
        ),
        (json!("error"), json!("Offline"))
    );
    assert_eq!(
        read["reading"]["message"],
        "OctoPrint answers, but no printer is connected to it."
    );
    // Not printing: nothing to stop; a heater: OctoPrint refuses (409).
    assert_eq!(
        probe.refused("device_command", json!({ "action": "stop" })),
        "The printer is not printing, so there is nothing to stop."
    );
    assert!(
        probe
            .refused(
                "device_command",
                json!({ "action": "temperature", "heater": "bed", "target": 50, "confirmed": true })
            )
            .starts_with("HTTP 409")
    );
    // A wrong key: 403, nothing sent, the key in no answer.
    let probe = Probe::on_host(OTHER, &printer.address(), Some("not-the-key"));
    let read = probe.read();
    assert_eq!(read["online"], false);
    assert!(
        read["error"].as_str().unwrap().starts_with("HTTP 403"),
        "{read}"
    );
    assert!(
        probe
            .refused(
                "device_command",
                json!({ "action": "home", "confirmed": true })
            )
            .starts_with("HTTP 403")
    );
    assert!(shared.lock().unwrap().posts.is_empty());
    probe.never_said("not-the-key");
    probe.never_said(KEY);
}

// ---- Elegoo Centauri Carbon (SDCP) ----

/// The SDCP websocket's port for the session's adapters (the printer's
/// own is 3030), while the test holds the lock.
fn sdcp_port(port: u16) {
    crate::hosts::elegoo::TEST_PORT.store(port, std::sync::atomic::Ordering::SeqCst);
}

fn sdcp_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::hosts::elegoo::TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

const CC1: &str = "Elegoo Centauri Carbon 0.4 nozzle";

#[derive(Default)]
struct Centauri {
    machine: i64,
    print: i64,
    fans: [i64; 3],
    light: i64,
    commands: Vec<Value>,
}

fn centauri_socket(shared: Arc<Mutex<Centauri>>) -> hosts_mock::Socket {
    hosts_mock::Socket::start(Box::new(move |message: &Value, _| {
        let mut cc = shared.lock().unwrap();
        let data = &message["Data"];
        let cmd = data["Cmd"].as_i64().unwrap_or(-1);
        if cmd == 0 {
            return Some(json!({ "Status": {
                "CurrentStatus": [cc.machine],
                "PrintInfo": { "Status": cc.print, "CurrentLayer": 30, "TotalLayer": 120, "CurrentTicks": 600, "TotalTicks": 2400,
                               "Progress": 25, "Filename": "boat.gcode", "PrintSpeedPct": 100 },
                "TempOfNozzle": 219.6, "TempTargetNozzle": 220, "TempOfHotbed": 59.8, "TempTargetHotbed": 60, "TempOfBox": 31.0,
                "CurrentFanSpeed": { "ModelFan": cc.fans[0], "AuxiliaryFan": cc.fans[1], "BoxFan": cc.fans[2] },
                "LightStatus": { "SecondLight": cc.light, "RgbLight": [0, 0, 0] },
            }, "MainboardID": "mock", "Topic": "sdcp/status/mock" }));
        }
        cc.commands.push(data.clone());
        let ack = match cmd {
            129 if cc.print == 13 => {
                cc.print = 6;
                0
            }
            131 if cc.print == 6 => {
                cc.print = 13;
                0
            }
            130 if cc.machine == 1 => {
                cc.print = 8;
                cc.machine = 0;
                0
            }
            403 => {
                if let Some(fans) = data["Data"]["TargetFanSpeed"].as_object() {
                    for (key, v) in fans {
                        let at = ["ModelFan", "AuxiliaryFan", "BoxFan"]
                            .iter()
                            .position(|k| k == key)
                            .unwrap();
                        cc.fans[at] = v.as_i64().unwrap();
                    }
                }
                if let Some(on) = data["Data"]["LightStatus"]["SecondLight"].as_i64() {
                    cc.light = on;
                }
                0
            }
            _ => 1,
        };
        Some(
            json!({ "Id": "", "Data": { "Cmd": cmd, "Data": { "Ack": ack }, "RequestID": data["RequestID"], "MainboardID": "mock" } }),
        )
    }))
}

#[test]
fn a_centauri_carbon_is_drawn_and_driven_as_the_forks_sdcp_monitor_does() {
    let shared = Arc::new(Mutex::new(Centauri {
        machine: 1,
        print: 13,
        ..Centauri::default()
    }));
    let _lock = sdcp_lock();
    let socket = centauri_socket(shared.clone());
    sdcp_port(socket.port);
    let probe = Probe::plain();
    probe.project("Elegoo", CC1);
    probe.ask(
        "print_host_set",
        json!({ "host_type": "elegoolink", "print_host": "127.0.0.1" }),
    );
    assert_eq!(probe.ask("device_info", json!({}))["family"], "elegoo");
    assert_eq!(probe.ask("print_host", json!({}))["native"], true);
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "elegoo");
    assert_eq!(ids(&caps["heaters"], "label"), ["Nozzle", "Bed", "Chamber"]);
    // Readings only, as the fork's CC1 caps say.
    assert!(
        caps["heaters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|h| h["settable"] == false)
    );
    assert_eq!(ids(&caps["fans"], "id"), ["part", "aux", "chamber"]);
    assert_eq!(
        (
            caps["light"].clone(),
            caps["jog"].clone(),
            caps["speed"].clone()
        ),
        (json!(true), Value::Null, Value::Null)
    );
    let r = &read["reading"];
    assert_eq!(
        (
            r["state"].clone(),
            r["file"].clone(),
            r["percent"].clone(),
            r["left"].clone(),
            r["layer"].clone(),
            r["total_layers"].clone()
        ),
        (
            json!("printing"),
            json!("boat.gcode"),
            json!(25),
            json!(1800),
            json!(30),
            json!(120)
        )
    );
    assert_eq!(
        r["heaters"][0],
        json!({ "id": "nozzle", "now": 219.6, "target": 220.0 })
    );
    // Commands: asked first, then SDCP 129 / 131 / 130 and 403.
    assert_eq!(
        probe.refused(
            "device_command",
            json!({ "action": "temperature", "heater": "bed", "target": 70 })
        ),
        "This printer cannot do that from here."
    );
    assert_eq!(
        probe.refused("device_command", json!({ "action": "home" })),
        "This printer cannot do that from here."
    );
    let asked = probe.ask("device_command", json!({ "action": "pause" }));
    assert!(asked["confirm"].is_object());
    assert!(
        shared.lock().unwrap().commands.is_empty(),
        "nothing sent before it is confirmed"
    );
    probe.confirmed(json!({ "action": "pause" }));
    assert_eq!(probe.read()["reading"]["state"], "paused");
    probe.confirmed(json!({ "action": "resume" }));
    probe.confirmed(json!({ "action": "fan", "fan": "chamber", "percent": 70 }));
    probe.confirmed(json!({ "action": "light", "on": true }));
    let r = probe.read()["reading"].clone();
    assert_eq!(
        (r["fans"][2]["percent"].clone(), r["light"].clone()),
        (json!(70), json!(true))
    );
    probe.confirmed(json!({ "action": "stop" }));
    assert_eq!(probe.read()["reading"]["state"], "cancelled");
    let sent: Vec<(i64, Value)> = shared
        .lock()
        .unwrap()
        .commands
        .iter()
        .map(|d| (d["Cmd"].as_i64().unwrap(), d["Data"].clone()))
        .collect();
    assert_eq!(
        sent,
        [
            (129, json!({})),
            (131, json!({})),
            (403, json!({ "TargetFanSpeed": { "BoxFan": 70 } })),
            (
                403,
                json!({ "LightStatus": { "SecondLight": 1, "RgbLight": [255, 255, 255] } })
            ),
            (130, json!({})),
        ]
    );
    // A refused command says the printer's answer.
    assert_eq!(
        probe.refused("device_command", json!({ "action": "stop" })),
        "The printer is not printing, so there is nothing to stop."
    );
    // Nothing on the port: offline.
    drop(socket);
    std::thread::sleep(Duration::from_millis(200));
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    sdcp_port(closed.local_addr().unwrap().port());
    drop(closed);
    let read = probe.ask("device_read", json!({ "refresh": true }));
    assert_eq!(
        (read["online"].clone(), read["said"].clone()),
        (json!(false), json!("Offline"))
    );
}

/// A Centauri Carbon's storage: its files, what command 0 says, its CANVAS.
#[derive(Default)]
struct Stock {
    files: Vec<&'static str>,
    machine: i64,
    print: i64,
    current: String,
    /// `AmsConnectStatus`, when command 0 says it.
    ams: Option<i64>,
    /// A CANVAS tray with filament in it (command 324).
    loaded: bool,
    /// 259 answers Ack 0 and deletes nothing.
    keeps: bool,
    commands: Vec<Value>,
}

fn storage_socket(shared: Arc<Mutex<Stock>>) -> hosts_mock::Socket {
    hosts_mock::Socket::start(Box::new(move |message: &Value, _| {
        let mut cc = shared.lock().unwrap();
        let data = &message["Data"];
        let cmd = data["Cmd"].as_i64().unwrap_or(-1);
        let answer = |inner: Value| {
            Some(
                json!({ "Id": "", "Data": { "Cmd": cmd, "Data": inner, "RequestID": data["RequestID"], "MainboardID": "mock" } }),
            )
        };
        if cmd == 0 {
            let mut status = json!({ "CurrentStatus": [cc.machine], "PrintInfo": { "Status": cc.print, "Filename": cc.current } });
            if let Some(ams) = cc.ams {
                status["AmsConnectStatus"] = json!(ams);
            }
            return Some(json!({ "Status": status, "MainboardID": "mock" }));
        }
        cc.commands.push(data.clone());
        match cmd {
            258 => answer(
                json!({ "Ack": 0, "FileList": cc.files.iter().enumerate().map(|(i, name)| json!({
                "name": format!("/local/{name}"), "FileSize": 1000 * (i + 1), "TotalLayers": 100 + i, "CreateTime": 1_700_000_000 + i as i64,
            })).collect::<Vec<_>>() }),
            ),
            259 => {
                if !cc.keeps {
                    let gone: Vec<String> = data["Data"]["FileList"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap().trim_start_matches("/local/").to_owned())
                        .collect();
                    cc.files.retain(|f| !gone.iter().any(|g| g == f));
                }
                answer(json!({ "Ack": 0, "ErrData": [] }))
            }
            324 => answer(
                json!({ "canvas_list": [{ "canvas_id": 0, "connected": 1, "tray_list": [
                { "tray_id": 0, "filament_type": if cc.loaded { "PLA" } else { "" }, "filament_color": if cc.loaded { "#FF0000" } else { "" } },
            ] }] }),
            ),
            128 => {
                cc.machine = 1;
                cc.print = 13;
                cc.current = data["Data"]["Filename"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("/local/")
                    .to_owned();
                answer(json!({ "Ack": 0 }))
            }
            _ => answer(json!({ "Ack": 1 })),
        }
    }))
}

#[test]
fn a_centauri_carbon_s_storage_is_listed_deleted_and_started_as_the_forks_cc1_files_do() {
    let _lock = sdcp_lock();
    let shared = Arc::new(Mutex::new(Stock {
        files: vec!["boat.gcode", "old.gcode", "model.3mf"],
        ams: Some(1),
        loaded: true,
        ..Stock::default()
    }));
    let socket = storage_socket(shared.clone());
    sdcp_port(socket.port);
    let probe = Probe::plain();
    probe.project("Elegoo", CC1);
    probe.ask(
        "print_host_set",
        json!({ "host_type": "elegoolink", "print_host": "127.0.0.1" }),
    );
    let info = probe.ask("print_host", json!({}));
    assert_eq!(
        (info["storage"].clone(), info["storage_reason"].clone()),
        (json!(true), Value::Null)
    );
    let files = probe.ask("printer_files", json!({}))["files"].clone();
    let names: Vec<&str> = files
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["model.3mf", "old.gcode", "boat.gcode"]);
    assert_eq!(
        (
            files[2]["size"].clone(),
            files[2]["layer_count"].clone(),
            files[2]["modified"].clone(),
            files[0]["printable"].clone(),
            files[2]["thumbnail"].clone()
        ),
        (
            json!(1000),
            json!(100),
            json!(1_700_000_000.0),
            json!(false),
            Value::Null
        )
    );
    // No preview pictures on this printer, and nothing asked for one.
    assert_eq!(
        probe.ask("printer_file_thumbnail", json!({ "path": "boat.gcode" }))["picture"],
        Value::Null
    );
    // Delete: exactly the reference's body, then the list asked again.
    assert_eq!(
        probe.ask("printer_file_delete", json!({ "name": "old.gcode" }))["gone"],
        false
    );
    let sent: Vec<Value> = shared.lock().unwrap().commands.clone();
    assert_eq!(sent[1]["Cmd"], 259);
    assert_eq!(sent[1]["Data"], json!({ "FileList": ["/local/old.gcode"] }));
    assert_eq!(sent[0]["Data"], json!({ "Url": "/local" }));
    assert_eq!(sent[2]["Cmd"], 258);
    // A delete the printer only acknowledges is said.
    shared.lock().unwrap().keeps = true;
    assert_eq!(
        probe.refused("printer_file_delete", json!({ "name": "model.3mf" })),
        "The printer acknowledged the delete, but it still lists the file."
    );
    shared.lock().unwrap().keeps = false;
    // Print: nothing without the check; the check reads the CANVAS and
    // says it picks the trays; then command 128 with an empty slot_map.
    assert_eq!(
        probe.refused("printer_file_print", json!({ "name": "boat.gcode" })),
        "Confirm the print first."
    );
    let options = json!({ "leveling": true, "timelapse": false, "smooth_plate": true });
    let check = probe.ask(
        "printer_file_check",
        json!({ "name": "boat.gcode", "options": options }),
    );
    assert_eq!(check["busy"], false);
    let advice = check["advice"].as_str().unwrap();
    assert!(
        advice.starts_with(
            "Bed leveling before printing: on. Timelapse: off. Plate: smooth PC side."
        ),
        "{advice}"
    );
    assert!(advice.contains("A CANVAS with loaded trays is attached."));
    shared.lock().unwrap().commands.clear();
    probe.ask(
        "printer_file_print",
        json!({ "name": "boat.gcode", "options": options }),
    );
    let sent = shared.lock().unwrap().commands.clone();
    let start = sent.iter().find(|c| c["Cmd"] == 128).unwrap();
    assert_eq!(
        start["Data"],
        json!({ "Filename": "/local/boat.gcode", "StartLayer": 0, "Calibration_switch": 1, "PrintPlatformType": 1, "Tlp_Switch": 0, "slot_map": [] })
    );
    // One confirmation, one start.
    assert_eq!(
        probe.refused(
            "printer_file_print",
            json!({ "name": "boat.gcode", "options": options })
        ),
        "Confirm the print first."
    );
    // Printing now: the check says busy, the file it is on is not deleted.
    let check = probe.ask("printer_file_check", json!({ "name": "model.3mf" }));
    assert_eq!(
        (
            check["busy"].clone(),
            check["state"].clone(),
            check["current"].clone()
        ),
        (json!(true), json!("printing"), json!("boat.gcode"))
    );
    assert_eq!(
        probe.refused("printer_file_delete", json!({ "name": "boat.gcode" })),
        "The printer is printing this file. Finish or cancel the print first."
    );
    // Idle, a CANVAS with nothing loaded at the check, loaded before the
    // start: refused, the person told to look again.
    {
        let mut cc = shared.lock().unwrap();
        cc.machine = 0;
        cc.print = 9;
        cc.loaded = false;
    }
    let check = probe.ask("printer_file_check", json!({ "name": "boat.gcode" }));
    assert!(!check["advice"].as_str().unwrap().contains("CANVAS"));
    shared.lock().unwrap().loaded = true;
    assert!(
        probe
            .refused("printer_file_print", json!({ "name": "boat.gcode" }))
            .starts_with("The printer changed since you confirmed")
    );
    // No CANVAS attached: 324 is not asked, 128 has no slot_map.
    {
        let mut cc = shared.lock().unwrap();
        cc.ams = Some(0);
        cc.commands.clear();
    }
    probe.ask("printer_file_check", json!({ "name": "boat.gcode" }));
    probe.ask("printer_file_print", json!({ "name": "boat.gcode" }));
    let sent = shared.lock().unwrap().commands.clone();
    assert!(sent.iter().all(|c| c["Cmd"] != 324), "{sent:?}");
    let start = sent.iter().find(|c| c["Cmd"] == 128).unwrap();
    assert!(start["Data"].get("slot_map").is_none());
    // A busy machine (homing, CurrentStatus 2) is not started either.
    {
        let mut cc = shared.lock().unwrap();
        cc.machine = 2;
        cc.print = 0;
    }
    assert_eq!(
        probe.ask("printer_file_check", json!({ "name": "boat.gcode" }))["state"],
        "busy"
    );
    drop(socket);
    sdcp_port(0);
    // Another Elegoo model: no Storage, the reference's reason.
    let other = Probe::plain();
    other.project("Elegoo", "Elegoo Centauri 0.4 nozzle");
    other.ask(
        "print_host_set",
        json!({ "host_type": "elegoolink", "print_host": "127.0.0.1" }),
    );
    let info = other.ask("print_host", json!({}));
    assert_eq!(info["storage"], false);
    assert!(
        info["storage_reason"]
            .as_str()
            .unwrap()
            .starts_with("The Storage tab speaks Elegoo Link (SDCP) only to the Centauri Carbon")
    );
}

// ---- PrusaLink ----

#[test]
fn a_prusalink_printer_is_shown_read_only_as_the_forks_status_reads_it() {
    let printing = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = printing.clone();
    let printer = hosts_mock::Printer::start(
        Box::new(move |got: &hosts_mock::Got, _: &[hosts_mock::Got]| {
            if got.header("X-Api-Key") != Some(KEY) {
                return (401, "Unauthorized".into());
            }
            let on = flag.load(std::sync::atomic::Ordering::SeqCst);
            match got.url.as_str() {
                "/api/v1/status" => (200, json!({
                    "printer": { "state": if on { "PRINTING" } else { "IDLE" }, "temp_nozzle": 214.8, "target_nozzle": 215.0, "temp_bed": 59.9, "target_bed": 60.0 },
                    "job": if on { json!({ "id": 7, "progress": 41.0, "time_remaining": 2460 }) } else { json!({}) },
                }).to_string()),
                "/api/v1/job" => (200, json!({ "id": 7, "state": "PRINTING", "progress": 42.0, "time_remaining": 2400,
                    "file": { "name": "BENCHY~1.BGC", "display_name": "benchy_0.4n_0.2mm.bgcode" } }).to_string()),
                _ => (404, "Not Found".into()),
            }
        }),
        None,
    );
    let probe = Probe::plain();
    probe.project("Snapmaker", OTHER);
    probe.ask(
        "print_host_set",
        json!({ "host_type": "prusalink", "print_host": printer.address(), "apikey": KEY }),
    );
    assert_eq!(probe.ask("device_info", json!({}))["family"], "prusalink");
    let read = probe.read();
    let caps = &read["caps"];
    assert_eq!(caps["family"], "prusalink");
    assert_eq!(ids(&caps["heaters"], "label"), ["Nozzle", "Bed"]);
    // No command in the reference's code: nothing drawn but the readings.
    assert_eq!(
        (
            caps["pause"].clone(),
            caps["stop"].clone(),
            caps["fans"].clone(),
            caps["jog"].clone(),
            caps["light"].clone()
        ),
        (
            json!(false),
            json!(false),
            json!([]),
            Value::Null,
            json!(false)
        )
    );
    let r = &read["reading"];
    assert_eq!(
        (
            r["state"].clone(),
            r["file"].clone(),
            r["percent"].clone(),
            r["left"].clone()
        ),
        (
            json!("printing"),
            json!("benchy_0.4n_0.2mm.bgcode"),
            json!(42),
            json!(2400)
        )
    );
    assert_eq!(
        r["heaters"][0],
        json!({ "id": "nozzle", "now": 214.8, "target": 215.0 })
    );
    for action in [
        json!({ "action": "pause" }),
        json!({ "action": "stop" }),
        json!({ "action": "temperature", "heater": "bed", "target": 50 }),
    ] {
        assert_eq!(
            probe.refused("device_command", action),
            "This printer cannot do that from here."
        );
    }
    // Idle: one request a reading (no job asked).
    printing.store(false, std::sync::atomic::Ordering::SeqCst);
    let before = printer.asked().len();
    let r = probe.read()["reading"].clone();
    assert_eq!(
        (r["state"].clone(), r["file"].clone(), r["percent"].clone()),
        (json!("idle"), json!(""), Value::Null)
    );
    assert_eq!(printer.asked().len() - before, 1);
    // Storage: the reference refuses PrusaLink, and says why.
    let info = probe.ask("print_host", json!({}));
    assert_eq!(
        (info["storage"].clone(), info["storage_reason"].clone()),
        (
            json!(false),
            json!("PrusaLink and Prusa Connect printers are not supported by the Storage tab.")
        )
    );
    probe.never_said(KEY);
    // A wrong key: offline with the printer's 401, the key in no answer.
    let probe = Probe::plain();
    probe.project("Snapmaker", OTHER);
    probe.ask("print_host_set", json!({ "host_type": "prusalink", "print_host": printer.address(), "apikey": "wrong-key-1" }));
    let read = probe.read();
    assert!(
        read["error"].as_str().unwrap().starts_with("HTTP 401"),
        "{read}"
    );
    probe.never_said("wrong-key-1");
}
