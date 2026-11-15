use std::path::Path;

use super::*;

const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
const BENCHY: &str = "resources/handy_models/3DBenchy.3mf";

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

fn open(session: &Arc<Session>, path: &str) -> u64 {
    ask(session, "project_open", json!({ "path": reference(path) }))["id"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_simplified_part_is_shown_applied_as_one_step_and_undone() {
    let session = Arc::new(Session::new(None));
    open(&session, BARE);
    let scene = ask(&session, "scene", json!({}));
    let object = &scene["objects"][0];
    let id = object["id"].clone();
    let before = object["triangles"].as_u64().unwrap();
    // A closed mesh read as it is: no warning.
    assert_eq!(object["errors"], Value::Null);
    assert_eq!(object["part_list"][0]["errors"], Value::Null);

    // "Detail level" "Low" (0.5).
    let made = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "max_error": 0.5 }),
    );
    assert_eq!(made["original"].as_u64(), Some(before));
    let after = made["triangles"].as_u64().unwrap();
    assert!(after < before, "{after} of {before}");
    let progress = ask(&session, "mesh_progress", json!({}));
    assert_eq!(progress["running"], false);
    assert_eq!(progress["id"], made["id"]);
    let bytes = session
        .call_bytes("mesh_result", &json!({ "id": made["id"] }))
        .unwrap();
    assert!(!bytes.is_empty());

    // "Decimate ratio" 50 %: the later one takes the place of the first.
    let half = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "wanted": before / 2 }),
    );
    assert!(half["triangles"].as_u64().unwrap() <= before / 2);
    assert!(
        session
            .call("mesh_apply", &json!({ "id": made["id"] }))
            .is_err()
    );

    let applied = ask(&session, "mesh_apply", json!({ "id": half["id"] }));
    assert_eq!(applied["done"], "Simplify OrcaToleranceTest.stl");
    assert_eq!(
        applied["project"]["edit"]["undo"],
        "Simplify OrcaToleranceTest.stl"
    );
    assert_eq!(applied["note"], Value::Null);
    assert_eq!(
        applied["scene"]["objects"][0]["triangles"],
        half["triangles"]
    );
    // Kept no longer: put in once.
    assert!(
        session
            .call("mesh_apply", &json!({ "id": half["id"] }))
            .is_err()
    );

    let undone = ask(&session, "undo", json!({}));
    assert_eq!(
        undone["scene"]["objects"][0]["triangles"].as_u64(),
        Some(before)
    );
}

#[test]
fn a_result_made_before_an_edit_is_not_put_in_after_it() {
    let session = Arc::new(Session::new(None));
    open(&session, BARE);
    let scene = ask(&session, "scene", json!({}));
    let id = scene["objects"][0]["id"].clone();
    let made = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "max_error": 1.0 }),
    );
    ask(
        &session,
        "scene_edit",
        json!({ "op": "rename", "object": id, "name": "other" }),
    );
    let refused = session
        .call("mesh_apply", &json!({ "id": made["id"] }))
        .unwrap_err();
    assert!(refused.contains("changed meanwhile"), "{refused}");
}

#[test]
fn a_simplify_is_called_off() {
    let session = Arc::new(Session::new(None));
    open(&session, BENCHY);
    let scene = ask(&session, "scene", json!({}));
    let id = scene["objects"][0]["id"].clone();
    let worker = {
        let session = session.clone();
        let id = id.clone();
        std::thread::spawn(move || {
            session.call("mesh_simplify", &json!({ "object": id, "wanted": 0 }))
        })
    };
    // Called off once it is under way.
    loop {
        let progress = ask(&session, "mesh_progress", json!({}));
        if progress["running"] == true {
            ask(&session, "mesh_cancel", json!({}));
            break;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(worker.join().unwrap().unwrap_err(), "stopped");
    assert_eq!(ask(&session, "mesh_progress", json!({}))["running"], false);
}
