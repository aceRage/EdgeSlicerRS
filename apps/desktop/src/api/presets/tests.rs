use std::path::{Path, PathBuf};

use super::*;

fn reference(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

/// A session that keeps its settings and presets in `dir`, on the
/// reference's profiles, with the Snapmaker U1 added and chosen.
fn session(dir: &Path) -> Arc<Session> {
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

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err(&format!("{name} was not refused"))
}

fn user_dir(dir: &Path) -> PathBuf {
    dir.join("user").join("default")
}

const U1: &str = "Snapmaker U1 (0.4 nozzle)";

/// The project's process preset, its own name and what is set over it.
fn process_of(session: &Arc<Session>) -> (String, Value) {
    let project = ask(session, "project", Value::Null);
    (
        project["choices"]["process"].as_str().unwrap().to_owned(),
        project["choices"]["over"].clone(),
    )
}

#[test]
fn a_process_preset_is_saved_listed_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let project = ask(&session, "printer_select", json!({ "initial": true }));
    assert_eq!(project["choices"]["machine"], U1);
    let (system, _) = process_of(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );

    // The name offered: "<name> - Copy" for a system preset.
    let check = ask(&session, "preset_check", json!({ "kind": "process" }));
    assert_eq!(check["suggested"], format!("{system} - Copy"));
    assert_eq!(check["level"], "ok");
    assert_eq!(check["title"], "Process");

    let saved = ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "My Process" }),
    );
    assert_eq!(saved["saved"], true);
    assert_eq!(saved["choices"]["process"], "My Process");
    assert_eq!(
        saved["choices"]["over"],
        json!({}),
        "nothing is set over it now"
    );

    // The file, as the reference writes it.
    let file = user_dir(dir.path()).join("process").join("My Process.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("{\n    \"from\": \"User\",\n"), "{text}");
    assert!(
        text.contains(&format!("\"inherits\": \"{system}\"")),
        "{text}"
    );
    assert!(
        text.contains("\"print_settings_id\": \"My Process\""),
        "{text}"
    );
    assert!(text.contains("\"wall_loops\": \"3\""), "{text}");
    assert!(text.contains("\"is_custom_defined\": \"0\""), "{text}");
    // Only what differs from the preset it was made from.
    assert_eq!(text.lines().count(), 9, "{text}");
    let info = std::fs::read_to_string(file.with_extension("info")).unwrap();
    assert!(info.starts_with("sync_info = create\n"), "{info}");

    // Listed for the printer, under the person's own.
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(lists["user_processes"], json!(["My Process"]));
    assert!(
        !lists["processes"]
            .as_array()
            .unwrap()
            .contains(&json!("My Process"))
    );
    // Its values are the pages' as the preset's, nothing set by hand.
    let process = ask(&session, "process", Value::Null);
    assert_eq!(process["values"]["wall_loops"], "3");
    assert_eq!(process["changed"], 0);

    // To a system preset and back.
    ask(&session, "project_set", json!({ "process": system }));
    assert_eq!(
        ask(&session, "process", Value::Null)["values"]["wall_loops"],
        "2"
    );
    ask(&session, "project_set", json!({ "process": "My Process" }));
    assert_eq!(
        ask(&session, "process", Value::Null)["values"]["wall_loops"],
        "3"
    );

    // Another run finds it.
    let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
    ask(
        &again,
        "set_profiles",
        json!({ "dir": reference("resources/profiles") }),
    );
    let listed = ask(&again, "user_presets", Value::Null);
    assert_eq!(listed["process"][0]["name"], "My Process");
    let project = ask(&again, "printer_select", json!({ "initial": true }));
    assert_eq!(
        project["choices"]["process"], "My Process",
        "the process last used with the printer"
    );
    assert_eq!(
        ask(&again, "process", Value::Null)["values"]["wall_loops"],
        "3"
    );
}

#[test]
fn a_name_is_checked_as_it_is_typed_and_a_preset_saved_over_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    let check = |name: &str| {
        ask(
            &session,
            "preset_check",
            json!({ "kind": "process", "name": name }),
        )
    };
    assert_eq!(
        check(&system)["message"],
        "Overwriting a system profile is not allowed."
    );
    assert_eq!(check("a/b")["level"], "invalid");
    assert_eq!(
        check("a/b")["message"],
        "Name is invalid;\nIllegal characters: <>[]:/\\|?*\""
    );
    assert_eq!(
        check("")["message"],
        "The name field is not allowed to be empty."
    );
    assert_eq!(
        check(" x")["message"],
        "The name is not allowed to start with a space."
    );
    assert_eq!(
        check("x ")["message"],
        "The name is not allowed to end with a space."
    );
    assert_eq!(check("Default Setting")["message"], "Name is unavailable.");
    assert_eq!(
        refused(
            &session,
            "preset_save",
            json!({ "kind": "process", "name": system })
        ),
        "Overwriting a system profile is not allowed."
    );

    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "Mine" }),
    );
    // Back on the system preset, saved again under the same name: asked.
    ask(&session, "project_set", json!({ "process": system }));
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 4 }),
    );
    let warned = check("Mine");
    assert_eq!(warned["level"], "warn");
    assert_eq!(
        warned["message"],
        "Preset \"Mine\" already exists.\nPlease note that saving will overwrite the current preset."
    );
    let asked = ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "Mine" }),
    );
    assert_eq!(asked["saved"], false);
    assert!(asked["ask"].as_str().unwrap().contains("already exists"));
    let text = |dir: &Path| {
        std::fs::read_to_string(user_dir(dir).join("process").join("Mine.json")).unwrap()
    };
    assert!(
        text(dir.path()).contains("\"wall_loops\": \"3\""),
        "unchanged until accepted"
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "Mine", "accept": true }),
    );
    assert!(text(dir.path()).contains("\"wall_loops\": \"4\""));
    let info =
        std::fs::read_to_string(user_dir(dir.path()).join("process").join("Mine.info")).unwrap();
    assert!(info.starts_with("sync_info = update\n"), "{info}");
    // A rename may not take a name that is taken.
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 5 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "Other" }),
    );
    assert_eq!(
        refused(
            &session,
            "preset_rename",
            json!({ "kind": "process", "preset": "Other", "name": "Mine" })
        ),
        "Preset \"Mine\" already exists."
    );
    assert_eq!(
        refused(
            &session,
            "preset_rename",
            json!({ "kind": "process", "preset": system, "name": "x" })
        ),
        "System presets cannot be renamed."
    );
}

#[test]
fn a_preset_is_renamed_and_deleted_and_the_projects_follow() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "First" }),
    );
    ask(
        &session,
        "process_set",
        json!({ "key": "sparse_infill_density", "value": "30%" }),
    );
    let renamed = ask(
        &session,
        "preset_rename",
        json!({ "kind": "process", "preset": "First", "name": "Second" }),
    );
    assert_eq!(renamed["choices"]["process"], "Second");
    assert_eq!(
        renamed["choices"]["over"]["sparse_infill_density"], "30%",
        "what is set over it stays"
    );
    let folder = user_dir(dir.path()).join("process");
    assert!(!folder.join("First.json").exists() && !folder.join("First.info").exists());
    assert!(folder.join("Second.json").exists() && folder.join("Second.info").exists());

    // The question first, with where it is used.
    let asked = ask(
        &session,
        "preset_delete",
        json!({ "kind": "process", "preset": "Second" }),
    );
    assert_eq!(asked["deleted"], false);
    assert_eq!(asked["title"], "Delete Preset");
    assert_eq!(
        asked["ask"],
        "Are you sure you want to Delete the selected preset?\n\nUsed by:\n - The plate's process"
    );
    assert!(folder.join("Second.json").exists());
    let deleted = ask(
        &session,
        "preset_delete",
        json!({ "kind": "process", "preset": "Second", "confirm": true }),
    );
    assert_eq!(deleted["deleted"], true);
    assert!(!folder.join("Second.json").exists() && !folder.join("Second.info").exists());
    // The project goes to the printer's usual process, nothing over it.
    assert_eq!(deleted["choices"]["process"], system);
    assert_eq!(deleted["choices"]["over"], json!({}));
    assert_eq!(
        refused(
            &session,
            "preset_delete",
            json!({ "kind": "process", "preset": system })
        ),
        "System presets cannot be deleted."
    );
}

#[test]
fn a_preset_others_are_laid_over_is_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let folder = user_dir(dir.path()).join("process");
    std::fs::create_dir_all(folder.join("base")).unwrap();
    std::fs::write(
        folder.join("base").join("Root.json"),
        "{\"name\": \"Root\", \"from\": \"User\", \"inherits\": \"\", \"version\": \"2.3.3.14\", \"wall_loops\": \"5\"}",
    )
    .unwrap();
    std::fs::write(
        folder.join("Leaf.json"),
        "{\"name\": \"Leaf\", \"from\": \"User\", \"inherits\": \"Root\", \"version\": \"2.3.3.14\"}",
    )
    .unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    assert_eq!(
        refused(
            &session,
            "preset_delete",
            json!({ "kind": "process", "preset": "Root" })
        ),
        "Presets inherited by other presets cannot be deleted!\nThe following preset inherits this preset.\n - Leaf"
    );
    assert!(folder.join("base").join("Root.json").exists());
}

#[test]
fn a_renamed_preset_keeps_what_is_laid_over_it() {
    let dir = tempfile::tempdir().unwrap();
    let folder = user_dir(dir.path()).join("process");
    std::fs::create_dir_all(folder.join("base")).unwrap();
    std::fs::write(
        folder.join("base").join("Root.json"),
        "{\"name\": \"Root\", \"from\": \"User\", \"inherits\": \"\", \"version\": \"2.3.3.14\", \"wall_loops\": \"5\"}",
    )
    .unwrap();
    std::fs::write(
        folder.join("Leaf.json"),
        "{\"name\": \"Leaf\", \"from\": \"User\", \"inherits\": \"Root\", \"version\": \"2.3.3.14\", \"sparse_infill_density\": \"30%\"}",
    )
    .unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    // Taken names and names the reference refuses are refused here too.
    for (name, why) in [
        ("Leaf", "Preset \"Leaf\" already exists."),
        ("a/b", "Name is invalid;\nIllegal characters: <>[]:/\\|?*\""),
        ("Default Setting", "Name is unavailable."),
        (" Trunk", "The name is not allowed to start with a space."),
        ("", "The name field is not allowed to be empty."),
    ] {
        assert_eq!(
            refused(
                &session,
                "preset_rename",
                json!({ "kind": "process", "preset": "Root", "name": name })
            ),
            why
        );
    }
    let (system, _) = process_of(&session);
    assert_eq!(
        refused(
            &session,
            "preset_rename",
            json!({ "kind": "process", "preset": system, "name": "Mine" })
        ),
        "System presets cannot be renamed."
    );
    ask(
        &session,
        "preset_rename",
        json!({ "kind": "process", "preset": "Root", "name": "Trunk" }),
    );
    assert!(!folder.join("base").join("Root.json").exists());
    let trunk = read(&folder.join("base").join("Trunk.json"));
    assert!(trunk.contains("\"name\": \"Trunk\""), "{trunk}");
    // The preset laid over it names it by its new name, in its file too,
    // and keeps its own values and what it inherits.
    let leaf = read(&folder.join("Leaf.json"));
    assert!(leaf.contains("\"inherits\": \"Trunk\""), "{leaf}");
    assert!(
        leaf.contains("\"sparse_infill_density\": \"30%\""),
        "{leaf}"
    );
    let listed = ask(&session, "user_presets", Value::Null);
    assert!(
        listed["process"]
            .as_array()
            .unwrap()
            .contains(&json!({ "name": "Leaf", "inherits": "Trunk" }))
    );
    // Read again from the disk, it is laid over the renamed one.
    let again = Session::with_settings(None, Some(dir.path().to_owned()));
    let users = again.users();
    assert_eq!(
        users
            .get(Kind::Process, "Leaf")
            .unwrap()
            .inherits
            .as_deref(),
        Some("Trunk")
    );
}

#[test]
fn a_deleted_preset_gives_way_to_the_next_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    let project = ask(&session, "project", Value::Null);
    let offered = ask(
        &session,
        "presets",
        json!({ "vendor": project["choices"]["vendor"], "machine": project["choices"]["machine"] }),
    );
    // A process the printer is offered that is not its usual one; the
    // preset saved is named to come just before it.
    let next = offered["processes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|name| *name != system && name.ends_with(')'))
        .unwrap()
        .to_owned();
    let mine = next.trim_end_matches(')').to_owned();
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 4 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": mine }),
    );
    let deleted = ask(
        &session,
        "preset_delete",
        json!({ "kind": "process", "preset": mine, "confirm": true }),
    );
    assert_eq!(deleted["choices"]["process"], next);
    assert_eq!(deleted["choices"]["over"], json!({}));
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn a_filament_preset_is_saved_for_filament_2_and_a_printer_preset_picked() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    ask(&session, "filament_edit", json!({ "op": "add" }));
    let project = ask(&session, "project", Value::Null);
    let filament = project["choices"]["filaments"][1]
        .as_str()
        .unwrap()
        .to_owned();
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 2, "key": "nozzle_temperature", "value": 212 }),
    );
    let check = ask(
        &session,
        "preset_check",
        json!({ "kind": "filament", "filament": 2 }),
    );
    assert_eq!(check["suggested"], format!("{filament} - Copy"));
    let saved = ask(
        &session,
        "preset_save",
        json!({ "kind": "filament", "filament": 2, "name": "My PLA" }),
    );
    assert_eq!(saved["choices"]["filaments"][1], "My PLA");
    assert_eq!(saved["choices"]["filament_over"], json!([]));
    let text =
        std::fs::read_to_string(user_dir(dir.path()).join("filament").join("My PLA.json")).unwrap();
    assert!(
        text.contains("\"filament_settings_id\": [\n        \"My PLA\"\n    ]"),
        "{text}"
    );
    assert!(
        text.contains("\"nozzle_temperature\": [\n        \"212\""),
        "{text}"
    );
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(lists["user_filaments"], json!(["My PLA"]));
    let shown = ask(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": 2 }),
    );
    assert_eq!(shown["values"]["nozzle_temperature"], "212");
    assert_eq!(shown["name"], "My PLA");

    // A printer preset of one's own: listed apart, picked by name, and
    // offered what the printer it is made from is offered.
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
    assert_eq!(saved["choices"]["printer_over"], json!({}));
    let printers = ask(&session, "printers", Value::Null);
    assert_eq!(printers["user"][0]["name"], "My U1");
    assert_eq!(printers["user"][0]["model"], "Snapmaker U1");
    assert_eq!(printers["selected"]["user"], true);
    assert_eq!(printers["selected"]["model"], "Snapmaker U1");
    let mine = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": "My U1" }),
    );
    let theirs = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(mine["processes"], theirs["processes"]);
    assert_eq!(
        mine["user_filaments"],
        json!(["My PLA"]),
        "made for the printer it comes from"
    );
    let settled = ask(&session, "settled", Value::Null);
    assert_eq!(settled["printer"]["height"], 250.0);
    // To the system printer and back by name.
    ask(
        &session,
        "printer_select",
        json!({ "vendor": "Snapmaker", "model": "Snapmaker U1" }),
    );
    let back = ask(
        &session,
        "printer_select",
        json!({ "vendor": "Snapmaker", "machine": "My U1" }),
    );
    assert_eq!(back["choices"]["machine"], "My U1");
}

#[test]
fn what_is_set_over_a_preset_is_listed_and_two_presets_compared() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    let changes = ask(&session, "preset_changes", json!({ "kind": "process" }));
    assert_eq!(changes["count"], 1);
    assert_eq!(changes["name"], system);
    let page = &changes["pages"][0];
    assert_eq!(page["title"], "Strength");
    let row = &page["groups"][0]["rows"][0];
    assert_eq!(row["key"], "wall_loops");
    assert_eq!(row["label"], "Wall loops");
    assert_eq!(row["old"], "2");
    assert_eq!(row["new"], "3");

    ask(
        &session,
        "preset_save",
        json!({ "kind": "process", "name": "Three walls" }),
    );
    let listing = ask(&session, "preset_compare", json!({ "kind": "process" }));
    assert_eq!(listing["user"], json!(["Three walls"]));
    assert!(
        listing["system"]
            .as_array()
            .unwrap()
            .contains(&json!(system))
    );
    let compared = ask(
        &session,
        "preset_compare",
        json!({ "kind": "process", "left": system, "right": "Three walls" }),
    );
    assert_eq!(compared["count"], 1);
    assert_eq!(
        compared["pages"][0]["groups"][0]["rows"][0]["key"],
        "wall_loops"
    );
    let two = ask(
        &session,
        "preset_compare",
        json!({ "kind": "process", "left": system, "right": "0.08mm Standard @Snapmaker U1 (0.4 nozzle)" }),
    );
    assert!(two["count"].as_u64().unwrap() > 1);
    let all = ask(
        &session,
        "preset_compare",
        json!({ "kind": "process", "all": true }),
    );
    assert!(all["system"].as_array().unwrap().len() > listing["system"].as_array().unwrap().len());
}

#[test]
fn changes_go_over_the_next_preset_only_where_the_reference_transfers_them() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    ask(
        &session,
        "process_set",
        json!({ "key": "sparse_infill_density", "value": "30%" }),
    );
    // A process: always, with what was kept said.
    let to = "0.08mm Standard @Snapmaker U1 (0.4 nozzle)";
    let process = ask(
        &session,
        "preset_changes",
        json!({ "kind": "process", "to": to }),
    );
    assert_eq!(process["count"], 2);
    assert_eq!(process["transfer"], true);
    assert_eq!(
        process["note"],
        format!("Kept 2 modified process settings from \"{system}\" (revertable).")
    );
    // Without a preset to go to, nothing is transferred.
    let asked = ask(&session, "preset_changes", json!({ "kind": "process" }));
    assert_eq!(asked["transfer"], false);
    assert_eq!(asked["note"], Value::Null);
    // Transferred: the settings stay over the next preset.
    let moved = ask(&session, "project_set", json!({ "process": to }));
    assert_eq!(moved["choices"]["process"], to);
    assert_eq!(moved["choices"]["over"]["wall_loops"], "3");

    // A filament: only to one of the same filament_type.
    let project = ask(&session, "project", Value::Null);
    let offered = ask(
        &session,
        "presets",
        json!({ "vendor": project["choices"]["vendor"], "machine": project["choices"]["machine"] }),
    );
    let filaments: Vec<&str> = offered["filaments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let first = project["choices"]["filaments"][0].as_str().unwrap();
    let shelf = session.shelf("Snapmaker").unwrap();
    let type_of = |name: &str| {
        session
            .resolve_named(&shelf, Kind::Filament, name)
            .unwrap()
            .texts("filament_type")
            .unwrap()[0]
            .clone()
    };
    let same = filaments
        .iter()
        .find(|name| **name != first && type_of(name) == type_of(first))
        .expect("another filament of the same type");
    let other = filaments
        .iter()
        .find(|name| type_of(name) != type_of(first))
        .expect("a filament of another type");
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "nozzle_temperature", "value": 212 }),
    );
    let alike = ask(
        &session,
        "preset_changes",
        json!({ "kind": "filament", "filament": 1, "to": same }),
    );
    assert_eq!(alike["transfer"], true);
    assert_eq!(
        alike["note"],
        format!("Kept 1 modified filament setting from \"{first}\" (revertable).")
    );
    let unlike = ask(
        &session,
        "preset_changes",
        json!({ "kind": "filament", "filament": 1, "to": other }),
    );
    assert_eq!(unlike["transfer"], false);
    assert_eq!(unlike["count"], 1);

    // A printer: never; the dialog asks to save or discard.
    ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "printable_height", "value": 250 }),
    );
    let printer = ask(
        &session,
        "preset_changes",
        json!({ "kind": "printer", "to": "Snapmaker U1 (0.6 nozzle)" }),
    );
    assert_eq!(printer["transfer"], false);
    assert_eq!(printer["count"], 1);
    assert_eq!(printer["pages"][0]["groups"][0]["rows"][0]["new"], "250");
}

#[test]
fn chosen_values_are_transferred_from_left_to_right() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let (system, _) = process_of(&session);
    let fine = "0.08mm Standard @Snapmaker U1 (0.4 nozzle)";
    let compared = ask(
        &session,
        "preset_compare",
        json!({ "kind": "process", "left": fine, "right": system }),
    );
    let rows: Vec<(String, String, String)> = compared["pages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|page| page["groups"].as_array().unwrap().clone())
        .flat_map(|group| group["rows"].as_array().unwrap().clone())
        .map(|row| {
            (
                row["key"].as_str().unwrap().to_owned(),
                row["old"].as_str().unwrap().to_owned(),
                row["new"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(rows.iter().any(|(key, _, _)| key == "layer_height"));
    // Only the right one, the preset the project is on, takes them.
    assert_eq!(
        refused(
            &session,
            "preset_transfer",
            json!({ "kind": "process", "from": system, "to": fine, "keys": ["layer_height"] })
        ),
        format!("the project is not on \"{fine}\"")
    );
    let keys: Vec<&str> = rows.iter().map(|(key, _, _)| key.as_str()).collect();
    let done = ask(
        &session,
        "preset_transfer",
        json!({ "kind": "process", "from": fine, "to": system, "keys": keys }),
    );
    assert_eq!(done["transferred"], rows.len());
    assert_eq!(done["choices"]["process"], system);
    // Every value differing is set over the preset, the left one's.
    let changes = ask(&session, "preset_changes", json!({ "kind": "process" }));
    assert_eq!(changes["count"], rows.len());
    for (key, left, _) in &rows {
        assert_eq!(&done["choices"]["over"][key], left, "{key}");
    }
    // With changes over it, another preset is not transferred to.
    assert_eq!(
        refused(
            &session,
            "preset_transfer",
            json!({ "kind": "process", "from": system, "to": fine, "keys": ["layer_height"] })
        ),
        "You can only transfer to current active profile because it has been modified."
    );
}

#[test]
fn presets_the_reference_wrote_are_offered_and_a_broken_one_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let folder = user_dir(dir.path()).join("process");
    std::fs::create_dir_all(&folder).unwrap();
    // As the C++ application writes them (Preset::save, save_info).
    std::fs::write(
        folder.join("From C++.json"),
        "{\n    \"from\": \"User\",\n    \"inherits\": \"0.20mm Standard @Snapmaker U1 (0.4 nozzle)\",\n    \"is_custom_defined\": \"0\",\n    \"name\": \"From C++\",\n    \"print_settings_id\": \"From C++\",\n    \"version\": \"2.3.3.14\",\n    \"wall_loops\": \"4\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("From C++.info"),
        "sync_info = create\nuser_id = \nsetting_id = \nbase_id = GP004\nupdated_time = 0\n",
    )
    .unwrap();
    std::fs::write(folder.join("Broken.json"), "{ \"name\": ").unwrap();
    let session = session(dir.path());
    ask(&session, "printer_select", json!({ "initial": true }));
    let listed = ask(&session, "user_presets", Value::Null);
    assert_eq!(listed["process"][0]["name"], "From C++");
    assert_eq!(listed["unread"].as_array().unwrap().len(), 1);
    assert!(
        listed["unread"][0]["file"]
            .as_str()
            .unwrap()
            .ends_with("Broken.json")
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("Broken.json")).unwrap(),
        "{ \"name\": "
    );
    let lists = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": U1 }),
    );
    assert_eq!(lists["user_processes"], json!(["From C++"]));
    ask(&session, "project_set", json!({ "process": "From C++" }));
    let process = ask(&session, "process", Value::Null);
    assert_eq!(process["values"]["wall_loops"], "4");
    assert_eq!(process["name"], "From C++");
}

#[test]
fn a_bare_model_is_sliced_with_a_preset_of_ones_own() {
    let dir = tempfile::tempdir().unwrap();
    let session = session(dir.path());
    let id = ask(
        &session,
        "project_open",
        json!({ "path": reference("resources/handy_models/OrcaToleranceTest.stl") }),
    )["id"]
        .as_u64()
        .unwrap();
    ask(
        &session,
        "printer_select",
        json!({ "project": id, "initial": true }),
    );
    ask(
        &session,
        "process_set",
        json!({ "project": id, "key": "wall_loops", "value": 4 }),
    );
    ask(
        &session,
        "preset_save",
        json!({ "project": id, "kind": "process", "name": "Four walls" }),
    );
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
    assert!(gcode.contains("; print_settings_id = Four walls\n"));
    assert!(gcode.contains("; wall_loops = 4\n"));
}
