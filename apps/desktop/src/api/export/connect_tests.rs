//! "Send to Bambu Connect" through the session: the file as "Export plate
//! sliced file" writes it, with the reference's one difference, and the
//! URL; each refusal. Nothing is opened: the session only answers the URL.

use std::{io::Read, path::Path, sync::Arc};

use serde_json::{Value, json};

use super::super::Session;
use super::connect::FOLDER_IN_TEST;
use crate::outside::{CONNECT_IN_TEST, Installed};

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const A1_MINI: &str = "Bambu Lab A1 mini 0.4 nozzle";

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session.call(name, &args).expect_err("should be refused")
}

fn wait(session: &Arc<Session>, id: u64) {
    loop {
        let status = ask(session, "status", json!({ "project": id }));
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The small project on an A1 mini's presets.
fn on_a1_mini(session: &Arc<Session>) -> u64 {
    let id = ask(session, "project_open", json!({ "path": reference(SMALL) }))["id"]
        .as_u64()
        .unwrap();
    let presets = ask(
        session,
        "presets",
        json!({ "vendor": "BBL", "machine": A1_MINI }),
    );
    ask(
        session,
        "project_set",
        json!({
            "project": id, "source": "presets", "vendor": "BBL", "machine": A1_MINI,
            "process": presets["processes"][0],
            "filaments": [presets["filaments"][0]],
            "initial": true,
        }),
    );
    id
}

fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|at| {
            let mut entry = archive.by_index(at).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().unwrap().into_owned(), bytes)
        })
        .collect()
}

fn text<'a>(all: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
    let (_, bytes) = all.iter().find(|(entry, _)| entry == name).unwrap();
    std::str::from_utf8(bytes).unwrap()
}

fn menu_entry(session: &Arc<Session>, id: &str) -> Option<Value> {
    fn walk(items: &Value, id: &str) -> Option<Value> {
        for item in items.as_array()? {
            if item["id"] == id {
                return Some(item.clone());
            }
            if let Some(found) = walk(&item["items"], id) {
                return Some(found);
            }
        }
        None
    }
    let menu = ask(session, "menu", json!({ "tab": "preview" }));
    walk(&menu["menus"], id)
}

#[test]
fn a_sliced_plate_goes_to_bambu_connect_as_the_reference_hands_it() {
    let dir = tempfile::tempdir().unwrap();
    FOLDER_IN_TEST.with(|folder| *folder.borrow_mut() = Some(dir.path().to_path_buf()));
    CONNECT_IN_TEST.with(|installed| installed.set(Installed::Yes));
    let session = Arc::new(Session::new(None));
    let id = on_a1_mini(&session);
    // A Bambu Lab printer: the entries are there, off until it is sliced.
    assert_eq!(
        ask(&session, "bambu_connect_send", json!({ "probe": true })),
        json!({ "probe": true, "bambu": true })
    );
    assert_eq!(
        menu_entry(&session, "export.bambu_connect").unwrap()["enabled"],
        false
    );
    assert_eq!(
        refused(&session, "bambu_connect_send", json!({})),
        "Slice the plate first, then send it to Bambu Connect."
    );
    assert_eq!(
        refused(&session, "bambu_connect_send", json!({ "all": true })),
        "Slice all plates first, then send them to Bambu Connect."
    );
    ask(&session, "slice", json!({ "project": id }));
    wait(&session, id);
    assert_eq!(
        menu_entry(&session, "export.bambu_connect").unwrap()["enabled"],
        true
    );
    // Not installed: said, and nothing written.
    CONNECT_IN_TEST.with(|installed| installed.set(Installed::No));
    let answer = ask(&session, "bambu_connect_send", json!({}));
    assert_eq!(answer["not_installed"], true);
    assert_eq!(
        answer["info_url"],
        "https://wiki.bambulab.com/en/software/bambu-connect"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // Installed (or not known): the file and its URL.
    CONNECT_IN_TEST.with(|installed| installed.set(Installed::Unknown));
    let answer = ask(&session, "bambu_connect_send", json!({}));
    assert_eq!(answer["name"], "twopart_groups");
    let path = dir.path().join("twopart_groups.gcode.3mf");
    assert_eq!(answer["path"], path.to_string_lossy().as_ref());
    assert_eq!(
        answer["url"],
        format!(
            "bambu-connect://import-file?path={}&name=twopart_groups&version=1.0.0",
            crate::outside::encode_uri_component(&path.to_string_lossy())
        )
    );
    let all = entries(&path);
    let names: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"Metadata/plate_1.gcode"), "{names:?}");
    // A sliced file keeps the tag it has always had
    // (`BambuStudio-<version>`), as "Export plate sliced file" does.
    assert!(
        text(&all, "3D/3dmodel.model")
            .contains(r#"<metadata name="Application">BambuStudio-2.4.9.0</metadata>"#),
        "{}",
        text(&all, "3D/3dmodel.model")
    );
    // The one difference: the printer's system preset named.
    let settings: Value =
        serde_json::from_str(text(&all, "Metadata/project_settings.config")).unwrap();
    assert_eq!(settings["print_compatible_printers"], json!([A1_MINI]));
    // "Send all plates": the one plate there is.
    let all_answer = ask(&session, "bambu_connect_send", json!({ "all": true }));
    assert_eq!(all_answer["path"], answer["path"]);
    FOLDER_IN_TEST.with(|folder| *folder.borrow_mut() = None);
    CONNECT_IN_TEST.with(|installed| installed.set(Installed::Yes));
}

#[test]
fn a_printer_not_bambu_labs_has_no_bambu_connect() {
    let session = Arc::new(Session::new(None));
    ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    );
    assert_eq!(
        ask(&session, "bambu_connect_send", json!({ "probe": true }))["bambu"],
        false
    );
    // The entries are not shown at all (`show_bambu_connect_menu_items`).
    assert!(menu_entry(&session, "export.bambu_connect").is_none());
    assert!(menu_entry(&session, "export.bambu_connect_all").is_none());
    assert_eq!(
        refused(&session, "bambu_connect_send", json!({})),
        "Send to Bambu Connect is for a Bambu Lab printer."
    );
}

#[test]
fn an_empty_project_is_refused() {
    let session = Arc::new(Session::new(None));
    assert_eq!(
        refused(&session, "bambu_connect_send", json!({})),
        "There are no objects in the project."
    );
}

#[test]
fn a_project_saved_on_other_presets_names_their_printers() {
    // The U1 project switched to an A1 mini and its process: "Save
    // Project" writes the process preset's `compatible_printers`, not the
    // U1 its file named (PresetBundle.cpp `full_fff_config` l.4456).
    let session = Arc::new(Session::new(None));
    let id = on_a1_mini(&session);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("saved.3mf");
    let answer = ask(
        &session,
        "project_save",
        json!({ "project": id, "path": path, "accept": true }),
    );
    assert_eq!(answer["saved"], true, "{answer}");
    let all = entries(&path);
    let settings: Value =
        serde_json::from_str(text(&all, "Metadata/project_settings.config")).unwrap();
    assert_eq!(settings["print_compatible_printers"], json!([A1_MINI]));
}
