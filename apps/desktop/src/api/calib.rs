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
        Some("retraction") => CalibMode::RetractionTower,
        Some("max_flowrate") => CalibMode::VolumetricSpeedTower,
        Some("vfa") => CalibMode::VfaTower,
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
    if mode == CalibMode::RetractionTower {
        // Retraction_Test_Dlg::on_start.
        let (start, end, step) = (number("start"), number("end"), number("step"));
        return match (start, end, step) {
            (Ok(start), Ok(end), Ok(step)) if start >= 0.0 && step > 0.0 && end >= start + step => {
                Ok(CalibParams::new(mode, start, end, step))
            }
            _ => Err(super::i18n::tr(
                "Please input valid values:\nstart > 0\nstep >= 0\nend > start + step",
            )),
        };
    }
    if mode == CalibMode::VolumetricSpeedTower {
        // MaxVolumetricSpeed_Test_Dlg::on_start.
        let (start, end, step) = (number("start"), number("end"), number("step"));
        return match (start, end, step) {
            (Ok(start), Ok(end), Ok(step)) if start > 0.0 && step > 0.0 && end >= start + step => {
                Ok(CalibParams::new(mode, start, end, step))
            }
            _ => Err(super::i18n::tr(
                "Please input valid values:\nstart > 0\nstep >= 0\nend > start + step",
            )),
        };
    }
    if mode == CalibMode::VfaTower {
        // VFA_Test_Dlg::on_start.
        let (start, end, step) = (number("start"), number("end"), number("step"));
        let mut params = match (start, end, step) {
            (Ok(start), Ok(end), Ok(step)) if start > 10.0 && step > 0.0 && end >= start + step => {
                CalibParams::new(mode, start, end, step)
            }
            _ => {
                return Err(super::i18n::tr(
                    "Please input valid values:\nstart > 10\nstep >= 0\nend > start + step",
                ));
            }
        };
        // "Auto-scale for nozzle", and the layer height the dialog's
        // check lowered to (0: half the nozzle).
        params.nozzle_based_resize = args.get("resize").and_then(Value::as_bool) != Some(false);
        params.vfa_layer_height = args
            .get("layer_height")
            .and_then(Value::as_f64)
            .filter(|height| height.is_finite() && *height > 0.0)
            .unwrap_or(0.0);
        return Ok(params);
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
    /// The settings a calibration made now would start from: the shown
    /// project's presets and what is set over them.
    fn calibration_settings(&self) -> Result<(super::Choices, edge_config::Settings), String> {
        let choices = {
            let state = lock(&self.state);
            let mut choices = state.projects[state.active_index()].choices.clone();
            choices.from_presets = true;
            choices.inside.clear();
            choices
        };
        if choices.machine.is_none() {
            return Err("choose a printer first: the test is made for it".into());
        }
        let basis = self.basis(&choices, None)?;
        let set = settings::overrides(&choices, &basis);
        let settled = edge_engine::settled(&basis.chosen, &set, None).map_err(|e| e.to_string())?;
        Ok((choices, settled))
    }

    /// The VFA dialog's check before its test is made (`calib_vfa_check`,
    /// VFA_Test_Dlg::on_start): whether the end speed is over what the
    /// filament melts at the tower's layer height, and what is done then
    /// (calib.cpp `calib_vfa_plan`), with the figures its messages give.
    pub(super) fn calib_vfa_check(&self, args: &Value) -> Result<Value, String> {
        let params = params(args)?;
        let (choices, settled) = self.calibration_settings()?;
        let first = |key: &str| -> f64 {
            settled
                .floats(key)
                .ok()
                .flatten()
                .and_then(|values| values.first().copied().flatten())
                .unwrap_or(0.0)
        };
        let mut nozzle = first("nozzle_diameter");
        if nozzle <= 0.0 {
            nozzle = 0.4;
        }
        let preset_layer = settled
            .float("layer_height")
            .ok()
            .flatten()
            .filter(|height| *height > 0.0)
            .unwrap_or(nozzle / 2.0);
        let default_layer = if params.nozzle_based_resize {
            nozzle / 2.0
        } else {
            preset_layer
        };
        let max_volumetric = first("filament_max_volumetric_speed");
        let width = |key: &str| -> f64 {
            settled
                .float_or_percent(key)
                .ok()
                .flatten()
                .map_or(0.0, |width| width.resolve(nozzle))
        };
        let mut line_width = width("outer_wall_line_width");
        if line_width <= 0.0 {
            line_width = width("line_width");
        }
        if line_width <= 0.0 {
            line_width = nozzle;
        }
        let limit = edge_engine::calib::vfa_speed_limit(max_volumetric, line_width, default_layer);
        let over = max_volumetric > 0.0 && params.end > limit;
        let mut answer = serde_json::json!({
            "over": over,
            "end": params.end,
            "max_volumetric_speed": max_volumetric,
            "limit": limit,
            "default_layer_height": default_layer,
        });
        if over && params.nozzle_based_resize {
            // The layer heights of the processes made for the printer.
            let mut candidates = Vec::new();
            if let (Some(vendor), Some(machine)) = (&choices.vendor, &choices.machine) {
                let shelf = self.shelf(vendor)?;
                let printer = self.system_printer(machine);
                for process in shelf.processes_for(&printer) {
                    if let Ok(own) = shelf.resolve(edge_config::Kind::Process, process)
                        && let Ok(Some(height)) = own.float("layer_height")
                        && height > 0.0
                    {
                        candidates.push(height);
                    }
                }
            }
            let plan = edge_engine::calib::vfa_plan(
                params.start,
                params.end,
                params.step,
                default_layer,
                candidates,
                first("min_layer_height"),
                first("max_layer_height"),
                max_volumetric,
                line_width,
                nozzle,
            );
            answer["plan"] = match plan {
                edge_engine::calib::VfaPlan::Fits => serde_json::json!({ "kind": "fits" }),
                edge_engine::calib::VfaPlan::LayerHeightReduced(height) => {
                    serde_json::json!({ "kind": "layer_height", "layer_height": height })
                }
                edge_engine::calib::VfaPlan::EndSpeedLowered(height, end) => {
                    serde_json::json!({ "kind": "end_speed", "layer_height": height, "end": end })
                }
                edge_engine::calib::VfaPlan::NoCandidates => {
                    serde_json::json!({ "kind": "no_candidates" })
                }
            };
        }
        Ok(answer)
    }

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
                CalibMode::RetractionTower => {
                    edge_engine::calib::retraction(params, &settled, &calib_dir, bed.as_ref())
                }
                CalibMode::VolumetricSpeedTower => {
                    edge_engine::calib::max_flowrate(params, &settled, bed.as_ref())
                }
                CalibMode::VfaTower => {
                    edge_engine::calib::vfa(params, &settled, &calib_dir, bed.as_ref())
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
            if value.contains(',') {
                // A list of the extruders' values, as the engine writes it:
                // the pages hold each extruder's own (`key#extruder`), set
                // where it differs from the printer's.
                let had = settled.texts(key).unwrap_or(&[]);
                for (extruder, own) in value.split(',').enumerate() {
                    if had.get(extruder).map(String::as_str) != Some(own) {
                        choices
                            .printer_over
                            .insert(format!("{key}#{extruder}"), own.to_owned());
                    }
                }
            } else {
                choices.printer_over.insert(key.clone(), value.clone());
            }
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

    #[test]
    fn the_retraction_test_is_checked_made_sliced_and_undone() {
        for bad in [
            json!({"test": "retraction", "start": -1, "end": 2, "step": 0.1}),
            json!({"test": "retraction", "start": 0, "end": 2, "step": 0}),
            json!({"test": "retraction", "start": 0, "end": 0.05, "step": 0.1}),
            json!({"test": "retraction", "start": 0, "end": 2}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values"),
                "{bad}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // The dialog's defaults: 0 to 2 by 0.1.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "retraction", "start": 0, "end": 2, "step": 0.1}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Retraction test");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["wipe_inward"], "0", "{choices}");
        assert_eq!(choices["over"]["initial_layer_print_height"], "0.2");
        assert_eq!(choices["over"]["seam_slope_type"], "none");
        assert_eq!(choices["printer_over"]["use_firmware_retraction"], "0");
        let gcode = slice_and_read(&session, id, dir.path());
        // 1.4 + 20 = 21.4 mm: the tower is 40 mm high, so cut; 107 layers.
        assert_eq!(gcode.matches(";LAYER_CHANGE").count(), 107);
        assert!(gcode.contains("; Calib_Retraction_tower: Z_HEIGHT: 21.4, length:2.1"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);
    }

    /// Saves a test's project where `EDGE_CALIB_DUMP` says, for the
    /// reference to slice as a plain print (`edge-parity`).
    fn dump(session: &Arc<Session>, id: u64, name: &str) {
        if let Ok(out) = std::env::var("EDGE_CALIB_DUMP") {
            ask(
                session,
                "project_save",
                json!({ "project": id, "path": std::path::Path::new(&out).join(name).to_string_lossy() }),
            );
        }
    }

    #[test]
    fn the_max_flowrate_test_is_checked_made_sliced_and_undone() {
        for bad in [
            json!({"test": "max_flowrate", "start": 0, "end": 20, "step": 0.5}),
            json!({"test": "max_flowrate", "start": 5, "end": 20, "step": 0}),
            json!({"test": "max_flowrate", "start": 5, "end": 5.2, "step": 0.5}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values"),
                "{bad}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // The dialog's defaults: 5 to 20 mm³/s by 0.5.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "max_flowrate", "start": 5, "end": 20, "step": 0.5}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Max volumetric speed test");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["spiral_mode"], "1", "{choices}");
        assert_eq!(choices["over"]["timelapse_type"], "0");
        assert_eq!(choices["printer_over"]["resonance_avoidance"], "0");
        assert_eq!(choices["filament_over"][0]["slow_down_layer_time"], "0");
        assert_eq!(
            choices["filament_over"][0]["filament_max_volumetric_speed"],
            "200"
        );
        // The U1's greatest layer, 0.32, is under the test's 0.4 × 0.8 in
        // double precision (0.32000000000000006), so the reference raises
        // it, to what is written 0.32 again: nothing shown changed.
        assert!(choices["printer_over"].get("max_layer_height#0").is_none());
        assert!(choices["printer_over"].get("max_layer_height").is_none());
        dump(&session, id, "max_flowrate.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        // Cut at (20 - 5 + 1) / 0.5 = 32 mm: the U1's first layer of
        // 0.25, then 99 of 0.32, to 31.93.
        assert!(gcode.contains("; total layer number: 100"));
        assert!(gcode.contains(";Z:31.930"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);
    }

    #[test]
    fn the_vfa_test_is_checked_made_sliced_and_undone() {
        for bad in [
            json!({"test": "vfa", "start": 10, "end": 200, "step": 10}),
            json!({"test": "vfa", "start": 40, "end": 200, "step": 0}),
            json!({"test": "vfa", "start": 40, "end": 45, "step": 10}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values"),
                "{bad}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // The dialog's check: the U1's PLA melts too little for 200 mm/s
        // at 0.2 mm; a thinner layer of its processes would.
        let check = ask(
            &session,
            "calib_vfa_check",
            json!({"test": "vfa", "start": 40, "end": 200, "step": 10, "resize": true}),
        );
        assert_eq!(check["default_layer_height"], 0.2, "{check}");
        // The dialog's defaults: 40 to 200 mm/s by 10, sized to the nozzle.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "vfa", "start": 40, "end": 200, "step": 10, "resize": true}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "VFA test");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["spiral_mode"], "1", "{choices}");
        assert_eq!(choices["over"]["bottom_shell_thickness"], "0");
        assert_eq!(choices["over"]["initial_layer_print_height"], "0.2");
        assert_eq!(choices["printer_over"]["resonance_avoidance"], "0");
        assert_eq!(choices["filament_over"][0]["slow_down_layer_time"], "0");
        dump(&session, id, "vfa.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        // 17 blocks of 5 mm: 425 layers of 0.2.
        assert!(gcode.contains("; total layer number: 425"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);
    }
}
