use std::path::Path;

use super::*;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
const MACHINE: &str = "Snapmaker U1 (0.4 nozzle)";

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
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err("the call should be refused")
}

fn open(session: &Arc<Session>, path: &str) -> u64 {
    ask(session, "project_open", json!({ "path": reference(path) }))["id"]
        .as_u64()
        .unwrap()
}

fn edit(session: &Arc<Session>, args: Value) -> Value {
    ask(session, "scene_edit", args)
}

fn sliced(session: &Arc<Session>, project: u64, plate: Option<u64>) {
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

/// The U1's usual process and filament, chosen for a project.
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

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is not a list"))
        .iter()
        .map(|number| number.as_f64().unwrap())
        .collect()
}

/// A copy's matrix in the scene, moved by `by` mm.
fn moved(placing: &Value, by: [f64; 3]) -> Value {
    let mut matrix = numbers(&placing["matrix"]);
    for axis in 0..3 {
        matrix[12 + axis] += by[axis];
    }
    json!({ "id": placing["id"], "matrix": matrix })
}

/// The G-code of the plate shown, without the line that says when it was made.
fn gcode(session: &Arc<Session>, project: u64, dir: &Path) -> String {
    let path = dir.join("out.gcode");
    ask(
        session,
        "export",
        json!({ "project": project, "path": path.to_string_lossy() }),
    );
    let text = std::fs::read_to_string(path).unwrap();
    text.split_once('\n').unwrap().1.to_owned()
}

#[test]
fn a_moved_object_is_sliced_where_it_stands_and_undo_puts_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    let project = open(&session, SMALL);
    let scene = ask(&session, "scene", json!({}));
    let placing = scene["placings"][0].clone();
    assert!(placing["id"].as_u64().is_some());
    assert_eq!(placing["over"], false);
    let detail = ask(&session, "project", json!({}));
    assert_eq!(detail["dirty"], false);
    assert_eq!(detail["edit"]["undo"], Value::Null);
    assert_eq!(
        refused(&session, "undo", json!({})),
        "there is nothing to undo"
    );

    sliced(&session, project, None);
    let before = gcode(&session, project, dir.path());
    let reach = numbers(&ask(&session, "toolpath", json!({}))["box"][0]);

    // Moved 12 mm along x and 7 along y: its slice is let go, the project
    // is unsaved, and there is something to undo.
    let answer = edit(
        &session,
        json!({ "op": "transform", "name": "Gizmo-Move", "copies": [moved(&placing, [12.0, 7.0, 0.0])] }),
    );
    assert_eq!(answer["done"], "Gizmo-Move");
    assert_eq!(answer["project"]["sliced"], false);
    assert_eq!(answer["project"]["dirty"], true);
    assert_eq!(answer["project"]["edit"]["undo"], "Gizmo-Move");
    assert_eq!(answer["project"]["edit"]["redo"], Value::Null);
    let now = &answer["scene"]["placings"][0];
    assert_eq!(now["id"], placing["id"]);
    assert_eq!(now["plate"], placing["plate"]);
    let (was, is) = (numbers(&placing["matrix"]), numbers(&now["matrix"]));
    assert!((is[12] - was[12] - 12.0).abs() < 1e-9 && (is[13] - was[13] - 7.0).abs() < 1e-9);
    // The object's mesh is the one the page has: nothing is sent again.
    assert_eq!(
        answer["scene"]["objects"][0]["mesh"],
        scene["objects"][0]["mesh"]
    );
    assert_eq!(answer["scene"]["key"], scene["key"]);
    assert_eq!(
        refused(&session, "summary", json!({})),
        "nothing has been sliced yet"
    );

    // Sliced again, the toolpaths are where the object now is.
    sliced(&session, project, None);
    let shifted = numbers(&ask(&session, "toolpath", json!({}))["box"][0]);
    assert!(
        (shifted[0] - reach[0] - 12.0).abs() < 0.01,
        "{shifted:?} {reach:?}"
    );
    assert!(
        (shifted[1] - reach[1] - 7.0).abs() < 0.01,
        "{shifted:?} {reach:?}"
    );
    assert_ne!(gcode(&session, project, dir.path()), before);

    // Taken back: the model is the one that was opened, and it slices to
    // the bytes it sliced to before.
    let undone = ask(&session, "undo", json!({}));
    assert_eq!(undone["done"], "Gizmo-Move");
    assert_eq!(undone["project"]["sliced"], false);
    assert_eq!(undone["project"]["dirty"], false);
    assert_eq!(undone["project"]["edit"]["undo"], Value::Null);
    assert_eq!(undone["project"]["edit"]["redo"], "Gizmo-Move");
    assert_eq!(undone["scene"]["placings"][0]["matrix"], placing["matrix"]);
    sliced(&session, project, None);
    assert_eq!(gcode(&session, project, dir.path()), before);

    // Made again, then something else done: there is nothing to redo.
    let redone = ask(&session, "redo", json!({}));
    assert_eq!(redone["scene"]["placings"][0]["matrix"], now["matrix"]);
    assert_eq!(redone["project"]["edit"]["undo"], "Gizmo-Move");
    assert_eq!(
        refused(&session, "redo", json!({})),
        "there is nothing to redo"
    );
    ask(&session, "undo", json!({}));
    let object = scene["objects"][0]["id"].clone();
    let renamed = edit(
        &session,
        json!({ "op": "rename", "object": object, "name": "bracket" }),
    );
    assert_eq!(renamed["scene"]["objects"][0]["name"], "bracket");
    assert_eq!(renamed["project"]["edit"]["redo"], Value::Null);
    assert_eq!(renamed["project"]["plates"][0]["objects"][0], "bracket");

    // A process setting set by hand is the project's, not the model's:
    // an edit leaves it, and taking the edit back does not take it back,
    // as the reference's undo history holds the model and not the presets.
    ask(&session, "project_set", json!({ "layer_height": 0.28 }));
    let moved_again = edit(
        &session,
        json!({ "op": "transform", "copies": [moved(&placing, [3.0, 0.0, 0.0])] }),
    );
    assert_eq!(moved_again["project"]["choices"]["layer_height"], 0.28);
    let undone = ask(&session, "undo", json!({}));
    assert_eq!(undone["done"], "Move Object");
    assert_eq!(undone["project"]["choices"]["layer_height"], 0.28);
    assert_eq!(undone["project"]["edit"]["undo"], "Rename Object");
    // And setting one is not a step of the history.
    ask(&session, "project_set", json!({ "layer_height": null }));
    assert_eq!(
        ask(&session, "project", json!({}))["edit"]["undo"],
        "Rename Object"
    );
    assert_eq!(
        ask(&session, "project", json!({}))["edit"]["redo"],
        "Move Object"
    );
    ask(&session, "redo", json!({}));
    ask(&session, "undo", json!({}));

    // What cannot be done changes nothing.
    assert!(
        refused(&session, "scene_edit", json!({ "op": "fold" })).contains("no edit named fold")
    );
    assert!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "delete", "copies": [0] })
        )
        .contains("no copy 0")
    );
    assert_eq!(
        refused(
            &session,
            "scene_edit",
            json!({ "op": "transform", "copies": [{ "id": placing["id"], "matrix": [1, 2, 3] }] })
        ),
        "matrix is not sixteen numbers"
    );
    assert_eq!(
        ask(&session, "project", json!({}))["edit"]["undo"],
        "Rename Object"
    );
    // Closing it asks first, for what was done to it.
    let close = ask(&session, "project_close", json!({ "id": project }));
    assert_eq!(close["ask"], "unsaved");
}

#[test]
fn only_the_plates_an_edit_touches_lose_their_slice() {
    let session = Arc::new(Session::new(None));
    let project = open(&session, THREE_PLATES);
    let scene = ask(&session, "scene", json!({}));
    let on = |plate: u64| -> Value {
        scene["placings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|placing| placing["plate"] == plate && placing["printable"] == true)
            .unwrap()
            .clone()
    };
    sliced(&session, project, Some(1));
    let plates = |detail: &Value| -> Vec<bool> {
        detail["plates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|plate| plate["sliced"] == true)
            .collect()
    };
    // Something moves on plate 2: plate 1 keeps its slice.
    let answer = edit(
        &session,
        json!({ "op": "transform", "copies": [moved(&on(2), [1.0, 0.0, 0.0])] }),
    );
    assert_eq!(answer["done"], "Move Object");
    assert_eq!(plates(&answer["project"]), [true, false, false]);
    // A name is not sliced: plate 1 keeps its slice.
    let first = on(1);
    let object = scene["objects"][first["object"].as_u64().unwrap() as usize]["id"].clone();
    let answer = edit(
        &session,
        json!({ "op": "rename", "object": object, "name": "renamed" }),
    );
    assert_eq!(plates(&answer["project"]), [true, false, false]);
    // Not to be printed: plate 1's slice is let go; so is taking that back.
    let answer = edit(
        &session,
        json!({ "op": "printable", "copies": [first["id"]], "value": false }),
    );
    assert_eq!(answer["done"], "Set Unprintable");
    assert_eq!(plates(&answer["project"]), [false, false, false]);
    let shown: Vec<&Value> = answer["scene"]["placings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|placing| placing["id"] == first["id"])
        .collect();
    assert_eq!(shown[0]["printable"], false);
    let names = answer["project"]["plates"][0]["objects"]
        .as_array()
        .unwrap()
        .len();
    let undone = ask(&session, "undo", json!({}));
    assert_eq!(
        undone["project"]["plates"][0]["objects"]
            .as_array()
            .unwrap()
            .len(),
        names + 1
    );
}

#[test]
fn objects_are_deleted_and_given_another_filament() {
    let session = Arc::new(Session::new(None));
    open(&session, THREE_PLATES);
    let scene = ask(&session, "scene", json!({}));
    let placings = scene["placings"].as_array().unwrap();
    let objects = scene["objects"].as_array().unwrap();
    let detail = ask(&session, "project", json!({}));
    let count = detail["filament_count"].as_u64().unwrap();
    // Another filament for the first object: its mesh is another, since
    // its colour is; the others' are the ones the page has.
    let other = if objects[0]["filament"] == 1 { 2 } else { 1 };
    let answer = edit(
        &session,
        json!({ "op": "filament", "object": objects[0]["id"], "filament": other }),
    );
    assert_eq!(answer["done"], "Change Filament");
    assert_eq!(answer["scene"]["objects"][0]["filament"], other);
    assert_ne!(answer["scene"]["objects"][0]["mesh"], objects[0]["mesh"]);
    assert_eq!(answer["scene"]["objects"][1]["mesh"], objects[1]["mesh"]);
    assert!(answer["project"]["filament_count"].as_u64().unwrap() >= count.min(other));
    ask(&session, "undo", json!({}));

    // Every copy of the first object deleted: the object goes.
    let copies: Vec<&Value> = placings
        .iter()
        .filter(|placing| placing["object"] == 0)
        .map(|placing| &placing["id"])
        .collect();
    let answer = edit(&session, json!({ "op": "delete", "copies": copies }));
    assert_eq!(answer["done"], "Delete selected");
    assert_eq!(
        answer["scene"]["objects"].as_array().unwrap().len(),
        objects.len() - 1
    );
    assert_eq!(answer["scene"]["objects"][0]["id"], objects[1]["id"]);
    assert_eq!(
        answer["scene"]["placings"].as_array().unwrap().len(),
        placings.len() - copies.len()
    );
    let undone = ask(&session, "undo", json!({}));
    assert_eq!(undone["scene"]["placings"], scene["placings"]);
    assert_eq!(undone["project"]["dirty"], false);
}

#[test]
fn plates_are_added_and_taken_away_and_an_object_goes_with_the_plate_it_is_moved_to() {
    let session = Arc::new(Session::new(None));
    let project = open(&session, BARE);
    choose_presets(&session, project);
    let scene = ask(&session, "scene", json!({}));
    assert_eq!(scene["bare"], true);
    assert_eq!(scene["plates"].as_array().unwrap().len(), 0);
    let placing = scene["placings"][0].clone();
    let stood = numbers(&placing["box"][0]);

    // A plate more: the model now has a place of its own, where it stood,
    // on plate 1, and the new plate is the one shown.
    let answer = edit(&session, json!({ "op": "add_plate" }));
    assert_eq!(answer["done"], "add partplate");
    assert_eq!(answer["scene"]["bare"], false);
    assert_eq!(answer["project"]["plate"], 2);
    let plates = answer["scene"]["plates"].as_array().unwrap();
    assert_eq!(plates.len(), 2);
    assert_eq!(numbers(&plates[1]["origin"]), [324.0, 0.0]);
    let now = &answer["scene"]["placings"][0];
    assert_eq!(now["plate"], 1);
    for (was, is) in stood.iter().zip(numbers(&now["box"][0])) {
        assert!((was - is).abs() < 2e-3, "{stood:?} {now}");
    }
    assert_eq!(answer["project"]["plates"][0]["filaments"], json!([1]));
    assert_eq!(answer["project"]["plates"][1]["filaments"], json!([]));

    // Moved across to plate 2, it is plate 2's.
    let across = edit(
        &session,
        json!({ "op": "transform", "name": "Gizmo-Move", "copies": [moved(now, [324.0, 0.0, 0.0])] }),
    );
    assert_eq!(across["scene"]["placings"][0]["plate"], 2);
    assert_eq!(across["project"]["plates"][0]["filaments"], json!([]));
    assert_eq!(across["project"]["plates"][1]["filaments"], json!([1]));
    assert_eq!(
        refused(&session, "slice", json!({ "plate": 1 })),
        "plate 1 has nothing printable on it"
    );
    // Half off the plate, it is in the way; off every plate, it is on none.
    let over = edit(
        &session,
        json!({ "op": "transform", "copies": [moved(&across["scene"]["placings"][0], [0.0, 135.0, 0.0])] }),
    );
    assert_eq!(over["scene"]["placings"][0]["plate"], 2);
    assert_eq!(over["scene"]["placings"][0]["over"], true);
    assert!(
        refused(&session, "slice", json!({ "plate": 2 }))
            .starts_with("An object is laid over the boundary of plate")
    );
    let off = edit(
        &session,
        json!({ "op": "transform", "copies": [moved(&over["scene"]["placings"][0], [0.0, 200.0, 0.0])] }),
    );
    assert_eq!(off["scene"]["placings"][0]["plate"], Value::Null);
    assert_eq!(off["project"]["plates_with_objects"], 0);
    ask(&session, "undo", json!({}));
    ask(&session, "undo", json!({}));

    // Plate 2 taken away: what stood on it is kept, beside the plate left.
    let gone = edit(&session, json!({ "op": "delete_plate", "plate": 2 }));
    assert_eq!(gone["done"], "delete partplate");
    assert_eq!(gone["project"]["plate"], 1);
    assert_eq!(gone["scene"]["plates"].as_array().unwrap().len(), 1);
    assert_eq!(gone["scene"]["placings"].as_array().unwrap().len(), 1);
    assert_eq!(gone["scene"]["placings"][0]["plate"], Value::Null);
    assert_eq!(
        refused(&session, "scene_edit", json!({ "op": "delete_plate" })),
        "the last plate cannot be removed"
    );
    // Everything taken back: the model as it was opened.
    for _ in 0..3 {
        ask(&session, "undo", json!({}));
    }
    let detail = ask(&session, "project", json!({}));
    assert_eq!(detail["dirty"], false);
    assert_eq!(detail["plate"], Value::Null);
    assert_eq!(detail["plates"], json!([]));
    assert_eq!(
        ask(&session, "scene", json!({}))["placings"],
        scene["placings"]
    );
}

#[test]
fn geometry_is_added_to_the_project_shown_cloned_and_arranged() {
    let session = Arc::new(Session::new(None));
    let project = ask(&session, "projects", json!({}))["active"]
        .as_u64()
        .unwrap();
    choose_presets(&session, project);
    assert_eq!(
        refused(&session, "scene_edit", json!({ "op": "add" })),
        "path was not given"
    );
    // Into the empty project: in the middle of its bed, on a plate.
    let added = edit(&session, json!({ "op": "add", "path": reference(BARE) }));
    assert_eq!(added["done"], "Import Object");
    assert_eq!(added["project"]["empty"], false);
    assert_eq!(added["project"]["name"], "Untitled");
    assert_eq!(added["project"]["dirty"], true);
    assert_eq!(added["project"]["plate"], 1);
    assert_eq!(
        added["scene"]["objects"][0]["name"],
        "OrcaToleranceTest.stl"
    );
    let first = &added["scene"]["placings"][0];
    assert_eq!(added["made"], json!([first["id"]]));
    let reach = [numbers(&first["box"][0]), numbers(&first["box"][1])];
    // The bed of the U1 runs from (0.5, 1) to (270.5, 271).
    assert!(
        ((reach[0][0] + reach[1][0]) / 2.0 - 135.5).abs() < 1e-3,
        "{reach:?}"
    );
    assert!(
        ((reach[0][1] + reach[1][1]) / 2.0 - 136.0).abs() < 1e-3,
        "{reach:?}"
    );
    assert_eq!(reach[0][2], 0.0);

    // The same again, and a clone: three objects, none on another.
    edit(&session, json!({ "op": "add", "path": reference(BARE) }));
    let cloned = edit(&session, json!({ "op": "clone", "copies": [first["id"]] }));
    assert_eq!(cloned["done"], "Clone");
    let boxes = |scene: &Value| -> Vec<[Vec<f64>; 2]> {
        scene["placings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|placing| [numbers(&placing["box"][0]), numbers(&placing["box"][1])])
            .collect()
    };
    let apart = |boxes: &[[Vec<f64>; 2]]| {
        boxes.iter().enumerate().all(|(at, a)| {
            boxes[at + 1..].iter().all(|b| {
                a[1][0] < b[0][0] || b[1][0] < a[0][0] || a[1][1] < b[0][1] || b[1][1] < a[0][1]
            })
        })
    };
    let stand = boxes(&cloned["scene"]);
    assert_eq!(stand.len(), 3);
    assert!(apart(&stand), "{stand:?}");
    assert_eq!(
        cloned["scene"]["objects"][2]["mesh"],
        cloned["scene"]["objects"][0]["mesh"]
    );
    assert!(
        cloned["scene"]["placings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|placing| placing["plate"] == 1)
    );

    let arranged = edit(&session, json!({ "op": "arrange", "all": true }));
    assert_eq!(arranged["done"], "Arrange");
    assert!(apart(&boxes(&arranged["scene"])));
    // What was put together here slices like anything else.
    sliced(&session, project, Some(1));
    assert_eq!(ask(&session, "project", json!({}))["sliced"], true);
    // Taken all the way back, the project is empty again.
    for _ in 0..4 {
        ask(&session, "undo", json!({}));
    }
    let detail = ask(&session, "project", json!({}));
    assert_eq!(detail["empty"], true);
    assert_eq!(detail["dirty"], false);
    assert_eq!(detail["edit"]["redo"], "Import Object");
}
