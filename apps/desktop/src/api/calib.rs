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
        Some("shaping_frequency") => CalibMode::InputShapingFrequency,
        Some("shaping_damping") => CalibMode::InputShapingDamping,
        Some("cornering") => CalibMode::Cornering,
        Some("stringing") => CalibMode::StringingGrid,
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
    if matches!(
        mode,
        CalibMode::InputShapingFrequency | CalibMode::InputShapingDamping
    ) {
        return shaping_params(mode, args);
    }
    if mode == CalibMode::Cornering {
        // Cornering_Test_Dlg::on_start: junction deviation (mm) up to 1,
        // else jerk (mm/s) up to 100.
        let deviation = args.get("junction_deviation").and_then(Value::as_bool) == Some(true);
        let most = if deviation { 1.0 } else { 100.0 };
        let (start, end) = (number("start"), number("end"));
        return match (start, end) {
            (Ok(start), Ok(end)) if start >= 0.0 && end <= most && start < end => {
                let mut params = CalibParams::new(mode, start, end, 0.1);
                params.test_model = args
                    .get("model")
                    .and_then(Value::as_u64)
                    .map_or(0, |model| model.min(2) as u32);
                Ok(params)
            }
            _ => Err(super::i18n::fill(
                &super::i18n::tr("Please input valid values:\n(0 <= Cornering <= %s)"),
                &[&format!("{most:.3}")],
            )),
        };
    }
    if mode == CalibMode::StringingGrid {
        return stringing_params(args);
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

/// The stringing grid dialog's values as read
/// (`TempRetract_Calibration_Dlg::read_params`): the numbers, the radius
/// only with the fillet on; checked against the bed by the caller.
fn stringing_params(args: &Value) -> Result<CalibParams, String> {
    let number = |key: &str| {
        args.get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    let tick =
        |key: &str, otherwise: bool| args.get(key).and_then(Value::as_bool).unwrap_or(otherwise);
    let invalid = || super::i18n::tr("Please input valid numbers.");
    let (Some(start), Some(end), Some(step)) = (number("start"), number("end"), number("step"))
    else {
        return Err(invalid());
    };
    let mut params = CalibParams::new(CalibMode::StringingGrid, start, end, step);
    let grid = &mut params.grid;
    grid.band_height = number("band_height").ok_or_else(invalid)?;
    grid.retract_start = number("retract_start").ok_or_else(invalid)?;
    grid.retract_end = number("retract_end").ok_or_else(invalid)?;
    grid.retract_step = number("retract_step").ok_or_else(invalid)?;
    grid.fillet_base = tick("fillet", true);
    if grid.fillet_base {
        grid.fillet_radius = number("fillet_radius").ok_or_else(invalid)?;
    }
    grid.print_labels = tick("labels", false);
    grid.wait_for_temperature = tick("wait", true);
    grid.disable_z_hop = tick("no_z_hop", false);
    Ok(params)
}

/// One of the stringing grid's checks (calib.cpp
/// `stringing_grid_params_valid`, whose message ids the engine gives) in
/// the language shown.
fn grid_message(id: &str) -> String {
    use super::i18n::tr;
    match id {
        "Temperature must be between 150 and 350 C" => {
            tr("Temperature must be between 150 and 350 C")
        }
        "Start temperature must be >= end temperature" => {
            tr("Start temperature must be >= end temperature")
        }
        "Temperature step must be between 1 and 20 C" => {
            tr("Temperature step must be between 1 and 20 C")
        }
        "The temperature range must be a multiple of the temperature step" => {
            tr("The temperature range must be a multiple of the temperature step")
        }
        "Band count must be between 2 and 10" => tr("Band count must be between 2 and 10"),
        "Band height must be between 5 and 20 mm" => tr("Band height must be between 5 and 20 mm"),
        "Total height must be <= 150 mm" => tr("Total height must be <= 150 mm"),
        "Retraction must be between 0 and 10 mm" => tr("Retraction must be between 0 and 10 mm"),
        "Retraction end must be >= start" => tr("Retraction end must be >= start"),
        "Retraction step must be > 0" => tr("Retraction step must be > 0"),
        "Fillet radius must be between 0.5 and %1% mm" => {
            tr("Fillet radius must be between 0.5 and %1% mm")
        }
        "Column count must be at least 2" => tr("Column count must be at least 2"),
        "Grid does not fit the bed (max %1% columns at 12 mm pitch)" => {
            tr("Grid does not fit the bed (max %1% columns at 12 mm pitch)")
        }
        other => tr(other),
    }
}

/// The stringing grid's check, in the reference's words.
fn stringing_check(params: &CalibParams, bed_width: f64) -> Result<(), String> {
    edge_engine::calib::stringing::params_valid(params, bed_width).map_err(|error| {
        let said = grid_message(error.message);
        match error.value {
            Some(value) => super::i18n::fill(&said, &[&value]),
            None => said,
        }
    })
}

/// The input shaping dialogs' values and their checks
/// (`Input_Shaping_Freq_Test_Dlg::on_start`,
/// `Input_Shaping_Damp_Test_Dlg::on_start`). RepRapFirmware (`reprap`)
/// takes one frequency (range) for both axes.
fn shaping_params(mode: CalibMode, args: &Value) -> Result<CalibParams, String> {
    let number = |key: &str| {
        args.get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
    };
    let reprap = args.get("reprap").and_then(Value::as_bool) == Some(true);
    let mut params = CalibParams::new(mode, 0.0, 1.0, 0.1);
    params.test_model = u32::from(args.get("model").and_then(Value::as_u64) == Some(1));
    params.shaper_type = args
        .get("shaper")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if mode == CalibMode::InputShapingFrequency {
        let (x0, x1) = (number("freq_start_x"), number("freq_end_x"));
        let (y0, y1) = if reprap {
            (x0, x1)
        } else {
            (number("freq_start_y"), number("freq_end_y"))
        };
        let damp = number("damping");
        let (Some(x0), Some(x1), Some(y0), Some(y1), Some(damp)) = (x0, x1, y0, y1, damp) else {
            return Err(super::i18n::tr(
                "Please input valid values:\n(0 < FreqStart < FreqEnd < 500)",
            ));
        };
        if x0 < 0.0
            || x1 > 500.0
            || (!reprap && (y0 < 0.0 || y1 > 500.0))
            || x0 >= x1
            || (!reprap && y0 >= y1)
        {
            return Err(super::i18n::tr(
                "Please input valid values:\n(0 < FreqStart < FreqEnd < 500)",
            ));
        }
        if !(0.0..1.0).contains(&damp) {
            return Err(super::i18n::tr(
                "Please input a valid damping factor (0 < Damping/zeta factor <= 1)",
            ));
        }
        params.freq_x = (x0, x1);
        params.freq_y = (y0, y1);
        params.start = damp;
    } else {
        let x = number("freq_x");
        let y = if reprap { x } else { number("freq_y") };
        let (start, end) = (number("damping_start"), number("damping_end"));
        let (Some(x), Some(y), Some(start), Some(end)) = (x, y, start, end) else {
            return Err(super::i18n::tr(
                "Please input valid values:\n(0 < Freq < 500)",
            ));
        };
        if !(0.0..=500.0).contains(&x) || (!reprap && !(0.0..=500.0).contains(&y)) {
            return Err(super::i18n::tr(
                "Please input valid values:\n(0 < Freq < 500)",
            ));
        }
        if start < 0.0 || end > 1.0 || start >= end {
            return Err(super::i18n::tr(
                "Please input a valid damping factor (0 <= DampingStart < DampingEnd <= 1)",
            ));
        }
        params.freq_x = (x, 1.0);
        params.freq_y = (y, 1.0);
        params.start = start;
        params.end = end;
    }
    Ok(params)
}

/// The input shaper types a firmware is offered (Tab.cpp
/// `input_shaper_types_for_flavor`, "Disable" left out as calib_dlg.cpp
/// `get_shaper_type_values` leaves it), as `input_shaping_type` writes them.
fn shaper_types(flavor: Option<&str>) -> &'static [&'static str] {
    match flavor {
        Some("klipper") => &["Default", "ZV", "MZV", "ZVD", "EI", "2HUMP_EI", "3HUMP_EI"],
        Some("reprapfirmware") => &[
            "Default", "MZV", "ZVD", "ZVDD", "ZVDDD", "EI2", "EI3", "DAA",
        ],
        Some("marlin2") => &["ZV"],
        _ => &["Default"],
    }
}

impl Session {
    /// What a calibration dialog shows that depends on the printer
    /// (`calib_printer`): its `gcode_flavor`, the input shaper types it is
    /// offered, and whether it corners by junction deviation (Marlin 2
    /// with `machine_max_junction_deviation` over 0).
    pub(super) fn calib_printer(&self) -> Result<Value, String> {
        let (_, settled) = self.calibration_settings()?;
        let flavor = settled.text("gcode_flavor").map(str::to_owned);
        let deviation = settled
            .floats("machine_max_junction_deviation")
            .ok()
            .flatten()
            .and_then(|values| values.first().copied().flatten())
            .unwrap_or(0.0);
        Ok(serde_json::json!({
            "flavor": flavor,
            "shaper_types": shaper_types(flavor.as_deref()),
            "junction_deviation": flavor.as_deref() == Some("marlin2") && deviation > 0.0,
        }))
    }

    /// The stringing grid dialog (`calib_stringing`): with no values, its
    /// defaults from the first filament (`TempRetract_Calibration_Dlg::
    /// reset_to_defaults`); with them, the line under its fields
    /// (`update_summary`): what is wrong, or the columns and bands.
    pub(super) fn calib_stringing(&self, args: &Value) -> Result<Value, String> {
        use edge_engine::calib::stringing;
        let (_, settled) = self.calibration_settings()?;
        let bed_width = stringing::bed_width(&settled);
        let most = stringing::max_columns(bed_width).min(17);
        if args.get("start").is_none() {
            let first = |key: &str| -> f64 {
                settled
                    .floats(key)
                    .ok()
                    .flatten()
                    .and_then(|values| values.first().copied().flatten())
                    .unwrap_or(0.0)
            };
            // F14: the printer has no greatest nozzle temperature of its own.
            let (params, needs_range) = stringing::default_params(
                first("nozzle_temperature"),
                first("nozzle_temperature_range_low"),
                first("nozzle_temperature_range_high"),
                5.0,
                0.0,
            );
            let grid = &params.grid;
            return Ok(serde_json::json!({
                "start": params.start, "end": params.end, "step": params.step,
                "band_height": grid.band_height, "retract_start": grid.retract_start,
                "retract_end": grid.retract_end, "retract_step": grid.retract_step,
                "labels": grid.print_labels, "wait": grid.wait_for_temperature,
                "no_z_hop": grid.disable_z_hop, "fillet": grid.fillet_base,
                "fillet_radius": grid.fillet_radius,
                "max_fillet_radius": stringing::max_fillet_radius(),
                "needs_range": needs_range,
            }));
        }
        let summary = stringing_params(args)
            .and_then(|params| stringing_check(&params, bed_width).map(|()| params))
            .map(|params| {
                super::i18n::fill(
                    &super::i18n::tr(
                        "%d columns × %d bands of %.0f mm  (max %d columns on this bed)",
                    ),
                    &[
                        &stringing::retract_schedule(&params).len(),
                        &stringing::band_count(&params),
                        &params.grid.band_height,
                        &most,
                    ],
                )
            });
        Ok(match summary {
            Ok(summary) => serde_json::json!({ "ok": true, "summary": summary }),
            Err(error) => serde_json::json!({ "ok": false, "summary": error }),
        })
    }

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
                CalibMode::InputShapingFrequency | CalibMode::InputShapingDamping => {
                    edge_engine::calib::input_shaping(params, &settled, &calib_dir, bed.as_ref())
                }
                CalibMode::Cornering => {
                    edge_engine::calib::cornering(params, &settled, &calib_dir, bed.as_ref())
                }
                CalibMode::StringingGrid => {
                    let width = edge_engine::calib::stringing::bed_width(&settled);
                    stringing_check(params, width)?;
                    edge_engine::calib::stringing::stringing_grid(params, &settled, bed.as_ref())
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

    #[test]
    fn the_input_shaping_tests_are_checked_made_sliced_and_undone() {
        for bad in [
            json!({"test": "shaping_frequency", "freq_start_x": 110, "freq_end_x": 15,
                   "freq_start_y": 15, "freq_end_y": 110, "damping": 0.15}),
            json!({"test": "shaping_frequency", "freq_start_x": 15, "freq_end_x": 510,
                   "freq_start_y": 15, "freq_end_y": 110, "damping": 0.15}),
            json!({"test": "shaping_damping", "freq_x": 600, "freq_y": 30,
                   "damping_start": 0, "damping_end": 0.4}),
        ] {
            assert!(
                params(&bad)
                    .unwrap_err()
                    .starts_with("Please input valid values"),
                "{bad}"
            );
        }
        let damp = params(
            &json!({"test": "shaping_frequency", "freq_start_x": 15, "freq_end_x": 110,
                   "freq_start_y": 15, "freq_end_y": 110, "damping": 1}),
        );
        assert!(
            damp.unwrap_err()
                .starts_with("Please input a valid damping factor")
        );
        let damp = params(
            &json!({"test": "shaping_damping", "freq_x": 30, "freq_y": 30,
                   "damping_start": 0.4, "damping_end": 0.4}),
        );
        assert!(
            damp.unwrap_err()
                .starts_with("Please input a valid damping factor")
        );
        // RepRapFirmware: one range for both axes.
        let one = params(
            &json!({"test": "shaping_frequency", "reprap": true, "freq_start_x": 20,
                   "freq_end_x": 90, "damping": 0}),
        )
        .unwrap();
        assert_eq!((one.freq_x, one.freq_y), ((20.0, 90.0), (20.0, 90.0)));

        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        let printer = ask(&session, "calib_printer", json!({}));
        assert_eq!(printer["flavor"], "klipper");
        assert_eq!(
            printer["shaper_types"],
            json!(["Default", "ZV", "MZV", "ZVD", "EI", "2HUMP_EI", "3HUMP_EI"])
        );
        assert_eq!(printer["junction_deviation"], false);
        // The dialog's defaults, the first type.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "shaping_frequency", "model": 0, "shaper": "Default",
                   "freq_start_x": 15, "freq_end_x": 110, "freq_start_y": 15, "freq_end_y": 110,
                   "damping": 0.15}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Input shaping Frequency test");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["spiral_mode"], "1", "{choices}");
        assert_eq!(choices["over"]["default_jerk"], "0");
        assert_eq!(choices["over"]["accel_to_decel_factor"], "100%");
        assert_eq!(choices["printer_over"]["input_shaping_emit"], "0");
        // The U1's PLA has pressure advance off: on at 0 for the test.
        assert_eq!(choices["filament_over"][0]["enable_pressure_advance"], "1");
        assert_eq!(choices["filament_over"][0]["pressure_advance"], "0");
        dump(&session, id, "shaping_frequency.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        assert!(gcode.contains("SET_INPUT_SHAPER DAMPING_RATIO_X=0.150 DAMPING_RATIO_Y=0.150"));
        assert!(gcode.contains("SET_INPUT_SHAPER SHAPER_FREQ_X=110.00 SHAPER_FREQ_Y=110.00"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);

        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "shaping_damping", "model": 1, "shaper": "ZV",
                   "freq_x": 30, "freq_y": 42.5, "damping_start": 0, "damping_end": 0.4}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({ "project": id }));
        assert_eq!(detail["name"], "Input shaping Damping test");
        dump(&session, id, "shaping_damping.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        assert!(gcode.contains("SET_INPUT_SHAPER SHAPER_TYPE=ZV SHAPER_FREQ_X=30.00"));
        assert!(gcode.contains("SET_INPUT_SHAPER SHAPER_TYPE=ZV SHAPER_FREQ_Y=42.50"));
        assert!(gcode.contains("SET_INPUT_SHAPER DAMPING_RATIO_X=0.400 DAMPING_RATIO_Y=0.400"));
    }

    #[test]
    fn the_cornering_test_is_checked_made_sliced_and_undone() {
        for (bad, most) in [
            (
                json!({"test": "cornering", "start": -1, "end": 15}),
                "100.000",
            ),
            (
                json!({"test": "cornering", "start": 1, "end": 101}),
                "100.000",
            ),
            (
                json!({"test": "cornering", "start": 15, "end": 15}),
                "100.000",
            ),
            (
                json!({"test": "cornering", "junction_deviation": true, "start": 0, "end": 1.2}),
                "1.000",
            ),
        ] {
            assert_eq!(
                params(&bad).unwrap_err(),
                format!("Please input valid values:\n(0 <= Cornering <= {most})"),
                "{bad}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // The dialog's defaults on a Klipper printer: jerk 1 to 15 mm/s.
        let listing = ask(
            &session,
            "calibrate",
            json!({"test": "cornering", "model": 2, "start": 1, "end": 15}),
        );
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Cornering test");
        assert_eq!(detail["dirty"], true);
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["spiral_mode"], "1", "{choices}");
        assert_eq!(choices["over"]["default_jerk"], "0");
        assert_eq!(choices["printer_over"]["machine_max_jerk_x"], "15");
        assert_eq!(choices["printer_over"]["machine_max_jerk_y"], "15");
        // A Klipper printer's input shaping is left as it is.
        assert!(choices["printer_over"].get("input_shaping_emit").is_none());
        assert_eq!(
            choices["filament_over"][0]["filament_max_volumetric_speed"],
            "200"
        );
        dump(&session, id, "cornering.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        assert!(gcode.contains("SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY=1\n"));
        assert!(gcode.contains("SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY=15\n"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);
    }

    #[test]
    fn the_stringing_grid_opens_on_the_filament_and_is_checked_made_and_sliced() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::new(None));
        choose_presets(&session);
        // The dialog's defaults from the U1's first filament.
        let d = ask(&session, "calib_stringing", json!({}));
        assert_eq!(d["band_height"], 8.0, "{d}");
        assert_eq!(d["retract_start"], 0.2);
        assert_eq!(d["fillet_radius"], 3.0);
        assert_eq!(d["needs_range"], false);
        let args = |end: f64, radius: f64| {
            json!({"test": "stringing", "start": 230, "end": end, "step": 5, "band_height": 8,
                   "retract_start": 0.2, "retract_end": 1.6, "retract_step": 0.2,
                   "fillet": true, "fillet_radius": radius, "labels": false, "wait": true,
                   "no_z_hop": true})
        };
        // The line under the fields: what will be printed, or what is wrong.
        let line = ask(&session, "calib_stringing", args(205.0, 3.0));
        assert_eq!(line["ok"], true);
        assert_eq!(
            line["summary"],
            "8 columns × 6 bands of 8 mm  (max 17 columns on this bed)"
        );
        let line = ask(&session, "calib_stringing", args(205.0, 4.0));
        assert_eq!(
            line["summary"],
            "Fillet radius must be between 0.5 and 3 mm"
        );
        let line = ask(&session, "calib_stringing", args(207.0, 3.0));
        assert_eq!(
            line["summary"],
            "The temperature range must be a multiple of the temperature step"
        );
        let refused = session.call("calibrate", &args(240.0, 3.0)).unwrap_err();
        assert_eq!(refused, "Start temperature must be >= end temperature");
        let listing = ask(&session, "calibrate", args(205.0, 3.0));
        let id = listing["id"].as_u64().unwrap();
        let detail = ask(&session, "project", json!({}));
        assert_eq!(detail["name"], "Stringing temp × retraction");
        let choices = &detail["choices"];
        assert_eq!(choices["over"]["print_sequence"], "by layer", "{choices}");
        assert_eq!(choices["over"]["initial_layer_print_height"], "0.2");
        assert_eq!(choices["printer_over"]["use_firmware_retraction"], "0");
        assert_eq!(choices["filament_over"][0]["filament_z_hop"], "0");
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 8);
        dump(&session, id, "stringing.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        assert!(gcode.contains("; total layer number: 245"));
        assert!(gcode.contains("; Calib_TempRetract: band=5 temp=205\nM109 S205 ;"));
        assert!(gcode.contains("; Calib_TempRetract: col=7 retract=1.6\n"));
        ask(&session, "undo", json!({ "project": id }));
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 0);

        // A Marlin printer, labelled: the label tower before the columns.
        let session = Arc::new(Session::new(None));
        let machine = "Creality Ender-3 V3 SE 0.4 nozzle";
        let presets = ask(
            &session,
            "presets",
            json!({ "vendor": "Creality", "machine": machine }),
        );
        ask(
            &session,
            "project_set",
            json!({
                "source": "presets", "vendor": "Creality", "machine": machine,
                "process": presets["processes"][0],
                "filaments": [presets["filaments"][0]], "initial": true,
            }),
        );
        let mut labelled = args(205.0, 3.0);
        labelled["labels"] = json!(true);
        labelled["no_z_hop"] = json!(false);
        let listing = ask(&session, "calibrate", labelled);
        let id = listing["id"].as_u64().unwrap();
        let objects = ask(&session, "plate_view", json!({ "project": id }));
        assert_eq!(objects["objects"].as_array().map_or(0, Vec::len), 9);
        dump(&session, id, "stringing_marlin.3mf");
        let gcode = slice_and_read(&session, id, dir.path());
        assert!(gcode.contains("; Calib_TempRetract: col=none retract="));
    }
}
