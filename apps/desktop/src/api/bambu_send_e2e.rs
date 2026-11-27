//! The send to an account's printer through the real bridge, driven from
//! this app's own code (tools/bambu_send_e2e.sh runs it): signed in to
//! UltraNet's `tools/fakecloud.py`, the account's printer offered in "Send
//! print job to", its address entered as 127.0.0.1, connected and sent to
//! with the code the account gave, against UltraNet's
//! `tools/fakeprinter.py`. `EDGE_E2E_SEND` names the case:
//!
//! - `print`: the plate prints; the file the fake printer got is the one
//!   UltraNet named in `project_file` (the same MD5), kept on its disk;
//! - `wrong_code`: the printer's FTP refuses the code (the fake's
//!   `--ftp-access-code`) while its MQTT session is up;
//! - `unreachable`: nothing listens where FTPS is looked for (the bridge's
//!   `ULTRANET_TEST_FTPS_PORT` names a closed loopback port);
//! - `upload`: the printer refuses the file after a good login (the fake's
//!   `--reject-stor-named verify_job`, in the root and in `sdcard/`).
//!
//! Each failure is said by its own true message. Skipped unless
//! `EDGE_ULTRANET_BRIDGE` names a bridge, `ULTRANET_TEST_CLOUD_ENDPOINT`
//! points it at a loopback fake cloud and `EDGE_E2E_SEND` is set: nothing
//! here can reach the network. The codes are kept in memory; the data
//! folder is the test's own, searched for every secret at the end.

use std::path::PathBuf;

use serde_json::{Value, json};

use super::{Probe, SERIAL};
use crate::ultranet::{
    account::Message,
    client::{Launcher, Timeouts},
};

/// What fakecloud.py hands out, and the code the wrong-code case's printer
/// wants: none may reach the page, or any file the app, the bridge or
/// UltraNet writes.
const SECRETS: [&str; 7] = [
    "GoodT1", "AT-fake-", "RT-fake-", "35792468", "B4K9Q2ZP", "Fail01", "11223344",
];

fn loopback_cloud() -> bool {
    std::env::var("ULTRANET_TEST_CLOUD_ENDPOINT").is_ok_and(|endpoint| {
        endpoint
            .rsplit_once(':')
            .is_some_and(|(host, _)| host == "localhost" || host == "127.0.0.1")
    })
}

#[test]
fn an_account_printer_gets_the_plate_over_the_lan_through_the_real_bridge() {
    let Some(bridge) = std::env::var_os("EDGE_ULTRANET_BRIDGE") else {
        eprintln!("skipped: EDGE_ULTRANET_BRIDGE is not set");
        return;
    };
    let Ok(case) = std::env::var("EDGE_E2E_SEND") else {
        eprintln!("skipped: EDGE_E2E_SEND is not set");
        return;
    };
    assert!(
        loopback_cloud(),
        "ULTRANET_TEST_CLOUD_ENDPOINT does not name a loopback fake: nothing is started"
    );
    let probe = Probe::with_launcher(Launcher::Program(bridge.into()), Timeouts::default());
    probe
        .session
        .set_sign_in_window(std::sync::Arc::new(|_: &url::Url| Ok(())));

    // Signed in; the account's printer offered in the send dialog.
    probe.ask("bambu_account_sign_in", json!({}));
    assert!(
        probe
            .session
            .bambu_sign_in_message(Message::Ticket("GoodT1".into()))
    );
    let status = probe.ask("bambu_account", json!({}));
    assert_eq!(status["signed_in"], true, "{status}");
    let project = probe.sliced_project();
    let dialog = probe.ask("bambu_send_prepare", json!({ "project": project }));
    let offered = dialog["account_printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["dev_id"] == SERIAL)
        .cloned()
        .unwrap_or_else(|| panic!("{dialog}"));
    assert_eq!(offered["has_access_code"], true);

    // Its address entered: 127.0.0.1, the fake printer. The code is the
    // account's.
    probe.ask(
        "bambu_printer_add",
        json!({ "ip": "127.0.0.1", "access_code": "", "name": offered["name"], "serial": SERIAL, "model": offered["model"] }),
    );
    let state = probe.ask("bambu_connect", json!({ "dev_id": SERIAL }));
    assert_eq!(state["session"], "connected", "{state}");
    let mut last = Value::Null;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let dialog = loop {
        let dialog = probe.ask(
            "bambu_send_prepare",
            json!({ "project": project, "dev_id": SERIAL, "use_ams": false }),
        );
        if dialog["status"]["can_send"] == true {
            break dialog;
        }
        last = dialog["status"].clone();
        assert!(
            std::time::Instant::now() < deadline,
            "the dialog never let it send: {last}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let _ = last;
    assert_eq!(dialog["ip"], "127.0.0.1");

    let sent = probe.ask(
        "bambu_send",
        json!({ "project": project, "dev_id": SERIAL, "options": { "use_ams": false, "timelapse": false } }),
    );
    let task = sent["task"].as_u64().unwrap_or_else(|| panic!("{sent}"));
    let end = probe.until_long("the send to end", || {
        let progress = probe.ask("bambu_task", json!({ "task": task }));
        (progress["done"] == true).then_some(progress)
    });
    eprintln!("e2e {case}: {end}");
    let (error, detail) = (
        end["error"].as_str().unwrap_or_default(),
        end["detail"].as_str().unwrap_or_default(),
    );
    match case.as_str() {
        "print" => {
            assert_eq!(end["ok"], true, "{end}");
            let kept = landed_whole();
            // The same plate written again, as "Export plate sliced file"
            // writes it (the send's own file is removed once sent): the
            // printer has these bytes, every one.
            let again = probe._dir.path().join("again.gcode.3mf");
            probe.ask(
                "export_sliced",
                json!({ "project": project, "path": again.to_string_lossy() }),
            );
            let written = std::fs::read(&again).unwrap();
            assert!(
                written == kept,
                "the printer's copy is the plate, byte for byte"
            );
            eprintln!(
                "e2e print: the printer's copy equals the plate written again ({} bytes)",
                written.len()
            );
        }
        "wrong_code" => {
            assert_eq!(
                error,
                "LAN send failed: the printer rejected the access code (error -5010)."
            );
            assert_eq!(end["refused"], true);
        }
        "unreachable" => {
            assert_eq!(
                error,
                "LAN send failed: could not connect to the printer (error -5010)."
            );
            assert_eq!(end["refused"], false);
            assert!(detail.contains("FTPS 990"), "{detail}");
        }
        "upload" => {
            assert_eq!(
                error,
                "LAN send failed: uploading the file to the printer failed (error -5010)."
            );
            assert_eq!(end["refused"], false);
            assert!(
                detail.starts_with("The printer accepted the access code"),
                "{detail}"
            );
        }
        other => panic!("no case {other}"),
    }
    if case != "print" {
        assert_eq!(end["ok"], false, "{end}");
        assert_eq!(end["ask_code"], false, "{end}");
    }

    probe.ask("bambu_account_sign_out", json!({}));
    {
        let answers = probe.answers.lock().unwrap();
        for answer in answers.iter() {
            for secret in SECRETS {
                assert!(!answer.contains(secret), "{secret} in {answer}");
            }
        }
    }
    probe.session.bambu.client.shutdown();
    // Nothing the app, the bridge or UltraNet wrote holds a secret.
    let mut stack = vec![probe._dir.path().to_owned()];
    let mut files = 0;
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files += 1;
            let bytes = std::fs::read(&path).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for secret in SECRETS {
                assert!(!text.contains(secret), "{secret} in {}", path.display());
            }
        }
    }
    eprintln!("e2e {case}: {files} files searched, none holds a secret");
}

/// The fake printer's record (`--out`) and disk (`--store-dir`): the plate
/// it got is the file UltraNet hashed before sending (`project_file`'s
/// `md5`), and is kept whole.
fn landed_whole() -> Vec<u8> {
    let record: PathBuf = std::env::var_os("EDGE_E2E_RECORD")
        .expect("EDGE_E2E_RECORD names the fake printer's record")
        .into();
    let store: PathBuf = std::env::var_os("EDGE_E2E_STORE")
        .expect("EDGE_E2E_STORE names the fake printer's disk")
        .into();
    // The fake writes its record as it goes; give it a moment.
    let mut read = Value::Null;
    for _ in 0..100 {
        read = std::fs::read_to_string(&record)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        if read["publishes"].as_array().is_some_and(|p| {
            p.iter().any(|p| {
                p["payload"]
                    .as_str()
                    .is_some_and(|t| t.contains("project_file"))
            })
        }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let start: Value = read["publishes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["payload"].as_str())
        .filter(|t| t.contains("project_file"))
        .filter_map(|t| serde_json::from_str::<Value>(t).ok())
        .next_back()
        .unwrap_or_else(|| panic!("no project_file in {read}"));
    let url = start["print"]["url"].as_str().unwrap();
    let name = url.rsplit('/').next().unwrap();
    let md5 = start["print"]["md5"].as_str().unwrap().to_lowercase();
    let upload = read["uploads"]
        .as_array()
        .into_iter()
        .flatten()
        .rfind(|u| u["name"].as_str().is_some_and(|n| n.ends_with(name)))
        .unwrap_or_else(|| panic!("no upload of {name} in {read}"));
    assert_eq!(
        upload["md5"].as_str().unwrap().to_lowercase(),
        md5,
        "the bytes the printer got are the bytes UltraNet sent"
    );
    let kept = std::fs::read(store.join(name)).unwrap();
    assert_eq!(kept.len() as u64, upload["size"].as_u64().unwrap());
    assert!(kept.starts_with(b"PK"), "a .gcode.3mf (a zip)");
    eprintln!(
        "e2e print: {name}, {} bytes, md5 {md5} on both sides",
        kept.len()
    );
    kept
}
