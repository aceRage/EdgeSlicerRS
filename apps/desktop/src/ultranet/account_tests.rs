//! The sign-in window's guard, its override, reading what the page hands
//! back, the User-Agent, and what is kept of the account. No window is
//! opened: these are the rules `main.rs` builds the window from.

use super::*;

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// What the window's script writes for a message.
fn handed_back(message: &Value) -> Url {
    let text = message.to_string();
    let encoded: String = url::form_urlencoded::byte_serialize(text.as_bytes()).collect();
    url(&format!("https://{RESULT_HOST}/#{encoded}"))
}

#[test]
fn the_page_is_bambu_s_bare_sign_in_unless_a_loopback_page_is_named() {
    assert_eq!(
        sign_in_url(None).unwrap().as_str(),
        "https://bambulab.com/sign-in"
    );
    assert_eq!(sign_in_url(Some("")).unwrap().as_str(), SIGN_IN_URL);
    assert_eq!(
        sign_in_url(Some("http://127.0.0.1:4711/sign-in"))
            .unwrap()
            .as_str(),
        "http://127.0.0.1:4711/sign-in"
    );
    assert!(sign_in_url(Some("http://localhost:9/x")).is_ok());
    // Anything else is refused, never the real page in its place.
    for bad in [
        "https://example.com/sign-in",
        "http://192.168.1.5:80/",
        "http://127.0.0.1.evil.com/",
        "http://user:pw@127.0.0.1:80/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "not a url",
    ] {
        let refused = sign_in_url(Some(bad)).unwrap_err();
        assert!(refused.contains(OVERRIDE), "{bad}: {refused}");
    }
}

#[test]
fn the_window_goes_only_to_bambu_s_https_pages() {
    let start = sign_in_url(None).unwrap();
    for ok in [
        "https://bambulab.com/sign-in",
        "https://bambulab.com/en/sign-in?x=1",
        "https://www.bambulab.com/",
        "https://api.bambulab.com/v1/x",
    ] {
        assert_eq!(navigation(&url(ok), &start), Navigation::Allow, "{ok}");
    }
    for bad in [
        "http://bambulab.com/sign-in",
        "https://bambulab.com.evil.com/",
        "https://evilbambulab.com/",
        "https://bambulab.com:8443/",
        "https://user@bambulab.com/",
        "https://accounts.google.com/",
        "https://127.0.0.1/",
        "tauri://localhost/index.html",
        "http://tauri.localhost/",
        "file:///Users/me/",
        "data:text/html,hi",
        "about:blank",
        "edgeslicer://x",
    ] {
        assert_eq!(navigation(&url(bad), &start), Navigation::Refuse, "{bad}");
    }
    // The result host is never a page: it is read, and over http it is refused.
    assert!(matches!(
        navigation(&url(&format!("https://{RESULT_HOST}/#x")), &start),
        Navigation::Result(Message::Ignored)
    ));
    assert_eq!(
        navigation(&url(&format!("http://{RESULT_HOST}/#x")), &start),
        Navigation::Refuse
    );
}

#[test]
fn with_the_loopback_page_the_window_stays_on_its_origin() {
    let start = sign_in_url(Some("http://127.0.0.1:4711/sign-in")).unwrap();
    assert_eq!(
        navigation(&url("http://127.0.0.1:4711/other"), &start),
        Navigation::Allow
    );
    assert_eq!(
        navigation(&url("http://127.0.0.1:4712/"), &start),
        Navigation::Refuse
    );
    assert_eq!(
        navigation(&url("https://bambulab.com/sign-in"), &start),
        Navigation::Refuse
    );
}

#[test]
fn a_ticket_login_is_read_as_the_reference_reads_it() {
    let start = sign_in_url(None).unwrap();
    let message = json!({ "command": "user_ticket_login", "data": { "ticket": "TK-123.abc_~" } });
    assert_eq!(
        navigation(&handed_back(&message), &start),
        Navigation::Result(Message::Ticket("TK-123.abc_~".into()))
    );
    // A ticket the bridge would refuse is not handed on.
    for ticket in [
        json!(""),
        json!("a b"),
        json!("a\"b"),
        json!(7),
        Value::Null,
    ] {
        let message = json!({ "command": "user_ticket_login", "data": { "ticket": ticket } });
        assert_eq!(
            navigation(&handed_back(&message), &start),
            Navigation::Result(Message::Ignored)
        );
    }
}

#[test]
fn a_token_login_carries_its_tokens_and_user() {
    let message = json!({ "command": "user_login", "data": {
        "token": "AT1", "refresh_token": "RT1", "expires_in": "7776000", "refresh_expires_in": 100,
        "user": { "uid": 1234567, "name": "Fake User", "account": "fake@example.com", "avatar": "https://x/a.png" } } });
    let Navigation::Result(Message::Tokens(session, user)) =
        navigation(&handed_back(&message), &sign_in_url(None).unwrap())
    else {
        panic!("not read")
    };
    assert_eq!(
        (
            session.access_token.as_str(),
            session.refresh_token.as_str(),
            session.expires_in,
            session.refresh_expires_in
        ),
        ("AT1", "RT1", 7776000, 100)
    );
    assert!((session.acquired_at - now()).abs() < 60);
    assert_eq!(
        user,
        User {
            id: "1234567".into(),
            name: "Fake User".into()
        }
    );
    // No e-mail, no avatar kept; and the debug text holds no token.
    let shown = format!("{:?}", Message::Tokens(session.clone(), user));
    assert!(!shown.contains("AT1") && !shown.contains("RT1"), "{shown}");
    assert!(!format!("{session:?}").contains("AT1"));
}

#[test]
fn other_messages_are_not_a_sign_in() {
    for message in [
        json!({ "command": "get_localhost_url", "sequence_id": "1" }),
        json!({ "command": "thirdparty_login", "data": { "url": "https://accounts.google.com/" } }),
    ] {
        assert_eq!(read_message_of(&message), Message::ThirdParty);
    }
    for message in [
        json!({ "command": "autotest_token", "data": { "token": "x" } }),
        json!({ "command": "new_webpage", "data": { "url": "https://x" } }),
        json!({ "command": "user_login", "data": {} }),
        json!([1, 2]),
        json!("text"),
    ] {
        assert_eq!(read_message_of(&message), Message::Ignored);
    }
    assert_eq!(read_message("%E0%A4%A"), Message::Ignored);
    assert_eq!(read_message("{"), Message::Ignored);
    assert_eq!(format!("{:?}", Message::Ticket("TK".into())), "Ticket(..)");
}

fn read_message_of(message: &Value) -> Message {
    let encoded: String =
        url::form_urlencoded::byte_serialize(message.to_string().as_bytes()).collect();
    read_message(&encoded)
}

#[test]
fn the_script_defines_wx_and_names_only_the_result_host() {
    let script = init_script();
    assert!(script.contains("postMessage"));
    assert!(script.contains("'wx'"));
    assert!(script.contains(&format!("https://{RESULT_HOST}/#")));
    assert!(script.contains("encodeURIComponent"));
    // No call into the app.
    assert!(!script.contains("__TAURI") && !script.contains("invoke") && !script.contains("ipc"));
}

#[test]
fn the_user_agent_is_bbl_login_user_agent() {
    assert_eq!(
        user_agent(false, true, "de_DE"),
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) BBL-Slicer/v02.08.02.61 (dark) BBL-Language/de"
    );
    assert_eq!(
        user_agent(true, false, ""),
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36 Edg/107.0.1418.52 BBL-Slicer/v02.08.02.61 (light) BBL-Language/en"
    );
    assert!(user_agent(false, false, "zh-CN").ends_with("BBL-Language/zh"));
}

#[test]
fn the_account_printers_and_their_codes_are_read_apart() {
    let (printers, codes) = read_printers(&json!([
        { "dev_id": "01p09c470100142", "dev_name": "Workshop", "dev_online": true, "dev_model_name": "C12",
          "task_status": "IDLE", "dev_access_code": "3579\n2468" },
        { "dev_id": "00M09A350100777", "dev_name": "Office", "dev_online": false, "dev_model_name": "BL-P001",
          "dev_access_code": "" },
        { "dev_name": "no serial" },
    ]));
    assert_eq!(printers.len(), 2);
    assert_eq!(printers[0].dev_id, "01P09C470100142");
    assert!(printers[0].online && printers[0].has_code && !printers[1].has_code);
    assert_eq!(
        codes,
        vec![("01P09C470100142".to_owned(), "35792468".to_owned())]
    );
    assert!(!format!("{printers:?}").contains("35792468"));
}

#[test]
fn the_store_s_account_field_is_versioned() {
    let mut secret = AccountSecret {
        user: User {
            id: "1".into(),
            name: "N".into(),
        },
        session: AccountSession {
            access_token: "AT".into(),
            refresh_token: "RT".into(),
            expires_in: 5,
            refresh_expires_in: 6,
            acquired_at: 7,
        },
        ..AccountSecret::default()
    };
    secret.codes.insert("S1".into(), "C1".into());
    let text = secret.text();
    let root: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(root["version"], 1);
    assert_eq!(AccountSecret::parse(&text), Some(secret.clone()));
    assert_eq!(
        AccountSecret::parse(&text.replace("\"version\":1", "\"version\":2")),
        None
    );
    assert_eq!(AccountSecret::parse("nonsense"), None);
    let mut secrets = secret.secrets();
    secrets.sort();
    assert_eq!(secrets, ["AT", "C1", "RT"]);
    let shown = format!("{secret:?}");
    assert!(!shown.contains("AT") && !shown.contains("C1") && shown.contains("S1"));
}
