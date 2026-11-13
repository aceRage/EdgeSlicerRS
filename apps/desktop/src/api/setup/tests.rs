use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::api::i18n::{fill, tr};
use edge_config::Kind;
use md5::Digest;
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
        .find(|row| row["key"] == "s|Snapmaker|Generic PLA")
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

/// A cube of 20 mm as an ASCII STL.
fn cube(dir: &Path) -> PathBuf {
    let corners = |x: f64, y: f64, z: f64| format!("vertex {x} {y} {z}");
    let s = 20.0;
    let faces: [[(f64, f64, f64); 3]; 12] = [
        [(0., 0., 0.), (s, s, 0.), (s, 0., 0.)],
        [(0., 0., 0.), (0., s, 0.), (s, s, 0.)],
        [(0., 0., s), (s, 0., s), (s, s, s)],
        [(0., 0., s), (s, s, s), (0., s, s)],
        [(0., 0., 0.), (s, 0., 0.), (s, 0., s)],
        [(0., 0., 0.), (s, 0., s), (0., 0., s)],
        [(0., s, 0.), (s, s, s), (s, s, 0.)],
        [(0., s, 0.), (0., s, s), (s, s, s)],
        [(0., 0., 0.), (0., s, s), (0., s, 0.)],
        [(0., 0., 0.), (0., 0., s), (0., s, s)],
        [(s, 0., 0.), (s, s, 0.), (s, s, s)],
        [(s, 0., 0.), (s, s, s), (s, 0., s)],
    ];
    let mut text = String::from("solid cube\n");
    for face in faces {
        text.push_str("facet normal 0 0 0\nouter loop\n");
        for (x, y, z) in face {
            text.push_str(&corners(x, y, z));
            text.push('\n');
        }
        text.push_str("endloop\nendfacet\n");
    }
    text.push_str("endsolid cube\n");
    let path = dir.join("cube.stl");
    std::fs::write(&path, text).unwrap();
    path
}

/// The first page of "Create Printer/Nozzle" as the dialog starts it,
/// with a vendor and model of the reference's list.
fn printer_form() -> Value {
    json!({
        "type": "printer",
        "custom": false,
        "vendor": "Snapmaker",
        "model": "Snapmaker U1",
        "nozzle": "0.4 mm",
        "bed": ["220", "230"],
        "origin": ["0", "0"],
        "height": "240",
    })
}

fn with(mut base: Value, more: Value) -> Value {
    for (key, value) in more.as_object().unwrap() {
        base[key.as_str()] = value.clone();
    }
    base
}

#[test]
fn every_check_of_create_printer_says_the_references_words() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let check = |args: Value| refused(&session, "create_printer_check", args);
    assert_eq!(
        check(with(printer_form(), json!({ "model": "" }))),
        tr("You have not selected the vendor and model or entered the custom vendor and model.")
    );
    assert_eq!(
        check(with(
            printer_form(),
            json!({ "custom": true, "vendor": "@;", "model": "Box" })
        )),
        tr(
            "There may be escape characters in the custom printer vendor or model. Please delete and re-enter."
        )
    );
    assert_eq!(
        check(with(
            printer_form(),
            json!({ "custom": true, "vendor": "   ", "model": "Box" })
        )),
        tr("All inputs in the custom printer vendor or model are spaces. Please re-enter.")
    );
    assert_eq!(
        check(with(printer_form(), json!({ "bed": ["0", "230"] }))),
        tr("Please check bed printable shape and origin input.")
    );
    assert_eq!(
        check(with(printer_form(), json!({ "origin": ["220", "0"] }))),
        tr("Please check bed printable shape and origin input.")
    );
    assert_eq!(
        check(json!({ "type": "nozzle", "nozzle": "0.6 mm" })),
        tr("You have not yet selected the printer to replace the nozzle, please choose.")
    );
    assert_eq!(
        ask(&session, "create_printer_check", printer_form())["ok"],
        true
    );

    // The second page.
    let create = |args: Value| refused(&session, "create_printer", args);
    assert_eq!(
        create(printer_form()),
        tr(
            "You have not yet chosen which printer preset to create based on. Please choose the vendor and model of the printer"
        )
    );
    let page2 = with(
        printer_form(),
        json!({ "preset_vendor": "Snapmaker", "preset_model": "Snapmaker U1 @ 0.4 nozzle", "template": true }),
    );
    assert_eq!(
        create(with(
            page2.clone(),
            json!({ "processes": ["process template"] })
        )),
        tr("You need to select at least one filament preset.")
    );
    assert_eq!(
        create(with(
            page2.clone(),
            json!({ "filaments": ["Generic PLA template"] })
        )),
        tr("You need to select at least one process preset.")
    );
    // "Printer Preset": a model's nozzle that is not there.
    assert_eq!(
        create(with(
            page2.clone(),
            json!({ "preset_model": "Snapmaker U1 @ 0.3 nozzle" })
        )),
        tr("The printer preset was not found, please reselect.")
    );
    assert_eq!(
        create(with(page2, json!({ "preset_model": "Snapmaker U1" }))),
        tr("The nozzle diameter was not found, please reselect.")
    );
    // The bed's files.
    let bad = dir.path().join("bed.obj");
    std::fs::write(&bad, "o").unwrap();
    assert_eq!(
        refused(
            &session,
            "create_printer_bed_file",
            json!({ "kind": "model", "path": bad.to_string_lossy() }),
        ),
        tr("Invalid file format.")
    );
    let big = dir.path().join("big.svg");
    std::fs::write(&big, vec![b' '; 3 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        refused(
            &session,
            "create_printer_bed_file",
            json!({ "kind": "texture", "path": big.to_string_lossy() }),
        ),
        fill(&tr("The file exceeds %d MB, please import again."), &[&3])
    );
    // The models of a vendor, the nearest nozzle first.
    let models = ask(
        &session,
        "create_printer_models",
        json!({ "vendor": "Snapmaker", "nozzle": "0.6 mm" }),
    );
    let first = models["models"][0].as_str().unwrap();
    assert!(first.ends_with("@ 0.6 nozzle"), "{first}");
}

/// The keys the reference's writer puts in a preset of a kind that stands
/// alone: its collection's options (Preset.cpp `s_Preset_print_options`,
/// `s_Preset_filament_options`, `s_Preset_printer_options` with
/// `s_Preset_machine_limits_options` and PrintConfig.cpp
/// `m_extruder_option_keys`), the ids PresetBundle.cpp adds to each
/// collection's default preset, and the header `save_to_json` writes, read
/// from the pinned source.
fn reference_keys(kind: Kind) -> BTreeSet<String> {
    let preset = std::fs::read_to_string(reference("src/libslic3r/Preset.cpp")).unwrap();
    let config = std::fs::read_to_string(reference("src/libslic3r/PrintConfig.cpp")).unwrap();
    let list = |source: &str, start: &str| -> Vec<String> {
        let at = source.find(start).unwrap_or_else(|| panic!("{start}"));
        let end = at + source[at..].find("};").unwrap();
        source[at..end]
            .lines()
            .map(|line| line.split("//").next().unwrap())
            .flat_map(|line| {
                line.split('"')
                    .enumerate()
                    .filter(|(i, _)| i % 2 == 1)
                    .map(|(_, key)| key.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let mut keys: BTreeSet<String> = match kind {
        Kind::Process => list(
            &preset,
            "static std::vector<std::string> s_Preset_print_options",
        )
        .into_iter()
        .chain(["print_settings_id".to_owned()])
        .collect(),
        Kind::Filament => list(
            &preset,
            "static std::vector<std::string> s_Preset_filament_options",
        )
        .into_iter()
        .chain(["filament_settings_id".to_owned(), "filament_id".to_owned()])
        .collect(),
        _ => list(
            &preset,
            "static std::vector<std::string> s_Preset_printer_options",
        )
        .into_iter()
        .chain(list(
            &preset,
            "static std::vector<std::string> s_Preset_machine_limits_options",
        ))
        .chain(list(&config, "m_extruder_option_keys = {"))
        .chain(
            [
                "printer_settings_id",
                "printer_model",
                "printer_variant",
                "thumbnails",
                "default_print_profile",
                "default_filament_profile",
            ]
            .map(str::to_owned),
        )
        .collect(),
    };
    keys.extend(["name", "from", "version", "is_custom_defined", "inherits"].map(str::to_owned));
    keys
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn a_printer_is_created_from_a_template_written_as_the_reference_writes_and_sliced() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let model = cube(dir.path());
    let id = ask(
        &session,
        "project_open",
        json!({ "path": model.to_string_lossy() }),
    )["id"]
        .clone();
    ask(
        &session,
        "printer_select",
        json!({ "project": id, "initial": true }),
    );
    // The second page's templates.
    let offered = ask(
        &session,
        "create_printer_presets",
        json!({ "vendor": "Snapmaker", "model": "Snapmaker U1 @ 0.4 nozzle", "template": true }),
    );
    let filaments = strings(&offered["filaments"]);
    let processes = strings(&offered["processes"]);
    assert!(
        filaments.contains(&"Generic PLA template".to_owned()),
        "{filaments:?}"
    );
    assert_eq!(processes, vec!["process template".to_owned()]);
    // A bed model of one's own, kept in the app's data folder.
    let picked = dir.path().join("picked");
    std::fs::create_dir_all(&picked).unwrap();
    let bed_stl = picked.join("my bed.stl");
    std::fs::copy(&model, &bed_stl).unwrap();
    let args = with(
        printer_form(),
        json!({
            "project": id,
            "custom": true,
            "vendor": "Test Vendor",
            "model": "Box",
            "origin": ["10", "5"],
            "bed_model": bed_stl.to_string_lossy(),
            "preset_vendor": "Snapmaker",
            "preset_model": "Snapmaker U1 @ 0.4 nozzle",
            "template": true,
            "filaments": ["Generic PLA template"],
            "processes": ["process template"],
        }),
    );
    let created = ask(&session, "create_printer", args.clone());
    let printer = "Test Vendor Box 0.4 nozzle";
    assert_eq!(created["created"], true);
    assert_eq!(created["printer"], printer);
    assert_eq!(created["choices"]["machine"], printer);
    let filament = format!("Generic PLA template @{printer}");
    let process = format!("process template @{printer}");
    assert_eq!(created["choices"]["process"], process);
    assert_eq!(created["choices"]["filaments"][0], filament);

    // What was written, read back from the disk.
    let base = |folder: &str, name: &str| {
        user_dir(dir.path())
            .join(folder)
            .join("base")
            .join(format!("{name}.json"))
    };
    let written_printer = read_json(&base("machine", printer));
    let written_filament = read_json(&base("filament", &filament));
    let written_process = read_json(&base("process", &process));
    // The header and ids (`save_to_json`, `clone_presets`, `save_current_preset`).
    for (written, name) in [
        (&written_printer, printer.to_owned()),
        (&written_filament, filament.clone()),
        (&written_process, process.clone()),
    ] {
        assert_eq!(written["name"], name.as_str());
        assert_eq!(written["from"], "User");
        assert_eq!(written["inherits"], "");
        assert_eq!(written["is_custom_defined"], "0");
    }
    assert_eq!(written_printer["printer_settings_id"], printer);
    assert_eq!(written_printer["printer_model"], "Test Vendor Box");
    assert_eq!(written_printer["printer_variant"], "0.4");
    assert_eq!(written_printer["nozzle_diameter"], json!(["0.4"]));
    // The bed less its origin (`save_printable_area_config`).
    assert_eq!(
        written_printer["printable_area"],
        json!(["-10x-5", "210x-5", "210x225", "-10x225"])
    );
    assert_eq!(written_printer["printable_height"], "240");
    let kept = written_printer["bed_custom_model"].as_str().unwrap();
    assert!(
        Path::new(kept).starts_with(dir.path().join("bed_files")),
        "{kept}"
    );
    assert!(Path::new(kept).is_file());
    assert_eq!(written_printer["bed_custom_texture"], "");
    // The template vendor's version, as the reference writes a version.
    assert_eq!(written_filament["version"], "1.7.0.2");
    assert_eq!(written_filament["filament_settings_id"], json!([filament]));
    assert_eq!(written_filament["compatible_printers"], json!([printer]));
    let id_written = written_filament["filament_id"].as_str().unwrap();
    assert!(
        id_written.starts_with('P') && id_written.len() == 8,
        "{id_written}"
    );
    assert_eq!(written_process["print_settings_id"], process.as_str());
    assert_eq!(written_process["compatible_printers"], json!([printer]));
    // `generate_process_presets_data` for a 0.4 nozzle.
    assert_eq!(written_process["layer_height"], "0.2");
    assert_eq!(written_process["initial_layer_print_height"], "0.2");
    assert_eq!(written_process["line_width"], "0.4");
    assert_eq!(written_process["wall_loops"], "2");
    assert_eq!(written_process["top_shell_layers"], "5");
    assert_eq!(written_process["bottom_shell_layers"], "3");
    // Key by key against the reference's writer: nothing written that it
    // would not write. What it writes beside them is each option's
    // default, which a reader takes for an option left out.
    for (written, kind) in [
        (&written_printer, Kind::Machine),
        (&written_filament, Kind::Filament),
        (&written_process, Kind::Process),
    ] {
        let keys = reference_keys(kind);
        let extra: Vec<&String> = written
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| !keys.contains(*key))
            .collect();
        assert!(
            extra.is_empty(),
            "{kind:?} writes keys the reference does not: {extra:?}"
        );
        // What the reference writes beside these is every other option of
        // the kind at its default (98 of a printer's, 105 of a filament's,
        // 350 of a process's here): left out, a reader of either app
        // takes the same default (PresetCollection::load_presets lays a
        // file over the default preset).
        let _ = keys;
    }
    // Each stands alone, with its .info beside it.
    assert!(base("machine", printer).with_extension("info").is_file());

    // Read back in another run: listed under the person's printers, and
    // offered its own presets.
    let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
    ask(
        &again,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    let printers = ask(&again, "printers", Value::Null);
    assert!(
        printers["user"]
            .as_array()
            .unwrap()
            .iter()
            .any(|preset| preset["name"] == printer && preset["vendor"] == "Snapmaker")
    );
    let lists = ask(
        &again,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": printer }),
    );
    assert_eq!(lists["user_processes"], json!([process]));
    assert_eq!(lists["user_filaments"], json!([filament]));

    // The scene shows the bed model of its own.
    let scene = ask(&session, "scene", json!({ "project": id }));
    assert!(
        scene["bed"]["model"]
            .as_str()
            .is_some_and(|name| name.ends_with(" bed.stl")),
        "{}",
        scene["bed"]
    );
    let bed = session
        .call_bytes("bed_model", &json!({ "project": id }))
        .expect("the bed's model");
    assert_eq!(&bed[..4], b"EDGM");

    // Sliceable: the cube on the new printer.
    ask(&session, "slice", json!({ "project": id }));
    loop {
        let status = ask(&session, "status", json!({ "project": id }));
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let out = dir.path().join("out.gcode");
    ask(
        &session,
        "export",
        json!({ "project": id, "path": out.to_string_lossy() }),
    );
    let gcode = std::fs::read_to_string(out).unwrap();
    assert!(gcode.contains(&format!("; printer_settings_id = {printer}\n")));
    assert!(gcode.contains(&format!("; print_settings_id = {process}\n")));

    // Made again: asked before the printer is written over, then before
    // its presets are.
    let asked = ask(&session, "create_printer", args.clone());
    assert_eq!(asked["created"], false);
    assert_eq!(asked["step"], "printer");
    let mut once = args.clone();
    once["rewrite"] = json!(true);
    assert_eq!(ask(&session, "create_printer", once)["created"], true);

    // Deleting it takes its two presets with it.
    let deleted = ask(
        &session,
        "preset_delete",
        json!({ "project": id, "kind": "printer", "preset": printer }),
    );
    assert_eq!(
        deleted["ask"],
        fill(
            &tr(
                "%d Filament Preset and %d Process Preset is attached to this printer. Those presets would be deleted if the printer is deleted."
            ),
            &[&1, &1]
        )
    );
    let done = ask(
        &session,
        "preset_delete",
        json!({ "project": id, "kind": "printer", "preset": printer, "confirm": true }),
    );
    assert_ne!(done["choices"]["machine"], printer);
    assert!(!base("filament", &filament).exists());
}

#[test]
fn a_nozzle_is_made_for_a_printer_of_ones_own_from_the_current_printers_presets() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let first = with(
        printer_form(),
        json!({
            "custom": true, "vendor": "Test Vendor", "model": "Box",
            "preset_vendor": "Snapmaker", "preset_model": "Snapmaker U1 @ 0.4 nozzle",
            "template": true, "filaments": ["Generic PLA template"], "processes": ["process template"],
        }),
    );
    ask(&session, "create_printer", first);
    let choices = ask(&session, "create_printer_choices", Value::Null);
    assert_eq!(choices["existing"], json!(["Test Vendor Box 0.4 nozzle"]));
    // "Create Based on Current Printer": what goes with the U1 0.6.
    let offered = ask(
        &session,
        "create_printer_presets",
        json!({ "vendor": "Snapmaker", "model": "Snapmaker U1 @ 0.6 nozzle" }),
    );
    let filament = strings(&offered["filaments"])
        .into_iter()
        .find(|name| name.starts_with("Generic PLA @"))
        .expect("a Generic PLA for the U1 0.6");
    let process = strings(&offered["processes"])[0].clone();
    let created = ask(
        &session,
        "create_printer",
        json!({
            "type": "nozzle", "printer": "Test Vendor Box 0.4 nozzle", "nozzle": "0.6 mm",
            "preset_vendor": "Snapmaker", "preset_model": "Snapmaker U1 @ 0.6 nozzle",
            "template": false, "filaments": [filament], "processes": [process],
        }),
    );
    let printer = "Test Vendor Box 0.6 nozzle";
    assert_eq!(created["printer"], printer);
    let base = |folder: &str, name: &str| {
        user_dir(dir.path())
            .join(folder)
            .join("base")
            .join(format!("{name}.json"))
    };
    let written = read_json(&base("machine", printer));
    // The bed is the existing printer's (`save_printable_area_config`).
    assert_eq!(
        written["printable_area"],
        json!(["0x0", "220x0", "220x230", "0x230"])
    );
    assert_eq!(written["printable_height"], "240");
    assert_eq!(written["printer_model"], "Test Vendor Box");
    assert_eq!(written["printer_variant"], "0.6");
    assert_eq!(written["nozzle_diameter"], json!(["0.6"]));
    // The filament made from the vendor's keeps its filament's id
    // (`get_filament_id` finds the vendor's "Generic PLA").
    let made = read_json(&base("filament", &format!("Generic PLA @{printer}")));
    // The first by name of the presets named "Generic PLA @…", as the
    // reference's collection is in order of name.
    let shelf = session.shelf("Snapmaker").unwrap();
    let mut named: Vec<String> = shelf
        .all_of(Kind::Filament)
        .into_iter()
        .filter(|name| name.starts_with("Generic PLA @"))
        .collect();
    named.sort();
    let first = shelf.resolve(Kind::Filament, &named[0]).unwrap();
    assert_eq!(made["filament_id"], first.text("filament_id").unwrap());
    assert_eq!(made["compatible_printers"], json!([printer]));
    // Not a template: the process keeps its own layer height.
    let made_process = read_json(&base(
        "process",
        &format!("{} @{printer}", process.split(" @").next().unwrap()),
    ));
    let source = session
        .shelf("Snapmaker")
        .unwrap()
        .resolve(Kind::Process, &process)
        .unwrap();
    assert_eq!(
        made_process["layer_height"],
        source.text("layer_height").unwrap()
    );
}

#[test]
fn every_check_of_create_filament_and_what_it_writes() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let choices = ask(&session, "create_filament_choices", Value::Null);
    assert_eq!(choices["vendors"][0], "Snapmaker");
    assert!(strings(&choices["types"]).contains(&"PLA".to_owned()));
    let form = json!({
        "vendor": "Polymaker", "type": "PLA", "serial": "Matte",
        "presets": [{ "printer": U1, "preset": "Generic PLA @U1 0.4 nozzle" }],
    });
    let create = |args: Value| refused(&session, "create_filament", args);
    assert_eq!(
        create(with(form.clone(), json!({ "vendor": "" }))),
        tr("Vendor is not selected, please reselect vendor.")
    );
    assert_eq!(
        create(with(
            form.clone(),
            json!({ "custom": true, "custom_vendor": "" })
        )),
        tr("Custom vendor is not input, please input custom vendor.")
    );
    assert_eq!(
        create(with(
            form.clone(),
            json!({ "custom": true, "custom_vendor": "Generic" })
        )),
        tr("\"Bambu\" or \"Generic\" cannot be used as a Vendor for custom filaments.")
    );
    assert_eq!(
        create(with(form.clone(), json!({ "type": "" }))),
        tr("Filament type is not selected, please reselect type.")
    );
    assert_eq!(
        create(with(form.clone(), json!({ "serial": "" }))),
        tr("Filament serial is not entered, please enter serial.")
    );
    assert_eq!(
        create(with(form.clone(), json!({ "serial": "@;" }))),
        tr(
            "There may be escape characters in the vendor or serial input of filament. Please delete and re-enter."
        )
    );
    assert_eq!(
        create(with(form.clone(), json!({ "serial": "   " }))),
        tr("All inputs in the custom vendor or serial are spaces. Please re-enter.")
    );
    assert_eq!(
        create(with(
            form.clone(),
            json!({ "custom": true, "custom_vendor": "123" })
        )),
        tr("The vendor cannot be a number. Please re-enter.")
    );
    assert_eq!(
        create(with(form.clone(), json!({ "presets": [] }))),
        tr("You have not selected a printer or preset yet. Please select at least one.")
    );
    // "Copy Current Filament Preset ": the table for PLA, the project's
    // printer and filament to start the first row.
    let offered = ask(
        &session,
        "create_filament_presets",
        json!({ "type": "PLA" }),
    );
    assert_eq!(offered["current"]["machine"], U1);
    let u1 = offered["table"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["printer"] == "Snapmaker U1")
        .expect("the U1 by its name without the nozzle");
    let nozzle = u1["nozzles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["nozzle"] == "0.4")
        .unwrap();
    assert_eq!(nozzle["machine"], U1);
    assert!(strings(&nozzle["presets"]).contains(&"Generic PLA @U1 0.4 nozzle".to_owned()));

    // Made, read back from the disk.
    let made = ask(&session, "create_filament", form.clone());
    assert_eq!(made["created"], true);
    let name = format!("Polymaker PLA Matte @{U1}");
    assert_eq!(made["presets"], json!([name]));
    let written = read_json(
        &user_dir(dir.path())
            .join("filament")
            .join("base")
            .join(format!("{name}.json")),
    );
    assert_eq!(written["name"], name.as_str());
    assert_eq!(written["inherits"], "");
    assert_eq!(written["from"], "User");
    assert_eq!(written["filament_vendor"], json!(["Polymaker"]));
    assert_eq!(written["filament_type"], json!(["PLA"]));
    assert_eq!(written["compatible_printers"], json!([U1]));
    assert_eq!(written["filament_settings_id"], json!([name]));
    let id = written["filament_id"].as_str().unwrap();
    assert!(id.starts_with('P') && id.len() == 8, "{id}");
    // The same id as `get_filament_id` gives a new name: "P" and the MD5's first seven.
    let digest: String = md5::Md5::digest(b"Polymaker PLA Matte")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(id, format!("P{}", &digest[..7]));
    let keys = reference_keys(Kind::Filament);
    assert!(
        written
            .as_object()
            .unwrap()
            .keys()
            .all(|key| keys.contains(key))
    );
    // Offered on the U1.
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert!(strings(&lists["user_filaments"]).contains(&name));

    // Again: the name is a filament's already, then the preset is there.
    let asked = ask(&session, "create_filament", form.clone());
    assert_eq!(asked["step"], "name");
    assert_eq!(
        asked["ask"],
        fill(
            &tr(
                "The Filament name %s you created already exists.\nIf you continue creating, the preset created will be displayed with its full name. Do you want to continue?"
            ),
            &[&"Polymaker PLA Matte"]
        )
    );
    let asked = ask(
        &session,
        "create_filament",
        with(form.clone(), json!({ "continue_name": true })),
    );
    assert_eq!(asked["step"], "rewrite");
    let done = ask(
        &session,
        "create_filament",
        with(form, json!({ "continue_name": true, "rewrite": true })),
    );
    assert_eq!(done["created"], true);
    assert_eq!(done["filament_id"], id, "the filament keeps its id");
    // "Create Based on Current Filament" now offers it: it stands alone.
    let offered = ask(
        &session,
        "create_filament_presets",
        json!({ "type": "PLA" }),
    );
    assert!(
        offered["base"]
            .as_array()
            .unwrap()
            .iter()
            .any(|choice| choice["name"] == "Polymaker PLA Matte")
    );
}

#[test]
fn favourites_are_kept_as_the_reference_keeps_them_and_head_the_lists() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    // The escaped list of `escape_strings_cstyle`, both ways.
    let items = vec![
        "Generic PLA".to_owned(),
        "Plain".to_owned(),
        "A \"q\" \\ b".to_owned(),
    ];
    let text = super::escape_list(&items);
    assert_eq!(text, r#""Generic PLA";Plain;"A \"q\" \\ b""#);
    assert_eq!(super::unescape_list(&text), items);
    assert_eq!(super::unescape_list(""), Vec::<String>::new());
    // A vendor's filament by its alias.
    let on = ask(&session, "favorite_toggle", json!({ "key": "Generic PLA" }));
    assert_eq!(on["on"], true);
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    let favs = strings(&lists["favorite_filaments"]);
    assert!(
        favs.contains(&"Generic PLA @U1 0.4 nozzle".to_owned()),
        "{favs:?}"
    );
    assert!(
        favs.iter().all(|name| name.starts_with("Generic PLA @")),
        "{favs:?}"
    );
    let mut sorted = favs.clone();
    sorted.sort();
    assert_eq!(favs, sorted, "in order of name");
    // The manager shows the star on its row.
    let table = ask(&session, "filament_manager", Value::Null);
    let row = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["fav"] == "Generic PLA")
        .unwrap()
        .clone();
    assert_eq!(row["star"], true);
    // Kept under the reference's key, and read back by another session.
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(written["app"]["favorite_filaments"], "\"Generic PLA\"");
    let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
    ask(
        &again,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    assert_eq!(
        ask(&again, "favorites", Value::Null)["favs"],
        json!(["Generic PLA"])
    );
    // Toggled again, it goes.
    let off = ask(&session, "favorite_toggle", json!({ "key": "Generic PLA" }));
    assert_eq!(off["on"], false);
    assert_eq!(ask(&session, "favorites", Value::Null)["favs"], json!([]));
    // One of one's own by its full name; its star follows a rename and goes with a delete.
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "nozzle_temperature", "value": 212 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "filament", "filament": 1, "name": "My PLA" }),
    );
    ask(&session, "favorite_toggle", json!({ "key": "My PLA" }));
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(lists["favorite_filaments"], json!(["My PLA"]));
    ask(
        &session,
        "preset_rename",
        json!({ "kind": "filament", "preset": "My PLA", "name": "Your PLA" }),
    );
    assert_eq!(
        ask(&session, "favorites", Value::Null)["favs"],
        json!(["Your PLA"])
    );
    ask(
        &session,
        "preset_delete",
        json!({ "kind": "filament", "preset": "Your PLA", "confirm": true }),
    );
    assert_eq!(ask(&session, "favorites", Value::Null)["favs"], json!([]));
}

#[test]
fn the_managers_temperatures_are_the_first_members_for_the_plate_in_force() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let project = ask(&session, "project", Value::Null);
    let plate = project["choices"]["bed_type"].as_str().map(str::to_owned);
    let table = ask(&session, "filament_manager", Value::Null);
    let row = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == "s|Snapmaker|Generic PLA")
        .unwrap()
        .clone();
    // Its members in order of name; the values are the first one's.
    let members = strings(&row["members"]);
    let mut sorted = members.clone();
    sorted.sort_by(|a, b| super::less_name(a, b));
    assert_eq!(members, sorted);
    let first = session
        .shelf("Snapmaker")
        .unwrap()
        .resolve(Kind::Filament, &members[0])
        .unwrap();
    let number = |key: &str| first.texts(key).unwrap()[0].parse::<f64>().unwrap();
    assert_eq!(
        row["nozzle_t"].as_f64().unwrap(),
        number("nozzle_temperature")
    );
    assert_eq!(
        row["nozzle_lo"].as_f64().unwrap(),
        number("nozzle_temperature_range_low")
    );
    assert_eq!(
        row["nozzle_hi"].as_f64().unwrap(),
        number("nozzle_temperature_range_high")
    );
    let key = super::bed_temperature_key(plate.as_deref());
    assert_eq!(row["bed_t"].as_f64().unwrap(), number(key));
    assert_eq!(row["mtime"], Value::Null);
    assert!(row["version"].as_str().is_some_and(|v| !v.is_empty()));
    // Another plate: its own setting.
    ask(&session, "project_set", json!({ "bed_type": "Cool Plate" }));
    let table = ask(&session, "filament_manager", Value::Null);
    let row = table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == "s|Snapmaker|Generic PLA")
        .unwrap()
        .clone();
    assert_eq!(row["bed_t"].as_f64().unwrap(), number("cool_plate_temp"));
    assert_eq!(super::bed_temperature_key(None), "hot_plate_temp");
    // The rows: the vendors' by maker and name, then the person's by name.
    let rows = table["rows"].as_array().unwrap();
    let system: Vec<(String, String)> = rows
        .iter()
        .filter(|row| row["source"] == "System")
        .map(|row| {
            (
                row["vendor"].as_str().unwrap().to_owned(),
                row["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut ordered = system.clone();
    ordered.sort_by(|a, b| super::less_name(&a.0, &b.0).then_with(|| super::less_name(&a.1, &b.1)));
    assert_eq!(system, ordered);
}

#[test]
fn the_manager_sorts_and_filters_as_its_table_does() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "nozzle_temperature", "value": 212 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "filament", "filament": 1, "name": "My PLA" }),
    );
    let view = |args: Value| -> (Vec<String>, Value) {
        let answer = ask(&session, "filament_manager", args);
        let rows: BTreeMap<String, Value> = answer["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| (row["key"].as_str().unwrap().to_owned(), row.clone()))
            .collect();
        (strings(&answer["view"]), json!(rows))
    };
    // No sort: as given, the vendors' first, the person's last.
    let (given, rows) = view(json!({}));
    assert_eq!(given.last().unwrap(), "u|My PLA");
    let name = |key: &str| rows[key]["name"].as_str().unwrap().to_owned();
    // By name, up and down.
    let (up, _) = view(json!({ "sort": { "key": "name", "dir": 1 } }));
    let names: Vec<String> = up.iter().map(|key| name(key)).collect();
    let mut sorted = names.clone();
    sorted.sort_by(|a, b| super::natural(a, b));
    assert_eq!(names, sorted);
    let (down, _) = view(json!({ "sort": { "key": "name", "dir": -1 } }));
    let mut reversed = up.clone();
    reversed.reverse();
    let down_names: Vec<String> = down.iter().map(|key| name(key)).collect();
    let reversed_names: Vec<String> = reversed.iter().map(|key| name(key)).collect();
    assert_eq!(down_names, reversed_names);
    // By nozzle temperature: rising, unknown values last either way.
    for dir in [1, -1] {
        let (order, _) = view(json!({ "sort": { "key": "nozzle_t", "dir": dir } }));
        let temps: Vec<Option<f64>> = order
            .iter()
            .map(|key| rows[key.as_str()]["nozzle_t"].as_f64())
            .collect();
        let known: Vec<f64> = temps.iter().map_while(|t| *t).collect();
        assert!(
            temps[known.len()..].iter().all(Option::is_none),
            "unknown last"
        );
        assert!(
            known
                .windows(2)
                .all(|w| if dir == 1 { w[0] <= w[1] } else { w[0] >= w[1] })
        );
    }
    // Each filter alone.
    let (users, _) = view(json!({ "filters": { "source": { "exclude": ["System"] } } }));
    assert_eq!(users, vec!["u|My PLA".to_owned()]);
    let (petg, _) = view(json!({ "filters": { "type": { "exclude": [] } , "name": "petg" } }));
    assert!(
        !petg.is_empty()
            && petg
                .iter()
                .all(|key| name(key).to_lowercase().contains("petg"))
    );
    let (hot, _) = view(json!({ "filters": { "nozzle_t": { "min": 240 } } }));
    assert!(
        !hot.is_empty()
            && hot
                .iter()
                .all(|key| rows[key.as_str()]["nozzle_t"].as_f64().unwrap() >= 240.0)
    );
    let answer = ask(&session, "filament_manager", json!({}));
    let vendors = strings(&answer["values"]["vendor"]);
    assert!(vendors.contains(&"Generic".to_owned()));
    let others: Vec<String> = vendors
        .iter()
        .filter(|v| *v != "Generic")
        .cloned()
        .collect();
    let (generic, _) = view(json!({ "filters": { "vendor": { "exclude": others } } }));
    assert!(
        !generic.is_empty()
            && generic
                .iter()
                .all(|key| rows[key.as_str()]["vendor"] == "Generic")
    );
    let (stars, _) = view(json!({ "filters": { "star": { "exclude": ["no"] } } }));
    assert!(stars.is_empty(), "no favourite yet");
    // The printer list puts the project's printer first.
    assert_eq!(answer["values"]["printers"][0], U1);
    let (on_u1, _) = view(
        json!({ "filters": { "printers": { "exclude": strings(&answer["values"]["printers"]).into_iter().filter(|p| p != U1).collect::<Vec<_>>() } } }),
    );
    assert!(
        on_u1
            .iter()
            .all(|key| strings(&rows[key.as_str()]["printers"]).contains(&U1.to_owned()))
    );
    // A filter with the search: both hold.
    let (both, _) = view(
        json!({ "search": "pla", "filters": { "vendor": { "exclude": vendors.iter().filter(|v| *v != "Generic").cloned().collect::<Vec<_>>() } } }),
    );
    assert!(!both.is_empty());
    for key in &both {
        let row = &rows[key.as_str()];
        assert_eq!(row["vendor"], "Generic");
        assert!(row.to_string().to_lowercase().contains("pla"));
    }
    assert!(both.len() < generic.len());
}
