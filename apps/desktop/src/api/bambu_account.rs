//! The Bambu Lab account (ULTRANET_BRIDGE.md section 16): "Log in to Bambu
//! Account...", who is signed in, "Log out of Bambu Account", and the
//! account's printers, through the bridge.
//!
//! The reference: the Account menu (MainFrame.cpp l.3297-3307), the
//! sign-in dialog (WebUserLoginDialog.cpp), `request_user_logout`
//! (GUI_App.cpp l.6445), an expired session (GUI_App.cpp l.6873-6880),
//! the account's printers (DeviceManager.cpp `parse_user_print_info`,
//! `clean_user_info`).
//!
//! The sign-in page is a window of its own (main.rs), which hands what it
//! got back here through [`Session::bambu_sign_in_message`], never through
//! a call of the page's. The ticket, the tokens and the account's codes
//! stay on the Rust side: no answer here holds one (a test searches them
//! all), and they are taken out of whatever the bridge says.
//!
//! **Differs**: the reference restores a sign-in when the app starts
//! (`bambu_network_start`); here the store is read only when the person
//! does something with the account (Device's "Refresh", or opening Device
//! when the store is at hand), so starting the app never asks the system's
//! store. Who is signed in and the account's printers, without codes, are
//! kept in the plain list meanwhile.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Session, lock};
use crate::api::i18n::tr;
use crate::ultranet::{
    account::{self, AccountSecret, AccountSession, Message, User},
    client::{ACCOUNT, State},
};

/// The calls answered here.
pub(super) const JSON: [&str; 4] = [
    "bambu_account",
    "bambu_account_sign_in",
    "bambu_account_sign_out",
    "bambu_account_refresh",
];

/// Opens the sign-in window on a page (main.rs); a session without one
/// (`--serve`, the tests) cannot sign in.
pub type Opener = Arc<dyn Fn(&url::Url) -> Result<(), String> + Send + Sync>;

/// Where a sign-in stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) enum Phase {
    #[default]
    Idle,
    /// The sign-in window is open.
    Window,
    /// The bridge is asked (the sign-in, the printers).
    Working,
}

#[derive(Default)]
pub(super) struct AccountState {
    pub(super) phase: Phase,
    /// What the panel says: a failure, an expired session.
    pub(super) said: Option<String>,
    pub(super) opener: Option<Opener>,
}

/// `ret` of section 16.2.
const EXPIRED: i64 = -2;

impl Session {
    /// The sign-in window's title ("Login", WebUserLoginDialog.cpp l.131)
    /// in the page's language, and that language's code (for the page's
    /// User-Agent, `BBL-Language`).
    pub fn sign_in_window_words(&self) -> (String, String) {
        let language = lock(&self.language).clone();
        let code = language.code.to_owned();
        (super::i18n::with(language, || tr("Login")), code)
    }

    /// The window that shows Bambu's sign-in page (main.rs gives it).
    pub fn set_sign_in_window(&self, opener: Opener) {
        lock(&self.bambu.account).opener = Some(opener);
    }

    pub(super) fn bambu_account_json(&self, name: &str, _args: &Value) -> Result<Value, String> {
        match name {
            "bambu_account" => Ok(self.account_status()),
            "bambu_account_sign_in" => self.account_sign_in(),
            "bambu_account_sign_out" => {
                self.account_sign_out()?;
                Ok(self.account_status())
            }
            "bambu_account_refresh" => {
                self.account_refresh();
                Ok(self.account_status())
            }
            other => Err(format!("there is no call named {other}")),
        }
    }

    /// `{ signed_in, name, phase, said, offered, can_sign_in }`: who, never
    /// a token.
    pub(super) fn account_status(&self) -> Value {
        if self.bambu.client.take_expired() {
            self.account_expired();
        }
        let kept = lock(&self.bambu.kept);
        let state = lock(&self.bambu.account);
        let client = &self.bambu.client;
        // Before the bridge said hello its methods are not known: offered
        // unless it is absent or refused.
        let offered = match client.state() {
            State::Absent | State::Refused(_) => false,
            State::Running => client.offers(&ACCOUNT),
            _ => client.hello().is_none_or(|_| client.offers(&ACCOUNT)),
        };
        json!({
            "signed_in": kept.account.is_some(),
            "name": kept.account.as_ref().map(|a| a.user.name.clone()),
            "phase": match state.phase {
                Phase::Idle => "idle",
                Phase::Window => "window",
                Phase::Working => "working",
            },
            "said": state.said.as_deref().map(|said| client.scrub(said)),
            "offered": offered,
            "can_sign_in": state.opener.is_some(),
        })
    }

    fn say_account(&self, said: Option<String>) {
        lock(&self.bambu.account).said = said;
    }

    fn set_phase(&self, phase: Phase) {
        lock(&self.bambu.account).phase = phase;
    }

    /// The window's sign-in taken up: only one, and only while the window
    /// is open (a page that posts twice would sign out the first).
    fn begin_sign_in(&self) -> bool {
        let mut state = lock(&self.bambu.account);
        if state.phase != Phase::Window {
            return false;
        }
        state.phase = Phase::Working;
        true
    }

    /// A sentence of the reference's in the page's language: the window's
    /// hand-back runs on a thread of its own, outside a call.
    fn words(&self, id: &str) -> String {
        let language = lock(&self.language).clone();
        super::i18n::with(language, || tr(id))
    }

    /// "Log in to Bambu Account...": the bridge checked (as
    /// `ShowUserLoginGuarded` checks the plug-in), then the window opened.
    fn account_sign_in(&self) -> Result<Value, String> {
        let opener = lock(&self.bambu.account).opener.clone().ok_or(
            "Signing in to Bambu Lab needs the app's own window; it cannot be done from a browser.",
        )?;
        self.bambu.client.start()?;
        if let Some(missing) = ACCOUNT.iter().find(|m| !self.bambu.client.offers(&[m])) {
            return Err(format!(
                "The UltraNet bridge is too old to sign in to Bambu Lab: it does not offer {missing}."
            ));
        }
        let url = account::sign_in_url_from_env()?;
        self.say_account(None);
        self.set_phase(Phase::Window);
        if let Err(why) = opener(&url) {
            self.set_phase(Phase::Idle);
            return Err(why);
        }
        Ok(self.account_status())
    }

    /// The sign-in window was closed without a sign-in.
    pub fn bambu_sign_in_closed(&self) {
        let mut state = lock(&self.bambu.account);
        if state.phase == Phase::Window {
            state.phase = Phase::Idle;
        }
    }

    /// What the sign-in page handed back (main.rs reads it from the
    /// window). Answers whether the window has done its work and is to be
    /// closed. Blocks while the bridge is asked: called on a thread of its
    /// own.
    pub fn bambu_sign_in_message(&self, message: Message) -> bool {
        match message {
            Message::Ignored => false,
            Message::ThirdParty => {
                self.say_account(Some(
                    "Signing in with Google or Apple is not offered here: sign in with your Bambu Lab account's e-mail and password.".into(),
                ));
                false
            }
            Message::Ticket(ticket) => {
                if !self.begin_sign_in() {
                    return false;
                }
                self.bambu.client.set_codes(vec![ticket.clone()]);
                let answer = self
                    .bambu
                    .client
                    .ask_account("account_sign_in", json!({ "ticket": ticket }));
                match answer {
                    Ok(result) if result["ret"] == 0 => {
                        let session = AccountSession::from(&result["session"]);
                        let user = User::from(&result["user"]);
                        match session {
                            Some(session) => self.signed_in(user, session),
                            None => self.sign_in_failed(),
                        }
                    }
                    _ => self.sign_in_failed(),
                }
                true
            }
            Message::Tokens(session, user) => {
                if !self.begin_sign_in() {
                    return false;
                }
                self.bambu.client.set_codes(session.secrets());
                let answer = self.bambu.client.ask_account(
                    "account_restore",
                    json!({ "session": session.json(), "user": user.json() }),
                );
                match answer {
                    Ok(result) if result["ret"] == 0 => {
                        let session = AccountSession::from(&result["session"]).unwrap_or(session);
                        self.signed_in(User::from(&result["user"]), session);
                    }
                    _ => self.sign_in_failed(),
                }
                true
            }
        }
    }

    /// "Login failed. Please try again." (WebUserLoginDialog.cpp l.368).
    fn sign_in_failed(&self) {
        self.say_account(Some(self.words("Login failed. Please try again.")));
        self.set_phase(Phase::Idle);
    }

    /// Signed in: kept, then the account's printers asked for
    /// (`kick_user_device_refresh`, WebUserLoginDialog.cpp l.360-365).
    fn signed_in(&self, user: User, session: AccountSession) {
        // An expiry of an earlier session, not seen yet, is not this one's.
        self.bambu.client.take_expired();
        let secret = AccountSecret {
            user,
            session,
            codes: Default::default(),
        };
        let kept = {
            let mut kept = lock(&self.bambu.kept);
            let done = kept.set_account(secret, Some(Vec::new()));
            self.bambu.client.set_codes(kept.codes());
            done
        };
        match kept {
            Ok(()) => {
                self.say_account(None);
                self.account_printers();
            }
            Err(why) => self.say_account(Some(why)),
        }
        self.set_phase(Phase::Idle);
    }

    /// The bridge ended the session, or refused it: signed out, as
    /// `request_user_logout` does on a 401, and said.
    fn account_expired(&self) {
        let mut kept = lock(&self.bambu.kept);
        let _ = kept.clear_account();
        drop(kept);
        self.say_account(Some(
            self.words("Login information expired. Please login again."),
        ));
    }

    /// The bridge's answer to a call that may have made new tokens: those
    /// kept in place of the old.
    fn take_session(&self, result: &Value) {
        let Some(session) = AccountSession::from(&result["session"]) else {
            return;
        };
        let mut kept = lock(&self.bambu.kept);
        if let Ok(Some(mut secret)) = kept.account_secret() {
            secret.session = session;
            let _ = kept.set_account(secret, None);
            self.bambu.client.set_codes(kept.codes());
        }
    }

    /// "Refresh": the kept session handed to the bridge (`account_restore`,
    /// which refreshes it when its access token is over), then the
    /// account's printers. Reads the store.
    fn account_refresh(&self) {
        let secret = {
            let mut kept = lock(&self.bambu.kept);
            if kept.account.is_none() {
                return;
            }
            match kept.account_secret() {
                Ok(Some(secret)) => {
                    self.bambu.client.set_codes(kept.codes());
                    secret
                }
                // Listed as signed in, but nothing kept: signed out.
                Ok(None) => {
                    let _ = kept.clear_account();
                    return;
                }
                Err(why) => {
                    drop(kept);
                    self.say_account(Some(why));
                    return;
                }
            }
        };
        self.set_phase(Phase::Working);
        let answer = self.bambu.client.ask_account(
            "account_restore",
            json!({ "session": secret.session.json(), "user": secret.user.json() }),
        );
        match answer {
            Ok(result) if result["ret"] == 0 => {
                self.take_session(&result);
                self.say_account(None);
                self.account_printers();
            }
            Ok(result) if result["ret"] == EXPIRED => self.account_expired(),
            Ok(result) => self.say_account(Some(cloud_failed(&result))),
            Err(why) => self.say_account(Some(why)),
        }
        self.set_phase(Phase::Idle);
    }

    /// `update_user_machine_list_info`: the list and its codes kept; a
    /// failure keeps the list as it was.
    fn account_printers(&self) {
        match self.bambu.client.ask_account("account_printers", json!({})) {
            Ok(result) if result["ret"] == 0 => {
                self.take_session(&result);
                let (printers, codes) = account::read_printers(&result["printers"]);
                let mut kept = lock(&self.bambu.kept);
                if let Ok(Some(mut secret)) = kept.account_secret() {
                    secret.codes = codes.into_iter().collect();
                    if let Err(why) = kept.set_account(secret, Some(printers)) {
                        drop(kept);
                        self.say_account(Some(why));
                        return;
                    }
                }
                self.bambu.client.set_codes(kept.codes());
            }
            Ok(result) if result["ret"] == EXPIRED => self.account_expired(),
            Ok(result) => {
                self.take_session(&result);
                self.say_account(Some(cloud_failed(&result)));
            }
            Err(why) => self.say_account(Some(why)),
        }
    }

    /// "Log out of Bambu Account" (`request_user_logout`): the bridge told
    /// when it runs (none is started for it), then the account's secrets
    /// and printers removed (`clean_user_info`).
    fn account_sign_out(&self) -> Result<(), String> {
        if self.bambu.client.running() {
            let _ = self.bambu.client.ask_account("account_sign_out", json!({}));
        }
        lock(&self.bambu.kept).clear_account()?;
        self.say_account(None);
        Ok(())
    }

    /// The account's printers for Device: `[{ dev_id, name, model,
    /// model_name, online, task_status, has_access_code, added }]`, never a
    /// code. `added`: also added by hand (its address and code its own).
    pub(super) fn account_printers_json(
        &self,
        models: &[crate::ultranet::printers::Model],
    ) -> Value {
        let kept = lock(&self.bambu.kept);
        let Some(account) = &kept.account else {
            return json!([]);
        };
        Value::Array(
            account
                .printers
                .iter()
                .map(|p| {
                    json!({
                        "dev_id": p.dev_id,
                        "name": p.name,
                        "model": p.model,
                        "model_name": models.iter().find(|m| m.id == p.model).map(|m| m.name.clone()),
                        "online": p.online,
                        "task_status": p.task_status,
                        "has_access_code": p.has_code,
                        "added": kept.get(&p.dev_id).is_some(),
                    })
                })
                .collect(),
        )
    }
}

/// A cloud call that failed, without anything the bridge said.
fn cloud_failed(result: &Value) -> String {
    match result["http"].as_i64().unwrap_or(0) {
        0 => "The Bambu Lab cloud could not be reached.".into(),
        http => format!("The Bambu Lab cloud refused the request (HTTP {http})."),
    }
}

#[cfg(test)]
#[path = "bambu_account_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "bambu_account_e2e.rs"]
mod e2e;
