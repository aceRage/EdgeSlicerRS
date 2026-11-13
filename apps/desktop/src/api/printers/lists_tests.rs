//! The filament colours and flushing volumes kept with each printer
//! (`orca_presets`: PresetBundle.cpp `export_selections`, `load_selections`).

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use crate::api::Session;

fn reference(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn session(dir: &Path) -> Arc<Session> {
    let session = Arc::new(Session::with_settings(None, Some(dir.to_owned())));
    ask(
        &session,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    session
}

fn add_printers(session: &Arc<Session>) {
    ask(
        session,
        "printers_save",
        json!({ "models": [
            { "vendor": "Snapmaker", "model": "Snapmaker U1" },
            { "vendor": "Snapmaker", "model": "Snapmaker J1" },
        ] }),
    );
}

/// The project's colours and flushing matrix.
fn lists(session: &Arc<Session>, id: u64) -> (Value, Value) {
    let flush = ask(session, "flush_volumes", json!({ "project": id }));
    (flush["colours"].clone(), flush["matrix"].clone())
}

/// A project on the U1 with two filaments, coloured and flushed as asked.
fn coloured(session: &Arc<Session>, colours: [&str; 2], volume: f64) -> u64 {
    let id = ask(session, "project_new", json!({}))["id"]
        .as_u64()
        .unwrap();
    ask(
        session,
        "printer_select",
        json!({ "project": id, "vendor": "Snapmaker", "model": "Snapmaker U1" }),
    );
    ask(
        session,
        "filament_edit",
        json!({ "project": id, "op": "add", "colour": colours[1] }),
    );
    ask(
        session,
        "filament_edit",
        json!({ "project": id, "op": "colour", "filament": 1, "colour": colours[0] }),
    );
    ask(
        session,
        "filament_edit",
        json!({ "project": id, "op": "flush", "matrix": [0.0, volume, volume, 0.0], "multiplier": 1.0 }),
    );
    id
}

fn kept(dir: &Path, machine: &str) -> Value {
    let text = std::fs::read_to_string(dir.join("settings.json")).unwrap();
    let settings: Value = serde_json::from_str(&text).unwrap();
    settings["printers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["machine"] == machine)
        .cloned()
        .unwrap_or(Value::Null)
}

const U1: &str = "Snapmaker U1 (0.4 nozzle)";

#[test]
fn colours_and_flushing_volumes_are_kept_with_the_printer_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    {
        let session = session(dir.path());
        add_printers(&session);
        coloured(&session, ["#123456", "#ABCDEF"], 321.0);
    }
    // As the reference writes them.
    let entry = kept(dir.path(), U1);
    assert_eq!(entry["filament_colors"], "#123456,#ABCDEF", "{entry}");
    assert_eq!(entry["filament_multi_colors"], "#123456,#ABCDEF");
    assert_eq!(entry["filament_colour_mode"], "0,0");
    assert_eq!(
        entry["flush_volumes_matrix"],
        "0.000000|321.000000|321.000000|0.000000"
    );
    // Another run: a new project that gets the U1 takes them.
    let session = session(dir.path());
    let id = ask(&session, "project", Value::Null)["id"]
        .as_u64()
        .unwrap();
    ask(
        &session,
        "printer_select",
        json!({ "project": id, "vendor": "Snapmaker", "model": "Snapmaker U1", "initial": true }),
    );
    let (colours, _) = lists(&session, id);
    assert_eq!(colours[0], "#123456");
    // Nothing of it is counted as a change, and the project is still the
    // empty one a file opened takes the place of.
    let project = ask(&session, "projects", Value::Null);
    assert_eq!(project["projects"][0]["dirty"], false, "{project}");
    let opened = ask(
        &session,
        "project_open",
        json!({ "path": reference("resources/handy_models/OrcaToleranceTest.stl") }),
    );
    assert_eq!(opened["id"].as_u64().unwrap(), id);
    let (colours, _) = lists(&session, id);
    assert_eq!(colours[0], "#123456");
}

#[test]
fn a_new_project_on_a_printer_takes_its_lists_and_two_printers_keep_theirs_apart() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    add_printers(&session);
    let first = coloured(&session, ["#111111", "#222222"], 111.0);
    // The J1 gets other ones.
    let other = ask(&session, "project_new", json!({}))["id"]
        .as_u64()
        .unwrap();
    ask(
        &session,
        "printer_select",
        json!({ "project": other, "vendor": "Snapmaker", "model": "Snapmaker J1" }),
    );
    ask(
        &session,
        "filament_edit",
        json!({ "project": other, "op": "colour", "filament": 1, "colour": "#333333" }),
    );
    let j1 = ask(&session, "project", json!({ "project": other }))["choices"]["machine"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(kept(dir.path(), &j1)["filament_colors"], "#333333");
    assert_eq!(kept(dir.path(), U1)["filament_colors"], "#111111,#222222");
    // A new project started from the U1 one takes the U1's.
    ask(&session, "project_select", json!({ "id": first }));
    let fresh = ask(&session, "project_new", json!({}))["id"]
        .as_u64()
        .unwrap();
    let (colours, matrix) = lists(&session, fresh);
    assert_eq!(colours[0], "#111111", "{colours}");
    let _ = matrix;
}

#[test]
fn a_printer_switch_keeps_the_projects_own_and_writes_them_for_the_new_printer() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    add_printers(&session);
    // The J1 has its own kept.
    let other = ask(&session, "project_new", json!({}))["id"]
        .as_u64()
        .unwrap();
    ask(
        &session,
        "printer_select",
        json!({ "project": other, "vendor": "Snapmaker", "model": "Snapmaker J1" }),
    );
    ask(
        &session,
        "filament_edit",
        json!({ "project": other, "op": "colour", "filament": 1, "colour": "#333333" }),
    );
    let id = coloured(&session, ["#444444", "#555555"], 222.0);
    let before = lists(&session, id);
    ask(
        &session,
        "printer_select",
        json!({ "project": id, "vendor": "Snapmaker", "model": "Snapmaker J1" }),
    );
    // Its own, not the J1's.
    assert_eq!(lists(&session, id), before);
    let j1 = ask(&session, "project", json!({ "project": id }))["choices"]["machine"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(kept(dir.path(), &j1)["filament_colors"], "#444444,#555555");
}
