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

fn sliced(session: &Arc<Session>) {
    ask(session, "slice", Value::Null);
    loop {
        let status = ask(session, "status", Value::Null);
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn exported(session: &Arc<Session>, name: &str) -> String {
    let out = std::env::temp_dir().join(format!("edge-ticks-{}-{name}.gcode", std::process::id()));
    ask(session, "export", json!({ "path": out }));
    let text = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_file(&out).ok();
    text
}

/// A pause, custom G-code and their menus' edits, each an edit of the
/// project as the reference makes it: unsaved, the plate's slice let go,
/// sliced again with the item where it was set; saved and opened again;
/// left as it is by undo.
#[test]
fn items_set_at_layers_are_sliced_saved_and_kept_through_undo() {
    let session = Arc::new(Session::new(None));
    ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    );
    sliced(&session);
    let codes = ask(&session, "layer_codes", Value::Null);
    assert_eq!(codes["items"], json!([]));
    assert_eq!(codes["sequential"], false);
    let tops = ask(&session, "toolpath", Value::Null)["layers"]["z"].clone();
    let z = |at: usize| tops[at].as_f64().unwrap();

    // A pause at the fifth layer, given by its height as the slider shows it.
    let set = ask(
        &session,
        "layer_code_set",
        json!({ "print_z": z(4), "kind": "pause" }),
    );
    assert_eq!(set["items"][0]["kind"], "pause");
    assert!((set["items"][0]["print_z"].as_f64().unwrap() - z(4)).abs() < 1e-3);
    assert_eq!(set["detail"]["dirty"], true);
    assert_eq!(
        set["detail"]["sliced"], false,
        "the plate's slice is let go"
    );
    // Nothing else is added where something is.
    let refused = session
        .call(
            "layer_code_set",
            &json!({ "print_z": z(4), "kind": "pause" }),
        )
        .unwrap_err();
    assert!(refused.contains("already"), "{refused}");
    // Not at a height no layer is at, once sliced; custom G-code needs text.
    sliced(&session);
    assert!(
        session
            .call(
                "layer_code_set",
                &json!({ "print_z": z(4) + 0.05, "kind": "pause" })
            )
            .is_err()
    );
    assert!(
        session
            .call(
                "layer_code_set",
                &json!({ "print_z": z(9), "kind": "custom", "extra": "" })
            )
            .is_err()
    );
    ask(
        &session,
        "layer_code_set",
        json!({ "print_z": z(9), "kind": "custom", "extra": "M117 one" }),
    );
    // "Edit Custom G-code": the same layer, the text replaced.
    let edited = ask(
        &session,
        "layer_code_set",
        json!({ "print_z": z(9), "kind": "custom", "extra": "M117 two\nG4 S1" }),
    );
    assert_eq!(edited["items"].as_array().unwrap().len(), 2);
    assert_eq!(edited["items"][1]["extra"], "M117 two\nG4 S1");
    sliced(&session);
    let text = exported(&session, "items");
    assert_eq!(text.matches(";PAUSE_PRINT").count(), 1);
    assert!(text.contains(";CUSTOM_GCODE\nM117 two\nG4 S1\n"));
    assert!(!text.contains("M117 one"));
    // A plate of one filament in a project of one: no filament change.
    let codes = ask(&session, "layer_codes", Value::Null);
    if codes["colours"].as_array().unwrap().len() < 2 {
        assert_eq!(codes["change_filament"], Value::Null);
        assert!(
            session
                .call(
                    "layer_code_set",
                    &json!({ "print_z": z(6), "kind": "filament", "extruder": 2 })
                )
                .is_err()
        );
    }

    // Undo takes back the plate added and leaves the items, and so does
    // redo.
    ask(&session, "scene_edit", json!({ "op": "add_plate" }));
    ask(&session, "undo", Value::Null);
    assert_eq!(
        ask(&session, "layer_codes", Value::Null)["items"],
        edited["items"]
    );
    ask(&session, "redo", Value::Null);
    assert_eq!(
        ask(&session, "layer_codes", Value::Null)["items"],
        edited["items"]
    );
    // An item set after an edit stays when the edit is taken back.
    let third = ask(
        &session,
        "layer_code_set",
        json!({ "print_z": z(2), "kind": "custom", "extra": "M117 three" }),
    );
    ask(&session, "undo", Value::Null);
    assert_eq!(
        ask(&session, "layer_codes", Value::Null)["items"],
        third["items"]
    );
    let edited = ask(&session, "layer_code_delete", json!({ "print_z": z(2) }));
    assert_eq!(
        ask(&session, "layer_codes", Value::Null)["items"],
        edited["items"]
    );

    // "Delete Pause".
    let deleted = ask(&session, "layer_code_delete", json!({ "print_z": z(4) }));
    assert_eq!(deleted["items"].as_array().unwrap().len(), 1);
    assert!(
        session
            .call("layer_code_delete", &json!({ "print_z": z(4) }))
            .is_err()
    );

    // Saved and opened again in another session.
    let out = std::env::temp_dir().join(format!("edge-ticks-{}.3mf", std::process::id()));
    let saved = ask(&session, "project_save", json!({ "path": out }));
    assert_eq!(saved["saved"], true, "{saved}");
    let other = Arc::new(Session::new(None));
    ask(&other, "project_open", json!({ "path": out }));
    let again = ask(&other, "layer_codes", Value::Null);
    assert_eq!(again["items"], deleted["items"]);
    std::fs::remove_file(&out).ok();
}
