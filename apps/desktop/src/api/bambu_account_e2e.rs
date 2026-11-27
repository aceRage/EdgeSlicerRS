//! The real bridge, driven from this app's own code: sign-in from a
//! ticket, the account's printers, a LAN dial with the code the account
//! gave, sign-out. Skipped unless `EDGE_ULTRANET_BRIDGE` names a bridge and
//! `ULTRANET_TEST_CLOUD_ENDPOINT` points it at a loopback stand-in of the
//! cloud (UltraNet's `tools/fakecloud.py`, with `SSL_CERT_FILE` its
//! certificate); `EDGE_E2E_PRINTER` set means `tools/fakeprinter.py` serves
//! the account's first printer on 127.0.0.1. Nothing here can reach the
//! network: without the loopback endpoint no bridge is started. Secrets
//! are kept in memory; the data folder is the test's own.

use std::sync::Arc;

use serde_json::json;

use super::Session;
use crate::secrets::Memory;
use crate::ultranet::{
    account::Message,
    client::{Launcher, Timeouts},
};

const SERIAL: &str = "01P09C470100142";
/// What fakecloud.py hands out: none may reach the page, or any file the
/// bridge or UltraNet writes.
const SECRETS: [&str; 6] = [
    "GoodT1", "AT-fake-", "RT-fake-", "35792468", "B4K9Q2ZP", "Fail01",
];

fn loopback_cloud() -> bool {
    std::env::var("ULTRANET_TEST_CLOUD_ENDPOINT").is_ok_and(|endpoint| {
        endpoint
            .rsplit_once(':')
            .is_some_and(|(host, _)| host == "localhost" || host == "127.0.0.1")
    })
}

#[test]
fn the_real_bridge_signs_in_lists_the_account_and_dials_with_its_code() {
    let Some(bridge) = std::env::var_os("EDGE_ULTRANET_BRIDGE") else {
        eprintln!("skipped: EDGE_ULTRANET_BRIDGE is not set");
        return;
    };
    if !loopback_cloud() {
        eprintln!("skipped: ULTRANET_TEST_CLOUD_ENDPOINT does not name a loopback fake");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::with_bridge_and_secrets(
        Some(dir.path().to_owned()),
        Launcher::Program(bridge.into()),
        Timeouts::default(),
        Box::new(Memory::default()),
    ));
    session.set_sign_in_window(Arc::new(|_: &url::Url| Ok(())));
    let mut answers = Vec::new();
    let mut ask = |name: &str, args: serde_json::Value| {
        let answer = session.call(name, &args).unwrap();
        answers.push(format!("{name}: {answer}"));
        answer
    };

    // A refused ticket, then the good one.
    ask("bambu_account_sign_in", json!({}));
    assert!(session.bambu_sign_in_message(Message::Ticket("Fail01".into())));
    let status = ask("bambu_account", json!({}));
    assert_eq!(status["signed_in"], false);
    assert_eq!(status["said"], "Login failed. Please try again.");
    ask("bambu_account_sign_in", json!({}));
    assert!(session.bambu_sign_in_message(Message::Ticket("GoodT1".into())));
    let status = ask("bambu_account", json!({}));
    assert_eq!(status["signed_in"], true, "{status}");
    assert_eq!(status["name"], "Fake User");
    let printers = ask("bambu_printers", json!({}));
    let account = printers["account_printers"].as_array().unwrap();
    assert_eq!(account.len(), 2, "{printers}");
    assert!(account.iter().all(|p| p["has_access_code"] == true));

    // "Refresh": the session handed back to the bridge, the list again.
    let status = ask("bambu_account_refresh", json!({}));
    assert_eq!(status["signed_in"], true);
    assert!(status["said"].is_null(), "{status}");

    // The account's printer added by its address alone, dialled with the
    // code the account gave (the fake printer's).
    if std::env::var_os("EDGE_E2E_PRINTER").is_some() {
        ask(
            "bambu_printer_add",
            json!({ "ip": "127.0.0.1", "access_code": "", "name": "Workshop", "serial": SERIAL, "model": "" }),
        );
        let state = ask("bambu_connect", json!({ "dev_id": SERIAL }));
        assert_eq!(state["session"], "connected", "{state}");
    }

    let status = ask("bambu_account_sign_out", json!({}));
    assert_eq!(status["signed_in"], false);
    assert!(
        ask("bambu_printers", json!({}))["account_printers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for answer in &answers {
        for secret in SECRETS {
            assert!(!answer.contains(secret), "{secret} in {answer}");
        }
    }
    session.bambu.client.shutdown();
    // Nothing the app, the bridge or UltraNet wrote holds a secret.
    let mut stack = vec![dir.path().to_owned()];
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
    eprintln!(
        "e2e: {} answers and {files} files searched, none holds a secret",
        answers.len()
    );
}
