//! Preferences: what is kept, what each setting does in the session.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use super::*;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";

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

fn kept_in(dir: &Path) -> Arc<Session> {
    Arc::new(Session::with_settings(None, Some(dir.to_owned())))
}

#[test]
fn the_dialog_is_described_with_the_references_defaults() {
    let session = Arc::new(Session::new(None));
    let prefs = ask(&session, "preferences", Value::Null);
    let pages: Vec<&str> = prefs["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["title"].as_str().unwrap())
        .collect();
    assert_eq!(pages, ["General", "Extras"]);
    let values = &prefs["values"];
    // AppConfig.cpp `set_defaults`.
    assert_eq!(values["use_inches"], "0");
    assert_eq!(values["zoom_to_mouse"], "false");
    assert_eq!(values["remember_printer_config"], "true");
    assert_eq!(values["max_recent_count"], "18");
    assert_eq!(values["backup_switch"], "true");
    assert_eq!(values["backup_interval"], "10");
    assert_eq!(values["autosave_switch"], "false");
    assert_eq!(values["autosave_interval"], "300");
    assert_eq!(values["preview_default_view_type"], "auto");
    assert_eq!(values["camera_orbit_mult"], "1.0");
    // Every word the page puts through `t` is a message id of the
    // reference's catalog, or listed as newer than it (wording.txt).
    let ids = crate::api::i18n::tests::message_ids();
    let (newer, _) = crate::api::i18n::tests::listed();
    let known =
        |text: &str| text.is_empty() || ids.contains(text) || newer.iter().any(|one| one == text);
    let mut unknown = Vec::new();
    for page in prefs["pages"].as_array().unwrap() {
        let mut words = vec![page["title"].as_str().unwrap()];
        for group in page["groups"].as_array().unwrap() {
            words.push(group["title"].as_str().unwrap());
            for item in group["items"].as_array().unwrap() {
                words.extend(
                    ["label", "tip", "unit"]
                        .iter()
                        .filter_map(|key| item[*key].as_str()),
                );
                for option in item["options"].as_array().into_iter().flatten() {
                    words.push(option["label"].as_str().unwrap());
                }
            }
        }
        unknown.extend(
            words
                .into_iter()
                .filter(|word| !known(word))
                .map(str::to_owned),
        );
    }
    assert!(
        unknown.is_empty(),
        "not message ids nor listed: {unknown:?}"
    );
}

#[test]
fn a_setting_is_checked_kept_and_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let session = kept_in(dir.path());
    let set = |key: &str, value: Value| {
        session.call("preference_set", &json!({ "key": key, "value": value }))
    };
    assert_eq!(
        set("use_inches", json!("1")).unwrap()["values"]["use_inches"],
        "1"
    );
    assert!(set("use_inches", json!("2")).is_err());
    assert!(set("no_such", json!("1")).is_err());
    assert_eq!(
        set("zoom_to_mouse", json!(true)).unwrap()["values"]["zoom_to_mouse"],
        "true"
    );
    // Put within its range, as the reference's fields put it.
    assert_eq!(
        set("camera_orbit_mult", json!("5")).unwrap()["values"]["camera_orbit_mult"],
        "2"
    );
    assert!(set("backup_interval", json!("2.5")).is_err());
    assert!(set("backup_interval", json!("ten")).is_err());
    assert_eq!(
        set("print_action_last", json!("sliced")).unwrap()["values"]["print_action_last"],
        "sliced"
    );
    // Kept in the app's own settings, in the `app` section.
    let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["app"]["use_inches"], "1");
    assert_eq!(file["app"]["camera_orbit_mult"], "2");
    let again = kept_in(dir.path());
    assert_eq!(
        ask(&again, "preferences", Value::Null)["values"]["zoom_to_mouse"],
        "true"
    );
}

#[test]
fn recent_files_follow_their_preferences() {
    let dir = tempfile::tempdir().unwrap();
    let session = kept_in(dir.path());
    let bare = reference(BARE);
    ask(&session, "project_open", json!({ "path": bare }));
    let menu = ask(&session, "menu", Value::Null);
    assert_eq!(menu["recent"], json!([]), "a model is not a recent project");
    ask(
        &session,
        "preference_set",
        json!({ "key": "recent_models", "value": "true" }),
    );
    // The next run, with the setting kept.
    let session = kept_in(dir.path());
    ask(&session, "project_open", json!({ "path": bare }));
    ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    );
    let menu = ask(&session, "menu", Value::Null);
    assert_eq!(menu["recent"].as_array().unwrap().len(), 2);
    // Fewer kept: cut at once, and kept so.
    ask(
        &session,
        "preference_set",
        json!({ "key": "max_recent_count", "value": "1" }),
    );
    let menu = ask(&session, "menu", Value::Null);
    let recent = menu["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 1);
    assert!(
        recent[0]["path"]
            .as_str()
            .unwrap()
            .ends_with("twopart_groups.3mf")
    );
}

#[test]
fn a_backup_is_written_offered_and_restored() {
    let dir = tempfile::tempdir().unwrap();
    let session = kept_in(dir.path());
    let id = ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    )["id"]
        .as_u64()
        .unwrap();
    // Nothing unsaved: nothing backed up.
    assert_eq!(
        ask(&session, "backup_write", Value::Null)["written"],
        json!([])
    );
    ask(
        &session,
        "project_set",
        json!({ "project": id, "layer_height": 0.3 }),
    );
    assert_eq!(
        ask(&session, "backup_write", Value::Null)["written"],
        json!([id])
    );
    // The next run finds it.
    let next = kept_in(dir.path());
    let backups = ask(&next, "backup_list", Value::Null);
    assert_eq!(backups["backups"].as_array().unwrap().len(), 1);
    assert_eq!(backups["backups"][0]["name"], "twopart_groups");
    let restored = ask(&next, "backup_restore", Value::Null);
    let id = restored["restored"][0].as_u64().unwrap();
    let project = ask(&next, "project", json!({ "project": id }));
    assert_eq!(project["name"], "twopart_groups");
    assert_eq!(project["dirty"], true);
    assert_eq!(ask(&next, "backup_list", Value::Null)["backups"], json!([]));
    // This run's own backups are not offered back to it.
    ask(
        &next,
        "project_set",
        json!({ "project": id, "layer_height": 0.25 }),
    );
    assert_eq!(
        ask(&next, "backup_write", Value::Null)["written"],
        json!([id])
    );
    assert_eq!(ask(&next, "backup_list", Value::Null)["backups"], json!([]));
    // Off: nothing written.
    ask(
        &next,
        "preference_set",
        json!({ "key": "backup_switch", "value": "false" }),
    );
    assert_eq!(
        ask(&next, "backup_write", Value::Null)["written"],
        json!([])
    );
}

#[test]
fn only_a_project_saved_once_is_auto_saved() {
    let dir = tempfile::tempdir().unwrap();
    let session = kept_in(dir.path());
    let copy = dir.path().join("mine.3mf");
    std::fs::copy(reference(SMALL), &copy).unwrap();
    let saved = ask(&session, "project_open", json!({ "path": copy }))["id"]
        .as_u64()
        .unwrap();
    ask(&session, "project_new", Value::Null);
    let bare = ask(&session, "project_open", json!({ "path": reference(BARE) }))["id"]
        .as_u64()
        .unwrap();
    for id in [saved, bare] {
        ask(
            &session,
            "project_set",
            json!({ "project": id, "layer_height": 0.3 }),
        );
    }
    assert_eq!(ask(&session, "autosave_due", Value::Null)["due"], json!([]));
    ask(
        &session,
        "preference_set",
        json!({ "key": "autosave_switch", "value": "true" }),
    );
    assert_eq!(
        ask(&session, "autosave_due", Value::Null)["due"],
        json!([saved])
    );
}
