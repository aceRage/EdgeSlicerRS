use std::path::Path;

use super::*;

const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";

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

/// A session that knows no system fonts: only the bundled ones.
fn session() -> Arc<Session> {
    let session = Arc::new(Session::new(None));
    session.use_font_dirs(Vec::new());
    session
}

fn style(face: &Value, text: &str) -> Value {
    json!({
        "face": face, "text": text, "size": 10.0, "depth": 1.0,
        "char_gap": 0, "line_gap": 0, "boldness": 0.0, "skew": 0.0,
        "horizontal": "center", "vertical": "middle",
    })
}

#[test]
fn the_fonts_offered_are_the_licensed_bundled_ones_and_the_systems() {
    let session = session();
    let fonts = ask(&session, "text_fonts", json!({}));
    let families = fonts["families"].as_array().unwrap();
    let names: Vec<&str> = families
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();
    // HarmonyOS Sans SC and Noto Sans KR have no licence beside them.
    assert_eq!(names, ["NanumGothic"]);
    assert_eq!(families[0]["bundled"], true);
    let styles: Vec<&str> = families[0]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face["style"].as_str().unwrap())
        .collect();
    assert_eq!(styles, ["Regular", "Bold"]);
    assert_eq!(fonts["default"], 0);
    assert_eq!(fonts["style"]["text"], "Embossed text");
    // No file's place is given to the page.
    assert!(!fonts.to_string().contains(".ttf"));
    let preview = ask(&session, "text_font_preview", json!({ "face": 0 }));
    assert!(preview["d"].as_str().unwrap().starts_with('M'));
    assert!(preview["box"][2].as_f64().unwrap() > 10.0);
}

#[test]
fn a_font_folder_with_what_is_not_a_font_is_read_past() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.ttf"), b"not a font").unwrap();
    std::fs::copy(
        reference("resources/fonts/NanumGothic-Bold.ttf"),
        dir.path().join("Copy.ttf"),
    )
    .unwrap();
    let list = find_fonts(&[dir.path().to_owned()], None);
    assert_eq!(list.faces.len(), 1);
    assert_eq!(list.skipped.len(), 1);
}

#[test]
fn a_text_is_added_on_an_object_edited_as_one_step_and_saved() {
    let session = session();
    ask(&session, "project_open", json!({ "path": reference(BARE) }));
    let scene = ask(&session, "scene", json!({}));
    let object = scene["objects"][0]["id"].clone();
    let copy = scene["placings"][0]["id"].clone();
    let reach = &scene["placings"][0]["box"];
    let middle: Vec<f64> = (0..2)
        .map(|i| (reach[0][i].as_f64().unwrap() + reach[1][i].as_f64().unwrap()) / 2.0)
        .collect();
    // A line of sight down onto the object's top.
    let added = ask(
        &session,
        "text_edit",
        json!({
            "op": "add", "object": object, "copy": copy, "kind": "part",
            "origin": [middle[0], middle[1], 100.0], "direction": [0.0, 0.0, -1.0],
            "style": style(&json!(0), "Hello"),
        }),
    );
    assert_eq!(added["done"], "Add Emboss text Volume");
    let part = added["part"].clone();
    let parts = added["scene"]["objects"][0]["part_list"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[1]["text"], true);
    assert_eq!(parts[1]["name"], "Hello");
    let info = ask(
        &session,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["style"]["text"], "Hello");
    assert_eq!(info["face"], 0);
    assert_eq!(info["kind"], "part");
    assert!(info["angle"].as_f64().unwrap().abs() < 1e-6);
    assert_eq!(info["not_yet"], Value::Null);

    // Two changes while the tool is open are one step.
    let edit = |text: &str, merge: bool, kind: &str| {
        ask(
            &session,
            "text_edit",
            json!({ "op": "set", "object": object, "part": part, "copy": copy, "kind": kind, "merge": merge, "style": style(&json!(1), text) }),
        )
    };
    edit("Hello w", false, "part");
    let last = edit("Hello world", true, "negative");
    assert_eq!(last["project"]["edit"]["undo"], "Emboss attribute change");
    let info = ask(
        &session,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["style"]["text"], "Hello world");
    assert_eq!(info["face"], 1);
    assert_eq!(info["kind"], "negative");
    let undone = ask(&session, "undo", json!({}));
    assert_eq!(undone["project"]["edit"]["undo"], "Add Emboss text Volume");

    // Not made here yet: said, nothing changed.
    let mut surface = style(&json!(0), "x");
    surface["use_surface"] = json!(true);
    let refused = session.call(
        "text_edit",
        &json!({ "op": "set", "object": object, "part": part, "style": surface }),
    );
    assert!(refused.unwrap_err().contains("Use surface"));

    // Saved and opened again, it is the same text in the same font.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text.3mf");
    let saved = ask(
        &session,
        "project_save",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(saved["saved"], true, "{saved}");
    let again = Arc::new(Session::new(None));
    again.use_font_dirs(Vec::new());
    ask(
        &again,
        "project_open",
        json!({ "path": path.to_string_lossy() }),
    );
    let scene = ask(&again, "scene", json!({}));
    let object = scene["objects"][0]["id"].clone();
    let part = scene["objects"][0]["part_list"][1]["id"].clone();
    let info = ask(
        &again,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["style"]["text"], "Hello");
    assert_eq!(info["face"], 0);
}

#[test]
fn a_text_with_no_object_is_an_object_on_the_plate() {
    let session = session();
    let added = ask(
        &session,
        "text_edit",
        json!({ "op": "add", "style": style(&json!(0), "Tag") }),
    );
    assert_eq!(added["done"], "Add Emboss text object");
    assert_eq!(added["scene"]["objects"][0]["name"], "Tag");
    assert_eq!(added["made"].as_array().unwrap().len(), 1);
}
