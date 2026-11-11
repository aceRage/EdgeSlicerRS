//! The filament's and the printer's pages: as the reference has them, a
//! value set for one filament or one extruder being that one's alone, and
//! what is set reaching the G-code.

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

/// A project on the U1's usual presets with two filaments of different
/// presets, as the page puts it.
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
            "process": presets["processes"][0],
            "filaments": [presets["filaments"][0], presets["filaments"][1]],
            "initial": true,
        }),
    );
    // The model printed in filament 2, so the project has two.
    let scene = ask(session, "scene", json!({ "project": id }));
    ask(
        session,
        "scene_edit",
        json!({ "project": id, "op": "filament", "object": scene["objects"][0]["id"], "filament": 2 }),
    );
    id
}

/// The settings the project would be sliced with now.
fn settled(session: &Arc<Session>) -> Settings {
    let (choices, file) = {
        let state = lock(&session.state);
        let project = &state.projects[state.target(&Value::Null).unwrap()];
        (project.choices.clone(), project.file.clone())
    };
    let basis = session.basis(&choices, file.as_deref()).unwrap();
    edge_engine::settled(
        &basis.chosen,
        &overrides(&choices, &basis),
        basis.project.as_deref(),
    )
    .unwrap()
}

/// The project's G-code as it is exported.
fn gcode(session: &Arc<Session>, id: u64, name: &str) -> String {
    ask(session, "slice", json!({ "project": id }));
    assert_eq!(wait(session, id)["state"], "done");
    let out =
        std::env::temp_dir().join(format!("edge-presets-{}-{name}.gcode", std::process::id()));
    ask(
        session,
        "export",
        json!({ "project": id, "path": out.to_string_lossy() }),
    );
    let text = std::fs::read_to_string(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    text
}

/// A setting's line in the list of settings at the end of a G-code file.
fn listed<'a>(gcode: &'a str, key: &str) -> &'a str {
    let head = format!("; {key} = ");
    gcode
        .lines()
        .find_map(|line| line.strip_prefix(head.as_str()))
        .unwrap_or_else(|| panic!("{key} is not in the G-code's settings"))
}

fn titles(pages: &Value) -> Vec<&str> {
    pages["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page["title"].as_str().unwrap())
        .collect()
}

#[test]
fn the_filament_and_printer_pages_are_the_references() {
    let session = Arc::new(Session::new(None));
    let filament = ask(&session, "settings_pages", json!({ "kind": "filament" }));
    assert_eq!(
        titles(&filament),
        [
            "Filament",
            "Cooling",
            "Setting Overrides",
            "Advanced",
            "Multimaterial",
            "Dependencies",
            "Notes"
        ]
    );
    assert_eq!(
        filament["pages"][0]["groups"][0]["title"],
        "Basic information"
    );
    let printer = ask(&session, "settings_pages", json!({ "kind": "printer" }));
    assert_eq!(
        titles(&printer),
        [
            "Basic information",
            "Machine G-code",
            "Multimaterial",
            "Extruder %d",
            "Motion ability",
            "Notes"
        ]
    );
    assert_eq!(printer["pages"][3]["per"], "extruder");
    // The reference's own words, and what it says the options are.
    let options = &filament["options"];
    assert_eq!(options["nozzle_temperature"]["label"], "Other layers");
    assert_eq!(options["nozzle_temperature"]["unit"], "℃");
    assert_eq!(options["filament_retraction_length"]["override"], "printer");
    assert_eq!(options["filament_ironing_flow"]["override"], "process");
    assert_eq!(printer["options"]["machine_start_gcode"]["code"], true);
    assert_eq!(
        printer["options"]["machine_max_speed_x"]["label"],
        "Maximum speed X"
    );
    assert_eq!(printer["options"]["printable_area"]["widget"], true);
    // Every option the pages name is described and is one the reference's
    // defaults table knows, but the tab's own count of extruders.
    for pages in [&filament, &printer] {
        let mut named = BTreeSet::new();
        for page in pages["pages"].as_array().unwrap() {
            for group in page["groups"].as_array().unwrap() {
                for line in group["lines"].as_array().unwrap() {
                    for key in line["keys"].as_array().unwrap() {
                        let key = key.as_str().unwrap();
                        assert!(named.insert(key), "{key} is on two lines");
                        assert!(pages["options"].get(key).is_some(), "{key}");
                        // The reference's own table has no default for
                        // an option that may be "nil" and few others.
                        if pages["options"][key].get("default").is_some() {
                            assert!(edge_config::reference_knows(key), "{key}");
                        }
                    }
                }
            }
        }
        assert_eq!(named.len(), pages["options"].as_object().unwrap().len());
    }
}

#[test]
fn a_filaments_value_is_that_filaments_alone_and_reaches_the_gcode() {
    let session = Arc::new(Session::new(None));
    let id = on_presets(&session);
    let filaments = ask(&session, "project", Value::Null)["filament_count"]
        .as_u64()
        .unwrap();
    assert_eq!(filaments, 2);
    // A filament the project does not have is refused.
    let missing = refused(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": filaments + 2 }),
    );
    assert!(missing.contains("no filament"), "{missing}");
    let first = ask(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": 1 }),
    );
    assert_eq!(first["kind"], "filament");
    assert_eq!(first["filament"], 1);
    assert_eq!(first["set"], json!({}));
    let temperature = first["values"]["nozzle_temperature"]
        .as_str()
        .unwrap()
        .to_owned();
    let flow = first["values"]["filament_flow_ratio"]
        .as_str()
        .unwrap()
        .to_owned();

    // Filament 1's own: set, counted on its page, and nobody else's.
    let set = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "filament_flow_ratio", "value": "0.91" }),
    );
    assert_eq!(set["set"], json!({ "filament_flow_ratio": "0.91" }));
    assert_eq!(set["counts"]["Filament"], 1);
    assert_eq!(set["values"]["filament_flow_ratio"], flow);
    let fan = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "fan_max_speed", "value": 60 }),
    );
    assert_eq!(fan["counts"]["Cooling"], 1);
    let project = ask(&session, "project", Value::Null);
    assert_eq!(project["dirty"], true);
    assert_eq!(
        project["choices"]["filament_over"],
        json!([{ "filament_flow_ratio": "0.91", "fan_max_speed": "60" }])
    );
    // Filament 2's temperature is filament 2's.
    let second = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 2, "key": "nozzle_temperature", "value": 245 }),
    )
    .clone();
    let shown_first = ask(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": 1 }),
    );
    assert_eq!(shown_first["values"]["nozzle_temperature"], temperature);
    assert_eq!(shown_first["set"].get("nozzle_temperature"), None);
    assert_eq!(second["set"], json!({ "nozzle_temperature": "245" }));
    let now = settled(&session);
    let temperatures = now.texts("nozzle_temperature").unwrap();
    assert_eq!(temperatures[0], temperature);
    assert_eq!(temperatures[1], "245");
    assert_eq!(now.texts("filament_flow_ratio").unwrap()[0], "0.91");
    assert_ne!(now.texts("filament_flow_ratio").unwrap()[1], "0.91");

    // In the G-code: the settings' list, and filament 2's temperature in
    // the moves themselves (the model prints in filament 2).
    let made = gcode(&session, id, "filament");
    assert!(
        listed(&made, "nozzle_temperature").starts_with(&format!("{temperature},245")),
        "{}",
        listed(&made, "nozzle_temperature")
    );
    assert!(listed(&made, "filament_flow_ratio").starts_with("0.91,"));
    assert!(listed(&made, "fan_max_speed").starts_with("60,"));
    assert!(made.contains(" S245"), "filament 2 is brought to 245 °C");

    // Given back: filament 1's, then all of filament 2's.
    let back = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "reset": true }),
    );
    assert_eq!(back["set"], json!({}));
    let back = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 2, "key": "nozzle_temperature", "value": null }),
    );
    assert_eq!(back["changed"], 0);
    let project = ask(&session, "project", Value::Null);
    assert_eq!(project["choices"]["filament_over"], json!([]));
}

#[test]
fn a_printers_value_and_an_extruders_reach_the_gcode() {
    let session = Arc::new(Session::new(None));
    let id = on_presets(&session);
    let printer = ask(&session, "settings", json!({ "kind": "printer" }));
    assert_eq!(printer["extruders"], 4);
    assert_eq!(
        printer["pages"],
        json!([
            "Basic information",
            "Machine G-code",
            "Multimaterial",
            "Extruder 1",
            "Extruder 2",
            "Extruder 3",
            "Extruder 4",
            "Motion ability",
            "Notes"
        ])
    );
    // The U1 is Klipper's: "Motion ability" is shown.
    assert_eq!(printer["away"], json!([]));
    assert_eq!(printer["values"]["retraction_length#1"], "1.5");
    assert_eq!(printer["values"]["extruders_count"], "4");
    assert!(printer["fixed"]["printable_area"].is_string());
    let start = printer["values"]["machine_start_gcode"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(start.contains('\n'), "{start:?}");

    // Extruder 2's retraction, extruder 2's alone. The model prints in
    // filament 2, on extruder 2; its filament's own retraction is turned
    // off so the printer's is what it retracts by.
    let set = ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "retraction_length#1", "value": "2.5" }),
    );
    assert_eq!(set["counts"]["Extruder 2"], 1);
    assert_eq!(set["counts"]["Extruder 1"], 0);
    ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 2, "key": "filament_retraction_length", "value": "nil" }),
    );
    let marked = format!("{start}\n; set on the printer's page");
    ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "machine_start_gcode", "value": marked }),
    );
    let limit = ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "key": "machine_max_speed_z", "value": "15" }),
    );
    assert_eq!(limit["counts"]["Motion ability"], 1);
    let now = settled(&session);
    assert_eq!(
        now.texts("retraction_length").unwrap(),
        ["1.5", "2.5", "1.5", "1.5"]
    );
    // Of a limit, the normal mode's; the silent one stays.
    let speeds = now.texts("machine_max_speed_z").unwrap();
    assert_eq!(speeds[0], "15");
    assert_eq!(speeds.len(), 2);

    let made = gcode(&session, id, "printer");
    assert!(made.contains("; set on the printer's page"));
    assert!(listed(&made, "retraction_length").starts_with("1.5,2.5,"));
    assert!(made.contains("G1 E-2.5"), "extruder 2 retracts 2.5 mm");
    assert!(listed(&made, "machine_max_speed_z").starts_with("15,"));

    // What is refused: another extruder than the printer has, an option
    // of one extruder named without it, one set in a dialog of the
    // reference's own, a value out of range.
    assert!(
        refused(
            &session,
            "settings_set",
            json!({ "kind": "printer", "key": "retraction_length#7", "value": 1 })
        )
        .contains("no extruder 8")
    );
    assert!(
        refused(
            &session,
            "settings_set",
            json!({ "kind": "printer", "key": "retraction_length", "value": 1 })
        )
        .contains("an extruder's own")
    );
    assert!(
        refused(
            &session,
            "settings_set",
            json!({ "kind": "printer", "key": "printable_area", "value": "0x0" })
        )
        .contains("cannot be changed here")
    );
    assert!(
        refused(
            &session,
            "settings_set",
            json!({ "kind": "printer", "key": "machine_max_speed_z", "value": -1 })
        )
        .contains("out of range")
    );
    assert!(
        refused(
            &session,
            "settings_set",
            json!({ "kind": "printer", "key": "extruder_offset#0", "value": "a" })
        )
        .contains("XxY")
    );

    // Kept through a change of printer preset, as the process's are.
    let other = ask(
        &session,
        "project_set",
        json!({ "machine": "Snapmaker U1 (0.6 nozzle)" }),
    );
    assert!(
        other["choices"]["machine"]
            .as_str()
            .unwrap()
            .contains("0.6")
    );
    assert_eq!(
        other["choices"]["printer_over"]["retraction_length#1"],
        "2.5"
    );
    let all = ask(
        &session,
        "settings_set",
        json!({ "kind": "printer", "reset": true }),
    );
    assert_eq!(all["changed"], 0);
}

#[test]
fn an_override_is_turned_on_and_off_and_falls_back_to_the_printer() {
    let session = Arc::new(Session::new(None));
    on_presets(&session);
    let shown = ask(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": 1 }),
    );
    // Off, it stands for the printer's value for this filament's extruder.
    assert_eq!(shown["fallback"]["filament_z_hop"], "0.4");
    // On, with a value; off again with "nil".
    let on = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "filament_z_hop", "value": "0.6" }),
    );
    assert_eq!(on["set"]["filament_z_hop"], "0.6");
    assert_eq!(settled(&session).texts("filament_z_hop").unwrap()[0], "0.6");
    let preset = &shown["values"]["filament_z_hop"];
    let off = ask(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "filament_z_hop", "value": "nil" }),
    );
    if preset == "nil" {
        assert_eq!(off["set"], json!({}));
    } else {
        assert_eq!(off["set"]["filament_z_hop"], "nil");
    }
    // "nil" is for an override only.
    let not = refused(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "nozzle_temperature", "value": "nil" }),
    );
    assert!(not.contains("Invalid numeric"), "{not}");
    // A setting of the preset's own, not a filament's, is not set here.
    let own = refused(
        &session,
        "settings_set",
        json!({ "kind": "filament", "filament": 1, "key": "compatible_printers_condition", "value": "x" }),
    );
    assert!(own.contains("cannot be changed here"), "{own}");
}

#[test]
fn the_rules_and_the_engine_answer_for_filament_and_printer() {
    let session = Arc::new(Session::new(None));
    on_presets(&session);
    let filament = ask(
        &session,
        "settings",
        json!({ "kind": "filament", "filament": 1 }),
    );
    let has = |answer: &Value, list: &str, key: &str| {
        answer[list]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed == key)
    };
    // The U1's plates, named its way.
    assert_eq!(
        filament["labels"]["supertack_plate_temp_initial_layer"],
        "Cool Steel Plate"
    );
    assert!(has(&filament, "hidden", "eng_plate_temp"));
    assert!(!has(&filament, "hidden", "textured_plate_temp"));
    // Read: the temperatures, flow and fans; not read: the cost.
    for key in ["nozzle_temperature", "filament_flow_ratio", "fan_max_speed"] {
        assert!(!has(&filament, "unread", key), "{key} is followed");
    }
    assert!(has(&filament, "unread", "filament_z_offset"));
    let printer = ask(&session, "settings", json!({ "kind": "printer" }));
    for key in ["machine_start_gcode", "retraction_length#0", "z_hop#2"] {
        assert!(!has(&printer, "unread", key), "{key} is followed");
    }
    // Retracting, extruder 1's lift settings are open.
    assert!(!has(&printer, "off", "z_hop#0"));
}
