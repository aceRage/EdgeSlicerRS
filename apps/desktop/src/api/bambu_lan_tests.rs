//! A Bambu Lab printer on the LAN through `Session::call`, against the
//! mock bridge (`ultranet::mock`): the whole cycle (add the printer,
//! connect, map the AMS, send, the print to its end, pause, resume, stop)
//! and each failure (no bridge, an old bridge, a wrong access code, a
//! printer printing, an AMS that holds nothing the plate prints with, the
//! bridge dying in the upload). Every answer the page could be given is
//! searched for the access code.

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
const SERIAL: &str = "01P09C470100142";
const CODE: &str = "12345678";
const WRONG: &str = "87654321";

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn quick() -> Timeouts {
    Timeouts {
        hello: Duration::from_secs(5),
        ask: Duration::from_secs(5),
        connect: Duration::from_secs(5),
        started: Duration::from_secs(5),
        restart: Duration::from_millis(0),
    }
}

/// A session and every answer it gave, to search for the code.
struct Probe {
    session: Arc<Session>,
    answers: Mutex<Vec<String>>,
    _dir: tempfile::TempDir,
}

impl Probe {
    fn new(mock: Option<&Mock>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let session = match mock {
            Some(mock) => {
                Session::with_bridge(Some(dir.path().to_owned()), mock.launcher(), quick())
            }
            None => Session::with_settings(None, Some(dir.path().to_owned())),
        };
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

    /// No answer held a code: neither the right one nor a wrong one typed.
    fn never_said_a_code(&self) {
        let answers = self.answers.lock().unwrap();
        assert!(!answers.is_empty());
        for answer in answers.iter() {
            assert!(
                !answer.contains(CODE) && !answer.contains(WRONG),
                "{answer}"
            );
        }
    }

    /// The small project on a P1S's presets with a PLA, sliced.
    fn sliced_project(&self) -> u64 {
        let id = self.ask("project_open", json!({ "path": reference(SMALL) }))["id"]
            .as_u64()
            .unwrap();
        let presets = self.ask("presets", json!({ "vendor": "BBL", "machine": P1S }));
        let pla = presets["filaments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| {
                f.as_str()
                    .is_some_and(|name| name.starts_with("Bambu PLA Basic"))
            })
            .cloned()
            .unwrap_or_else(|| presets["filaments"][0].clone());
        self.ask(
            "project_set",
            json!({
                "project": id, "source": "presets", "vendor": "BBL", "machine": P1S,
                "process": presets["processes"][0], "filaments": [pla], "initial": true,
            }),
        );
        self.ask("slice", json!({ "project": id }));
        loop {
            let status = self.ask("status", json!({ "project": id }));
            if status["state"] != "slicing" {
                assert_eq!(status["state"], "done", "{status}");
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        id
    }

    fn add_printer(&self, code: &str) -> Value {
        self.ask(
            "bambu_printer_add",
            json!({ "name": "Workshop P1S", "ip": "192.168.1.20", "access_code": code, "serial": SERIAL.to_lowercase(), "model": "C12" }),
        )
    }

    /// Connected, and its first report read.
    fn connected(&self) -> Value {
        let state = self.ask("bambu_connect", json!({}));
        assert_eq!(state["session"], "connected", "{state}");
        self.until("the first report", || {
            let state = self.ask("bambu_state", json!({}));
            state["machine"]["ready"]
                .as_bool()
                .unwrap_or(false)
                .then_some(state)
        })
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

    fn task_end(&self, task: u64) -> Value {
        self.until("the send to end", || {
            let progress = self.ask("bambu_task", json!({ "task": task }));
            (progress["done"] == true).then_some(progress)
        })
    }
}

#[test]
fn a_plate_goes_to_the_printer_and_prints_to_its_end() {
    let mock = Mock::new(Setup::default());
    let probe = Probe::new(Some(&mock));
    // Found, not started: nothing is asked of the bridge yet.
    let bridge = probe.ask("bambu_bridge", json!({}));
    assert_eq!(bridge["state"], "idle");
    assert_eq!(mock.seen.lock().unwrap().starts, 0);
    // The checks of "Connect the printer using IP and access code".
    let base = json!({ "name": "P", "ip": "192.168.1.20", "access_code": CODE, "serial": SERIAL, "model": "C12" });
    for (key, value, said) in [
        ("ip", "192.168.1", "The IP address is not valid."),
        (
            "access_code",
            "1234",
            "The access code is eight letters or digits.",
        ),
        (
            "serial",
            "01P00A12",
            "The serial number is fifteen characters.",
        ),
        ("name", " ", "The name field is not allowed to be empty."),
        ("model", "XX9", "That printer model is not known."),
    ] {
        let mut args = base.clone();
        args[key] = json!(value);
        assert_eq!(probe.refused("bambu_printer_add", args), said);
    }
    let printers = probe.add_printer(CODE);
    assert_eq!(
        printers["printers"][0]["dev_id"], SERIAL,
        "the serial in upper case"
    );
    assert_eq!(printers["printers"][0]["has_access_code"], true);
    assert_eq!(printers["printers"][0]["model_name"], "Bambu Lab P1S");
    assert_eq!(printers["selected"], SERIAL);
    assert!(
        printers["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "N2S")
    );
    // Kept between runs, the code apart from the rest.
    let dir = probe._dir.path();
    let kept = std::fs::read_to_string(dir.join("bambu_printers.json")).unwrap();
    assert!(kept.contains(SERIAL) && !kept.contains(CODE), "{kept}");
    assert!(
        std::fs::read_to_string(dir.join("bambu_printers_secrets.json"))
            .unwrap()
            .contains(CODE)
    );

    let state = probe.connected();
    assert_eq!(state["machine"]["state"], "IDLE");
    assert_eq!(state["machine"]["said"], "Idle");
    assert_eq!(state["machine"]["ams"][0]["trays"][0]["name"], "A1");
    assert_eq!(state["machine"]["ams"][0]["trays"][2]["type"], "PETG");
    assert_eq!(state["machine"]["bed"], json!([54.8, 55.0]));

    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["printer"], SERIAL);
    assert_eq!(dialog["status"]["key"], "ams_mapping_success", "{dialog}");
    assert_eq!(
        dialog["status"]["message"],
        "Filaments to AMS slots mappings have been established. You can click a filament above to change its mapping AMS slot"
    );
    assert_eq!(dialog["status"]["can_send"], true);
    assert_eq!(
        dialog["options"]["use_ams"],
        json!({ "shown": true, "value": true })
    );
    assert_eq!(dialog["options"]["bed_leveling"]["value"], true);
    let filament = &dialog["filaments"][0];
    assert_eq!(filament["type"], "PLA");
    // A PLA goes to one of the two PLA trays.
    assert!(filament["tray"] == 0 || filament["tray"] == 1, "{filament}");
    // A tray picked by hand is checked: an empty slot is not valid.
    let picked = probe.ask(
        "bambu_send_prepare",
        json!({ "project": project, "mapping": [3] }),
    );
    assert_eq!(picked["status"]["key"], "ams_mapping_invalid");
    assert_eq!(picked["status"]["can_send"], false);

    let sent = probe.ask(
        "bambu_send",
        json!({ "project": project, "options": { "bed_leveling": false, "flow_cali": true, "timelapse": false, "use_ams": true } }),
    );
    let task = sent["task"].as_u64().unwrap_or_else(|| panic!("{sent}"));
    let end = probe.task_end(task);
    assert_eq!(end["ok"], true, "{end}");
    assert_eq!(end["percent"], 100);
    assert_eq!(
        end["message"],
        "Successfully sent. Will automatically jump to the device page in 3s"
    );
    // What the bridge was asked: the access-code check, then the print.
    let checks = mock.requests("start_send_gcode_to_sdcard");
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["params"]["project_name"], "verify_job");
    let prints = mock.requests("start_local_print");
    assert_eq!(prints.len(), 1);
    let params = &prints[0]["params"];
    assert_eq!(
        params["password"], CODE,
        "the code goes to the bridge, and only there"
    );
    assert_eq!(params["username"], "bblp");
    assert_eq!(params["dev_ip"], "192.168.1.20");
    assert_eq!(params["dev_id"], SERIAL);
    assert_eq!(params["connection_type"], "lan");
    assert_eq!(params["ftp_folder"], "sdcard/");
    assert_eq!(params["plate_index"], 1);
    assert_eq!(params["task_bed_leveling"], false);
    assert_eq!(params["task_flow_cali"], true);
    assert_eq!(params["task_record_timelapse"], false);
    assert_eq!(params["task_layer_inspect"], true);
    assert_eq!(params["task_use_ams"], true);
    // The plate's bed, as the G-code names it.
    assert_eq!(params["task_bed_type"], dialog["bed_gcode"]);
    assert_eq!(params["task_bed_type"], "cool_plate");
    assert!(params["filename"].as_str().unwrap().ends_with(".gcode.3mf"));
    let v0: Value = serde_json::from_str(params["ams_mapping"].as_str().unwrap()).unwrap();
    assert_eq!(v0[0], filament["tray"]);
    // The file is gone once sent.
    assert!(!Path::new(params["filename"].as_str().unwrap()).exists());
    // The options are remembered.
    let again = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(again["options"]["bed_leveling"]["value"], false);
    assert_eq!(again["options"]["timelapse"]["value"], false);
    // Busy now: the dialog says so, and nothing is sent.
    assert_eq!(
        again["status"]["message"],
        "The printer is busy with another print job."
    );
    assert_eq!(
        probe.refused("bambu_send", json!({ "project": project })),
        "The printer is busy with another print job."
    );

    // Device: the print as the printer reports it, to its end.
    let state = probe.ask("bambu_state", json!({}));
    assert_eq!(state["machine"]["said"], "Printing");
    assert_eq!(state["machine"]["left_text"], "42m");
    assert_eq!(state["machine"]["can_pause"], true);
    mock.report(json!({ "mc_percent": 50, "layer_num": 60, "mc_remaining_time": 21 }));
    let half = probe.until("half way", || {
        let state = probe.ask("bambu_state", json!({}));
        (state["machine"]["percent"] == 50).then_some(state)
    });
    assert_eq!(half["machine"]["layer"], 60);
    assert_eq!(half["machine"]["total_layers"], 120);
    // Pause and resume at once; stop only once confirmed.
    assert_eq!(
        probe.ask("bambu_control", json!({ "action": "pause" }))["sent"],
        "pause"
    );
    probe.until("paused", || {
        (probe.ask("bambu_state", json!({}))["machine"]["said"] == "Paused").then_some(())
    });
    assert_eq!(
        probe.ask("bambu_control", json!({ "action": "resume" }))["sent"],
        "resume"
    );
    let ask_first = probe.ask("bambu_control", json!({ "action": "stop" }));
    assert_eq!(ask_first["confirm"]["title"], "Cancel print");
    assert_eq!(
        ask_first["confirm"]["lines"][0],
        "Are you sure you want to cancel this print?"
    );
    let commands: Vec<String> = mock
        .requests("send_message_to_printer")
        .iter()
        .filter_map(|p| {
            serde_json::from_str::<Value>(p["json"].as_str()?).ok()?["print"]["command"]
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(
        commands,
        vec!["pause", "resume"],
        "nothing stopped before it was confirmed"
    );
    mock.report(json!({ "gcode_state": "RUNNING", "mc_percent": 100, "layer_num": 120, "mc_remaining_time": 0 }));
    mock.report(json!({ "gcode_state": "FINISH" }));
    let done = probe.until("the end", || {
        let state = probe.ask("bambu_state", json!({}));
        (state["machine"]["state"] == "FINISH").then_some(state)
    });
    assert_eq!(done["machine"]["said"], "Finished");
    assert_eq!(done["machine"]["percent"], 100);
    assert_eq!(done["machine"]["can_stop"], false);
    probe.never_said_a_code();
}

#[test]
fn without_a_bridge_it_says_so_and_tries_nothing() {
    let probe = Probe::new(None);
    let bridge = probe.ask("bambu_bridge", json!({ "start": true }));
    assert_eq!(bridge["state"], "absent");
    assert_eq!(bridge["found"], false);
    assert!(
        bridge["said"]
            .as_str()
            .unwrap()
            .starts_with("Network plug-in missing No UltraNet bridge was found")
    );
    probe.add_printer(CODE);
    let connect = probe.ask("bambu_connect", json!({}));
    assert_eq!(connect["error"], "Network plug-in missing");
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["status"]["key"], "bridge");
    assert_eq!(dialog["status"]["can_send"], false);
    assert!(
        probe
            .refused("bambu_send", json!({ "project": project }))
            .starts_with("Network plug-in missing")
    );
    probe.never_said_a_code();
}

#[test]
fn an_old_bridge_is_refused_before_anything_is_sent() {
    let mock = Mock::new(Setup {
        protocol: 0,
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    let connect = probe.ask("bambu_connect", json!({}));
    assert_eq!(
        connect["error"],
        "The UltraNet bridge is too old for this app (protocol 0, this app needs 1)."
    );
    assert_eq!(connect["bridge"]["state"], "refused");
    assert!(mock.requests("connect_printer").is_empty());
    probe.never_said_a_code();
}

#[test]
fn a_wrong_access_code_is_said_without_the_code() {
    // A bridge that echoes the code it was given, as the reference's own
    // NetworkAgent.cpp logs it: still never shown.
    let mock = Mock::new(Setup {
        echo_code: true,
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(WRONG);
    let state = probe.ask("bambu_connect", json!({}));
    assert_eq!(state["session"], "failed");
    assert_eq!(state["refused"], true);
    assert_eq!(state["said"], "Incorrect password");
    // Forgotten, to be entered again (`erase_user_access_code`).
    let printers = probe.ask("bambu_printers", json!({}));
    assert_eq!(printers["printers"][0]["has_access_code"], false);
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(
        dialog["status"]["message"],
        "The printer refused the access code. Re-enter it"
    );
    assert_eq!(dialog["refused"], true);
    // Entered again, the right one this time.
    probe.ask(
        "bambu_printer_code",
        json!({ "dev_id": SERIAL, "access_code": CODE }),
    );
    probe.connected();
    probe.never_said_a_code();
}

#[test]
fn an_ams_that_holds_nothing_the_plate_prints_with_blocks_the_send() {
    let mock = Mock::new(Setup {
        trays: vec![(0, "PETG", "FF0000FF"), (1, "ABS", "00FF00FF")],
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["status"]["key"], "ams_mapping_invalid", "{dialog}");
    assert_eq!(
        dialog["status"]["message"],
        "Please click each filament above to specify its mapping AMS slot before sending the print job"
    );
    assert_eq!(dialog["filaments"][0]["tray_name"], "-");
    assert!(
        probe
            .refused("bambu_send", json!({ "project": project }))
            .starts_with("Please click each filament")
    );
    // "Enable AMS" off: sent from the external spool, without a mapping.
    let off = probe.ask(
        "bambu_send_prepare",
        json!({ "project": project, "use_ams": false }),
    );
    assert_eq!(off["status"]["key"], "ready");
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "options": { "use_ams": false } }),
    )["task"]
        .as_u64()
        .unwrap();
    assert_eq!(probe.task_end(task)["ok"], true);
    let params = &mock.requests("start_local_print")[0]["params"];
    assert_eq!(params["task_use_ams"], false);
    assert_eq!(params["ams_mapping"], "");
    probe.never_said_a_code();
}

#[test]
fn a_printer_that_is_printing_is_not_sent_to() {
    let mock = Mock::new(Setup {
        state: "RUNNING".into(),
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["status"]["key"], "in_printing");
    assert_eq!(
        probe.refused("bambu_send", json!({ "project": project })),
        "The printer is busy with another print job."
    );
    assert!(mock.requests("start_local_print").is_empty());
}

#[test]
fn a_bridge_that_dies_in_the_upload_ends_the_send() {
    let mock = Mock::new(Setup {
        die_in_upload: true,
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let task = probe.ask("bambu_send", json!({ "project": project }))["task"]
        .as_u64()
        .unwrap();
    let end = probe.task_end(task);
    assert_eq!(end["ok"], false);
    assert_eq!(end["error"], "The UltraNet bridge stopped.");
    let state = probe.ask("bambu_state", json!({}));
    assert_eq!(state["session"], "lost");
    assert_eq!(state["bridge"]["state"], "failed");
    probe.never_said_a_code();
}

#[test]
fn a_printer_not_in_developer_mode_is_said_before_sending() {
    let mock = Mock::new(Setup {
        secured: true,
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let task = probe.ask("bambu_send", json!({ "project": project }))["task"]
        .as_u64()
        .unwrap();
    let end = probe.task_end(task);
    assert_eq!(
        end["error"],
        "This printer is not in Developer Mode, so it rejects commands from EdgeSlicer. Turn on LAN Only + Developer Mode on the printer."
    );
    assert!(mock.requests("start_local_print").is_empty());
    let refused = probe.refused("bambu_control", json!({ "action": "pause" }));
    assert!(refused.starts_with("This printer is not in Developer Mode"));
}
