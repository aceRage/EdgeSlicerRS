use std::path::Path;

use super::*;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";

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

/// An event's answer: its JSON, and how many triangles its blocks hold.
fn event(session: &Arc<Session>, args: Value) -> (Value, usize) {
    let bytes = session
        .call_bytes("paint_event", &args)
        .unwrap_or_else(|error| panic!("paint_event {args}: {error}"));
    let length = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let answer: Value = serde_json::from_slice(&bytes[4..4 + length]).unwrap();
    let mut at = (4 + length).next_multiple_of(4);
    let mut triangles = 0;
    while at + 12 <= bytes.len() {
        let count = u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
        at += 12 + count * 48;
        triangles += count;
    }
    assert_eq!(at, bytes.len(), "the blocks fill the answer");
    (answer, triangles)
}

/// The first copy of a project's scene: its id, matrix and box.
fn first_copy(session: &Arc<Session>, project: u64) -> (u64, Value, [[f64; 3]; 2]) {
    let scene = ask(session, "scene", json!({ "project": project }));
    let copy = &scene["placings"][0];
    let corner = |at: usize| -> [f64; 3] {
        std::array::from_fn(|axis| copy["box"][at][axis].as_f64().unwrap())
    };
    (
        copy["id"].as_u64().unwrap(),
        copy["matrix"].clone(),
        [corner(0), corner(1)],
    )
}

#[test]
fn a_stroke_is_painted_kept_undone_and_redone() {
    let session = Arc::new(Session::new(None));
    let project = ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    )["id"]
        .as_u64()
        .unwrap();
    let (copy, matrix, [low, high]) = first_copy(&session, project);
    let opened = ask(
        &session,
        "paint_open",
        json!({ "project": project, "copy": copy, "kind": "seam" }),
    );
    assert!(!opened["parts"].as_array().unwrap().is_empty());
    // The unpainted mesh: as many triangles as the parts have.
    let mesh = session.call_bytes("paint_mesh", &json!({})).unwrap();
    let count = u32::from_le_bytes(mesh[0..4].try_into().unwrap()) as usize;
    let listed: u64 = opened["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| part["triangles"].as_u64().unwrap())
        .sum();
    assert_eq!(count as u64, listed);
    assert_eq!(mesh.len(), 8 + count * 48);

    // Straight down onto the middle of the copy's top.
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
    let base = json!({ "project": project, "matrix": matrix, "eye": [middle[0], middle[1], high[2] + 100.0] });
    let with = |more: Value| {
        let mut args = base.clone();
        args.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        args
    };
    let ray = json!([middle[0], middle[1], high[2] + 50.0, 0.0, 0.0, -1.0]);
    let (hover, _) = event(
        &session,
        with(json!({ "op": "hover", "ray": ray, "tool": "circle" })),
    );
    assert!(hover["hit"].is_object(), "{hover}");
    let (painted, triangles) = event(
        &session,
        with(json!({ "op": "paint", "rays": [ray], "state": 1, "tool": "sphere", "radius": 2.0 })),
    );
    assert!(triangles > 0, "{painted}");
    assert_eq!(painted["states"], json!([1]));
    // A drag: a ray a few millimetres on, and the one before.
    let ray2 = json!([middle[0] + 3.0, middle[1], high[2] + 50.0, 0.0, 0.0, -1.0]);
    event(
        &session,
        with(
            json!({ "op": "paint", "rays": [ray2, ray], "state": 1, "tool": "circle", "radius": 1.0 }),
        ),
    );
    let (done, _) = event(
        &session,
        with(json!({ "op": "commit", "name": "Enforce seam" })),
    );
    assert_eq!(done["done"], "Enforce seam");
    let detail = ask(&session, "project", json!({ "project": project }));
    assert_eq!(detail["edit"]["undo"], "Enforce seam");
    assert_eq!(detail["dirty"], true);

    // Nothing more painted: no step.
    let (none, _) = event(
        &session,
        with(json!({ "op": "commit", "name": "Enforce seam" })),
    );
    assert!(none["done"].is_null());

    // Undone, the tool reads the model again: nothing painted.
    ask(&session, "undo", json!({ "project": project }));
    let (back, _) = event(&session, with(json!({ "op": "refresh" })));
    assert_eq!(back["all"], true);
    assert_eq!(back["states"], json!([]));
    ask(&session, "redo", json!({ "project": project }));
    let (again, triangles) = event(&session, with(json!({ "op": "refresh" })));
    assert_eq!(again["states"], json!([1]));
    assert!(triangles > 0);

    // "Erase all painting": one step, "Reset selection".
    let (erased, _) = event(
        &session,
        with(json!({ "op": "erase_all", "name": "Reset selection" })),
    );
    assert_eq!(erased["done"], "Reset selection");
    assert_eq!(erased["states"], json!([]));
    ask(&session, "paint_close", json!({}));
    assert!(
        session
            .call_bytes("paint_event", &with(json!({ "op": "refresh" })))
            .is_err()
    );
}

#[test]
fn supports_painted_say_what_the_engine_does_with_them() {
    let session = Arc::new(Session::new(None));
    let project = ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    )["id"]
        .as_u64()
        .unwrap();
    let (copy, matrix, [low, high]) = first_copy(&session, project);
    ask(
        &session,
        "paint_open",
        json!({ "project": project, "copy": copy, "kind": "supports" }),
    );
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
    let args = |op: Value| {
        let mut args = json!({ "project": project, "matrix": matrix, "eye": [middle[0], middle[1], high[2] + 100.0] });
        args.as_object_mut()
            .unwrap()
            .extend(op.as_object().unwrap().clone());
        args
    };
    let ray = json!([middle[0], middle[1], high[2] + 50.0, 0.0, 0.0, -1.0]);
    // A smart fill of the face under the pointer, marked as the pointer
    // hovers and painted as the button goes down.
    let (marked, _) = event(
        &session,
        args(json!({ "op": "hover", "ray": ray, "tool": "fill", "angle": 30.0 })),
    );
    assert_eq!(marked["marked"], true);
    event(
        &session,
        args(json!({ "op": "paint", "rays": [ray], "state": 2, "tool": "fill", "angle": 30.0 })),
    );
    let (done, _) = event(
        &session,
        args(json!({ "op": "commit", "name": "Block supports" })),
    );
    assert_eq!(done["done"], "Block supports");
    assert!(
        done["note"]
            .as_str()
            .unwrap()
            .contains("supports painted on it"),
        "{done}"
    );
    // A state the tool does not paint is refused.
    assert!(
        session
            .call_bytes(
                "paint_event",
                &args(json!({ "op": "paint", "rays": [ray], "state": 9 }))
            )
            .is_err()
    );
}
