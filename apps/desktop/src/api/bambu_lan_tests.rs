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
use crate::secrets::{
    self, FileStore, Memory, Refusing, Secrets, System,
    tests::{Asked, Counted},
};
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
        account: Duration::from_secs(5),
    }
}

/// A session and every answer it gave, to search for the code.
struct Probe {
    session: Arc<Session>,
    answers: Mutex<Vec<String>>,
    _dir: tempfile::TempDir,
}

impl Probe {
    /// A session over `dir`'s settings, its codes in `secrets`.
    fn with_store(mock: &Mock, dir: tempfile::TempDir, secrets: Box<dyn Secrets>) -> Self {
        let session = Session::with_bridge_and_secrets(
            Some(dir.path().to_owned()),
            mock.launcher(),
            quick(),
            secrets,
        );
        Probe {
            session: Arc::new(session),
            answers: Mutex::new(Vec::new()),
            _dir: dir,
        }
    }

    /// A session over a data folder of its own that starts `launcher`'s
    /// bridge (the real one, for the end-to-end test), its codes in memory.
    fn with_launcher(launcher: crate::ultranet::client::Launcher, timeouts: Timeouts) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::with_bridge_and_secrets(
            Some(dir.path().to_owned()),
            launcher,
            timeouts,
            Box::new(Memory::default()),
        );
        Probe {
            session: Arc::new(session),
            answers: Mutex::new(Vec::new()),
            _dir: dir,
        }
    }

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
        self.sliced_project_in("Bambu PLA Basic")
    }

    /// The small project on a P1S's presets with the first filament whose
    /// name starts with `filament`, sliced.
    fn sliced_project_in(&self, filament: &str) -> u64 {
        let id = self.ask("project_open", json!({ "path": reference(SMALL) }))["id"]
            .as_u64()
            .unwrap();
        let presets = self.ask("presets", json!({ "vendor": "BBL", "machine": P1S }));
        let pla = presets["filaments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f.as_str().is_some_and(|name| name.starts_with(filament)))
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

    fn until<T>(&self, what: &str, ready: impl FnMut() -> Option<T>) -> T {
        self.until_within(what, Duration::from_secs(20), ready)
    }

    /// As `until`, for a real bridge and printer (the end-to-end test).
    fn until_long<T>(&self, what: &str, ready: impl FnMut() -> Option<T>) -> T {
        self.until_within(what, Duration::from_secs(150), ready)
    }

    fn until_within<T>(
        &self,
        what: &str,
        within: Duration,
        mut ready: impl FnMut() -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + within;
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
        json!({ "project": project, "options": { "bed_leveling": false, "flow_cali": true, "timelapse": false, "use_ams": true }, "confirmed": "printer" }),
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
        json!({ "project": project, "options": { "use_ams": false }, "confirmed": "printer" }),
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
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    )["task"]
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
fn a_printer_of_the_account_is_offered_in_the_send_dialog_and_prints_over_the_lan() {
    let mock = Mock::new(Setup::default());
    let probe = Probe::new(Some(&mock));
    probe
        .session
        .set_sign_in_window(Arc::new(|_: &url::Url| Ok(())));
    probe.ask("bambu_account_sign_in", json!({}));
    assert!(
        probe
            .session
            .bambu_sign_in_message(crate::ultranet::account::Message::Ticket(
                crate::ultranet::mock::TICKET.into()
            ))
    );
    let project = probe.sliced_project();
    // None added by hand: the dialog lists the account's, without codes.
    let dialog = probe.ask(
        "bambu_send_prepare",
        json!({ "project": project, "connect": true }),
    );
    assert!(dialog["printer"].is_null(), "{dialog}");
    let offered: Vec<&str> = dialog["account_printers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["dev_id"].as_str().unwrap())
        .collect();
    assert_eq!(offered, [SERIAL, "00M09A350100777"]);
    assert_eq!(dialog["account_printers"][0]["has_access_code"], true);
    // Its address entered (the cloud gives none), its code the account's.
    probe.ask(
        "bambu_printer_add",
        json!({ "ip": "127.0.0.1", "access_code": "", "name": "Workshop P1S", "serial": SERIAL, "model": "C12" }),
    );
    probe.connected();
    let connects = mock.requests("connect_printer");
    assert_eq!(connects.last().unwrap()["dev_ip"], "127.0.0.1");
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["printer"], SERIAL);
    assert_eq!(dialog["ip"], "127.0.0.1");
    assert_eq!(dialog["status"]["can_send"], true, "{dialog}");
    let offered: Vec<&str> = dialog["account_printers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["dev_id"].as_str().unwrap())
        .collect();
    assert_eq!(offered, ["00M09A350100777"], "listed once, as added");
    // Nothing made the printer act before "Send".
    assert!(mock.requests("start_send_gcode_to_sdcard").is_empty());
    assert!(mock.requests("start_local_print").is_empty());
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    )["task"]
        .as_u64()
        .unwrap();
    let end = probe.task_end(task);
    assert_eq!(end["ok"], true, "{end}");
    let prints = mock.requests("start_local_print");
    assert_eq!(prints.len(), 1);
    assert_eq!(prints[0]["params"]["dev_ip"], "127.0.0.1");
    assert_eq!(
        prints[0]["params"]["password"], CODE,
        "the account's code, to the bridge only"
    );
    // No cloud print: only the LAN jobs and the account's calls went out.
    assert!(mock.requests("start_local_print_with_record").is_empty());
    probe.never_said_a_code();
}

#[test]
fn an_account_code_the_printer_refuses_gives_way_to_one_typed() {
    // The printer's code changed since the cloud last gave it.
    let mock = Mock::new(Setup {
        code: "99998888".into(),
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe
        .session
        .set_sign_in_window(Arc::new(|_: &url::Url| Ok(())));
    probe.ask("bambu_account_sign_in", json!({}));
    assert!(
        probe
            .session
            .bambu_sign_in_message(crate::ultranet::account::Message::Ticket(
                crate::ultranet::mock::TICKET.into()
            ))
    );
    probe.ask(
        "bambu_printer_add",
        json!({ "ip": "127.0.0.1", "access_code": "", "name": "Workshop P1S", "serial": SERIAL, "model": "C12" }),
    );
    let state = probe.ask("bambu_connect", json!({}));
    assert_eq!(state["session"], "failed");
    assert_eq!(state["said"], "Incorrect password");
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["status"]["key"], "refused");
    assert_eq!(dialog["refused"], true);
    // Typed: it wins over the account's, and the printer takes it.
    probe.ask(
        "bambu_printer_code",
        json!({ "dev_id": SERIAL, "access_code": "99998888" }),
    );
    probe.connected();
    assert_eq!(
        mock.requests("connect_printer").last().unwrap()["password"],
        "99998888"
    );
    assert!(mock.requests("start_local_print").is_empty());
}

#[test]
fn a_failed_access_code_check_says_what_failed_and_only_that() {
    // UltraNet answers -5010 for every failed upload; the bridge names the
    // cause in `job_done`'s `failure` (ULTRANET_BRIDGE.md section 17).
    let mock = Mock::new(Setup::default());
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let send = |setup: &dyn Fn(&mut Setup)| {
        setup(&mut mock.setup.lock().unwrap());
        let task = probe.ask(
            "bambu_send",
            json!({ "project": project, "confirmed": "printer" }),
        )["task"]
            .as_u64()
            .unwrap();
        let end = probe.task_end(task);
        assert_eq!(end["ok"], false, "{end}");
        end
    };

    // FTP refuses the code while the session (MQTT) is up.
    let end = send(&|s| s.ftp_code = Some(WRONG.into()));
    assert_eq!(
        end["error"],
        "LAN send failed: the printer rejected the access code (error -5010)."
    );
    assert_eq!(end["refused"], true, "the code is offered to be re-entered");
    assert_eq!(end["ask_code"], false);

    // The printer cannot be reached over FTPS.
    let end = send(&|s| {
        s.ftp_code = None;
        s.upload_fails = Some(("", "unreachable"));
    });
    assert_eq!(
        end["error"],
        "LAN send failed: could not connect to the printer (error -5010)."
    );
    assert_eq!(end["refused"], false, "not said to be the code");
    assert_eq!(end["ask_code"], false);
    assert!(
        end["detail"].as_str().unwrap().contains("FTPS 990"),
        "{end}"
    );

    // Logged in, but the file did not get there.
    let end = send(&|s| s.upload_fails = Some(("", "upload")));
    assert_eq!(
        end["error"],
        "LAN send failed: uploading the file to the printer failed (error -5010)."
    );
    assert!(
        end["detail"].as_str().unwrap().starts_with(
            "The printer accepted the access code, but the file did not reach it whole."
        ),
        "{end}"
    );
    assert_eq!(end["refused"], false);

    // A bridge from before section 17: no cause is claimed.
    let end = send(&|s| s.names_failure = false);
    assert_eq!(
        end["error"],
        "The access-code check failed: the printer did not take the small test file (error -5010)."
    );
    let detail = end["detail"].as_str().unwrap();
    assert!(detail.starts_with("The printer may have refused the access code, it may not be reachable over FTPS (port 990), or its card may be full or write-protected."), "{detail}");
    assert!(
        detail.contains("STOR rejected"),
        "the plug-in's words: {detail}"
    );
    assert_eq!(end["refused"], false);
    assert_eq!(end["ask_code"], true, "the code may still be entered again");

    // Nothing was printed, and every check was the small test file.
    assert!(mock.requests("start_local_print").is_empty());
    assert!(
        mock.requests("start_send_gcode_to_sdcard")
            .iter()
            .all(|r| r["params"]["project_name"] == "verify_job")
    );
    // The code is never shown, and still kept: none of these forgot it.
    assert_eq!(
        probe.ask("bambu_printers", json!({}))["printers"][0]["has_access_code"],
        true
    );
    probe.never_said_a_code();
}

#[test]
fn a_failed_upload_of_the_print_says_its_proven_cause() {
    // The check passes, the print's own upload fails: -4020, said by the
    // cause the bridge proves, not by reading the plug-in's text.
    let mock = Mock::new(Setup {
        upload_fails: Some(("start_local_print", "upload")),
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    )["task"]
        .as_u64()
        .unwrap();
    let end = probe.task_end(task);
    assert_eq!(
        end["error"],
        "LAN send failed: uploading the file to the printer failed (error -4020)."
    );
    assert_eq!(end["refused"], false);
    assert_eq!(mock.requests("start_send_gcode_to_sdcard").len(), 1);
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
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    )["task"]
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

#[test]
fn bambu_pet_cf_in_an_ams_slot_is_refused_as_1039_decides() {
    // aceRage/EdgeSlicer#1039: the presets' "Bambu Lab" is the blacklist's
    // "Bambulab", so Bambu PET-CF is prohibited in an AMS slot.
    let mock = Mock::new(Setup {
        trays: vec![(0, "PET-CF", "222222FF")],
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project_in("Bambu PET-CF");
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    assert_eq!(dialog["filaments"][0]["type"], "PET-CF", "{dialog}");
    assert_eq!(dialog["filaments"][0]["tray"], 0);
    let refused = probe.ask("bambu_send", json!({ "project": project }));
    assert_eq!(refused["confirm"]["title"], "Errors");
    assert_eq!(
        refused["confirm"]["lines"][0],
        "Bambu PET-CF/PA6-CF is not supported by AMS."
    );
    assert_eq!(refused["confirm"]["blocking"], true);
    // Not even once confirmed.
    let again = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "warning" }),
    );
    assert_eq!(again["confirm"]["blocking"], true);
    assert!(mock.requests("start_local_print").is_empty());
}

// ---- where the code is kept (crate::secrets) ----

/// The system's store as a test has it: an item on `keyring-core`'s mock
/// store, never the real keychain.
fn mock_keychain(store: &Arc<keyring_core::mock::Store>) -> (Box<dyn Secrets>, Arc<Mutex<Asked>>) {
    let (item, asked) = Counted::on(store.clone());
    (Box::new(System::new(Box::new(item))), asked)
}

#[test]
fn no_answer_holds_the_code_whichever_store_keeps_it() {
    type Made = fn(&Path) -> Box<dyn Secrets>;
    let stores: [(&str, Made); 4] = [
        ("memory", |_| Box::new(Memory::default())),
        ("file", |dir| {
            Box::new(FileStore::new(Some(dir.join(secrets::FILE))))
        }),
        ("system", |_| {
            mock_keychain(&keyring_core::mock::Store::new().unwrap()).0
        }),
        ("refused", |_| {
            Box::new(System::new(Box::new(Refusing(
                "the keychain is locked".into(),
            ))))
        }),
    ];
    for (name, store) in stores {
        // A bridge that echoes the code it is given.
        let mock = Mock::new(Setup {
            echo_code: true,
            ..Setup::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let secrets = store(dir.path());
        let probe = Probe::with_store(&mock, dir, secrets);
        probe.add_printer(WRONG);
        let state = probe.ask("bambu_connect", json!({}));
        assert_eq!(state["refused"], true, "{name}: {state}");
        probe.ask(
            "bambu_printer_code",
            json!({ "dev_id": SERIAL, "access_code": CODE }),
        );
        probe.connected();
        let printers = probe.ask("bambu_printers", json!({}));
        assert_eq!(printers["printers"][0]["has_access_code"], true, "{name}");
        let kept_in = if name == "refused" { "file" } else { name };
        assert_eq!(printers["secrets"]["kept_in"], kept_in, "{name}");
        assert_eq!(
            printers["secrets"]["notice"].is_string(),
            name == "refused",
            "{name}: {printers}"
        );
        // The plain list says there is a code, never what it is.
        let plain = std::fs::read_to_string(probe._dir.path().join("bambu_printers.json")).unwrap();
        assert!(!plain.contains(CODE) && !plain.contains(WRONG), "{plain}");
        assert!(plain.contains("\"access_code_kept\": true"), "{plain}");
        probe.ask("bambu_printer_remove", json!({ "dev_id": SERIAL }));
        probe.never_said_a_code();
        let connects = mock.requests("connect_printer");
        assert_eq!(connects.last().unwrap()["password"], CODE, "{name}");
    }
}

#[test]
fn the_keychain_is_asked_only_when_a_code_is_needed_and_once_a_run() {
    let mock = Mock::new(Setup::default());
    let store = keyring_core::mock::Store::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let kept = dir.path().to_owned();
    {
        // The first run adds the printer: one write, nothing read but the
        // empty store.
        let (secrets, asked) = mock_keychain(&store);
        let probe = Probe::with_store(&mock, dir, secrets);
        probe.add_printer(CODE);
        let asked = asked.lock().unwrap();
        assert_eq!((asked.reads, asked.writes), (1, 1));
        assert!(
            !kept.join(secrets::FILE).exists(),
            "no file beside the store"
        );
        // The folder outlives this run.
        let Probe { _dir, .. } = probe;
        let _ = _dir.keep();
    }
    // The next run: the app starting, Device opened, the state looked at,
    // the send dialog opened without connecting: the store is not asked.
    let (secrets, asked) = mock_keychain(&store);
    let session = Arc::new(Session::with_bridge_and_secrets(
        Some(kept.clone()),
        mock.launcher(),
        quick(),
        secrets,
    ));
    let ask = |name: &str, args: Value| {
        session
            .call(name, &args)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
    };
    let printers = ask("bambu_printers", json!({}));
    assert_eq!(printers["printers"][0]["has_access_code"], true);
    assert_eq!(
        printers["secrets"]["at_hand"], false,
        "the page does not dial by itself"
    );
    ask("bambu_state", json!({ "redial": true }));
    ask("bambu_bridge", json!({}));
    assert_eq!(
        asked.lock().unwrap().reads,
        0,
        "nothing read before Connect"
    );
    // Connect: read once; again, and the state, read nothing more.
    let state = ask("bambu_connect", json!({}));
    assert_eq!(state["session"], "connected", "{state}");
    ask("bambu_connect", json!({}));
    ask("bambu_state", json!({ "redial": true }));
    let printers = ask("bambu_printers", json!({}));
    assert_eq!(printers["secrets"]["at_hand"], true);
    {
        let asked = asked.lock().unwrap();
        assert_eq!((asked.reads, asked.writes), (1, 0), "one read, no write");
    }
    // The same code entered again writes nothing; a new one writes once.
    ask(
        "bambu_printer_code",
        json!({ "dev_id": SERIAL, "access_code": CODE }),
    );
    assert_eq!(asked.lock().unwrap().writes, 0);
    ask(
        "bambu_printer_code",
        json!({ "dev_id": SERIAL, "access_code": "ABCDEFGH" }),
    );
    assert_eq!(asked.lock().unwrap().writes, 1);
    // Removing the printer removes its code: the item goes with the last.
    ask("bambu_printer_remove", json!({ "dev_id": SERIAL }));
    assert_eq!(asked.lock().unwrap().removes, 1);
    let (mut later, _) = mock_keychain(&store);
    assert_eq!(later.get(SERIAL).unwrap(), None);
    let _ = std::fs::remove_dir_all(&kept);
}

/// A data folder as a version before the system store left it: the
/// printer listed without saying it has a code, the code in the file.
fn kept_before(dir: &Path) {
    std::fs::write(
        dir.join("bambu_printers.json"),
        format!(
            r#"{{"version":1,"selected":"{SERIAL}","print":{{}},"printers":[{{"dev_id":"{SERIAL}","dev_name":"Workshop P1S","dev_ip":"192.168.1.20","printer_type":"C12"}}]}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join(secrets::FILE),
        format!(r#"{{"{SERIAL}":{{"access_code":"{CODE}"}}}}"#),
    )
    .unwrap();
}

#[test]
fn codes_in_the_old_file_move_to_the_system_store_on_first_use() {
    let mock = Mock::new(Setup::default());
    let store = keyring_core::mock::Store::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    kept_before(dir.path());
    let (secrets, asked) = mock_keychain(&store);
    let probe = Probe::with_store(&mock, dir, secrets);
    // Listed with its code, nothing moved yet.
    let printers = probe.ask("bambu_printers", json!({}));
    assert_eq!(printers["printers"][0]["has_access_code"], true);
    assert_eq!(asked.lock().unwrap().reads, 0);
    let file = probe._dir.path().join(secrets::FILE);
    assert!(file.exists());
    // Connect moves it: written, read back, the file removed.
    probe.connected();
    assert!(!file.exists(), "the file goes once the store gave it back");
    let (mut later, _) = mock_keychain(&store);
    assert_eq!(later.get(SERIAL).unwrap().as_deref(), Some(CODE));
    let printers = probe.ask("bambu_printers", json!({}));
    assert_eq!(printers["secrets"]["notice"], Value::Null);
    assert_eq!(printers["secrets"]["kept_in"], "system");
    probe.never_said_a_code();
}

#[test]
fn a_move_the_store_does_not_give_back_leaves_the_file_and_says_so() {
    let mock = Mock::new(Setup::default());
    let dir = tempfile::tempdir().unwrap();
    kept_before(dir.path());
    let (item, asked) = Counted::new();
    asked.lock().unwrap().forgetful = true;
    let probe = Probe::with_store(&mock, dir, Box::new(System::new(Box::new(item))));
    let file = probe._dir.path().join(secrets::FILE);
    let before = std::fs::read_to_string(&file).unwrap();
    // Still connects, with the code from the file.
    probe.connected();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before, "untouched");
    let printers = probe.ask("bambu_printers", json!({}));
    let notice = printers["secrets"]["notice"].as_str().unwrap();
    assert!(
        notice.starts_with("The access codes kept in a file could not be moved to ")
            && notice.contains("it did not give back what was written")
            && notice.ends_with(
                "They stay in the file on this computer, readable by your user account only."
            ),
        "{notice}"
    );
    assert_eq!(printers["secrets"]["kept_in"], "file");
    probe.never_said_a_code();
}

#[test]
fn a_store_that_refuses_gives_way_to_the_file_and_says_so() {
    let mock = Mock::new(Setup::default());
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::with_store(
        &mock,
        dir,
        Box::new(System::new(Box::new(Refusing(
            "the macOS Keychain did not allow it (User canceled the operation.)".into(),
        )))),
    );
    // Nothing said before anything was asked of it.
    let printers = probe.ask("bambu_printers", json!({}));
    assert_eq!(printers["secrets"]["notice"], Value::Null);
    probe.add_printer(CODE);
    let printers = probe.ask("bambu_printers", json!({}));
    let notice = printers["secrets"]["notice"].as_str().unwrap();
    assert!(
        notice.ends_with(
            "could not be used: the macOS Keychain did not allow it (User canceled the operation.). Access codes are kept in a file on this computer instead, readable by your user account only."
        ),
        "{notice}"
    );
    assert!(
        std::fs::read_to_string(probe._dir.path().join(secrets::FILE))
            .unwrap()
            .contains(CODE)
    );
    probe.connected();
    probe.never_said_a_code();
}

#[path = "bambu_send_e2e.rs"]
mod e2e;

// ---- the question before a send (`send_ask`) ----

#[test]
fn send_asks_one_question_and_never_none() {
    use super::{SendAsk, send_ask};
    // (warned, blocking, confirmed) -> what is asked.
    let table = [
        (false, false, None, SendAsk::Printer),
        (false, false, Some("printer"), SendAsk::Send),
        (false, false, Some("warning"), SendAsk::Send),
        (false, false, Some("yes"), SendAsk::Printer),
        (true, false, None, SendAsk::Warning),
        (true, false, Some("warning"), SendAsk::Send),
        // Only the plain question answered, and a warning since: shown.
        (true, false, Some("printer"), SendAsk::Warning),
        (true, true, Some("warning"), SendAsk::Blocked),
        (false, true, Some("printer"), SendAsk::Blocked),
        (true, true, None, SendAsk::Blocked),
    ];
    for (warned, blocking, confirmed, asked) in table {
        assert_eq!(
            send_ask(warned, blocking, confirmed),
            asked,
            "{warned} {blocking} {confirmed:?}"
        );
    }
}

#[test]
fn a_send_with_no_warning_asks_which_printer_first() {
    let mock = Mock::new(Setup::default());
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let asked = probe.ask("bambu_send", json!({ "project": project }));
    assert!(asked.get("task").is_none(), "{asked}");
    assert_eq!(asked["confirm"]["title"], "Send print job to");
    assert_eq!(
        asked["confirm"]["lines"],
        json!(["Print this plate on Workshop P1S (192.168.1.20)?"])
    );
    assert_eq!(asked["confirm"]["answer"], "printer");
    assert_eq!(asked["confirm"]["blocking"], false);
    // An answer that is not the question's is no answer.
    let again = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": true }),
    );
    assert_eq!(again["confirm"]["answer"], "printer", "{again}");
    assert!(mock.requests("start_send_gcode_to_sdcard").is_empty());
    assert!(mock.requests("start_local_print").is_empty());
    let task = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    )["task"]
        .as_u64()
        .unwrap();
    assert_eq!(probe.task_end(task)["ok"], true);
    assert_eq!(mock.requests("start_local_print").len(), 1);
    probe.never_said_a_code();
}

#[test]
fn the_reference_warning_accepted_is_the_only_question() {
    // The printer's nozzle is 0.6, the plate sliced for 0.4: `on_ok_btn`'s
    // warning, which does not block.
    let mock = Mock::new(Setup {
        nozzle_diameter: 0.6,
        ..Setup::default()
    });
    let probe = Probe::new(Some(&mock));
    probe.add_printer(CODE);
    probe.connected();
    let project = probe.sliced_project();
    let warned = probe.ask("bambu_send", json!({ "project": project }));
    assert_eq!(warned["confirm"]["title"], "Warning", "{warned}");
    assert_eq!(warned["confirm"]["answer"], "warning");
    assert_eq!(warned["confirm"]["blocking"], false);
    let lines = warned["confirm"]["lines"].to_string();
    assert!(lines.contains("nozzle diameter"), "{lines}");
    assert!(!lines.contains("Print this plate"), "{lines}");
    // Only the plain question answered: the warning is still asked.
    let again = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "printer" }),
    );
    assert_eq!(again["confirm"]["answer"], "warning", "{again}");
    assert!(mock.requests("start_send_gcode_to_sdcard").is_empty());
    // The warning accepted: sent, nothing more asked.
    let sent = probe.ask(
        "bambu_send",
        json!({ "project": project, "confirmed": "warning" }),
    );
    let task = sent["task"].as_u64().unwrap_or_else(|| panic!("{sent}"));
    assert_eq!(probe.task_end(task)["ok"], true);
    assert_eq!(mock.requests("start_local_print").len(), 1);
    probe.never_said_a_code();
}
