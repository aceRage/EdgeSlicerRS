//! The Bambu Lab account (ULTRANET_BRIDGE.md section 16): the sign-in
//! page and the one place its address and the hosts it may go to are
//! named, the User-Agent it wants, the script that hands its result back,
//! reading that result, and the account's session and printers as the app
//! keeps them.
//!
//! No Tauri here: the window is made in `main.rs` from what this module
//! says, so the guard and the reading are tested without a window.
//!
//! The reference: `WebUserLoginDialog.cpp` (the page, its script
//! messages), `LoginUserAgent.cpp` (the User-Agent), `DeviceManager.cpp`
//! `parse_user_print_info` (the account's printers).

use serde_json::{Value, json};
use url::Url;

/// `bambu_network_get_bambulab_host` in UltraNet (`Agent::bambulabHost`),
/// and the bare, not localed, path the reference loads
/// (WebUserLoginDialog.cpp l.84-90).
pub const SIGN_IN_URL: &str = "https://bambulab.com/sign-in";

/// The sites the sign-in window may show, each with its subdomains, over
/// https only. The guard applies to every frame of the page (wry's macOS
/// navigation delegate asks for subframes too), so a frame the sign-in
/// needs from another site (a captcha) is refused until its site is named
/// here.
pub const ALLOWED_DOMAINS: &[&str] = &["bambulab.com"];

/// Where the window's script sends the page's message: a name that never
/// resolves (RFC 6761 `.invalid`), stopped by the navigation guard.
pub const RESULT_HOST: &str = "sign-in-result.edgeslicer.invalid";

/// The window's label: the app's commands refuse every window but `main`.
pub const WINDOW: &str = "bambu-sign-in";

/// A test-only override of the page: `EDGE_BAMBU_SIGN_IN`, loopback only.
pub const OVERRIDE: &str = "EDGE_BAMBU_SIGN_IN";

/// The version bambulab.com gates its slicer sign-in on
/// (LoginUserAgent.hpp `BBL_LOGIN_UA_VERSION`), not this app's own.
pub const UA_VERSION: &str = "02.08.02.61";

/// The page the window opens: [`SIGN_IN_URL`], or the override when it
/// names a loopback page (refused otherwise, never the real page instead).
pub fn sign_in_url(override_value: Option<&str>) -> Result<Url, String> {
    match override_value.filter(|value| !value.is_empty()) {
        None => Ok(Url::parse(SIGN_IN_URL).expect("a URL")),
        Some(value) => match Url::parse(value) {
            Ok(url) if is_loopback_page(&url) => Ok(url),
            _ => Err(format!(
                "{OVERRIDE} must name a page on 127.0.0.1 or localhost; the sign-in was not opened."
            )),
        },
    }
}

/// The override from the environment.
pub fn sign_in_url_from_env() -> Result<Url, String> {
    sign_in_url(std::env::var(OVERRIDE).ok().as_deref())
}

fn is_loopback_page(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
}

/// What the window does with a navigation.
#[derive(Debug, PartialEq)]
pub enum Navigation {
    /// A page it may show.
    Allow,
    /// The page's message, handed back by the window's script: stopped,
    /// and read.
    Result(Message),
    /// Anywhere else: stopped, and not opened elsewhere.
    Refuse,
}

/// The guard: Bambu's https pages (and, with the override, the loopback
/// page's origin), the result, nothing else.
pub fn navigation(url: &Url, start: &Url) -> Navigation {
    if url.scheme() == "https" && url.host_str() == Some(RESULT_HOST) {
        return Navigation::Result(read_message(url.fragment().unwrap_or_default()));
    }
    if url.username() != "" || url.password().is_some() {
        return Navigation::Refuse;
    }
    if is_loopback_page(start) {
        // The test page: its own origin only.
        return if url.origin() == start.origin() {
            Navigation::Allow
        } else {
            Navigation::Refuse
        };
    }
    let bambu = url.host_str().is_some_and(|host| {
        ALLOWED_DOMAINS
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    });
    if url.scheme() == "https" && bambu && url.port().is_none() {
        Navigation::Allow
    } else {
        Navigation::Refuse
    }
}

/// What the page handed back (WebUserLoginDialog.cpp `OnScriptMessage`).
#[derive(Clone, PartialEq)]
pub enum Message {
    /// `user_ticket_login`: a ticket for `account_sign_in`.
    Ticket(String),
    /// `user_login`: the tokens themselves, for `account_restore`.
    Tokens(AccountSession, User),
    /// Google or Apple through the system's browser (`get_localhost_url`,
    /// `thirdparty_login`): not built, it needs a listening port.
    ThirdParty,
    /// Anything else (`autotest_token`, `new_webpage`, nonsense).
    Ignored,
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the ticket or a token.
        match self {
            Message::Ticket(_) => f.write_str("Ticket(..)"),
            Message::Tokens(_, user) => write!(f, "Tokens(.., {user:?})"),
            Message::ThirdParty => f.write_str("ThirdParty"),
            Message::Ignored => f.write_str("Ignored"),
        }
    }
}

/// The fragment the script wrote: the page's message, percent-encoded.
pub fn read_message(fragment: &str) -> Message {
    let text: String = url::form_urlencoded::parse(format!("m={fragment}").as_bytes())
        .find(|(key, _)| key == "m")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return Message::Ignored;
    };
    let data = &root["data"];
    match root["command"].as_str() {
        Some("user_ticket_login") => match data["ticket"].as_str() {
            Some(ticket) if credential_ok(ticket) => Message::Ticket(ticket.to_owned()),
            _ => Message::Ignored,
        },
        Some("user_login") => {
            let session = AccountSession {
                access_token: data["token"].as_str().unwrap_or_default().to_owned(),
                refresh_token: data["refresh_token"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                expires_in: number(&data["expires_in"]),
                refresh_expires_in: number(&data["refresh_expires_in"]),
                acquired_at: now(),
            };
            let user = User {
                id: text_or_number(&data["user"]["uid"]),
                name: data["user"]["name"].as_str().unwrap_or_default().to_owned(),
            };
            if credential_ok(&session.access_token) {
                Message::Tokens(session, user)
            } else {
                Message::Ignored
            }
        }
        Some("get_localhost_url" | "thirdparty_login") => Message::ThirdParty,
        _ => Message::Ignored,
    }
}

/// The bridge's rule for a ticket or token (section 16.2): letters,
/// digits and `-._~+/=`, at most 8 KiB.
pub fn credential_ok(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 8192
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
}

fn number(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or(0)
}

fn text_or_number(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Seconds since 1970.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The script the window runs before the page's own: the slicer's
/// `window.wx.postMessage`, which wxWebView defines for its message
/// handler "wx" (WebView.cpp `AddScriptMessageHandler("wx")`), made a
/// navigation to [`RESULT_HOST`] that the guard stops and reads. No call
/// into the app is made.
pub fn init_script() -> String {
    format!(
        r#"(function () {{
  var send = function (message) {{
    var text = typeof message === 'string' ? message : JSON.stringify(message);
    window.location.href = 'https://{RESULT_HOST}/#' + encodeURIComponent(text);
  }};
  try {{ Object.defineProperty(window, 'wx', {{ value: {{ postMessage: send }}, writable: false, configurable: false }}); }}
  catch (e) {{ window.wx = {{ postMessage: send }}; }}
  // A page that calls WebKit's handler itself (`window.webkit.messageHandlers.wx`).
  try {{
    if (window.webkit && window.webkit.messageHandlers && !window.webkit.messageHandlers.wx)
      Object.defineProperty(window.webkit.messageHandlers, 'wx', {{ value: {{ postMessage: send }} }});
  }} catch (e) {{}}
}})();"#
    )
}

/// `bbl_login_user_agent` (LoginUserAgent.cpp): the browser prefix, then
/// `BBL-Slicer/v<gated version> (<theme>) BBL-Language/<primary subtag>`.
pub fn user_agent(windows: bool, dark: bool, language: &str) -> String {
    let lang = language
        .split(['_', '-'])
        .next()
        .filter(|lang| !lang.is_empty())
        .unwrap_or("en");
    let prefix = if windows {
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/107.0.0.0 Safari/537.36 Edg/107.0.1418.52"
    } else {
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)"
    };
    format!(
        "{prefix} BBL-Slicer/v{UA_VERSION} ({}) BBL-Language/{lang}",
        if dark { "dark" } else { "light" }
    )
}

// ---- what is kept ----

/// The signed-in user: the profile's `uidStr` and `name`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct User {
    pub id: String,
    pub name: String,
}

impl User {
    pub fn json(&self) -> Value {
        json!({ "id": self.id, "name": self.name })
    }

    pub fn from(value: &Value) -> User {
        User {
            id: text_or_number(&value["id"]),
            name: value["name"].as_str().unwrap_or_default().to_owned(),
        }
    }
}

/// The tokens (section 16.2 `session`). Only ever in the store, the
/// bridge's pipe, and memory.
#[derive(Clone, Default, PartialEq)]
pub struct AccountSession {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
    pub acquired_at: i64,
}

impl std::fmt::Debug for AccountSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountSession")
            .field("expires_in", &self.expires_in)
            .field("acquired_at", &self.acquired_at)
            .finish_non_exhaustive()
    }
}

impl AccountSession {
    pub fn json(&self) -> Value {
        json!({
            "access_token": self.access_token, "refresh_token": self.refresh_token,
            "expires_in": self.expires_in, "refresh_expires_in": self.refresh_expires_in,
            "acquired_at": self.acquired_at,
        })
    }

    /// From a result's or the store's `session`; `None` without an access
    /// token.
    pub fn from(value: &Value) -> Option<AccountSession> {
        let session = AccountSession {
            access_token: value["access_token"].as_str()?.to_owned(),
            refresh_token: value["refresh_token"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            expires_in: number(&value["expires_in"]),
            refresh_expires_in: number(&value["refresh_expires_in"]),
            acquired_at: number(&value["acquired_at"]),
        };
        (!session.access_token.is_empty()).then_some(session)
    }

    /// The texts to take out of whatever the bridge says.
    pub fn secrets(&self) -> Vec<String> {
        [&self.access_token, &self.refresh_token]
            .into_iter()
            .filter(|s| !s.is_empty())
            .cloned()
            .collect()
    }
}

/// A printer of the account, without its code (`parse_user_print_info`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountPrinter {
    pub dev_id: String,
    pub name: String,
    /// `dev_model_name`: the model's id ("C12").
    pub model: String,
    pub online: bool,
    /// `task_status`.
    pub task_status: String,
    /// The account gave a code for it (kept in the store).
    pub has_code: bool,
}

/// The account's printers from `account_printers`' list, and their codes.
pub fn read_printers(list: &Value) -> (Vec<AccountPrinter>, Vec<(String, String)>) {
    let mut printers = Vec::new();
    let mut codes = Vec::new();
    for entry in list.as_array().into_iter().flatten() {
        let dev_id = entry["dev_id"].as_str().unwrap_or_default().to_uppercase();
        if dev_id.is_empty() {
            continue;
        }
        let code: String = entry["dev_access_code"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(|c| *c != '\n')
            .collect();
        if !code.is_empty() {
            codes.push((dev_id.clone(), code.clone()));
        }
        printers.push(AccountPrinter {
            name: entry["dev_name"].as_str().unwrap_or_default().to_owned(),
            model: entry["dev_model_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            online: entry["dev_online"].as_bool().unwrap_or(false),
            task_status: entry["task_status"].as_str().unwrap_or_default().to_owned(),
            has_code: !code.is_empty(),
            dev_id,
        });
    }
    (printers, codes)
}

/// What the store's `account` field holds (section 16.4), version 1.
#[derive(Clone, Default, PartialEq)]
pub struct AccountSecret {
    pub user: User,
    pub session: AccountSession,
    /// The account's access codes, by serial.
    pub codes: std::collections::BTreeMap<String, String>,
}

impl std::fmt::Debug for AccountSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountSecret")
            .field("user", &self.user)
            .field("printers", &self.codes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl AccountSecret {
    pub const VERSION: i64 = 1;

    pub fn text(&self) -> String {
        json!({
            "version": Self::VERSION,
            "user": self.user.json(),
            "session": self.session.json(),
            "codes": self.codes,
        })
        .to_string()
    }

    /// `None` for another version, or text that is not one.
    pub fn parse(text: &str) -> Option<AccountSecret> {
        let root: Value = serde_json::from_str(text).ok()?;
        if root["version"].as_i64() != Some(Self::VERSION) {
            return None;
        }
        Some(AccountSecret {
            user: User::from(&root["user"]),
            session: AccountSession::from(&root["session"]).unwrap_or_default(),
            codes: root["codes"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(id, code)| Some((id.clone(), code.as_str()?.to_owned())))
                .collect(),
        })
    }

    /// Every secret it holds, to take out of what the bridge says.
    pub fn secrets(&self) -> Vec<String> {
        let mut out = self.session.secrets();
        out.extend(self.codes.values().cloned());
        out
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
