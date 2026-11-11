//! Saving a project through the session: where it goes, what is asked
//! first, what the file holds, and that the project is saved afterwards.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use super::super::Session;

const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
const UNTRUSTED: &str = "tests/data/untrusted_3mf/post_process_project.3mf";
const MACHINE: &str = "Snapmaker U1 (0.4 nozzle)";

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

/// Opens a copy of a reference file kept in `dir`, so that saving over
/// it writes nothing anywhere else.
fn open_copy(session: &Arc<Session>, dir: &Path, path: &str) -> (u64, std::path::PathBuf) {
    let from = reference(path);
    let copy = dir.join(from.file_name().unwrap());
    std::fs::copy(&from, &copy).unwrap();
    let id = ask(session, "project_open", json!({ "path": copy }))["id"]
        .as_u64()
        .unwrap();
    (id, copy)
}

fn tile(session: &Arc<Session>, id: u64) -> Value {
    ask(session, "projects", Value::Null)["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tile| tile["id"] == id)
        .cloned()
        .unwrap()
}

fn choose_presets(session: &Arc<Session>, project: u64) {
    let presets = ask(
        session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": MACHINE }),
    );
    ask(
        session,
        "project_set",
        json!({
            "project": project, "source": "presets", "vendor": "Snapmaker",
            "machine": MACHINE, "process": presets["processes"][0],
            "filaments": [presets["filaments"][0]], "initial": true,
        }),
    );
}

#[test]
fn an_edited_project_is_saved_over_its_file_and_is_no_longer_unsaved() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    let (id, path) = open_copy(&session, dir.path(), THREE_PLATES);
    let before = std::fs::read(&path).unwrap();
    let scene = ask(&session, "scene", json!({ "project": id }));
    let object = scene["objects"][0]["id"].clone();
    ask(
        &session,
        "scene_edit",
        json!({ "op": "rename", "object": object, "name": "Saved arm" }),
    );
    assert_eq!(tile(&session, id)["dirty"], true);
    let unsaved = ask(&session, "unsaved", Value::Null);
    assert_eq!(unsaved["unsaved"][0]["id"], id);

    let answer = ask(&session, "project_save", json!({ "project": id }));
    assert_eq!(answer["saved"], true, "{answer}");
    assert_eq!(tile(&session, id)["dirty"], false);
    assert_eq!(ask(&session, "unsaved", Value::Null)["unsaved"], json!([]));
    // Written over the file it was opened from, which holds the edit.
    assert_ne!(std::fs::read(&path).unwrap(), before);
    let model = edge_engine::Model::open(&path).unwrap();
    assert!(
        model
            .objects()
            .iter()
            .any(|object| object.name() == "Saved arm")
    );
    assert_eq!(model.objects().len(), 16);
    // Undone, it is unsaved again.
    ask(&session, "undo", json!({ "project": id }));
    assert_eq!(tile(&session, id)["dirty"], true);
}

#[test]
fn a_model_without_a_project_file_asks_where_and_is_saved_as_a_3mf() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    let id = ask(&session, "project_open", json!({ "path": reference(BARE) }))["id"]
        .as_u64()
        .unwrap();
    choose_presets(&session, id);
    ask(
        &session,
        "process_set",
        json!({ "project": id, "key": "wall_loops", "value": "5" }),
    );
    let answer = ask(&session, "project_save", json!({ "project": id }));
    assert_eq!(answer["saved"], false);
    assert_eq!(answer["ask"], "path");
    // Beside the model, as a project.
    let suggested = answer["suggested"].as_str().unwrap();
    assert!(suggested.ends_with("OrcaToleranceTest.3mf"), "{suggested}");
    assert!(Path::new(suggested).is_absolute());
    let only = ask(
        &session,
        "project_save",
        json!({ "project": id, "suggest": true }),
    );
    assert_eq!(only["suggested"], suggested);

    // A name typed without its extension is given it.
    let to = dir.path().join("tolerance");
    let answer = ask(
        &session,
        "project_save",
        json!({ "project": id, "path": to }),
    );
    assert_eq!(answer["saved"], true, "{answer}");
    let file = dir.path().join("tolerance.3mf");
    assert!(file.is_file());
    let shown = tile(&session, id);
    assert_eq!(shown["name"], "tolerance");
    assert_eq!(shown["dirty"], false);
    assert!(shown["path"].as_str().unwrap().ends_with("tolerance.3mf"));

    // The file holds the presets chosen and what was set over them.
    let model = edge_engine::Model::open(&file).unwrap();
    let settings = model.settings().unwrap();
    assert_eq!(settings.text("printer_settings_id"), Some(MACHINE));
    assert_eq!(settings.text("wall_loops"), Some("5"));
    let groups = settings.texts("different_settings_to_system").unwrap();
    assert_eq!(groups[0], "wall_loops");
    assert!(settings.get("print_host").is_none());
    assert_eq!(model.plates().len(), 1);
    // Saved again, it goes to the same file without asking.
    let again = ask(&session, "project_save", json!({ "project": id }));
    assert_eq!(again["saved"], true);
    assert_eq!(again["path"], shown["path"]);
}

#[test]
fn what_would_not_be_kept_is_asked_about_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    let (id, _) = open_copy(&session, dir.path(), UNTRUSTED);
    let to = dir.path().join("cleaned.3mf");
    let answer = ask(
        &session,
        "project_save",
        json!({ "project": id, "path": to }),
    );
    assert_eq!(answer["ask"], "losses", "{answer}");
    assert!(
        answer["losses"][0]
            .as_str()
            .unwrap()
            .contains("reaches outside the file")
    );
    assert!(!to.exists());
    let answer = ask(
        &session,
        "project_save",
        json!({ "project": id, "path": to, "accept": true }),
    );
    assert_eq!(answer["saved"], true);
    let model = edge_engine::Model::open(&to).unwrap();
    // The printer's address the file named is not saved.
    assert!(model.settings().unwrap().get("print_host").is_none());
}

#[test]
fn an_empty_project_is_saved_on_the_printer_it_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    let id = ask(&session, "projects", Value::Null)["active"]
        .as_u64()
        .unwrap();
    choose_presets(&session, id);
    let to = dir.path().join("empty.3mf");
    let answer = ask(
        &session,
        "project_save",
        json!({ "project": id, "path": to }),
    );
    assert_eq!(answer["saved"], true, "{answer}");
    let model = edge_engine::Model::open(&to).unwrap();
    assert!(model.objects().is_empty());
    assert_eq!(
        model.settings().unwrap().text("printer_settings_id"),
        Some(MACHINE)
    );
}
