//! The Calibration menu: a calibration's dialog asks for its values, then
//! `calibrate` makes its print a project of its own, ready to slice.
//!
//! What the reference does (Plater.cpp `calib_pa` and the rest): it starts
//! a new project in place of the one shown (asking to save that one, and
//! whether the presets' changes are kept), names it after the test, loads
//! the test's model, sets settings over the process, filament and printer
//! presets and on the object, and keeps the test's values on its print for
//! the slice. Here the new project goes in the strip beside the one shown,
//! which stays open (or takes the place of the one shown when that is new
//! and untouched, as "New Project" does), so nothing is asked; it starts
//! from the presets and what was set over them in the one shown, as the
//! reference's "Keep" does. The settings the test sets are then set over
//! those, where the pages show them, and the project's own values go to
//! each of its slices (`Project::calibration`). As in the reference they
//! are not saved with it: a project saved and opened again slices as any
//! other, with the settings the test set.

use std::sync::Arc;

use edge_engine::calib::{CalibMode, CalibParams, FlowTest};
use serde_json::Value;

use super::{File, Session, edit, lock, settings};

/// The Flow rate menu's tests, which have no dialog: the menu item makes
/// the test straight away (MainFrame.cpp, `calib_flowrate`).
fn flow_test(args: &Value) -> Option<FlowTest> {
    match args.get("test").and_then(Value::as_str)? {
        "flow_pass1" => Some(FlowTest::Pass1),
        "flow_pass2" => Some(FlowTest::Pass2),
        "flow_yolo" => Some(FlowTest::Yolo),
        "flow_yolo_perfectionist" => Some(FlowTest::YoloPerfectionist),
        _ => None,
    }
}

/// The test's values from the page's dialog.
fn params(args: &Value) -> Result<CalibParams, String> {
    let number = |key: &str| -> Result<f64, String> {
        args.get(key)
            .and_then(Value::as_f64)
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("{key} was not given"))
    };
    let list = |key: &str| -> Vec<f64> {
        args.get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default()
    };
    let mode = match args.get("test").and_then(Value::as_str) {
        Some("pa_tower") => CalibMode::PressureAdvanceTower,
        Some("pa_line") => CalibMode::PressureAdvanceLine,
        Some("pa_pattern") => CalibMode::PressureAdvancePattern,
        Some("temp_tower") => CalibMode::TemperatureTower,
        Some(other) => return Err(format!("there is no calibration \"{other}\" yet")),
        None => return Err("test was not given".into()),
    };
    if mode == CalibMode::TemperatureTower {
        // Temp_Calibration_Dlg::on_start: whole numbers (`ToULong`), the
        // step fixed at 5.
        let whole = |key: &str| {
            number(key)
                .ok()
                .filter(|value| *value >= 0.0 && value.fract() == 0.0)
        };
        let (start, end) = (whole("start"), whole("end"));
        let valid = matches!((start, end), (Some(start), Some(end))
            if start <= 350.0 && end >= 170.0 && end <= start - 5.0);
        if !valid {
            return Err(super::i18n::tr(
                "Please input valid values:\nStart temp: <= 350\nEnd temp: >= 170\nStart temp > End temp + 5",
            ));
        }
        return Ok(CalibParams::new(
            mode,
            start.unwrap_or(0.0),
            end.unwrap_or(0.0),
            5.0,
        ));
    }
    let mut params = CalibParams::new(mode, number("start")?, number("end")?, number("step")?);
    params.print_numbers = args.get("print_numbers").and_then(Value::as_bool) == Some(true);
    params.accelerations = list("accelerations");
    params.speeds = list("speeds");
    // The dialog's own check (calib_dlg.cpp `PA_Calibration_Dlg::on_start`),
    // made again here, which the page shows in the reference's words.
    let valid =
        params.start >= 0.0 && params.step >= 1e-4 && params.end >= params.start + params.step;
    if !valid {
        return Err(super::i18n::tr(
            "Please input valid values:\nStart PA: >= 0.0\nEnd PA: > Start PA\nPA step: >= 0.001",
        ));
    }
    Ok(params)
}

impl Session {
    /// Makes a calibration's project (`calibrate`): the strip's listing,
    /// with `id` the new project's, or `full` when the strip has no room.
    pub(super) fn calibrate(&self, args: &Value) -> Result<Value, String> {
        let flow = flow_test(args);
        let params = match flow {
            Some(_) => None,
            None => Some(params(args)?),
        };
        // What the new project starts from: the presets of the one shown
        // and what was set over them.
        let (choices, calib_dir) = {
            let state = lock(&self.state);
            let shown = &state.projects[state.active_index()];
            let blank = shown.blank();
            if !blank && state.projects.len() >= super::MAX_OPEN {
                return Ok(state.listing_with(serde_json::json!({ "full": true })));
            }
            let mut choices = shown.choices.clone();
            choices.from_presets = true;
            choices.inside.clear();
            let calib_dir = state
                .library
                .as_ref()
                .and_then(|library| library.dir().parent().map(|dir| dir.join("calib")))
                .ok_or("there are no profiles, so no calibration models either")?;
            (choices, calib_dir)
        };
        if choices.machine.is_none() {
            return Err("choose a printer first: the test is made for it".into());
        }
        let basis = self.basis(&choices, None)?;
        let set = settings::overrides(&choices, &basis);
        let settled = edge_engine::settled(&basis.chosen, &set, None).map_err(|e| e.to_string())?;
        let bed = super::bed_of(&settled).map(edge_engine::Bed::of);
        let made = match (flow, &params) {
            (Some(test), _) => {
                edge_engine::calib::flow_rate(test, &settled, &calib_dir, bed.as_ref())
            }
            (None, Some(params)) => match params.mode {
                CalibMode::PressureAdvanceTower
                | CalibMode::PressureAdvanceLine
                | CalibMode::PressureAdvancePattern => {
                    edge_engine::calib::pressure_advance(params, &settled, &calib_dir, bed.as_ref())
                }
                CalibMode::TemperatureTower => {
                    edge_engine::calib::temperature(params, &calib_dir, bed.as_ref())
                }
            },
            (None, None) => return Err("test was not given".into()),
        }
        .map_err(|e| e.to_string())?;

        // The reference's notifications while it made the test.
        let notes: Vec<String> = made
            .notes
            .iter()
            .map(|(said, value)| {
                format!(
                    "{}\n{}{value}",
                    super::i18n::tr("INFO:"),
                    super::i18n::tr(said)
                )
            })
            .collect();
        let mut state = lock(&self.state);
        let shown = state.active_index();
        let at = if state.projects[shown].blank() {
            shown
        } else {
            if state.projects.len() >= super::MAX_OPEN {
                return Ok(state.listing_with(serde_json::json!({ "full": true })));
            }
            let name = state.untitled();
            state.add(name)
        };
        let project = &mut state.projects[at];
        let mut choices = choices;
        // What the test sets over the presets, as the pages hold it.
        for (key, value) in &made.process {
            choices.over.insert(key.clone(), value.clone());
        }
        for (key, value) in &made.printer {
            choices.printer_over.insert(key.clone(), value.clone());
        }
        if !made.filament.is_empty() {
            if choices.filament_over.is_empty() {
                choices.filament_over.push(Default::default());
            }
            for (key, value) in &made.filament {
                choices.filament_over[0].insert(key.clone(), value.clone());
            }
        }
        project.baseline = choices.clone();
        project.choices = choices;
        project.name = made.name.clone();
        project.tab = "prepare";
        project.plate = made.model.plates().first().map(|plate| plate.id());
        let before = project.file.take();
        project.file = Some(Arc::new(File::of(made.model)));
        project.opened = None;
        edit::push_step(project, "Import Object", before, None);
        project.calibration = made.params;
        let id = project.id;
        state.active = id;
        Ok(state.listing_with(serde_json::json!({ "id": id, "notes": notes })))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;

    const MACHINE: &str = "Snapmaker U1 (0.4 nozzle)";

    fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
        session
            .call(name, &args)
            .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
    }

    fn choose_presets(session: &Arc<Session>) {
        let presets = ask(
            session,
            "presets",
            json!({ "vendor": "Snapmaker", "machine": MACHINE }),
        );
        ask(
            session,
            "project_set",
            json!({
                "source": "presets", "vendor": "Snapmaker",
                "machine": MACHINE, "process": presets["processes"][0],
                "filaments": [presets["filaments"][0]], "initial": true,
            }),
        );
    }

    fn slice_and_read(session: &Arc<Session>, project: u64, dir: &std::path::Path) -> String {
        ask(session, "slice", json!({ "project": project }));
        loop {
            let status = ask(session, "status", json!({ "project": project }));
            if status["state"] != "slicing" {
                assert_eq!(status["state"], "done", "{status}");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let path = dir.join("out.gcode");
        ask(
            session,
            "export",
            json!({ "project": project, "path": path.to_string_lossy() }),
        );
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_pa_tower_is_a_new_project_ready_to_slice_and_undone_as_any() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // No printer chosen in a project: nothing to make the test for.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "pa_tower", "start": 0, "end": 0.04, "step": 0.002}),
        );
        let id = listing["id"].as_u64().unwrap();
        // The blank project shown took it.
        assert_eq!(id, 1);
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Pressure Advance Test", "{detail}");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["wipe_inward"], "0", "{choices}");
        assert_eq!(choices["over"]["overhang_reverse"], "0");
        assert_eq!(choices["printer_over"]["resonance_avoidance"], "0");
        assert_eq!(choices["filament_over"][0]["slow_down_layer_time"], "1");
        let gcode = slice_and_read(&session, id, dir.path());
        let advances: Vec<&str> = gcode
            .lines()
            .filter(|line| line.starts_with("SET_PRESSURE_ADVANCE"))
            .collect();
        // One for each layer of the 21 mm tower.
        assert_eq!(advances.len(), gcode.matches(";LAYER_CHANGE").count());
        assert_eq!(
            advances.last().copied(),
            Some("SET_PRESSURE_ADVANCE ADVANCE=0.04; Override pressure advance value")
        );
        // Another goes in a project of its own, the first staying open.
        let again = ask(
            &session,
            "calibrate",
            json!({"test": "pa_tower", "start": 0, "end": 0.02, "step": 0.002}),
        );
        let second = again["id"].as_u64().unwrap();
        assert_ne!(second, id);
        // Undone, the model goes; what was set stays, as the reference's
        // preset changes do.
        ask(&session, "undo", json!({ "project": second }));
        let detail = ask(&session, "project", json!({ "project": second }));
        assert_eq!(detail["choices"]["over"]["wipe_inward"], "0");
        // A model added lets the calibration go.
        ask(&session, "redo", json!({ "project": second }));
        ask(
            &session,
            "scene_edit",
            json!({
                "project": second, "op": "add",
                "path": std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../reference/EdgeSlicer/resources/handy_models/OrcaToleranceTest.stl")
                    .to_string_lossy(),
            }),
        );
        let gcode = slice_and_read(&session, second, dir.path());
        assert!(!gcode.contains("SET_PRESSURE_ADVANCE ADVANCE=0.02"));
    }

    #[test]
    fn the_dialogs_values_are_checked_as_the_reference_checks_them() {
        let ok =
            params(&json!({"test": "pa_tower", "start": 0, "end": 0.1, "step": 0.002})).unwrap();
        assert_eq!(ok.mode, CalibMode::PressureAdvanceTower);
        for bad in [
            json!({"test": "pa_tower", "start": -0.1, "end": 0.1, "step": 0.002}),
            json!({"test": "pa_tower", "start": 0, "end": 0.001, "step": 0.002}),
            json!({"test": "pa_tower", "start": 0, "end": 0.1, "step": 0}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values")
            );
        }
        assert!(params(&json!({"test": "nothing"})).is_err());
        let temp = params(&json!({"test": "temp_tower", "start": 230, "end": 190})).unwrap();
        assert_eq!((temp.start, temp.end, temp.step), (230.0, 190.0, 5.0));
        for bad in [
            json!({"test": "temp_tower", "start": 355, "end": 190}),
            json!({"test": "temp_tower", "start": 230, "end": 165}),
            json!({"test": "temp_tower", "start": 230, "end": 226}),
            json!({"test": "temp_tower", "start": 230.5, "end": 190}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values"),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_temperature_tower_is_named_and_cools_as_it_rises() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "temp_tower", "start": 230, "end": 220, "step": 5}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Nozzle temperature test");
        let own = &detail["choices"]["filament_over"][0];
        assert_eq!(own["nozzle_temperature"], "230", "{own}");
        assert_eq!(own["nozzle_temperature_initial_layer"], "230");
        let gcode = slice_and_read(&session, id, dir.path());
        for temperature in [230, 225, 220] {
            assert!(
                gcode.contains(&format!("M104 S{temperature} ; set nozzle temperature")),
                "{temperature}"
            );
        }
    }

    #[test]
    fn the_pa_pattern_stands_a_handle_for_each_speed_and_acceleration() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "pa_pattern", "start": 0, "end": 0.04, "step": 0.005,
                   "speeds": [100, 150], "accelerations": [3000, 6000]}),
        );
        let id = listing["id"].as_u64().unwrap();
        assert_eq!(listing["notes"], json!([]));
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["choices"]["over"]["wall_loops"], "3");
        assert_eq!(detail["choices"]["over"]["outer_wall_acceleration"], "6000");
        let gcode = slice_and_read(&session, id, dir.path());
        // Four patterns of 9 blocks each, on each of the four layers.
        let starts = gcode
            .matches("; start pressure advance pattern for layer")
            .count();
        assert_eq!(starts, 4 * 4, "patterns");
        assert!(gcode.contains("SET_VELOCITY_LIMIT ACCEL=3000"));
        assert!(gcode.contains("SET_VELOCITY_LIMIT ACCEL=6000"));
        // The line test, with no speed given: said as the reference says it.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "pa_line", "start": 0, "end": 0.02, "step": 0.002, "print_numbers": false}),
        );
        let line = listing["id"].as_u64().unwrap();
        let gcode = slice_and_read(&session, line, dir.path());
        assert_eq!(
            gcode.matches("SET_PRESSURE_ADVANCE").count(),
            1 + 11 + 1 + 1
        );
        assert!(!gcode.contains(";LAYER_CHANGE"));
    }

    #[test]
    fn the_flow_rate_tests_are_projects_of_tiles_each_with_its_own_flow() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        for (test, name, tiles) in [
            ("flow_pass1", "Flowrate Test - Pass1", 9),
            ("flow_pass2", "Flowrate Test - Pass2", 10),
            ("flow_yolo", "Orca YOLO Flow Calibration", 11),
            (
                "flow_yolo_perfectionist",
                "Orca YOLO Flow Calibration - Perfectionist version",
                16,
            ),
        ] {
            let listing = ask(&session, "calibrate", json!({ "test": test }));
            let id = listing["id"].as_u64().unwrap();
            let detail = ask(&session, "project", json!({ "project": id }));
            assert_eq!(detail["name"], name);
            assert_eq!(detail["dirty"], true);
            let over = &detail["choices"]["over"];
            assert_eq!(over["layer_height"], "0.2", "{over}");
            assert_eq!(over["seam_slope_type"], "none");
            assert_eq!(
                detail["choices"]["printer_over"]["resonance_avoidance"],
                "0"
            );
            let objects = ask(&session, "plate_view", json!({ "project": id }));
            let count = objects["objects"].as_array().map_or(0, Vec::len);
            assert_eq!(count, tiles, "{test}: {objects}");
            if test == "flow_pass1" {
                let gcode = slice_and_read(&session, id, dir.path());
                assert_eq!(gcode.matches(";LAYER_CHANGE").count(), 10);
                // Each tile with its own flow: EXCLUDE_OBJECT names them.
                assert!(gcode.contains("EXCLUDE_OBJECT_START NAME=flowrate_m20"));
                if let Ok(out) = std::env::var("EDGE_CALIB_DUMP") {
                    ask(
                        &session,
                        "project_save",
                        json!({ "project": id, "path": std::path::Path::new(&out).join("flow_pass1.3mf").to_string_lossy() }),
                    );
                }
                // Undone, the tiles go; what was set stays.
                ask(&session, "undo", json!({ "project": id }));
                let objects = ask(&session, "plate_view", json!({ "project": id }));
                assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);
                let detail = ask(&session, "project", json!({ "project": id }));
                assert_eq!(detail["choices"]["over"]["layer_height"], "0.2");
            }
        }
    }
}
