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

    // "Use surface": the part is cut from the object's top, from 0.015 mm
    // under it to the full depth over it (the flat text stops 0.015 mm
    // short of that), following the steps of the tolerance test's top.
    let text_mesh = |session: &Arc<Session>| {
        let state = lock(&session.state);
        let file = state.projects[0].file.clone().unwrap();
        let mesh = file.model.objects()[0].shape().parts()[1].mesh.clone();
        mesh.bounds().unwrap()
    };
    let (_, high) = text_mesh(&session);
    assert!((high[2] - 0.985).abs() < 1e-5, "{high:?}");
    let mut surface = style(&json!(0), "Hello");
    surface["use_surface"] = json!(true);
    let on = |surface: &Value| {
        ask(
            &session,
            "text_edit",
            json!({ "op": "set", "object": object, "part": part, "copy": copy, "kind": "part", "merge": true, "style": surface }),
        )
    };
    let _ = on(&surface);
    let (low, high) = text_mesh(&session);
    assert!(
        high[2] > 1.0 - 1e-5 && low[2] < -0.015 + 1e-5,
        "{low:?} {high:?}"
    );
    let info = ask(
        &session,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["style"]["use_surface"], true);
    // A keystroke more: the surface kept from the change before is used.
    let kept = lock(&session.fonts)
        .surface
        .as_ref()
        .map(|(_, s)| Arc::as_ptr(s));
    surface["text"] = json!("Hello!");
    let _ = on(&surface);
    let now = lock(&session.fonts)
        .surface
        .as_ref()
        .map(|(_, s)| Arc::as_ptr(s));
    assert!(kept.is_some() && kept == now);
    // Turned off, flat again.
    surface["use_surface"] = json!(false);
    surface["text"] = json!("Hello");
    let _ = on(&surface);
    let (_, high) = text_mesh(&session);
    assert!((high[2] - 0.985).abs() < 1e-5, "{high:?}");

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

/// "Use surface" turned on for a text standing off the surface moves it
/// onto the surface first (`calc_surface_offset`); an object of one text
/// has no other surface, and stays flat (`is_the_only_one_part`).
#[test]
fn use_surface_moves_the_text_onto_the_surface_and_not_for_a_lone_text() {
    let lone = session();
    let session = session();
    ask(&session, "project_open", json!({ "path": reference(BARE) }));
    let scene = ask(&session, "scene", json!({}));
    let object = scene["objects"][0]["id"].clone();
    let copy = scene["placings"][0]["id"].clone();
    let reach = &scene["placings"][0]["box"];
    let middle: Vec<f64> = (0..2)
        .map(|i| (reach[0][i].as_f64().unwrap() + reach[1][i].as_f64().unwrap()) / 2.0)
        .collect();
    let added = ask(
        &session,
        "text_edit",
        json!({
            "op": "add", "object": object, "copy": copy, "kind": "part",
            "origin": [middle[0], middle[1], 100.0], "direction": [0.0, 0.0, -1.0],
            "style": style(&json!(0), "Up"),
        }),
    );
    let part = added["part"].clone();
    // 2 mm over the top ("From surface").
    let mut away = style(&json!(0), "Up");
    away["distance"] = json!(2.0);
    ask(
        &session,
        "text_edit",
        json!({ "op": "set", "object": object, "part": part, "copy": copy, "kind": "part", "style": away }),
    );
    let info = ask(
        &session,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert!(
        (info["distance"].as_f64().unwrap() - 2.0).abs() < 1e-6,
        "{info}"
    );
    let mut on = style(&json!(0), "Up");
    on["use_surface"] = json!(true);
    on["distance"] = json!(2.0);
    ask(
        &session,
        "text_edit",
        json!({ "op": "set", "object": object, "part": part, "copy": copy, "kind": "part", "style": on }),
    );
    // Its origin is on the surface now: no distance left.
    let info = ask(
        &session,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["distance"], Value::Null, "{info}");
    assert_eq!(info["style"]["use_surface"], true);

    // A text that is an object of its own: "Use surface" is not used.
    let added = ask(
        &lone,
        "text_edit",
        json!({ "op": "add", "style": style(&json!(0), "Tag") }),
    );
    let object = added["scene"]["objects"][0]["id"].clone();
    let part = added["part"].clone();
    let mut on = style(&json!(0), "Tag");
    on["use_surface"] = json!(true);
    ask(
        &lone,
        "text_edit",
        json!({ "op": "set", "object": object, "part": part, "kind": "part", "style": on }),
    );
    let info = ask(
        &lone,
        "text_part",
        json!({ "object": object, "part": part }),
    );
    assert_eq!(info["style"]["use_surface"], false);
}
