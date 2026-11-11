use std::path::Path;

use super::*;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
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
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn refused(session: &Arc<Session>, name: &str, args: Value) -> String {
    session
        .call(name, &args)
        .expect_err("the call should be refused")
}

fn wait(session: &Arc<Session>, project: u64) -> Value {
    loop {
        let status = ask(session, "status", json!({ "project": project }));
        if status["state"] != "slicing" {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// A project opened and put on the U1's usual presets, as the page does.
fn on_presets(session: &Arc<Session>) -> u64 {
    let id = ask(session, "project_open", json!({ "path": reference(SMALL) }))["id"]
        .as_u64()
        .unwrap();
    let presets = ask(
        session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": MACHINE }),
    );
    ask(
        session,
        "project_set",
        json!({
            "project": id, "source": "presets", "vendor": "Snapmaker", "machine": MACHINE,
            "process": presets["processes"][0], "filaments": [presets["filaments"][0]],
            "initial": true,
        }),
    );
    id
}

fn listed(answer: &Value, list: &str, key: &str) -> bool {
    answer[list]
        .as_array()
        .unwrap()
        .iter()
        .any(|listed| listed == key)
}

#[test]
fn the_pages_are_the_references_in_its_order_and_words() {
    let pages = ask(&Arc::new(Session::new(None)), "process_pages", Value::Null);
    let titles: Vec<&str> = pages["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        [
            "Quality",
            "Strength",
            "Speed",
            "Support",
            "Multimaterial",
            "Others"
        ]
    );
    let quality = &pages["pages"][0];
    assert_eq!(quality["groups"][0]["title"], "Layer height");
    assert_eq!(
        quality["groups"][0]["lines"],
        json!([{ "keys": ["layer_height"] }, { "keys": ["initial_layer_print_height"] }])
    );
    assert_eq!(quality["groups"][2]["title"], "Seam");
    let options = pages["options"].as_object().unwrap();
    assert!(options.len() > 400, "{} options", options.len());
    // Every option a page names is described, and nothing else is.
    let mut named = BTreeSet::new();
    for page in pages["pages"].as_array().unwrap() {
        for group in page["groups"].as_array().unwrap() {
            assert!(!group["lines"].as_array().unwrap().is_empty());
            for line in group["lines"].as_array().unwrap() {
                for key in line["keys"].as_array().unwrap() {
                    assert!(named.insert(key.as_str().unwrap()), "{key} is on two lines");
                    assert!(options.contains_key(key.as_str().unwrap()), "{key}");
                }
            }
        }
    }
    assert_eq!(named.len(), options.len());
    // And each is a setting the reference's defaults table knows: the
    // two are made from the same pin.
    for key in &named {
        assert!(edge_config::reference_knows(key), "{key}");
    }
    // The reference's own words, kinds, units and limits.
    let height = &options["layer_height"];
    assert_eq!(height["label"], "Layer height");
    assert_eq!(
        height["tooltip"],
        "Slicing height for each layer. Smaller layer height means more accurate and more printing time."
    );
    assert_eq!(
        (
            &height["type"],
            &height["unit"],
            &height["min"],
            &height["mode"],
            &height["default"]
        ),
        (
            &json!("float"),
            &json!("mm"),
            &json!(0),
            &json!("simple"),
            &json!("0.2")
        )
    );
    let scarf = &options["seam_slope_type"];
    assert_eq!(scarf["label"], "Scarf joint seam (beta)");
    assert_eq!(scarf["values"], json!(["none", "external", "all"]));
    assert_eq!(
        scarf["labels"],
        json!(["None", "Contour", "Contour and hole"])
    );
    assert_eq!(scarf["mode"], "advanced");
    assert_eq!(options["line_width"]["unit"], "mm or %");
    assert_eq!(options["line_width"]["type"], "float_or_percent");
    assert_eq!(options["enable_arc_fitting"]["type"], "bool");
    // A speed has a value for each flow variant; a line of several
    // options has the line's own label.
    assert_eq!(options["outer_wall_speed"]["list"], true);
    let speed = pages["pages"][2]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["title"] == "Overhang speed")
        .unwrap();
    let line = speed["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|line| line["label"] == "Overhang speed")
        .unwrap();
    assert_eq!(line["keys"].as_array().unwrap().len(), 4);
    // Every list has a label for each value, and its default among them.
    for (key, option) in options {
        if option["type"] == "enum" {
            let values = option["values"].as_array().unwrap();
            assert_eq!(
                values.len(),
                option["labels"].as_array().unwrap().len(),
                "{key}"
            );
            assert!(values.contains(&option["default"]), "{key}");
        }
    }
}

#[test]
fn a_value_set_is_the_projects_own_over_the_preset() {
    let session = Arc::new(Session::new(None));
    let id = on_presets(&session);
    let fresh = ask(&session, "process", Value::Null);
    assert_eq!(fresh["source"], "presets");
    assert!(fresh["name"].as_str().unwrap().contains("Snapmaker U1"));
    assert_eq!(fresh["set"], json!({}));
    assert_eq!(fresh["changed"], 0);
    assert_eq!(fresh["values"]["layer_height"], "0.2");
    // Of a speed, which the preset has as a list, the first.
    assert_eq!(fresh["values"]["outer_wall_speed"], "200");
    assert_eq!(fresh["values"]["sparse_infill_density"], "15%");
    assert_eq!(fresh["refused"], Value::Null);

    ask(&session, "slice", json!({ "project": id }));
    assert_eq!(wait(&session, id)["state"], "done");
    let before = ask(&session, "summary", json!({ "project": id }));
    let has = |summary: &Value, role: &str| {
        summary["roles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|present| present == role)
    };
    assert!(has(&before, "Sparse infill"), "{}", before["roles"]);

    // Set, it is the project's: counted on its page, marked on the
    // project, and the slices made without it are let go of.
    let set = ask(
        &session,
        "process_set",
        json!({ "key": "sparse_infill_density", "value": "0" }),
    );
    assert_eq!(set["set"], json!({ "sparse_infill_density": "0%" }));
    assert_eq!(set["counts"]["Strength"], 1);
    assert_eq!(set["counts"]["Quality"], 0);
    assert_eq!(set["changed"], 1);
    // The preset's own value stays what a reset goes back to.
    assert_eq!(set["values"]["sparse_infill_density"], "15%");
    let project = ask(&session, "project", Value::Null);
    assert_eq!(
        (&project["dirty"], &project["sliced"]),
        (&json!(true), &json!(false))
    );
    assert_eq!(project["choices"]["over"], set["set"]);
    // And it is what the next slice is made with.
    ask(&session, "slice", json!({ "project": id }));
    assert_eq!(wait(&session, id)["state"], "done");
    let after = ask(&session, "summary", json!({ "project": id }));
    assert!(!has(&after, "Sparse infill"), "{}", after["roles"]);

    // More than one, on more than one page; a speed is set as the list
    // the preset has it as.
    ask(
        &session,
        "process_set",
        json!({ "key": "outer_wall_speed", "value": 120 }),
    );
    let more = ask(
        &session,
        "process_set",
        json!({ "key": "seam_slope_type", "value": "all" }),
    );
    assert_eq!(
        (
            &more["counts"]["Quality"],
            &more["counts"]["Speed"],
            &more["changed"]
        ),
        (&json!(1), &json!(1), &json!(3))
    );
    {
        // Out of the lock before the presets are looked up, which takes it.
        let (choices, file) = {
            let state = lock(&session.state);
            let project = &state.projects[state.target(&Value::Null).unwrap()];
            (project.choices.clone(), project.file.clone())
        };
        let basis = session.basis(&choices, file.as_deref()).unwrap();
        let over = overrides(&choices, &basis).settings;
        // A value for each flow variant the preset has one for.
        let speeds = over.texts("outer_wall_speed").unwrap();
        assert_eq!(
            speeds.len(),
            basis
                .under(Book::Process)
                .unwrap()
                .texts("outer_wall_speed")
                .unwrap()
                .len()
        );
        assert!(speeds.iter().all(|speed| speed == "120"), "{speeds:?}");
        assert_eq!(over.text("seam_slope_type"), Some("all"));
        assert_eq!(over.len(), 3);
    }

    // Given back to the preset one by one; set to the preset's own
    // value, an option is the preset's again.
    let back = ask(
        &session,
        "process_set",
        json!({ "key": "sparse_infill_density", "value": null }),
    );
    assert_eq!(back["counts"]["Strength"], 0);
    let same = ask(
        &session,
        "process_set",
        json!({ "key": "outer_wall_speed", "value": "200.0" }),
    );
    assert_eq!(same["set"], json!({ "seam_slope_type": "all" }));
    // All of them at once.
    let none = ask(&session, "process_set", json!({ "reset": true }));
    assert_eq!((&none["set"], &none["changed"]), (&json!({}), &json!(0)));
    assert_eq!(ask(&session, "project", Value::Null)["dirty"], false);
}

#[test]
fn a_value_an_option_does_not_take_is_refused_with_the_reason() {
    let session = Arc::new(Session::new(None));
    on_presets(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "wall_loops", "value": 3 }),
    );
    for (bad, why) in [
        (
            json!({ "key": "layer_height", "value": -1 }),
            "Value -1 is out of range",
        ),
        (
            json!({ "key": "elefant_foot_layers_density", "value": "20%" }),
            "The valid range is from 50 to 100.",
        ),
        (
            json!({ "key": "seam_slope_type", "value": "some" }),
            "it is one of none, external, all",
        ),
        (json!({ "key": "wall_loops", "value": 1.5 }), "whole number"),
        (
            json!({ "key": "layer_height", "value": "thin" }),
            "Invalid numeric.",
        ),
        (
            json!({ "key": "layer_height", "value": "50%" }),
            "can't be a percentage",
        ),
        (
            json!({ "key": "enable_arc_fitting", "value": "perhaps" }),
            "on or off",
        ),
        // More than a length may be; as a percentage it is taken.
        (
            json!({ "key": "line_width", "value": "20" }),
            "The valid range is from 0 to 10.",
        ),
        (
            json!({ "key": "nothing", "value": 1 }),
            "no process setting named nothing",
        ),
        (json!({ "key": "layer_height" }), "value was not given"),
        (json!({ "value": 1 }), "key was not given"),
        // Within what the option takes, and still nothing the engine
        // will slice with: its own reason is given.
        (
            json!({ "key": "layer_height", "value": 5 }),
            "too narrow for a 5 mm layer",
        ),
        (
            json!({ "key": "layer_height", "value": 0 }),
            "layer heights must be positive",
        ),
    ] {
        let said = refused(&session, "process_set", bad.clone());
        assert!(said.contains(why), "{bad}: {said}");
    }
    // Nothing of what was refused was kept.
    let now = ask(&session, "process", Value::Null);
    assert_eq!(now["set"], json!({ "wall_loops": "3" }));
    assert_eq!(now["refused"], Value::Null);
    let taken = ask(
        &session,
        "process_set",
        json!({ "key": "line_width", "value": "120%" }),
    );
    assert_eq!(taken["set"]["line_width"], "120%");
    // A number is kept as short as it is written; on is 1.
    let tidy = ask(
        &session,
        "process_set",
        json!({ "key": "layer_height", "value": "0.240" }),
    );
    assert_eq!(tidy["set"]["layer_height"], "0.24");
    let on = ask(
        &session,
        "process_set",
        json!({ "key": "precise_z_height", "value": true }),
    );
    assert_eq!(on["set"]["precise_z_height"], "1");
}

#[test]
fn what_the_engine_does_with_an_option_is_its_own_answer() {
    let session = Arc::new(Session::new(None));
    let id = on_presets(&session);
    let plain = ask(&session, "process", Value::Null);
    // Read by a slice: followed.
    for key in [
        "layer_height",
        "seam_slope_type",
        "wall_generator",
        "enable_arc_fitting",
        "top_shell_layers",
        "elefant_foot_compensation",
    ] {
        assert!(!listed(&plain, "unread", key), "{key} is not read");
    }
    // Never asked for: not followed, and still there to set.
    for key in ["hollow_interior", "interlocking_beam", "print_sequence"] {
        assert!(listed(&plain, "unread", key), "{key} is read");
    }
    assert_eq!(plain["notes"].get("spiral_mode"), None);
    let hollow = ask(
        &session,
        "process_set",
        json!({ "key": "hollow_interior", "value": true }),
    );
    assert_eq!(hollow["set"]["hollow_interior"], "1");
    assert!(listed(&hollow, "unread", "hollow_interior"));
    // Read and followed: nothing is said of it.
    let last = ask(
        &session,
        "process_set",
        json!({ "key": "unsupported_wall_last", "value": true }),
    );
    assert!(!listed(&last, "unread", "unsupported_wall_last"));
    assert_eq!(last["notes"].get("unsupported_wall_last"), None);
    // Read, and not done as asked: followed with the engine's own note.
    let vase = ask(
        &session,
        "process_set",
        json!({ "key": "spiral_mode", "value": true }),
    );
    assert!(
        vase["notes"]["spiral_mode"][0]
            .as_str()
            .unwrap()
            .contains("vase mode is not followed")
    );
    // The same is said with the print: the engine's notes, and after
    // them what was set here that the slice never asked for.
    ask(&session, "slice", json!({ "project": id }));
    assert_eq!(wait(&session, id)["state"], "done");
    let summary = ask(&session, "summary", json!({ "project": id }));
    let gaps: Vec<&str> = summary["gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|gap| gap.as_str().unwrap())
        .collect();
    assert!(
        gaps.iter()
            .any(|gap| gap.contains("vase mode is not followed")),
        "{gaps:?}"
    );
    assert!(
        gaps.contains(
            &"hollow_interior \"1\": not read by the engine; it changes nothing in this print"
        ),
        "{gaps:?}"
    );
    // What was read is not among them, set by hand or not.
    assert!(!gaps.iter().any(|gap| gap.starts_with("layer_height")));
    ask(
        &session,
        "process_set",
        json!({ "key": "top_shell_layers", "value": 6 }),
    );
    ask(&session, "slice", json!({ "project": id }));
    assert_eq!(wait(&session, id)["state"], "done");
    let summary = ask(&session, "summary", json!({ "project": id }));
    assert!(
        !summary["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.as_str().unwrap().starts_with("top_shell_layers")),
        "{}",
        summary["gaps"]
    );
}

#[test]
fn options_are_shown_and_greyed_by_the_references_rules() {
    let session = Arc::new(Session::new(None));
    on_presets(&session);
    // The U1's process: classic walls, no scarf seam, no ironing.
    let plain = ask(&session, "process", Value::Null);
    assert_eq!(plain["values"]["wall_generator"], "classic");
    for key in ["wall_transition_angle", "seam_slope_steps", "ironing_flow"] {
        assert!(listed(&plain, "hidden", key), "{key} is shown");
    }
    assert!(!listed(&plain, "off", "detect_thin_wall"));
    assert!(listed(&plain, "off", "support_type"));
    // What is set by hand counts as much as what the preset says.
    let arachne = ask(
        &session,
        "process_set",
        json!({ "key": "wall_generator", "value": "arachne" }),
    );
    assert!(!listed(&arachne, "hidden", "wall_transition_angle"));
    assert!(listed(&arachne, "off", "detect_thin_wall"));
    let scarf = ask(
        &session,
        "process_set",
        json!({ "key": "seam_slope_type", "value": "external" }),
    );
    assert!(!listed(&scarf, "hidden", "seam_slope_steps"));
    assert!(listed(&scarf, "hidden", "scarf_angle_threshold"));
    let supported = ask(
        &session,
        "process_set",
        json!({ "key": "enable_support", "value": true }),
    );
    assert!(!listed(&supported, "off", "support_type"));
    // A line of several options goes with any one of them: the four
    // overhang speeds are one line.
    let level = ask(
        &session,
        "process_set",
        json!({ "key": "enable_overhang_speed", "value": false }),
    );
    for key in ["overhang_1_4_speed", "overhang_4_4_speed"] {
        assert!(listed(&level, "hidden", key), "{key} is shown");
    }
}

#[test]
fn what_was_set_stays_through_a_change_of_preset_and_of_printer() {
    let session = Arc::new(Session::new(None));
    let id = on_presets(&session);
    ask(
        &session,
        "process_set",
        json!({ "key": "top_shell_layers", "value": 7 }),
    );
    ask(
        &session,
        "process_set",
        json!({ "key": "enable_arc_fitting", "value": true }),
    );
    let presets = ask(
        &session,
        "presets",
        json!({ "vendor": "Snapmaker", "machine": MACHINE }),
    );
    let other = presets["processes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|name| name.as_str().unwrap().starts_with("0.12"))
        .expect("the U1 has a 0.12 mm process");
    ask(
        &session,
        "project_set",
        json!({ "project": id, "process": other }),
    );
    let kept = ask(&session, "process", Value::Null);
    assert_eq!(&kept["name"], other);
    assert_eq!(kept["values"]["layer_height"], "0.12");
    assert_eq!(
        kept["set"],
        json!({ "top_shell_layers": "7", "enable_arc_fitting": "1" })
    );
    // The names they had before the pages are the same settings.
    let old = ask(
        &session,
        "project_set",
        json!({ "project": id, "layer_height": 0.3, "wall_loops": 4 }),
    );
    assert_eq!(old["choices"]["layer_height"], 0.3);
    assert_eq!(old["choices"]["wall_loops"], 4);
    let both = ask(&session, "process", Value::Null);
    assert_eq!(both["set"]["layer_height"], "0.3");
    assert_eq!(both["set"]["wall_loops"], "4");
    assert_eq!(both["counts"]["Quality"], 2);
    assert_eq!(both["counts"]["Strength"], 2);
    // On the project's own settings they lie over those.
    let own = ask(
        &session,
        "project_set",
        json!({ "project": id, "source": "project" }),
    );
    assert_eq!(own["choices"]["over"]["top_shell_layers"], "7");
    let over_own = ask(&session, "process", Value::Null);
    assert_eq!(over_own["source"], "project");
    assert_eq!(over_own["changed"], 4);
    ask(&session, "slice", json!({ "project": id }));
    assert_eq!(wait(&session, id)["state"], "done");
    assert_eq!(
        ask(&session, "summary", json!({ "project": id }))["layer_height"],
        0.3
    );
}

#[test]
fn a_project_with_nothing_chosen_has_the_references_defaults() {
    let session = Arc::new(Session::new(None));
    let empty = ask(&session, "process", Value::Null);
    assert_eq!(empty["name"], Value::Null);
    assert_eq!(empty["values"]["layer_height"], "0.2");
    assert_eq!(empty["values"]["wall_generator"], "arachne");
    assert_eq!(empty["set"], json!({}));
}

#[test]
fn values_are_shown_and_held_in_the_shape_the_settings_have() {
    let pages = pages(Book::Process);
    let option = |key: &str| &pages.options[key];
    let many =
        |items: &[&str]| Setting::Many(items.iter().map(|item| (*item).to_owned()).collect());
    let one = |text: &str| Setting::One(text.to_owned());
    // Shown: the first of a list, a number at its shortest.
    assert_eq!(
        shown(option("outer_wall_speed"), &many(&["200", "150"])),
        "200"
    );
    assert_eq!(shown(option("layer_height"), &one("0.20")), "0.2");
    assert_eq!(shown(option("sparse_infill_density"), &one("15%")), "15%");
    assert_eq!(shown(option("line_width"), &one("0.42")), "0.42");
    assert_eq!(shown(option("line_width"), &one("105%")), "105%");
    assert_eq!(shown(option("enable_support"), &one("true")), "1");
    assert_eq!(
        shown(option("post_process"), &many(&["a.py", "\nb.py --quiet"])),
        "a.py\nb.py --quiet"
    );
    // Held: a list keeps its length, one value stays one.
    let speed = option("outer_wall_speed");
    assert_eq!(
        held(speed, "120", Some(&many(&["200", "150"]))),
        many(&["120", "120"])
    );
    assert_eq!(held(speed, "120", Some(&one("200"))), one("120"));
    assert_eq!(held(speed, "120", None), one("120"));
    assert_eq!(
        held(option("post_process"), "a.py\nb.py", None),
        many(&["a.py", "b.py"])
    );
    assert_eq!(held(option("post_process"), "", None), many(&[""]));
    assert_eq!(
        held(option("layer_height"), "0.3", Some(&one("0.2"))),
        one("0.3")
    );
}
