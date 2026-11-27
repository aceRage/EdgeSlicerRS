//! The Bambu Lab account through `Session::call` and the sign-in window's
//! hand-back (`Session::bambu_sign_in_message`), against the mock bridge
//! and `keyring-core`'s mock store: sign-in, who is signed in, the
//! account's printers beside those added by hand, "Refresh" in a later
//! run, an expired session, sign-out; and that no answer to the page ever
//! holds a ticket, a token or an access code. No window is opened: the
//! window's opener is a test's, which notes the page it was asked for.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::Session;
use crate::secrets::{
    Item, Secrets, System,
    tests::{Asked, Counted},
};
use crate::ultranet::{
    account::{self, AccountSession, Message, User},
    client::Timeouts,
    mock::{Mock, Setup, TICKET},
};

const SERIAL: &str = "01P09C470100142";
const OFFICE: &str = "00M09A350100777";
const CODE: &str = "12345678";
const OFFICE_CODE: &str = "B4K9Q2ZP";

fn quick() -> Timeouts {
    let d = std::time::Duration::from_secs(5);
    Timeouts {
        hello: d,
        ask: d,
        connect: d,
        started: d,
        restart: std::time::Duration::from_millis(0),
        account: d,
    }
}

/// Every secret of these tests: none may be in an answer.
const SECRETS: [&str; 6] = [
    "AT-mock-",
    "RT-mock-",
    TICKET,
    CODE,
    OFFICE_CODE,
    "AT-page-1",
];

struct Probe {
    session: Arc<Session>,
    answers: Mutex<Vec<String>>,
    opened: Arc<Mutex<Vec<String>>>,
    asked: Arc<Mutex<Asked>>,
    store: Arc<keyring_core::mock::Store>,
}

impl Probe {
    /// A session over `dir`, its secrets in an item on `store`, with a
    /// window opener of the test's.
    fn new(mock: &Mock, dir: &tempfile::TempDir, store: Arc<keyring_core::mock::Store>) -> Self {
        let (item, asked) = Counted::on(store.clone());
        let secrets: Box<dyn Secrets> = Box::new(System::new(Box::new(item)));
        let session = Arc::new(Session::with_bridge_and_secrets(
            Some(dir.path().to_owned()),
            mock.launcher(),
            quick(),
            secrets,
        ));
        let opened: Arc<Mutex<Vec<String>>> = Arc::default();
        let noted = opened.clone();
        session.set_sign_in_window(Arc::new(move |url: &url::Url| {
            noted.lock().unwrap().push(url.to_string());
            Ok(())
        }));
        Probe {
            session,
            answers: Mutex::default(),
            opened,
            asked,
            store,
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

    fn account(&self) -> Value {
        self.ask("bambu_account", json!({}))
    }

    fn printers(&self) -> Value {
        self.ask("bambu_printers", json!({}))
    }

    /// What the store's item holds now, read past the session.
    fn item(&self) -> Value {
        let (mut raw, _) = Counted::on(self.store.clone());
        raw.read()
            .unwrap()
            .map(|text| serde_json::from_str(&text).unwrap())
            .unwrap_or(Value::Null)
    }

    fn never_said_a_secret(&self) {
        let answers = self.answers.lock().unwrap();
        assert!(!answers.is_empty());
        for answer in answers.iter() {
            for secret in SECRETS {
                assert!(!answer.contains(secret), "{secret} in {answer}");
            }
        }
    }

    fn sign_in(&self) {
        let opened = self.ask("bambu_account_sign_in", json!({}));
        assert_eq!(opened["phase"], "window");
        assert!(
            self.session
                .bambu_sign_in_message(Message::Ticket(TICKET.into()))
        );
    }
}

fn mock() -> Mock {
    Mock::new(Setup::default())
}

#[test]
fn signing_in_opens_bambu_s_page_and_keeps_the_session_in_the_store_only() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    let before = probe.account();
    assert_eq!(before["signed_in"], false);
    assert_eq!(before["offered"], true);
    assert_eq!(before["can_sign_in"], true);
    probe.sign_in();
    assert_eq!(
        *probe.opened.lock().unwrap(),
        vec![account::SIGN_IN_URL.to_owned()]
    );
    // The bridge was asked with the ticket, then for the printers.
    assert_eq!(mock.requests("account_sign_in")[0]["ticket"], TICKET);
    assert_eq!(mock.requests("account_printers").len(), 1);
    let status = probe.account();
    assert_eq!(status["signed_in"], true);
    assert_eq!(status["name"], "Fake User");
    assert_eq!(status["phase"], "idle");
    assert!(status["said"].is_null());
    // The store: the account's own field, the tokens and the codes there.
    let item = probe.item();
    assert_eq!(item["version"], 1);
    assert_eq!(item["account"]["version"], 1);
    assert_eq!(item["account"]["user"]["name"], "Fake User");
    assert_eq!(item["account"]["session"]["access_token"], "AT-mock-0001");
    assert_eq!(item["account"]["codes"][OFFICE], OFFICE_CODE);
    assert_eq!(item["codes"], json!({}));
    // The plain list: who and the printers, no secret.
    let list = std::fs::read_to_string(dir.path().join("bambu_printers.json")).unwrap();
    assert!(list.contains("Fake User") && list.contains(OFFICE));
    for secret in SECRETS {
        assert!(!list.contains(secret), "{secret} in bambu_printers.json");
    }
    // Device: the account's printers, without their codes, marked.
    let printers = probe.printers();
    let account = printers["account_printers"].as_array().unwrap();
    assert_eq!(account.len(), 2);
    let office = account.iter().find(|p| p["dev_id"] == OFFICE).unwrap();
    assert_eq!(office["online"], false);
    assert_eq!(office["has_access_code"], true);
    assert_eq!(office["added"], false);
    assert_eq!(printers["account"]["name"], "Fake User");
    // Whatever the bridge says is cleared of the tokens and the codes.
    let said = probe
        .session
        .bambu
        .client
        .scrub("AT-mock-0001 RT-mock-0001 B4K9Q2ZP");
    assert!(
        !said.contains("mock") && !said.contains(OFFICE_CODE),
        "{said}"
    );
    assert!(!format!("{:?}", probe.session.bambu.kept.lock().unwrap()).contains("AT-mock"));
    probe.never_said_a_secret();
}

#[test]
fn a_second_hand_back_or_one_with_no_window_open_is_not_taken() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    // No window open: nothing asked.
    assert!(
        !probe
            .session
            .bambu_sign_in_message(Message::Ticket(TICKET.into()))
    );
    assert!(mock.requests("account_sign_in").is_empty());
    probe.sign_in();
    // The page posts again: the sign-in it made stands.
    assert!(
        !probe
            .session
            .bambu_sign_in_message(Message::Ticket(TICKET.into()))
    );
    assert_eq!(mock.requests("account_sign_in").len(), 1);
    let status = probe.account();
    assert_eq!(status["signed_in"], true);
    assert!(status["said"].is_null());
}

#[test]
fn a_refused_ticket_is_login_failed_and_nothing_is_kept() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    probe.ask("bambu_account_sign_in", json!({}));
    assert!(
        probe
            .session
            .bambu_sign_in_message(Message::Ticket("TICKET-bad".into()))
    );
    let status = probe.account();
    assert_eq!(status["signed_in"], false);
    assert_eq!(status["said"], "Login failed. Please try again.");
    assert_eq!(probe.item(), Value::Null);
    probe.never_said_a_secret();
}

#[test]
fn messages_that_are_no_sign_in_leave_the_window_open() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    probe.ask("bambu_account_sign_in", json!({}));
    assert!(!probe.session.bambu_sign_in_message(Message::Ignored));
    assert!(!probe.session.bambu_sign_in_message(Message::ThirdParty));
    let status = probe.account();
    assert_eq!(status["phase"], "window");
    assert!(status["said"].as_str().unwrap().contains("Google or Apple"));
    // Closed without a sign-in.
    probe.session.bambu_sign_in_closed();
    assert_eq!(probe.account()["phase"], "idle");
    assert!(mock.requests("account_sign_in").is_empty());
}

#[test]
fn a_token_login_is_restored_through_the_bridge() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    probe.ask("bambu_account_sign_in", json!({}));
    let session = AccountSession {
        access_token: "AT-page-1".into(),
        refresh_token: "RT-page-1".into(),
        expires_in: 100,
        refresh_expires_in: 1000,
        acquired_at: account::now(),
    };
    let user = User {
        id: "42".into(),
        name: "Page User".into(),
    };
    assert!(
        probe
            .session
            .bambu_sign_in_message(Message::Tokens(session, user))
    );
    let restore = &mock.requests("account_restore")[0];
    assert_eq!(restore["session"]["access_token"], "AT-page-1");
    assert_eq!(restore["user"]["name"], "Page User");
    assert_eq!(probe.account()["name"], "Page User");
    assert_eq!(
        probe.item()["account"]["session"]["access_token"],
        "AT-page-1"
    );
    probe.never_said_a_secret();
}

#[test]
fn a_later_run_shows_the_account_without_the_store_and_refresh_restores_it() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let store = keyring_core::mock::Store::new().unwrap();
    {
        let probe = Probe::new(&mock, &dir, store.clone());
        probe.sign_in();
    }
    let probe = Probe::new(&mock, &dir, store);
    // Shown from the plain list: the store not read, the bridge not started.
    let status = probe.account();
    assert_eq!(status["signed_in"], true);
    assert_eq!(status["name"], "Fake User");
    assert_eq!(
        probe.printers()["account_printers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(probe.asked.lock().unwrap().reads, 0);
    let starts = mock.seen.lock().unwrap().starts;
    // "Refresh": read once, the session handed to the bridge, the list asked.
    probe.ask("bambu_account_refresh", json!({}));
    probe.ask("bambu_account_refresh", json!({}));
    assert_eq!(probe.asked.lock().unwrap().reads, 1);
    assert!(mock.seen.lock().unwrap().starts > starts);
    let restore = mock.requests("account_restore");
    assert_eq!(restore.len(), 2);
    assert_eq!(restore[0]["session"]["access_token"], "AT-mock-0001");
    assert_eq!(restore[0]["user"]["id"], "1234567");
    assert_eq!(mock.requests("account_printers").len(), 3);
    probe.never_said_a_secret();
}

#[test]
fn an_expired_session_signs_out_and_says_so() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    probe.sign_in();
    mock.setup.lock().unwrap().account_expires = true;
    let status = probe.ask("bambu_account_refresh", json!({}));
    assert_eq!(status["signed_in"], false);
    assert_eq!(
        status["said"],
        "Login information expired. Please login again."
    );
    assert!(probe.item()["account"].is_null());
    assert!(
        probe.printers()["account_printers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    probe.never_said_a_secret();
}

#[test]
fn a_cloud_failure_keeps_the_printers_it_had() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    probe.sign_in();
    mock.setup.lock().unwrap().cloud_fails = Some(503);
    let status = probe.ask("bambu_account_refresh", json!({}));
    assert_eq!(status["signed_in"], true);
    assert_eq!(
        status["said"],
        "The Bambu Lab cloud refused the request (HTTP 503)."
    );
    assert_eq!(
        probe.printers()["account_printers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(probe.item()["account"]["codes"][OFFICE], OFFICE_CODE);
}

#[test]
fn a_printer_in_both_lists_keeps_its_own_code_and_sign_out_leaves_it() {
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let probe = Probe::new(&mock, &dir, keyring_core::mock::Store::new().unwrap());
    // Added by hand with a code of its own, then the account signed in.
    probe.ask(
        "bambu_printer_add",
        json!({ "ip": "127.0.0.1", "access_code": "11112222", "name": "Mine", "serial": SERIAL, "model": "" }),
    );
    probe.sign_in();
    let printers = probe.printers();
    let mine = printers["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["dev_id"] == SERIAL)
        .unwrap();
    assert_eq!(mine["in_account"], true);
    let listed = printers["account_printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["dev_id"] == SERIAL)
        .unwrap();
    assert_eq!(listed["added"], true);
    // The typed code wins (`get_access_code`).
    assert_eq!(
        probe.session.bambu_code(SERIAL).unwrap().as_deref(),
        Some("11112222")
    );
    // The account's printer added with its address only: the account's code.
    probe.ask(
        "bambu_printer_add",
        json!({ "ip": "127.0.0.2", "access_code": "", "name": "Office", "serial": OFFICE, "model": "" }),
    );
    assert_eq!(
        probe.session.bambu_code(OFFICE).unwrap().as_deref(),
        Some(OFFICE_CODE)
    );
    // An empty code for a printer the account has none for is refused.
    assert!(
        probe
            .call(
                "bambu_printer_add",
                json!({ "ip": "127.0.0.3", "access_code": "", "name": "X", "serial": "01S00A000000001", "model": "" }),
            )
            .is_err()
    );
    // Signed out: the account's printers and codes gone, the bridge told;
    // the printers added by hand stay, the typed code with them.
    let status = probe.ask("bambu_account_sign_out", json!({}));
    assert_eq!(status["signed_in"], false);
    assert_eq!(mock.requests("account_sign_out").len(), 1);
    let item = probe.item();
    assert!(item["account"].is_null());
    assert_eq!(item["codes"][SERIAL], "11112222");
    let printers = probe.printers();
    assert!(printers["account_printers"].as_array().unwrap().is_empty());
    let left: Vec<&str> = printers["printers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["dev_id"].as_str().unwrap())
        .collect();
    assert_eq!(left, [SERIAL, OFFICE]);
    // The account's printer added by its address has no code any more.
    assert_eq!(probe.session.bambu_code(OFFICE).unwrap(), None);
    let office = printers["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["dev_id"] == OFFICE)
        .unwrap();
    assert_eq!(office["has_access_code"], false);
    probe.never_said_a_secret();
}

#[test]
fn without_a_window_or_with_an_old_bridge_there_is_no_sign_in() {
    // `--serve`: no window to open.
    let mock = mock();
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::with_bridge(
        Some(dir.path().to_owned()),
        mock.launcher(),
        quick(),
    ));
    assert_eq!(
        session.call("bambu_account", &json!({})).unwrap()["can_sign_in"],
        false
    );
    let refused = session
        .call("bambu_account_sign_in", &json!({}))
        .unwrap_err();
    assert!(refused.contains("window"), "{refused}");
    // A bridge of protocol 1.0: too old, nothing opened.
    let mut setup = Setup {
        minor: 0,
        ..Setup::default()
    };
    setup.methods.retain(|m| !m.starts_with("account_"));
    let old = Mock::new(setup);
    let probe = Probe::new(&old, &dir, keyring_core::mock::Store::new().unwrap());
    let refused = probe.call("bambu_account_sign_in", json!({})).unwrap_err();
    assert!(
        refused.contains("too old") && refused.contains("account_sign_in"),
        "{refused}"
    );
    assert!(probe.opened.lock().unwrap().is_empty());
    assert_eq!(probe.account()["offered"], false);
    // No bridge at all: "Network plug-in missing".
    let none = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
    none.set_sign_in_window(Arc::new(|_: &url::Url| panic!("opened")));
    let refused = none.call("bambu_account_sign_in", &json!({})).unwrap_err();
    assert!(refused.contains("Network plug-in missing"), "{refused}");
}
