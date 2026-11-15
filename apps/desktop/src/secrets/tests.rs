//! The three stores through the one interface, the system's on
//! `keyring-core`'s mock store (never the real keychain), and the one
//! item: read once for the run, written only when something changed.

use std::sync::{Arc, Mutex};

use super::*;

/// What an item was asked, shared with the test.
#[derive(Default)]
pub(crate) struct Asked {
    pub reads: usize,
    pub writes: usize,
    pub removes: usize,
    /// The next calls refuse with this.
    pub refuse: Option<String>,
    /// Writes are taken but reads give back something else.
    pub forgetful: bool,
}

/// An item on `keyring-core`'s mock store, counting what it is asked.
pub(crate) struct Counted {
    inner: KeyringItem,
    pub asked: Arc<Mutex<Asked>>,
}

impl Counted {
    pub(crate) fn new() -> (Self, Arc<Mutex<Asked>>) {
        Self::on(keyring_core::mock::Store::new().unwrap())
    }

    /// One on `store`: several sessions' items on one store are the same
    /// item, as the keychain's of two runs are.
    pub(crate) fn on(store: Arc<keyring_core::mock::Store>) -> (Self, Arc<Mutex<Asked>>) {
        let open: OpenStore =
            Box::new(move || Ok(store.clone() as Arc<keyring_core::CredentialStore>));
        let asked = Arc::new(Mutex::new(Asked::default()));
        (
            Counted {
                inner: KeyringItem::new(open),
                asked: asked.clone(),
            },
            asked,
        )
    }

    fn refusal(&self) -> Result<(), Refused> {
        match &self.asked.lock().unwrap().refuse {
            Some(why) => Err(Refused(why.clone())),
            None => Ok(()),
        }
    }
}

impl Item for Counted {
    fn read(&mut self) -> Result<Option<String>, Refused> {
        self.asked.lock().unwrap().reads += 1;
        self.refusal()?;
        if self.asked.lock().unwrap().forgetful {
            return Ok(Some(r#"{"version":1,"codes":{}}"#.into()));
        }
        self.inner.read()
    }
    fn write(&mut self, text: &str) -> Result<(), Refused> {
        self.asked.lock().unwrap().writes += 1;
        self.refusal()?;
        self.inner.write(text)
    }
    fn remove(&mut self) -> Result<(), Refused> {
        self.asked.lock().unwrap().removes += 1;
        self.refusal()?;
        self.inner.remove()
    }
}

/// get, set (and set again the same), delete, as every store answers them.
fn behaves(store: &mut dyn Secrets) {
    assert_eq!(store.get("A").unwrap(), None);
    store.set("A", "12345678").unwrap();
    store.set("B", "abcdefgh").unwrap();
    store.set("A", "12345678").unwrap();
    assert_eq!(store.get("A").unwrap().as_deref(), Some("12345678"));
    store.reread();
    assert_eq!(store.get("B").unwrap().as_deref(), Some("abcdefgh"));
    store.set("A", "87654321").unwrap();
    assert_eq!(store.get("A").unwrap().as_deref(), Some("87654321"));
    store.delete("A").unwrap();
    store.delete("A").unwrap();
    store.reread();
    assert_eq!(store.get("A").unwrap(), None);
    assert_eq!(store.get("B").unwrap().as_deref(), Some("abcdefgh"));
    store.delete("B").unwrap();
    assert_eq!(store.get("B").unwrap(), None);
}

#[test]
fn every_store_gets_sets_and_deletes() {
    behaves(&mut Memory::default());
    let dir = tempfile::tempdir().unwrap();
    let mut file = FileStore::new(Some(dir.path().join(FILE)));
    behaves(&mut file);
    let (item, _) = Counted::new();
    behaves(&mut System::new(Box::new(item)));
}

#[test]
fn the_file_is_readable_by_its_user_alone_and_holds_the_old_layout() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE);
    let mut file = FileStore::new(Some(path.clone()));
    file.set("01P09C470100142", "12345678").unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let root: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(root["01P09C470100142"]["access_code"], "12345678");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn the_system_store_keeps_every_code_in_one_item_read_once() {
    let (item, asked) = Counted::new();
    let mut store = System::new(Box::new(item));
    // Made, nothing asked of the store.
    assert_eq!(asked.lock().unwrap().reads, 0);
    for (id, code) in [("A", "11111111"), ("B", "22222222"), ("C", "33333333")] {
        store.set(id, code).unwrap();
    }
    for id in ["A", "B", "C", "A", "B"] {
        store.get(id).unwrap();
    }
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.reads, 1, "read once for the run");
        assert_eq!(asked.writes, 3, "one write for each code added");
    }
    // The same code again writes nothing; many at once write once.
    store.set("A", "11111111").unwrap();
    assert_eq!(asked.lock().unwrap().writes, 3);
    let many: BTreeMap<String, String> = [("D", "44444444"), ("E", "55555555")]
        .map(|(id, code)| (id.to_owned(), code.to_owned()))
        .into();
    store.set_many(&many).unwrap();
    assert_eq!(asked.lock().unwrap().writes, 4);
    // The last one deleted removes the item.
    for id in ["A", "B", "C", "D", "E"] {
        store.delete(id).unwrap();
    }
    assert_eq!(asked.lock().unwrap().removes, 1);
    store.reread();
    assert_eq!(store.get("A").unwrap(), None);
}

#[test]
fn a_refusal_is_said_in_plain_words_without_the_code() {
    let mut store = System::new(Box::new(Refusing("the keychain is locked".into())));
    let Refused(why) = store.set("A", "12345678").unwrap_err();
    assert_eq!(why, "the keychain is locked");
    let said = refused(keyring_core::Error::BadEncoding(b"12345678".to_vec())).0;
    assert!(!said.contains("12345678"), "{said}");
    assert!(said.contains(system_name()), "{said}");
}

#[test]
fn a_test_build_never_has_the_system_store() {
    // Whatever EDGE_SECRETS says, the "system" store of a test build
    // refuses; it is never the real one.
    let mut store = open(Kind::System, None);
    assert!(store.get("A").is_err());
    assert_ne!(Kind::chosen(), Kind::System);
}
