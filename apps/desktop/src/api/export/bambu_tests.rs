//! "Export Bambu 3MF" and "Export && Open in Bambu Studio" through the
//! session. Nothing is started: the session answers how Bambu Studio
//! would be.

use std::{io::Read, path::Path, sync::Arc};

use serde_json::{Value, json};

use super::super::Session;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";

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

fn menu_entry(session: &Arc<Session>, id: &str) -> Value {
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
    let menu = ask(session, "menu", json!({ "tab": "prepare" }));
    walk(&menu["menus"], id).unwrap()
}

#[test]
fn a_project_is_exported_for_bambu_studio_as_the_reference_exports_it() {
    let session = Arc::new(Session::new(None));
    // Off with nothing to export, on with objects (`can_export_model`),
    // and no longer waiting for anything.
    assert_eq!(menu_entry(&session, "export.bambu_3mf")["enabled"], false);
    let id = ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    )["id"]
        .as_u64()
        .unwrap();
    for entry in ["export.bambu_3mf", "export.bambu_open"] {
        let item = menu_entry(&session, entry);
        assert_eq!(item["enabled"], true, "{item}");
        assert!(
            !item["tip"].as_str().unwrap_or("").starts_with("not yet"),
            "{item}"
        );
    }
    assert_eq!(
        menu_entry(&session, "export.bambu_3mf")["label"],
        "Export Bambu 3MF..."
    );
    assert_eq!(
        menu_entry(&session, "export.bambu_open")["label"],
        "Export & Open in Bambu Studio..."
    );
    let suggested = ask(&session, "export_bambu", json!({ "suggest": true }));
    assert!(
        suggested["suggested"]
            .as_str()
            .unwrap()
            .ends_with("twopart_groups.3mf")
    );
    let dir = tempfile::tempdir().unwrap();
    let answer = ask(
        &session,
        "export_bambu",
        json!({ "project": id, "path": dir.path().join("for bambu") }),
    );
    let path = dir.path().join("for bambu.3mf");
    assert_eq!(answer["path"], path.to_string_lossy().as_ref());
    // The reference's words, with how many were left out.
    let dropped = answer["dropped"].as_array().unwrap().len();
    assert!(dropped > 0);
    assert_eq!(
        answer["said"],
        format!(
            "Exported for Bambu Studio: {dropped} settings not supported by Bambu Studio were left out."
        )
    );
    assert!(answer.get("open").is_none());
    let all = entries(&path);
    assert!(
        text(&all, "3D/3dmodel.model")
            .contains(r#"<metadata name="Application">BambuStudio-02.08.00.00</metadata>"#)
    );
    let settings: Value =
        serde_json::from_str(text(&all, "Metadata/project_settings.config")).unwrap();
    assert_eq!(settings["version"], "02.08.00.00");
    for dropped in answer["dropped"].as_array().unwrap() {
        let key = dropped.as_str().unwrap();
        assert!(settings.get(key).is_none(), "{key}");
    }
    // Every key written is one of Bambu's.
    for key in settings.as_object().unwrap().keys() {
        if !["name", "from", "version"].contains(&key.as_str()) {
            assert!(
                edge_config::bambu::find_key(key).is_some(),
                "{key} is not Bambu's"
            );
        }
    }
    // Asked about when it is there.
    let again = ask(&session, "export_bambu", json!({ "path": path }));
    assert_eq!(again["exists"], true);

    // "Export && Open": the same file, and how Bambu Studio would be
    // started with it on this system; nothing is started here.
    let answer = ask(
        &session,
        "export_bambu",
        json!({ "path": path, "replace": true, "open": true }),
    );
    let shown = path.to_string_lossy().into_owned();
    let open: Vec<Vec<String>> = serde_json::from_value(answer["open"].clone()).unwrap();
    if cfg!(target_os = "macos") {
        assert_eq!(
            open,
            [
                vec![
                    "open".to_owned(),
                    "-a".into(),
                    "BambuStudio".into(),
                    shown.clone()
                ],
                vec![
                    "open".to_owned(),
                    "-b".into(),
                    "com.bambulab.bambu-studio".into(),
                    shown
                ],
            ]
        );
    } else {
        assert!(open.iter().all(|argv| argv.last() == Some(&shown)));
    }
}

#[test]
fn an_empty_project_has_nothing_to_export_for_bambu() {
    let session = Arc::new(Session::new(None));
    assert_eq!(
        session
            .call("export_bambu", &json!({ "path": "/nowhere/x.3mf" }))
            .unwrap_err(),
        "There are no objects in the project."
    );
}
