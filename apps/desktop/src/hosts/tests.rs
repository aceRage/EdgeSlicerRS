//! Each host against a mock of its printer on 127.0.0.1 (`mock`): "Test",
//! "Upload", "Upload and Print", a wrong key, a printer that does not
//! answer, a cancel, a busy printer, and the bytes the printer receives.

use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use serde_json::{Value, json};

use super::{
    Connection, Failure, Host, Options, Progress, TIMED_OUT, Timeouts, Upload,
    elegoo::{ElegooLink, PART},
    mock::{self, Got, Printer, Silent, Socket},
    octoprint::OctoPrint,
    queue::Queue,
    snapmaker::{BUSY, SnapmakerLan, mapping_script},
};

const KEY: &str = "the-right-key";

fn quick() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(2),
        ask: Duration::from_millis(600),
        write: Duration::from_secs(2),
        answer: Duration::from_millis(600),
        start: Duration::from_secs(3),
    }
}

fn connection(host_type: &str, address: &str, model: &str) -> Connection {
    Connection {
        host_type: host_type.into(),
        print_host: address.into(),
        apikey: Some(KEY.into()),
        printer_model: model.into(),
        ..Connection::default()
    }
}

fn upload(name: &str, data: Arc<Vec<u8>>, start: bool) -> Upload {
    Upload {
        name: name.into(),
        data,
        start,
        options: Options::default(),
    }
}

// ---- OctoPrint ----

/// OctoPrint as it answers: its version, and files to `api/files/local`;
/// 403 without the right key.
fn octoprint() -> Printer {
    Printer::start(
        Box::new(|got: &Got, _: &[Got]| {
            if got.header("X-Api-Key") != Some(KEY) {
                return (403, "Invalid API key".into());
            }
            match (got.method.as_str(), got.url.as_str()) {
                ("GET", "/api/version") => (
                    200,
                    json!({ "api": "0.1", "server": "1.9.0", "text": "OctoPrint 1.9.0" })
                        .to_string(),
                ),
                ("POST", "/api/files/local") => (201, json!({ "done": true }).to_string()),
                _ => (404, "Not Found".into()),
            }
        }),
        None,
    )
}

#[test]
fn octoprint_is_tested_and_sent_to() {
    let printer = octoprint();
    let host = OctoPrint::new(&connection("octoprint", &printer.address(), ""), quick()).unwrap();
    assert_eq!(host.test(), Ok(()));
    assert_eq!(
        host.test_ok(),
        "Connection to OctoPrint is working correctly."
    );

    let data = mock::gcode(300_000);
    let progress = Progress::default();
    host.upload(
        &upload("cube_PLA_10m.gcode", data.clone(), false),
        &progress,
    )
    .unwrap();
    assert_eq!(progress.sent.load(Ordering::Relaxed), data.len() as u64);
    let sent = printer.got().last().unwrap().clone();
    assert_eq!(sent.url, "/api/files/local");
    assert_eq!(
        sent.header("Content-Length"),
        Some(sent.body.len().to_string().as_str())
    );
    let form = sent.form();
    let names: Vec<&str> = form.iter().map(|(name, _, _)| name.as_str()).collect();
    assert_eq!(names, ["print", "path", "file"]);
    assert_eq!(sent.field("print").as_deref(), Some("false"));
    assert_eq!(form[2].1.as_deref(), Some("cube_PLA_10m.gcode"));
    assert!(form[2].2 == *data, "the bytes received are the bytes sent");
    // "Test" goes first, as in the reference.
    assert_eq!(
        printer.asked(),
        [
            "GET /api/version",
            "GET /api/version",
            "POST /api/files/local"
        ]
    );

    host.upload(&upload("cube.gcode", data, true), &Progress::default())
        .unwrap();
    assert_eq!(
        printer.got().last().unwrap().field("print").as_deref(),
        Some("true")
    );
}

#[test]
fn octoprint_with_the_wrong_key_says_so() {
    let printer = octoprint();
    let mut wrong = connection("octoprint", &printer.address(), "");
    wrong.apikey = Some("not-it".into());
    let host = OctoPrint::new(&wrong, quick()).unwrap();
    let reason = host.test().unwrap_err();
    assert_eq!(reason, "HTTP 403: Invalid API key");
    assert!(
        host.test_failed(&reason)
            .starts_with("Could not connect to OctoPrint: HTTP 403")
    );
    let failed = host.upload(
        &upload("a.gcode", mock::gcode(10), false),
        &Progress::default(),
    );
    assert_eq!(
        failed,
        Err(Failure::Error("HTTP 403: Invalid API key".into()))
    );
    // Nothing was sent but the test.
    assert_eq!(printer.asked(), ["GET /api/version", "GET /api/version"]);
    // The key is in a header, never in the URL.
    assert!(printer.got().iter().all(|got| !got.url.contains("not-it")));
}

#[test]
fn a_moonraker_answering_as_octoprint_is_taken() {
    let printer = Printer::start(
        Box::new(|_: &Got, _: &[Got]| {
            (
                200,
                json!({ "api": "0.1", "server": "1.1.1", "text": "OctoPrint (Moonraker v0.9)" })
                    .to_string(),
            )
        }),
        None,
    );
    let host = OctoPrint::new(&connection("octoprint", &printer.address(), ""), quick()).unwrap();
    assert_eq!(host.test(), Ok(()));
    let other = Printer::start(
        Box::new(|_: &Got, _: &[Got]| {
            (
                200,
                json!({ "api": "0.1", "text": "Something else" }).to_string(),
            )
        }),
        None,
    );
    let host = OctoPrint::new(&connection("octoprint", &other.address(), ""), quick()).unwrap();
    assert_eq!(
        host.test(),
        Err("Mismatched type of print host: Something else".into())
    );
}

#[test]
fn a_printer_that_does_not_answer_times_out() {
    let silent = Silent::start();
    let address = format!("127.0.0.1:{}", silent.port);
    let started = std::time::Instant::now();
    let host = OctoPrint::new(&connection("octoprint", &address, ""), quick()).unwrap();
    assert_eq!(host.test(), Err(TIMED_OUT.into()));
    let lan =
        SnapmakerLan::new(&connection("octoprint", &address, "Snapmaker U1"), quick()).unwrap();
    assert_eq!(lan.test(), Err(TIMED_OUT.into()));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn nothing_answers_on_a_closed_port() {
    // A port taken and let go at once: nothing listens there.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let host = OctoPrint::new(
        &connection("octoprint", &format!("127.0.0.1:{port}"), ""),
        quick(),
    )
    .unwrap();
    let reason = host.test().unwrap_err();
    assert!(
        reason.starts_with("Could not connect to the printer"),
        "{reason}"
    );
}

#[test]
fn an_upload_is_called_off() {
    // The printer reads slowly: the file is still going when it is called off.
    let printer = Printer::start(
        Box::new(|got: &Got, _: &[Got]| match got.url.as_str() {
            "/api/version" => (
                200,
                json!({ "api": "0.1", "text": "OctoPrint" }).to_string(),
            ),
            _ => (201, "{}".into()),
        }),
        Some(Duration::from_millis(300)),
    );
    let host = OctoPrint::new(&connection("octoprint", &printer.address(), ""), quick()).unwrap();
    let progress = Arc::new(Progress::default());
    let watcher = {
        let progress = progress.clone();
        std::thread::spawn(move || {
            while progress.sent.load(Ordering::Relaxed) == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
            progress.cancel.store(true, Ordering::Relaxed);
        })
    };
    let result = host.upload(
        &upload("big.gcode", mock::gcode(16 * 1024 * 1024), false),
        &progress,
    );
    watcher.join().unwrap();
    assert_eq!(result, Err(Failure::Cancelled));
    assert!(progress.sent.load(Ordering::Relaxed) < 16 * 1024 * 1024);
}

// ---- Snapmaker U1 ----

/// A U1's Moonraker: `state` is what `print_stats` says.
fn u1(state: &'static str) -> Printer {
    Printer::start(
        Box::new(move |got: &Got, before: &[Got]| {
            let url = got.url.as_str();
            match (got.method.as_str(), url) {
                ("GET", "/server/info") => (
                    200,
                    json!({ "result": { "klippy_state": "ready" } }).to_string(),
                ),
                ("GET", "/access/info") => (
                    200,
                    json!({ "result": { "login_required": false } }).to_string(),
                ),
                ("GET", "/printer/objects/query?print_stats") => (
                    200,
                    json!({ "result": { "status": { "print_stats": { "state": state } } } })
                        .to_string(),
                ),
                ("POST", "/server/files/upload") => (
                    201,
                    json!({ "result": { "item": { "path": "x" } } }).to_string(),
                ),
                ("GET", _) if url.starts_with("/server/files/metadata?filename=") => {
                    let size = before
                        .iter()
                        .rev()
                        .find(|got| got.url == "/server/files/upload")
                        .map(|got| {
                            got.form()
                                .into_iter()
                                .find(|(name, _, _)| name == "file")
                                .unwrap()
                                .2
                                .len()
                        })
                        .unwrap_or(0);
                    (200, json!({ "result": { "size": size } }).to_string())
                }
                ("POST", _) if url.starts_with("/printer/gcode/script?script=") => {
                    (200, json!({ "result": "ok" }).to_string())
                }
                ("POST", _) if url.starts_with("/printer/print/start?filename=") => {
                    (200, json!({ "result": "ok" }).to_string())
                }
                ("GET", "/server/files/list?root=gcodes") => (
                    200,
                    json!({ "result": [
                        { "path": "old.gcode", "modified": 100.0, "size": 10 },
                        { "path": "new one.gcode", "modified": 200.0, "size": 20 }
                    ] })
                    .to_string(),
                ),
                ("DELETE", "/server/files/gcodes/new%20one.gcode") => {
                    (200, json!({ "result": {} }).to_string())
                }
                _ => (
                    404,
                    json!({ "error": { "code": 404, "message": "Not Found" } }).to_string(),
                ),
            }
        }),
        None,
    )
}

#[test]
fn a_u1_is_tested_and_sent_a_file() {
    let printer = u1("standby");
    let host = SnapmakerLan::new(
        &connection("octoprint", &printer.address(), "Snapmaker U1"),
        quick(),
    )
    .unwrap();
    assert_eq!(host.test(), Ok(()));
    let data = mock::gcode(200_000);
    host.upload(&upload("cube", data.clone(), false), &Progress::default())
        .unwrap();
    let sent = printer.got().last().unwrap().clone();
    assert_eq!(sent.url, "/server/files/upload");
    let form = sent.form();
    let names: Vec<&str> = form.iter().map(|(name, _, _)| name.as_str()).collect();
    assert_eq!(names, ["print", "root", "file"]);
    assert_eq!(sent.field("print").as_deref(), Some("false"));
    assert_eq!(sent.field("root").as_deref(), Some("gcodes"));
    assert_eq!(form[2].1.as_deref(), Some("cube.gcode"));
    assert!(form[2].2 == *data);
    // Plain "Upload" starts nothing.
    assert!(
        !printer
            .asked()
            .iter()
            .any(|asked| asked.contains("print/start"))
    );
}

#[test]
fn a_u1_is_started_after_the_upload_with_its_toolheads() {
    let printer = u1("complete");
    let host = SnapmakerLan::new(
        &connection("octoprint", &printer.address(), "Snapmaker U1"),
        quick(),
    )
    .unwrap();
    let mut sending = upload("two colours.gcode", mock::gcode(50_000), true);
    sending.options.filaments = vec![0, 2];
    sending.options.unload_at_end = true;
    host.upload(&sending, &Progress::default()).unwrap();
    let asked = printer.asked();
    let script = mapping_script(&[0, 2], true);
    assert_eq!(
        script,
        "SET_PRINT_EXTRUDER_MAP CONFIG_EXTRUDER=0 MAP_EXTRUDER=0\nSET_PRINT_EXTRUDER_MAP CONFIG_EXTRUDER=2 MAP_EXTRUDER=2\nSET_PRINT_USED_EXTRUDERS EXTRUDERS=0,2\nSET_PRINT_PREFERENCES BED_LEVEL=0 FLOW_CALIBRATE=0 TIME_LAPSE_CAMERA=0 END_UNLOAD_FILAMENT=[1,0,1,0]"
    );
    assert_eq!(
        asked,
        [
            "GET /printer/objects/query?print_stats".to_owned(),
            "POST /server/files/upload".into(),
            "GET /server/files/metadata?filename=two%20colours.gcode".into(),
            "GET /printer/objects/query?print_stats".into(),
            format!(
                "POST /printer/gcode/script?script={}",
                super::snapmaker::encode(&script, false)
            ),
            "POST /printer/print/start?filename=two%20colours.gcode".into(),
        ]
    );
}

#[test]
fn a_busy_u1_is_sent_nothing() {
    for state in ["printing", "paused"] {
        let printer = u1(state);
        let host = SnapmakerLan::new(
            &connection("octoprint", &printer.address(), "Snapmaker U1"),
            quick(),
        )
        .unwrap();
        let result = host.upload(
            &upload("a.gcode", mock::gcode(1000), true),
            &Progress::default(),
        );
        assert_eq!(result, Err(Failure::Error(BUSY.into())));
        assert_eq!(printer.asked(), ["GET /printer/objects/query?print_stats"]);
        // A plain upload to a busy printer is fine: it starts nothing.
        host.upload(
            &upload("a.gcode", mock::gcode(1000), false),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(host.print("old.gcode"), Err(BUSY.into()));
    }
}

#[test]
fn a_u1_lists_deletes_and_prints_its_files() {
    let printer = u1("standby");
    let host = SnapmakerLan::new(
        &connection("octoprint", &printer.address(), "Snapmaker U1"),
        quick(),
    )
    .unwrap();
    let files = host.files().unwrap().unwrap();
    let names: Vec<&str> = files.iter().map(|file| file.name.as_str()).collect();
    assert_eq!(names, ["new one.gcode", "old.gcode"]);
    host.delete("new one.gcode").unwrap();
    host.print("old.gcode").unwrap();
    assert!(
        printer
            .asked()
            .contains(&"POST /printer/print/start?filename=old.gcode".to_owned())
    );
}

#[test]
fn a_moonraker_that_wants_a_key_refuses_the_wrong_one() {
    let printer = Printer::start(
        Box::new(|got: &Got, _: &[Got]| {
            if got.header("X-Api-Key") == Some(KEY) {
                (200, json!({ "result": {} }).to_string())
            } else {
                (
                    401,
                    json!({ "error": { "code": 401, "message": "Unauthorized" } }).to_string(),
                )
            }
        }),
        None,
    );
    let mut wrong = connection("octoprint", &printer.address(), "Snapmaker U1");
    wrong.apikey = Some("wrong".into());
    let host = SnapmakerLan::new(&wrong, quick()).unwrap();
    assert_eq!(host.test(), Err("HTTP 401: Unauthorized".into()));
    let right = SnapmakerLan::new(
        &connection("octoprint", &printer.address(), "Snapmaker U1"),
        quick(),
    )
    .unwrap();
    assert_eq!(right.test(), Ok(()));
}

// ---- Elegoo Centauri Carbon ----

/// The Centauri Carbon's web server: its page, the parts of a file, no
/// OctoPrint.
fn centauri() -> Printer {
    Printer::start(
        Box::new(
            |got: &Got, _: &[Got]| match (got.method.as_str(), got.url.as_str()) {
                ("GET", "/") => (200, "<html><title>ELEGOO</title></html>".into()),
                ("POST", "/uploadFile/upload") => (
                    200,
                    json!({ "code": "000000", "messages": null, "data": null, "success": true })
                        .to_string(),
                ),
                _ => (404, "Not Found".into()),
            },
        ),
        None,
    )
}

/// The printer's websocket: `states` is `CurrentStatus` for each status
/// asked for (the first, every time), `ack` the answer to command 128,
/// none for silence.
fn centauri_socket(states: Vec<i64>, ack: Option<i64>) -> Socket {
    Socket::start(Box::new(move |message: &Value, _| {
        match message["Data"]["Cmd"].as_i64()? {
        0 => Some(json!({ "Status": { "CurrentStatus": [states[0]] }, "MainboardID": "board", "Topic": "sdcp/status/board" })),
        128 => ack.map(|ack| json!({ "Id": "", "Data": { "Cmd": 128, "Data": { "Ack": ack }, "RequestID": "x" }, "Topic": "sdcp/response/board" })),
        _ => None,
    }
    }))
}

fn centauri_host(printer: &Printer, socket: &Socket) -> ElegooLink {
    let mut host = ElegooLink::new(
        &connection("elegoolink", &printer.address(), "Elegoo Centauri Carbon"),
        quick(),
    )
    .unwrap();
    host.ws_port = socket.port;
    host
}

#[test]
fn a_centauri_carbon_is_tested() {
    let printer = centauri();
    let socket = centauri_socket(vec![0], Some(0));
    let host = centauri_host(&printer, &socket);
    assert_eq!(host.test(), Ok(()));
    assert_eq!(
        host.test_ok(),
        "Connection to ElegooLink is working correctly."
    );
    assert_eq!(printer.asked(), ["GET /api/version", "GET /"]);
    let other = Printer::start(
        Box::new(|_: &Got, _: &[Got]| (200, "<html>a router</html>".into())),
        None,
    );
    let host = ElegooLink::new(
        &connection("elegoolink", &other.address(), "Elegoo Centauri Carbon"),
        quick(),
    )
    .unwrap();
    assert_eq!(host.test(), Err("ElegooLink not detected".into()));
    let cc2 = ElegooLink::new(
        &connection("elegoolink", &other.address(), "Elegoo Centauri Carbon 2"),
        quick(),
    )
    .unwrap();
    assert!(cc2.test().unwrap_err().contains("Centauri Carbon 2"));
}

#[test]
fn a_centauri_carbon_is_sent_a_file_in_parts() {
    let printer = centauri();
    let socket = centauri_socket(vec![0], Some(0));
    let host = centauri_host(&printer, &socket);
    let data = mock::gcode(2 * PART + 12345);
    let progress = Progress::default();
    host.upload(&upload("cube.gcode", data.clone(), false), &progress)
        .unwrap();
    assert_eq!(progress.sent.load(Ordering::Relaxed), data.len() as u64);
    let parts: Vec<Got> = printer
        .got()
        .into_iter()
        .filter(|got| got.url == "/uploadFile/upload")
        .collect();
    assert_eq!(parts.len(), 3);
    let md5 = format!("{:x}", <md5::Md5 as md5::Digest>::digest(&data[..]));
    let mut whole = Vec::new();
    for (n, part) in parts.iter().enumerate() {
        let form = part.form();
        let names: Vec<&str> = form.iter().map(|(name, _, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["Check", "S-File-MD5", "Offset", "Uuid", "TotalSize", "File"]
        );
        assert_eq!(part.field("Check").as_deref(), Some("1"));
        assert_eq!(part.field("S-File-MD5"), Some(md5.clone()));
        assert_eq!(part.field("Offset"), Some((n * PART).to_string()));
        assert_eq!(part.field("TotalSize"), Some(data.len().to_string()));
        assert_eq!(part.field("Uuid"), parts[0].field("Uuid"));
        assert_eq!(form[5].1.as_deref(), Some("cube.gcode"));
        whole.extend_from_slice(&form[5].2);
    }
    assert!(whole == *data, "the parts put together are the file");
    assert!(
        socket.got().is_empty(),
        "a plain upload says nothing to the websocket"
    );
}

#[test]
fn a_centauri_carbon_is_started_after_it_has_checked_the_file() {
    let printer = centauri();
    let checked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let asked = checked.clone();
    // Idle before the upload; then checking the file once; then idle.
    let socket = Socket::start(Box::new(move |message: &Value, _| {
        match message["Data"]["Cmd"].as_i64()? {
            0 => {
                let n = asked.fetch_add(1, Ordering::Relaxed);
                let state = if n == 1 { 8 } else { 0 };
                Some(json!({ "Status": { "CurrentStatus": [state] } }))
            }
            128 => Some(json!({ "Data": { "Cmd": 128, "Data": { "Ack": 0 } } })),
            _ => None,
        }
    }));
    let host = centauri_host(&printer, &socket);
    let mut sending = upload("cube.gcode", mock::gcode(5000), true);
    sending.options = Options {
        timelapse: true,
        leveling: false,
        smooth_plate: true,
        ..Options::default()
    };
    host.upload(&sending, &Progress::default()).unwrap();
    let said = socket.got();
    let cmds: Vec<i64> = said
        .iter()
        .map(|message| message["Data"]["Cmd"].as_i64().unwrap())
        .collect();
    assert_eq!(cmds, [0, 0, 0, 128]);
    let start = &said[3]["Data"]["Data"];
    assert_eq!(start["Filename"], "/local/cube.gcode");
    assert_eq!(start["StartLayer"], 0);
    assert_eq!(start["Tlp_Switch"], 1);
    assert_eq!(start["Calibration_switch"], 0);
    assert_eq!(start["PrintPlatformType"], 1);
    assert_eq!(said[3]["Data"]["From"], 1);
}

#[test]
fn a_busy_centauri_carbon_is_sent_nothing() {
    let printer = centauri();
    let socket = centauri_socket(vec![1], Some(0));
    let host = centauri_host(&printer, &socket);
    let result = host.upload(
        &upload("a.gcode", mock::gcode(1000), true),
        &Progress::default(),
    );
    assert_eq!(result, Err(Failure::Error(BUSY.into())));
    assert!(
        !printer
            .asked()
            .iter()
            .any(|asked| asked.contains("uploadFile"))
    );
}

#[test]
fn a_centauri_carbon_that_refuses_the_start_says_why() {
    let printer = centauri();
    let socket = centauri_socket(vec![0], Some(7));
    let host = centauri_host(&printer, &socket);
    let result = host.upload(
        &upload("a.gcode", mock::gcode(1000), true),
        &Progress::default(),
    );
    assert_eq!(
        result,
        Err(Failure::Error(
            "The file does not match the printer, please check and try again. Error code: 7".into()
        ))
    );
}

#[test]
fn a_centauri_carbon_that_goes_quiet_times_out() {
    let printer = centauri();
    let socket = centauri_socket(vec![0], None);
    let host = centauri_host(&printer, &socket);
    let started = std::time::Instant::now();
    let result = host.upload(
        &upload("a.gcode", mock::gcode(1000), true),
        &Progress::default(),
    );
    assert_eq!(result, Err(Failure::Error("Start print timeout".into())));
    assert!(started.elapsed() < Duration::from_secs(10));
}

// ---- the queue ----

#[test]
fn the_queue_sends_one_after_another_and_calls_off_one_waiting() {
    let printer = Printer::start(
        Box::new(|got: &Got, _: &[Got]| match got.url.as_str() {
            "/api/version" => (
                200,
                json!({ "api": "0.1", "text": "OctoPrint" }).to_string(),
            ),
            _ => (201, "{}".into()),
        }),
        Some(Duration::from_millis(200)),
    );
    let queue = Queue::default();
    let make = || {
        Box::new(OctoPrint::new(&connection("octoprint", &printer.address(), ""), quick()).unwrap())
    };
    let first = queue.add(
        make(),
        "http://127.0.0.1/",
        "P",
        upload("one.gcode", mock::gcode(1000), false),
    );
    let second = queue.add(
        make(),
        "http://127.0.0.1/",
        "P",
        upload("two.gcode", mock::gcode(1000), false),
    );
    queue.cancel(second).unwrap();
    let listing = loop {
        let listing = queue.listing();
        if listing["busy"] == false {
            break listing;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let jobs = listing["jobs"].as_array().unwrap();
    assert_eq!(jobs[0]["id"], first);
    assert_eq!(jobs[0]["said"], "Completed");
    assert_eq!(jobs[0]["progress"], 100);
    assert_eq!(jobs[1]["said"], "Canceled");
    let uploads = printer
        .asked()
        .iter()
        .filter(|asked| asked.contains("files/local"))
        .count();
    assert_eq!(uploads, 1, "the one called off was never sent");
    queue.clear_done();
    assert_eq!(queue.listing()["jobs"], json!([]));

    // An error is kept with the job, in the reference's words.
    let mut wrong = connection("octoprint", &printer.address(), "");
    wrong.host_type = "octoprint".into();
    let bad = Printer::start(
        Box::new(|_: &Got, _: &[Got]| (403, "Invalid API key".into())),
        None,
    );
    wrong.print_host = bad.address();
    queue.add(
        Box::new(OctoPrint::new(&wrong, quick()).unwrap()),
        "x",
        "P",
        upload("three.gcode", mock::gcode(10), false),
    );
    let listing = loop {
        let listing = queue.listing();
        if listing["busy"] == false {
            break listing;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(listing["jobs"][0]["said"], "Error");
    assert_eq!(
        listing["jobs"][0]["message"],
        "Error uploading to print host:\nHTTP 403: Invalid API key"
    );
}
