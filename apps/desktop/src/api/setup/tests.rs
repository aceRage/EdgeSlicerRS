use std::path::{Path, PathBuf};

use super::*;
use std::sync::Arc;

pub(super) fn reference(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

/// A session that keeps its settings and presets in `dir`, on the
/// reference's profiles, with the Snapmaker U1 added and chosen.
pub(super) fn session(dir: &Path) -> Arc<Session> {
    let session = Arc::new(Session::with_settings(None, Some(dir.to_owned())));
    ask(
        &session,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    ask(
        &session,
        "printers_save",
        json!({ "models": [{ "vendor": "Snapmaker", "model": "Snapmaker U1" }] }),
    );
    session
}

pub(super) fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

pub(super) fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err(&format!("{name} was not refused"))
}

pub(super) fn user_dir(dir: &Path) -> PathBuf {
    dir.join("user").join("default")
}

pub(super) const U1: &str = "Snapmaker U1 (0.4 nozzle)";

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

/// The filament list of the U1 as the sidebar has it.
fn u1_filaments(session: &Arc<Session>) -> Vec<String> {
    strings(
        &ask(
            session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": U1 }),
        )["filaments"],
    )
}

#[test]
fn the_guides_filament_page_lists_the_printers_filaments() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let page = ask(&session, "filament_selection", Value::Null);
    // The printers ticked, with their nozzles.
    assert_eq!(page["printers"][0]["model"], "Snapmaker U1");
    assert!(strings(&page["printers"][0]["nozzles"]).contains(&"0.4".to_owned()));
    // PLA first among the types, Generic first among the vendors.
    assert_eq!(page["types"][0], "PLA");
    assert_eq!(page["vendors"][0], "Generic");
    let rows = page["filaments"].as_array().unwrap();
    assert!(rows.len() > 5, "{} filaments", rows.len());
    // One row for Generic PLA, for every nozzle of the U1.
    let pla = rows
        .iter()
        .find(|row| row["name"] == "Generic PLA" && row["vendor"] == "Generic")
        .expect("Generic PLA is listed");
    let members = strings(&pla["members"]);
    assert!(
        members.contains(&"Generic PLA @U1 0.4 nozzle".to_owned()),
        "{members:?}"
    );
    assert!(
        members.contains(&"Generic PLA @U1 0.2 nozzle".to_owned()),
        "{members:?}"
    );
    assert!(strings(&pla["models"]).contains(&"[Snapmaker U1++0.4]".to_owned()));
    // Nothing enabled yet; the U1's usual filaments are marked to tick.
    assert_eq!(pla["selected"], false);
    assert_eq!(pla["default"], true);
    // Generic rows come before the rest.
    let first_other = rows
        .iter()
        .position(|row| row["vendor"] != "Generic")
        .unwrap();
    assert!(
        rows[first_other..]
            .iter()
            .all(|row| row["vendor"] != "Generic")
    );
}

#[test]
fn a_filament_list_shows_only_the_filaments_ticked() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    // Without a selection, every filament made for the printer.
    let all = u1_filaments(&session);
    assert!(all.len() > 3, "{all:?}");
    // Nothing ticked is refused, nothing kept.
    let error = refused(&session, "filament_selection_save", json!({ "names": [] }));
    assert_eq!(error, "At least one filament must be selected.");
    // The guide's Finish keeps the presets ticked.
    let page = ask(&session, "filament_selection", Value::Null);
    let pla = page["filaments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "Generic PLA" && row["vendor"] == "Generic")
        .unwrap()
        .clone();
    ask(
        &session,
        "filament_selection_save",
        json!({ "names": pla["members"] }),
    );
    let shown = u1_filaments(&session);
    // The project's filament stays in the list while it is on it.
    let project = ask(&session, "project", Value::Null);
    let on = project["choices"]["filaments"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let mut wanted = vec!["Generic PLA @U1 0.4 nozzle".to_owned()];
    if !wanted.contains(&on) {
        wanted.push(on.clone());
    }
    let mut got = shown.clone();
    got.sort();
    wanted.sort();
    assert_eq!(got, wanted);
    // Kept as the reference keeps it, and read back in the next run.
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(written["filaments"]["Generic PLA @U1 0.4 nozzle"], "true");
    let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
    ask(
        &again,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    let page = ask(&again, "filament_selection", Value::Null);
    assert!(
        page["filaments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "Generic PLA" && row["selected"] == true)
    );
}

#[test]
fn a_project_on_a_filament_not_ticked_still_shows_it() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let all = u1_filaments(&session);
    let other = all
        .iter()
        .find(|name| !name.starts_with("Generic PLA"))
        .unwrap()
        .clone();
    ask(&session, "project_set", json!({ "filaments": [other] }));
    ask(
        &session,
        "filament_selection_save",
        json!({ "names": ["Generic PLA @U1 0.4 nozzle"] }),
    );
    let shown = u1_filaments(&session);
    assert!(shown.contains(&other), "{shown:?}");
    assert!(shown.contains(&"Generic PLA @U1 0.4 nozzle".to_owned()));
    // One neither ticked nor on a project is not listed.
    let third = all
        .iter()
        .find(|name| **name != other && !name.starts_with("Generic PLA"))
        .unwrap();
    assert!(!shown.contains(third), "{third} is listed");
}

#[test]
fn the_filament_manager_enables_and_disables_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let table = ask(&session, "filament_manager", Value::Null);
    let rows = table["rows"].as_array().unwrap();
    // Every row is enabled while nothing has been chosen.
    assert!(rows.iter().all(|row| row["enabled"] == true));
    let pla = rows
        .iter()
        .find(|row| row["key"] == "s|Generic|Generic PLA")
        .expect("a row for Generic PLA");
    assert!(strings(&pla["printers"]).contains(&U1.to_owned()));
    // The filament the project is on is in use: it cannot be unticked.
    let project = ask(&session, "project", Value::Null);
    let on = project["choices"]["filaments"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let used = rows
        .iter()
        .find(|row| strings(&row["members"]).contains(&on))
        .unwrap()
        .clone();
    assert_eq!(used["lock"], "in_use");
    // Unticking another row takes all its presets out of the lists.
    let other = rows
        .iter()
        .find(|row| {
            row["source"] == "System"
                && row["lock"] == ""
                && strings(&row["printers"]).contains(&U1.to_owned())
        })
        .unwrap()
        .clone();
    let after = ask(
        &session,
        "filament_enable",
        json!({ "changes": [
            { "key": other["key"], "on": false },
            { "key": used["key"], "on": false },
        ] }),
    );
    let row = |key: &Value| {
        after["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["key"] == *key)
            .unwrap()
            .clone()
    };
    assert_eq!(row(&other["key"])["enabled"], false);
    assert_eq!(row(&used["key"])["enabled"], true, "in use stays shown");
    let shown = u1_filaments(&session);
    for member in strings(&other["members"]) {
        assert!(!shown.contains(&member), "{member} is still listed");
    }
    assert!(shown.contains(&on));
    // Ticked again, it is back.
    ask(
        &session,
        "filament_enable",
        json!({ "changes": [{ "key": other["key"], "on": true }] }),
    );
    let shown = u1_filaments(&session);
    assert!(strings(&other["members"]).iter().any(|m| shown.contains(m)));
}

/// Writes one of the person's presets as the reference's writer lays it
/// out, standing alone (`base/`).
fn write_alone(dir: &Path, folder: &str, name: &str, body: Value) {
    let base = user_dir(dir).join(folder).join("base");
    std::fs::create_dir_all(&base).unwrap();
    let mut body = body;
    body["name"] = json!(name);
    body["from"] = json!("User");
    body["inherits"] = json!("");
    body["version"] = json!("2.3.0.0");
    std::fs::write(
        base.join(format!("{name}.json")),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
    std::fs::write(
        base.join(format!("{name}.info")),
        "sync_info = \nuser_id = \nsetting_id = \nbase_id = \nupdated_time = 0\n",
    )
    .unwrap();
}

#[test]
fn deleting_a_printer_of_ones_own_takes_only_its_own_presets() {
    let dir = tempfile::tempdir().unwrap();
    let printer = "Test Vendor Box 0.4 nozzle";
    write_alone(
        dir.path(),
        "machine",
        printer,
        json!({ "printer_settings_id": printer, "printer_model": "Test Vendor Box", "printer_variant": "0.4", "nozzle_diameter": ["0.4"] }),
    );
    let filament = format!("Generic PLA template @{printer}");
    let process = format!("process template @{printer}");
    write_alone(
        dir.path(),
        "filament",
        &filament,
        json!({ "filament_settings_id": [filament], "compatible_printers": [printer], "filament_type": ["PLA"] }),
    );
    write_alone(
        dir.path(),
        "process",
        &process,
        json!({ "print_settings_id": process, "compatible_printers": [printer] }),
    );
    // Made for this printer and another: not the printer's alone.
    write_alone(
        dir.path(),
        "filament",
        "Shared PLA",
        json!({ "filament_settings_id": ["Shared PLA"], "compatible_printers": [printer, U1], "filament_type": ["PLA"] }),
    );
    // Made for every printer.
    write_alone(
        dir.path(),
        "filament",
        "Any PLA",
        json!({ "filament_settings_id": ["Any PLA"], "compatible_printers": [], "filament_type": ["PLA"] }),
    );
    let session = session(dir.path());
    let before = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );

    // A system printer is never deleted.
    let error = refused(
        &session,
        "preset_delete",
        json!({ "kind": "printer", "preset": U1 }),
    );
    assert_eq!(error, "System presets cannot be deleted.");

    // The reference's question, with the presets that go with it.
    let asked = ask(
        &session,
        "preset_delete",
        json!({ "kind": "printer", "preset": printer }),
    );
    assert_eq!(asked["deleted"], false);
    assert_eq!(
        asked["ask"],
        "1 Filament Preset and 1 Process Preset is attached to this printer. Those presets would be deleted if the printer is deleted."
    );
    let mut attached = strings(&asked["attached"]);
    attached.sort();
    assert_eq!(attached, vec![filament.clone(), process.clone()]);
    // Nothing went yet.
    assert!(
        user_dir(dir.path())
            .join("machine/base")
            .join(format!("{printer}.json"))
            .is_file()
    );

    let done = ask(
        &session,
        "preset_delete",
        json!({ "kind": "printer", "preset": printer, "confirm": true }),
    );
    assert_eq!(done["deleted"], true);
    let gone = |folder: &str, name: &str| {
        let base = user_dir(dir.path()).join(folder).join("base");
        !base.join(format!("{name}.json")).exists() && !base.join(format!("{name}.info")).exists()
    };
    assert!(gone("machine", printer));
    assert!(gone("filament", &filament));
    assert!(gone("process", &process));
    // What is not the printer's alone stays, on disk and in the session.
    assert!(!gone("filament", "Shared PLA"));
    assert!(!gone("filament", "Any PLA"));
    let mine = ask(&session, "user_presets", Value::Null);
    let names: Vec<&str> = mine["filament"]
        .as_array()
        .unwrap()
        .iter()
        .map(|preset| preset["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Shared PLA") && names.contains(&"Any PLA"),
        "{names:?}"
    );
    assert!(mine["printer"].as_array().unwrap().is_empty());
    // The vendor's presets are untouched.
    let after = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(before["filaments"], after["filaments"]);
    assert_eq!(before["processes"], after["processes"]);
}

#[test]
fn a_project_on_a_deleted_printer_of_ones_own_goes_to_its_system_printer() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "printable_height", "value": 250 }),
    );
    let saved = ask(
        &session,
        "preset_save",
        json!({ "kind": "printer", "name": "My U1" }),
    );
    assert_eq!(saved["choices"]["machine"], "My U1");
    // Laid over the U1: the usual question, nothing attached.
    let asked = ask(
        &session,
        "preset_delete",
        json!({ "kind": "printer", "preset": "My U1" }),
    );
    assert_eq!(
        asked["ask"],
        "Are you sure you want to Delete the selected preset?\n\nUsed by:\n - Printer"
    );
    let done = ask(
        &session,
        "preset_delete",
        json!({ "kind": "printer", "preset": "My U1", "confirm": true }),
    );
    assert_eq!(done["choices"]["machine"], U1);
}
