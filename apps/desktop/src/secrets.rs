//! Where this app keeps a secret: a Bambu Lab printer's LAN access code,
//! by the printer's serial (`ultranet::printers`).
//!
//! One small interface, [`Secrets`] (get, set, delete by id), with three
//! stores behind it:
//!
//! - [`System`]: the operating system's store, through the `keyring-core`
//!   crate and one back-end for each system: the macOS login keychain,
//!   Windows Credential Manager, the Secret Service on Linux. It is the
//!   real app's.
//! - [`FileStore`]: `bambu_printers_secrets.json` in the app's data folder,
//!   readable by this user alone (mode 0600), as the codes were kept before
//!   the system store: the fallback where the system's store is not there
//!   or refuses, and what the system store is filled from the first time.
//! - [`Memory`]: for the run only; tests, and checks of the page.
//!
//! ## Few prompts on macOS
//!
//! The keychain asks the person ("<app> wants to use your confidential
//! information stored in ...") when a program reads or changes an item
//! that is not on that item's list of programs allowed, which holds the
//! program that made it (by its code signature) and those the person
//! answered "Always Allow" for; each item has its own list. So:
//!
//! - every code is in ONE item (service "EdgeSlicerRS", account
//!   "Bambu Lab access codes", the codes as JSON), so one answer covers
//!   every printer;
//! - nothing is read until something needs a code (connect, send, a code
//!   added, changed or removed), never when the app starts or a page is
//!   looked at; whether a printer has a code is kept, without the code, in
//!   the plain list of printers;
//! - what was read is kept for the run, so a run asks at most once to read;
//! - the item is written only when a code is added, changed or removed.
//!
//! A build signed again with another identity, or not signed (each build
//! of a development copy is signed ad hoc and so is a new program to the
//! keychain), is asked again: "Always Allow" sticks across updates only for
//! builds signed with the same Developer ID.
//!
//! The store is chosen by `EDGE_SECRETS` (`system`, `file`, `memory`, or
//! `unavailable`, a system store that refuses everything, to see the
//! fallback); with none the real app uses the system's. A test build never
//! makes a [`System`] store on the real back-end: [`System`] there is only
//! ever handed a mock.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::hosts::kept::write_whole;

/// Why a store would not do what it was asked, in plain words. Never holds
/// a secret.
#[derive(Clone, Debug, PartialEq)]
pub struct Refused(pub String);

/// Secrets by id (a printer's serial).
pub trait Secrets: Send {
    fn get(&mut self, id: &str) -> Result<Option<String>, Refused>;
    fn set(&mut self, id: &str, secret: &str) -> Result<(), Refused>;
    fn delete(&mut self, id: &str) -> Result<(), Refused>;
    /// Several set at once; a store that keeps them together writes once.
    fn set_many(&mut self, items: &BTreeMap<String, String>) -> Result<(), Refused> {
        for (id, secret) in items {
            self.set(id, secret)?;
        }
        Ok(())
    }
    /// Forgets what it read, so the next `get` reads the store itself (to
    /// check what was written).
    fn reread(&mut self) {}
    /// Which store this is.
    fn kind(&self) -> Kind;
}

/// The stores there are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    System,
    File,
    Memory,
    /// `EDGE_SECRETS=unavailable`: a system store that refuses all.
    Unavailable,
}

impl Kind {
    /// What `EDGE_SECRETS` asks for; with nothing (or anything else), the
    /// system's store, but never in a test build, whose default is the
    /// file in the test's own folder.
    pub fn chosen() -> Kind {
        let asked = std::env::var("EDGE_SECRETS").unwrap_or_default();
        match asked.trim() {
            "file" => Kind::File,
            "memory" => Kind::Memory,
            "unavailable" => Kind::Unavailable,
            _ if cfg!(test) => Kind::File,
            _ => Kind::System,
        }
    }

    /// Whether it is the system's store (or stands for it).
    pub fn is_system(self) -> bool {
        matches!(self, Kind::System | Kind::Unavailable)
    }
}

/// A store of `kind`; `dir` is the app's data folder (none: for the run).
pub fn open(kind: Kind, dir: Option<&Path>) -> Box<dyn Secrets> {
    match kind {
        Kind::System => system(),
        Kind::File => Box::new(FileStore::new(dir.map(|dir| dir.join(FILE)))),
        Kind::Memory => Box::new(Memory::default()),
        Kind::Unavailable => Box::new(System::new(Box::new(Refusing(
            "EDGE_SECRETS=unavailable".into(),
        )))),
    }
}

#[cfg(not(test))]
fn system() -> Box<dyn Secrets> {
    Box::new(System::new(Box::new(KeyringItem::new(Box::new(
        platform::store,
    )))))
}

/// A test build has no way to the real keychain.
#[cfg(test)]
fn system() -> Box<dyn Secrets> {
    Box::new(System::new(Box::new(Refusing(
        "not in a test build".into(),
    ))))
}

/// What the system's store is called where this app runs.
pub fn system_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "the macOS Keychain"
    } else if cfg!(windows) {
        "Windows Credential Manager"
    } else {
        "the system keyring (Secret Service)"
    }
}

// ---- in memory ----

/// For the run only.
#[derive(Debug, Default)]
pub struct Memory {
    secrets: BTreeMap<String, String>,
}

impl Secrets for Memory {
    fn get(&mut self, id: &str) -> Result<Option<String>, Refused> {
        Ok(self.secrets.get(id).cloned())
    }
    fn set(&mut self, id: &str, secret: &str) -> Result<(), Refused> {
        self.secrets.insert(id.to_owned(), secret.to_owned());
        Ok(())
    }
    fn delete(&mut self, id: &str) -> Result<(), Refused> {
        self.secrets.remove(id);
        Ok(())
    }
    fn kind(&self) -> Kind {
        Kind::Memory
    }
}

// ---- the file ----

/// The file the codes were kept in before the system's store, and are
/// kept in where it cannot be used.
pub const FILE: &str = "bambu_printers_secrets.json";

/// `{ "<serial>": { "access_code" } }`, readable by this user alone,
/// written whole or not at all; read when first needed.
#[derive(Debug)]
pub struct FileStore {
    path: Option<PathBuf>,
    read: Option<BTreeMap<String, String>>,
}

impl FileStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        FileStore { path, read: None }
    }

    fn load(&mut self) -> Result<&mut BTreeMap<String, String>, Refused> {
        if self.read.is_none() {
            let held = match &self.path {
                Some(path) => read_file(path)?,
                None => BTreeMap::new(),
            };
            self.read = Some(held);
        }
        Ok(self.read.get_or_insert_with(BTreeMap::new))
    }

    fn write(&self, secrets: &BTreeMap<String, String>) -> Result<(), Refused> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|error| Refused(format!("{FILE}: {}", error.kind())))?;
        }
        write_whole(path, json_of(secrets).as_bytes(), true)
            .map_err(|error| Refused(format!("{FILE}: {}", error.kind())))
    }
}

/// The codes the file at `path` holds: none when there is no file.
pub fn read_file(path: &Path) -> Result<BTreeMap<String, String>, Refused> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(Refused(format!("{FILE}: {}", error.kind()))),
        Ok(text) => parse(&text).ok_or_else(|| Refused(format!("{FILE} could not be read"))),
    }
}

/// `{ "<id>": { "access_code": "<code>" } }`.
fn parse(text: &str) -> Option<BTreeMap<String, String>> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut out = BTreeMap::new();
    for (id, entry) in root.as_object()? {
        let code = entry["access_code"].as_str().unwrap_or_default();
        if !code.is_empty() {
            out.insert(id.clone(), code.to_owned());
        }
    }
    Some(out)
}

fn json_of(secrets: &BTreeMap<String, String>) -> String {
    let root: serde_json::Map<String, serde_json::Value> = secrets
        .iter()
        .map(|(id, code)| (id.clone(), serde_json::json!({ "access_code": code })))
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Object(root)).expect("plain JSON")
}

impl Secrets for FileStore {
    fn get(&mut self, id: &str) -> Result<Option<String>, Refused> {
        Ok(self.load()?.get(id).cloned())
    }
    fn set(&mut self, id: &str, secret: &str) -> Result<(), Refused> {
        let mut next = self.load()?.clone();
        if next.get(id).map(String::as_str) == Some(secret) {
            return Ok(());
        }
        next.insert(id.to_owned(), secret.to_owned());
        self.write(&next)?;
        self.read = Some(next);
        Ok(())
    }
    fn delete(&mut self, id: &str) -> Result<(), Refused> {
        let mut next = self.load()?.clone();
        if next.remove(id).is_none() {
            return Ok(());
        }
        self.write(&next)?;
        self.read = Some(next);
        Ok(())
    }
    fn reread(&mut self) {
        self.read = None;
    }
    fn kind(&self) -> Kind {
        Kind::File
    }
}

// ---- the system's store ----

/// The one item every code is kept in, as the system's store holds it:
/// read whole, written whole, removed. `Ok(None)` when there is none.
pub trait Item: Send {
    fn read(&mut self) -> Result<Option<String>, Refused>;
    fn write(&mut self, text: &str) -> Result<(), Refused>;
    fn remove(&mut self) -> Result<(), Refused>;
}

/// The item's names in the store.
pub const SERVICE: &str = "EdgeSlicerRS";
pub const ACCOUNT: &str = "Bambu Lab access codes";

/// Every code in one item of the system's store, read the first time one
/// is needed and kept for the run.
pub struct System {
    item: Box<dyn Item>,
    read: Option<BTreeMap<String, String>>,
}

impl System {
    pub fn new(item: Box<dyn Item>) -> Self {
        System { item, read: None }
    }

    fn load(&mut self) -> Result<&mut BTreeMap<String, String>, Refused> {
        if self.read.is_none() {
            let held = match self.item.read()? {
                None => BTreeMap::new(),
                Some(text) => parse_item(&text).ok_or_else(|| {
                    Refused(format!(
                        "what {} holds for this app could not be read",
                        system_name()
                    ))
                })?,
            };
            self.read = Some(held);
        }
        Ok(self.read.get_or_insert_with(BTreeMap::new))
    }

    fn store(&mut self, next: BTreeMap<String, String>) -> Result<(), Refused> {
        if next.is_empty() {
            self.item.remove()?;
        } else {
            self.item.write(&item_text(&next))?;
        }
        self.read = Some(next);
        Ok(())
    }
}

/// `{ "version": 1, "codes": { "<id>": "<code>" } }`.
fn parse_item(text: &str) -> Option<BTreeMap<String, String>> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut out = BTreeMap::new();
    for (id, code) in root["codes"].as_object()? {
        out.insert(id.clone(), code.as_str()?.to_owned());
    }
    Some(out)
}

fn item_text(codes: &BTreeMap<String, String>) -> String {
    serde_json::json!({ "version": 1, "codes": codes }).to_string()
}

impl Secrets for System {
    fn get(&mut self, id: &str) -> Result<Option<String>, Refused> {
        Ok(self.load()?.get(id).cloned())
    }
    fn set(&mut self, id: &str, secret: &str) -> Result<(), Refused> {
        let mut next = self.load()?.clone();
        if next.get(id).map(String::as_str) == Some(secret) {
            return Ok(());
        }
        next.insert(id.to_owned(), secret.to_owned());
        self.store(next)
    }
    fn delete(&mut self, id: &str) -> Result<(), Refused> {
        let mut next = self.load()?.clone();
        if next.remove(id).is_none() {
            return Ok(());
        }
        self.store(next)
    }
    fn set_many(&mut self, items: &BTreeMap<String, String>) -> Result<(), Refused> {
        let mut next = self.load()?.clone();
        let before = next.clone();
        next.extend(items.iter().map(|(id, code)| (id.clone(), code.clone())));
        if next == before {
            return Ok(());
        }
        self.store(next)
    }
    fn reread(&mut self) {
        self.read = None;
    }
    fn kind(&self) -> Kind {
        Kind::System
    }
}

/// An item of a store that refuses everything: where there is none.
pub struct Refusing(pub String);

impl Item for Refusing {
    fn read(&mut self) -> Result<Option<String>, Refused> {
        Err(Refused(self.0.clone()))
    }
    fn write(&mut self, _: &str) -> Result<(), Refused> {
        Err(Refused(self.0.clone()))
    }
    fn remove(&mut self) -> Result<(), Refused> {
        Err(Refused(self.0.clone()))
    }
}

/// Makes the store an item is on, when it is first used.
pub type OpenStore =
    Box<dyn Fn() -> keyring_core::Result<std::sync::Arc<keyring_core::CredentialStore>> + Send>;

/// The item through `keyring-core`, on whichever store it is handed.
pub struct KeyringItem {
    open: OpenStore,
    entry: Option<keyring_core::Entry>,
}

impl KeyringItem {
    /// `open` makes the store when the item is first used (the Secret
    /// Service's connects to the session bus then, not when the app starts).
    pub fn new(open: OpenStore) -> Self {
        KeyringItem { open, entry: None }
    }

    fn entry(&mut self) -> Result<&keyring_core::Entry, Refused> {
        if self.entry.is_none() {
            let store = (self.open)().map_err(refused)?;
            self.entry = Some(store.build(SERVICE, ACCOUNT, None).map_err(refused)?);
        }
        Ok(self.entry.as_ref().expect("made above"))
    }
}

/// A store's error in plain words; never the bytes it read.
fn refused(error: keyring_core::Error) -> Refused {
    use keyring_core::Error;
    let name = system_name();
    Refused(match error {
        Error::NoStorageAccess(why) => {
            format!("{name} did not allow it ({why}); it may be locked, or access was denied")
        }
        Error::PlatformFailure(why) => format!("{name} failed ({why})"),
        Error::NoDefaultStore | Error::NotSupportedByStore(_) => format!("{name} is not available"),
        Error::BadEncoding(_) | Error::BadDataFormat(..) | Error::BadStoreFormat(_) => {
            format!("what {name} holds for this app could not be read")
        }
        _ => format!("{name} could not be used"),
    })
}

impl Item for KeyringItem {
    fn read(&mut self) -> Result<Option<String>, Refused> {
        match self.entry()?.get_password() {
            Ok(text) => Ok(Some(text)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(refused(error)),
        }
    }
    fn write(&mut self, text: &str) -> Result<(), Refused> {
        self.entry()?.set_password(text).map_err(refused)
    }
    fn remove(&mut self) -> Result<(), Refused> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(error) => Err(refused(error)),
        }
    }
}

/// The real back-ends: never built into a test.
#[cfg(not(test))]
mod platform {
    use std::sync::Arc;

    use keyring_core::CredentialStore;

    /// This system's store. Making it touches nothing on macOS and
    /// Windows; the Secret Service's connects to the session bus.
    pub fn store() -> keyring_core::Result<Arc<CredentialStore>> {
        #[cfg(target_os = "macos")]
        return Ok(apple_native_keyring_store::keychain::Store::new()?);
        #[cfg(windows)]
        return Ok(windows_native_keyring_store::Store::new()?);
        #[cfg(all(unix, not(target_os = "macos")))]
        return Ok(zbus_secret_service_keyring_store::Store::new()?);
        #[allow(unreachable_code)]
        Err(keyring_core::Error::NoDefaultStore)
    }
}

#[cfg(test)]
pub(crate) mod tests;
