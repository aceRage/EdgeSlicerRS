//! An object's, a part's and a height range's own settings through the
//! session: what each may set, what is in force, the rules, each edit one
//! step of the history, the slices it lets go, and the round trip through
//! a saved file.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};

use crate::api::Session;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
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

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err("the call should be refused")
}

fn open(session: &Arc<Session>, path: &Path) -> u64 {
    ask(session, "project_open", json!({ "path": path }))["id"]
        .as_u64()
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

fn edit(session: &Arc<Session>, args: Value) -> Value {
    ask(session, "scene_edit", args)
}

fn sliced(session: &Arc<Session>, project: u64, plate: u64) {
    ask(
        session,
        "slice",
        json!({ "project": project, "plate": plate }),
    );
    loop {
        let status = ask(session, "status", json!({ "project": project }));
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn has(list: &Value, key: &str) -> bool {
    list.as_array()
        .unwrap()
        .iter()
        .any(|held| held.as_str() == Some(key))
}

#[test]
fn each_kind_of_item_offers_the_references_options() {
    let session = Arc::new(Session::new(None));
    let project = open(&session, &reference(SMALL));
    let scene = ask(&session, "scene", json!({ "project": project }));
    let object = scene["objects"][0]["id"].clone();
    let part = scene["objects"][0]["part_list"][0]["id"].clone();
    let of = |args: Value| ask(&session, "item_settings", args);
    let on_object = of(json!({ "object": object }));
    let on_part = of(json!({ "object": object, "part": part }));
    assert_eq!(on_object["item"]["kind"], "object");
    assert_eq!(on_part["item"]["kind"], "part");
    // An object: its own (PrintObjectConfig) and its regions' options.
    assert!(has(&on_object["keys"], "layer_height"));
    assert!(has(&on_object["keys"], "enable_support"));
    assert!(has(&on_object["keys"], "wall_loops"));
    // Not what is the printer's or the whole print's.
    assert!(!has(&on_object["keys"], "skirt_loops"));
    assert!(!has(&on_object["keys"], "spiral_mode"));
    // A part: its region's and the part support keys, no layer height.
    assert!(!has(&on_part["keys"], "layer_height"));
    assert!(!has(&on_part["keys"], "enable_support"));
    assert!(has(&on_part["keys"], "support_interface_top_layers"));
    assert!(has(&on_part["keys"], "sparse_infill_density"));
    // Every option not followed per item yet, said once.
    assert!(
        on_object["unread_note"]
            .as_str()
            .unwrap()
            .starts_with("Not followed on an object")
    );
    assert_eq!(
        on_object["unread"].as_array().unwrap().len(),
        on_object["keys"].as_array().unwrap().len()
    );
}

#[test]
fn an_object_a_part_and_a_range_take_settings_of_their_own_one_step_each() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("twopart.3mf");
    std::fs::copy(reference(SMALL), &copy).unwrap();
    let session = Arc::new(Session::new(None));
    let project = open(&session, &copy);
    choose_presets(&session, project);
    let scene = ask(&session, "scene", json!({ "project": project }));
    let object = scene["objects"][0]["id"].clone();
    let parts = scene["objects"][0]["part_list"].as_array().unwrap().clone();
    let part = parts[0]["id"].clone();
    let global_walls =
        ask(&session, "process", json!({ "project": project }))["values"]["wall_loops"].clone();

    // The object: in force, the project's; set, its own; the global value
    // stays; one step; unsaved.
    let before = ask(&session, "item_settings", json!({ "object": object }));
    assert_eq!(before["values"]["wall_loops"], global_walls);
    assert!(before["set"].get("wall_loops").is_none());
    let answer = edit(
        &session,
        json!({ "op": "item_set", "object": object, "key": "wall_loops", "value": 5 }),
    );
    assert_eq!(answer["done"], "Change Option wall_loops");
    assert_eq!(answer["project"]["dirty"], true);
    assert_eq!(answer["scene"]["objects"][0]["settings"], 1);
    let after = ask(&session, "item_settings", json!({ "object": object }));
    assert_eq!(after["set"]["wall_loops"], "5");
    assert_eq!(after["counts"]["Frequent"], 1);
    assert_eq!(after["counts"]["Strength"], 1);
    let process = ask(&session, "process", json!({ "project": project }));
    assert_eq!(process["values"]["wall_loops"], global_walls);
    assert!(process["set"].get("wall_loops").is_none());

    // A part takes its object's as what is in force over it.
    let on_part = ask(
        &session,
        "item_settings",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(on_part["values"]["wall_loops"], "5");
    let answer = edit(
        &session,
        json!({ "op": "item_set", "object": object, "part": part, "key": "sparse_infill_density", "value": "60%" }),
    );
    assert_eq!(answer["done"], "Change Option sparse_infill_density");
    let on_part = ask(
        &session,
        "item_settings",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(on_part["set"]["sparse_infill_density"], "60%");
    // What a part cannot set, a value out of range, a helper part.
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "item_set", "object": object, "part": part, "key": "layer_height", "value": 0.1 })
        )
        .contains("cannot be set on a part")
    );
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "item_set", "object": object, "key": "wall_loops", "value": -1 })
        )
        .contains("out of range")
    );
    if let Some(helper) = parts
        .iter()
        .find(|held| held["kind"] != "part" && held["kind"] != "modifier")
    {
        refused(
            &session,
            "scene_edit",
            json!({ "op": "item_set", "object": object, "part": helper["id"], "key": "wall_loops", "value": 2 }),
        );
    }

    // The rules go by the item's own settings: no infill on the object
    // hides its pattern there, not on the project's pages.
    edit(
        &session,
        json!({ "op": "item_set", "object": object, "key": "sparse_infill_density", "value": "0%" }),
    );
    let ruled = ask(&session, "item_settings", json!({ "object": object }));
    assert!(has(&ruled["hidden"], "sparse_infill_pattern"));
    let process = ask(&session, "process", json!({ "project": project }));
    assert!(!has(&process["hidden"], "sparse_infill_pattern"));

    // A height range: added from 0 to 2 mm with the object's layer height,
    // edited, given its own, another after it, one deleted.
    let answer = edit(&session, json!({ "op": "range_add", "object": object }));
    assert_eq!(answer["done"], "Add layers");
    let ranges = &answer["scene"]["objects"][0]["ranges"];
    assert_eq!(ranges[0]["min_z"], 0.0);
    assert_eq!(ranges[0]["max_z"], 2.0);
    let on_range = ask(
        &session,
        "item_settings",
        json!({ "object": object, "range": 0 }),
    );
    assert!(has(&on_range["keys"], "layer_height"));
    assert!(!has(&on_range["keys"], "enable_support"));
    assert_eq!(
        on_range["values"]["wall_loops"], "5",
        "the object's is over it"
    );
    let answer = edit(
        &session,
        json!({ "op": "range_set", "object": object, "range": 0, "max_z": 3 }),
    );
    assert_eq!(answer["done"], "Edit height range");
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "range_set", "object": object, "range": 0, "max_z": 0 })
        )
        .contains("above its bottom")
    );
    let answer = edit(
        &session,
        json!({ "op": "item_set", "object": object, "range": 0, "key": "layer_height", "value": 0.1 }),
    );
    assert_eq!(
        answer["scene"]["objects"][0]["ranges"][0]["layer_height"],
        "0.1"
    );
    let answer = edit(
        &session,
        json!({ "op": "range_add", "object": object, "range": 0 }),
    );
    assert_eq!(answer["done"], "Add height range");
    let ranges = answer["scene"]["objects"][0]["ranges"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(ranges.len(), 2);
    assert_eq!(
        (ranges[1]["min_z"].clone(), ranges[1]["max_z"].clone()),
        (json!(3.0), json!(5.0))
    );
    // A bottom typed into the range under it.
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "range_set", "object": object, "range": 1, "min_z": 2 })
        )
        .starts_with("Height ranges cannot overlap")
    );
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "range_set", "object": object, "range": 1, "min_z": -1 })
        )
        .contains("Invalid numeric.")
    );

    // Saved as, opened in another session: as left.
    let saved = dir.path().join("saved.3mf");
    ask(
        &session,
        "project_save",
        json!({ "project": project, "path": saved, "accept": true }),
    );
    let other = Arc::new(Session::new(None));
    let again = open(&other, &saved);
    let scene = ask(&other, "scene", json!({ "project": again }));
    let object_again = scene["objects"][0]["id"].clone();
    let part_again = scene["objects"][0]["part_list"][0]["id"].clone();
    let on_object = ask(
        &other,
        "item_settings",
        json!({ "project": again, "object": object_again }),
    );
    assert_eq!(on_object["set"]["wall_loops"], "5");
    assert_eq!(on_object["set"]["sparse_infill_density"], "0%");
    let on_part = ask(
        &other,
        "item_settings",
        json!({ "project": again, "object": object_again, "part": part_again }),
    );
    assert_eq!(on_part["set"]["sparse_infill_density"], "60%");
    let on_range = ask(
        &other,
        "item_settings",
        json!({ "project": again, "object": object_again, "range": 0 }),
    );
    assert_eq!(
        on_range["item"]["heights"],
        json!({ "min_z": 0.0, "max_z": 3.0 })
    );
    assert_eq!(on_range["set"]["layer_height"], "0.1");

    // The arrow, the reset of all, the delete; then every step undone back
    // to the project as it was opened.
    let answer = edit(
        &session,
        json!({ "op": "item_set", "object": object, "key": "wall_loops", "value": null }),
    );
    assert_eq!(answer["done"], "Change Option wall_loops");
    let answer = edit(&session, json!({ "op": "item_reset", "object": object }));
    assert_eq!(answer["done"], "Reset Options");
    assert_eq!(answer["scene"]["objects"][0]["settings"], 0);
    let answer = edit(
        &session,
        json!({ "op": "range_delete", "object": object, "range": 1 }),
    );
    assert_eq!(answer["done"], "Remove height range");
    let mut names = Vec::new();
    loop {
        let detail = ask(&session, "project", json!({ "project": project }));
        if detail["edit"]["undo"].is_null() {
            break;
        }
        names.push(ask(&session, "undo", json!({ "project": project }))["done"].clone());
    }
    assert_eq!(
        names,
        [
            "Remove height range",
            "Reset Options",
            "Change Option wall_loops",
            "Add height range",
            "Change Option layer_height",
            "Edit height range",
            "Add layers",
            "Change Option sparse_infill_density",
            "Change Option sparse_infill_density",
            "Change Option wall_loops",
        ]
    );
    let on_object = ask(&session, "item_settings", json!({ "object": object }));
    assert!(on_object["set"].as_object().unwrap().is_empty());
    // Saved with them, the project is unsaved once they are taken back.
    let detail = ask(&session, "project", json!({ "project": project }));
    assert_eq!(detail["dirty"], true);
}

#[test]
fn only_the_plate_the_object_is_on_loses_its_slice() {
    let session = Arc::new(Session::new(None));
    let project = open(&session, &reference(THREE_PLATES));
    let scene = ask(&session, "scene", json!({ "project": project }));
    let on_two = scene["placings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|placing| placing["plate"] == 2)
        .unwrap()
        .clone();
    let object = scene["objects"][on_two["object"].as_u64().unwrap() as usize]["id"].clone();
    sliced(&session, project, 1);
    sliced(&session, project, 2);
    let answer = edit(
        &session,
        json!({ "op": "item_set", "object": object, "key": "wall_loops", "value": 5 }),
    );
    let plates: Vec<bool> = answer["project"]["plates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|plate| plate["sliced"] == true)
        .collect();
    assert_eq!(plates, [true, false, false]);
    // The same value again is no step.
    let again = edit(
        &session,
        json!({ "op": "item_set", "object": object, "key": "wall_loops", "value": 5 }),
    );
    assert!(again["done"].is_null());
}
