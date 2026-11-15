//! The Calibration menu's test prints: for each, the model the reference
//! loads (from its `resources/calib`), where it stands, the settings it
//! sets over the presets and on the object, and what the slice then does
//! at each layer (`edge_gcode::calib`).
//!
//! Ported from Plater.cpp (`calib_pa`, `_calib_pa_tower`) and calib.cpp
//! (`CalibPressureAdvance::find_optimal_PA_speed`). The reference keeps
//! the calibration's parameters on its print, in memory (`Print::
//! set_calib_params`), and forgets them when a model or project is loaded
//! (`load_files`): they are not saved with the project. So here: the
//! project carries them while it is open ([`CalibParams`]), and they go
//! to the slice through [`crate::Overrides::calibration`].

use std::path::Path;

use edge_config::Settings;
use edge_gcode::{AdvanceFirmware, Calibration, CalibrationKind};

use crate::{Bed, CutOptions, Failure, Model, Transform};

/// Which calibration (calib.hpp `CalibMode`), of those built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibMode {
    /// `Calib_PA_Tower`.
    PressureAdvanceTower,
    /// `Calib_Temp_Tower`.
    TemperatureTower,
}

/// What the calibration's dialog asked for (calib.hpp `Calib_Params`).
#[derive(Clone, Debug, PartialEq)]
pub struct CalibParams {
    pub mode: CalibMode,
    pub start: f64,
    pub end: f64,
    pub step: f64,
    pub print_numbers: bool,
    pub accelerations: Vec<f64>,
    pub speeds: Vec<f64>,
}

impl CalibParams {
    pub fn new(mode: CalibMode, start: f64, end: f64, step: f64) -> Self {
        Self {
            mode,
            start,
            end,
            step,
            print_numbers: false,
            accelerations: Vec::new(),
            speeds: Vec::new(),
        }
    }

    /// What the writer does with it, for a printer of these settings.
    pub fn for_writer(&self, settings: &Settings, bambu: bool) -> Calibration {
        let firmware = AdvanceFirmware::of(settings.text("gcode_flavor"), bambu);
        let kind = match self.mode {
            CalibMode::PressureAdvanceTower => CalibrationKind::PressureAdvanceTower {
                start: self.start,
                end: self.end,
                step: self.step,
            },
            CalibMode::TemperatureTower => CalibrationKind::TemperatureTower { start: self.start },
        };
        Calibration { kind, firmware }
    }
}

/// A calibration made ready: the new project's model, its name, the
/// settings it sets over the process, the first filament and the printer
/// presets (key and value as profiles write them), and what the slice is
/// to do with it.
#[derive(Clone, Debug)]
pub struct CalibProject {
    /// The project's name (`new_project`'s `project_name`).
    pub name: String,
    pub model: Model,
    pub process: Vec<(String, String)>,
    pub filament: Vec<(String, String)>,
    pub printer: Vec<(String, String)>,
    pub params: CalibParams,
}

/// The first nozzle's diameter.
fn nozzle_diameter(settings: &Settings) -> Result<f64, Failure> {
    let nozzles = settings.floats("nozzle_diameter")?.unwrap_or_default();
    Ok(nozzles.first().copied().flatten().unwrap_or(0.4))
}

/// The first entry of a list of numbers (a filament's or a process's in
/// the reference's flow variants, `get_value_at`), or a number.
fn first_number(settings: &Settings, key: &str) -> Result<Option<f64>, Failure> {
    Ok(settings
        .floats(key)?
        .and_then(|values| values.first().copied().flatten()))
}

/// A width setting's value in mm (`get_abs_value(key, nozzle)`): a
/// percentage is of the nozzle; 0 is "auto", left to the caller.
fn width(settings: &Settings, key: &str, nozzle: f64) -> Result<f64, Failure> {
    Ok(settings
        .float_or_percent(key)?
        .map_or(0.0, |width| width.resolve(nozzle)))
}

/// Plastic laid per mm of a line (Flow.cpp `Flow::mm3_per_mm`): the
/// reference's flow keeps its width and height in single precision and
/// gives the result in single precision too.
pub fn line_mm3_per_mm(width: f64, height: f64) -> f64 {
    let (width, height) = (f64::from(width as f32), f64::from(height as f32));
    f64::from((height * (width - height * (1.0 - 0.25 * std::f64::consts::PI))) as f32)
}

/// The speed to print the pressure advance test at (calib.cpp
/// `CalibPressureAdvance::find_optimal_PA_speed`): the outer wall speed,
/// at least 100 mm/s, held to what the filament's maximum volumetric
/// speed allows for a line of this width and height, rounded down. A
/// width of 0 is the reference's automatic one (Flow.cpp
/// `auto_extrusion_width`, `frPerimeter`).
pub fn optimal_pa_speed(
    settings: &Settings,
    line_width: f64,
    layer_height: f64,
    filament: usize,
) -> Result<f64, Failure> {
    const SUGGESTED_MIN_SPEED: f64 = 100.0;
    let max_volumetric = settings
        .floats("filament_max_volumetric_speed")?
        .and_then(|values| values.get(filament).or(values.last()).copied().flatten())
        .unwrap_or(0.0);
    let nozzle = nozzle_diameter(settings)? as f32;
    let line_width = if line_width <= 0.0 {
        f64::from(1.125_f32 * nozzle)
    } else {
        line_width
    };
    let outer_wall_speed = first_number(settings, "outer_wall_speed")?.unwrap_or(0.0);
    let speed = SUGGESTED_MIN_SPEED
        .max(outer_wall_speed)
        .min(max_volumetric / line_mm3_per_mm(line_width, layer_height));
    // Returned as a float by the reference.
    Ok(f64::from((speed.floor()) as f32))
}

/// A number as the reference's options write it: `2`, `0.4`, `120`.
fn written(number: f64) -> String {
    edge_gcode::calib::general(number, 15)
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Reads a model the reference ships in `resources/calib` and stands it
/// in the middle of the bed, as loading it into a new project does.
fn load(calib_dir: &Path, file: &str, bed: Option<&Bed>) -> Result<Model, Failure> {
    let path = calib_dir.join(file);
    let read = Model::open(&path).map_err(|error| -> Failure {
        format!(
            "the calibration model {} could not be read: {error}",
            path.display()
        )
        .into()
    })?;
    let mut model = Model::new();
    model.import(bed, &read, None)?;
    Ok(model)
}

/// Which side of a cut is kept.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    Lower,
    Upper,
}

/// Cuts the model's one object at `height` mm above its bottom, when it
/// is taller, keeping one side (Plater.cpp `cut_horizontal`); what is
/// kept stands on the bed.
fn cut(model: &mut Model, bed: Option<&Bed>, height: f64, keep: Keep) -> Result<(), Failure> {
    let copy = model
        .objects()
        .first()
        .and_then(|object| object.copies().first())
        .map(|copy| copy.id())
        .ok_or("the calibration model has nothing in it")?;
    let Some((_, Some([low, high]))) = model.in_scene(bed, copy) else {
        return Err("the calibration model has no printed material".into());
    };
    if height >= high[2] - low[2] {
        return Ok(());
    }
    let plane = Transform {
        rows: [
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [
                (low[0] + high[0]) / 2.0,
                (low[1] + high[1]) / 2.0,
                low[2] + height,
            ],
        ],
    };
    let options = CutOptions {
        keep_upper: keep == Keep::Upper,
        keep_lower: keep == Keep::Lower,
        editable: false,
        ..CutOptions::default()
    };
    model.cut(bed, copy, &plane, &options)?;
    Ok(())
}

/// The model's object's height, in mm.
fn height_of(model: &Model, bed: Option<&Bed>) -> Result<f64, Failure> {
    let copy = model
        .objects()
        .first()
        .and_then(|object| object.copies().first())
        .map(|copy| copy.id())
        .ok_or("the calibration model has nothing in it")?;
    match model.in_scene(bed, copy) {
        Some((_, Some([low, high]))) => Ok(high[2] - low[2]),
        _ => Err("the calibration model has no printed material".into()),
    }
}

/// Sets what the reference sets on the test's object. One this engine
/// does not follow set on an object (`crate::settable`: `seam_position`,
/// the scarf seam) goes over the process instead, which for the test's
/// project, of that one object, gives the same print; set on the object
/// it would be noted after the slice as not followed.
fn set_on_object(
    model: &mut Model,
    bed: Option<&Bed>,
    own: &[(&str, &str)],
    process: &mut Set,
) -> Result<(), Failure> {
    let object = model.objects()[0].id();
    let mut on_object = Vec::new();
    for (key, value) in own {
        if crate::settable(key).followed_on_object {
            on_object.push(((*key).to_owned(), Some((*value).to_owned())));
        } else {
            process.push(((*key).to_owned(), (*value).to_owned()));
        }
    }
    model.set_object_settings(bed, object, &on_object)
}

/// "Pressure advance" (Plater.cpp `calib_pa`). `settings` are those the
/// new project starts from: the presets and what is set over them.
pub fn pressure_advance(
    params: &CalibParams,
    settings: &Settings,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    // `calib_pa`, for every method.
    let mut process = pairs(&[("overhang_reverse", "0")]);
    let filament;
    let printer = pairs(&[("resonance_avoidance", "0")]);
    let model = match params.mode {
        CalibMode::TemperatureTower => {
            return Err("the temperature tower is made by `temperature`".into());
        }
        CalibMode::PressureAdvanceTower => {
            let (model, process_more, filament_more) = pa_tower(params, settings, calib_dir, bed)?;
            process.extend(process_more);
            filament = filament_more;
            model
        }
    };
    // `calib_pa`, after the method's own: no inward wipe hiding the test.
    process.push(("wipe_inward".to_owned(), "0".to_owned()));
    Ok(CalibProject {
        name: "Pressure Advance Test".to_owned(),
        model,
        process,
        filament,
        printer,
        params: params.clone(),
    })
}

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// "Temperature" (Plater.cpp `calib_temp`): the tower of 10 mm blocks,
/// 350 °C at its foot and 5 °C less each block up, cut above the block of
/// `end` and below the block of `start`; the filament's temperatures set
/// to `start`, which the slice then lowers block by block.
pub fn temperature(
    params: &CalibParams,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let mut model = load(calib_dir, "temperature_tower/temperature_tower.stl", bed)?;
    let start = params.start.round();
    let start_text = written(start);
    let filament = pairs(&[
        ("nozzle_temperature_initial_layer", start_text.as_str()),
        ("nozzle_temperature", start_text.as_str()),
    ]);
    let mut process = Vec::new();
    set_on_object(
        &mut model,
        bed,
        &[
            ("brim_type", "outer_only"),
            ("brim_width", "5"),
            ("brim_object_gap", "0"),
            ("alternate_extra_wall", "0"),
            ("seam_slope_type", "none"),
            ("overhang_reverse", "0"),
        ],
        &mut process,
    )?;
    // Cut the upper blocks: those above `end`'s. EPSILON over the block's
    // top so the cut misses its flat face. (Upstream OrcaSlicer eac352f95d,
    // #11861, cuts EPSILON under it instead; measured here it changes no
    // layer, the layers being cut at their middles, so the pin's is kept.)
    let blocks = ((350.0 - params.end) / 5.0 + 1.0).round();
    if blocks > 0.0 {
        let height = blocks * 10.0 + EPSILON;
        if height < height_of(&model, bed)? {
            cut(&mut model, bed, height, Keep::Lower)?;
        }
    }
    // And the lower: those below `start`'s.
    let blocks = ((350.0 - params.start) / 5.0).round();
    if blocks > 0.0 {
        let height = blocks * 10.0 + EPSILON;
        if height < height_of(&model, bed)? {
            cut(&mut model, bed, height, Keep::Upper)?;
        }
    }
    Ok(CalibProject {
        name: "Nozzle temperature test".to_owned(),
        model,
        process,
        filament,
        printer: pairs(&[("resonance_avoidance", "0")]),
        params: params.clone(),
    })
}

/// Settings set on one side or another.
type Set = Vec<(String, String)>;

/// Plater.cpp `_calib_pa_tower`: the tower with its seam groove, walls at
/// the test's speed, no top, bottom or fill, ear brim; cut to one mm a
/// step, plus one.
fn pa_tower(
    params: &CalibParams,
    settings: &Settings,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<(Model, Set, Set), Failure> {
    let mut model = load(calib_dir, "pressure_advance/tower_with_seam.stl", bed)?;
    let nozzle = nozzle_diameter(settings)?;
    let layer_height = settings
        .float_or_percent("layer_height")?
        .map_or(0.2, |h| h.resolve(nozzle));
    let wall_speed = optimal_pa_speed(
        settings,
        width(settings, "line_width", nozzle)?,
        layer_height,
        0,
    )?;
    let speed = written(wall_speed);
    let mut process = pairs(&[("max_volumetric_extrusion_rate_slope", "0")]);
    let own = [
        ("alternate_extra_wall", "0"),
        ("outer_wall_speed", speed.as_str()),
        ("inner_wall_speed", speed.as_str()),
        ("seam_position", "back"),
        ("wall_loops", "2"),
        ("top_shell_layers", "0"),
        ("bottom_shell_layers", "0"),
        ("sparse_infill_density", "0%"),
        ("brim_type", "brim_ears"),
        ("brim_object_gap", "0"),
        ("brim_ears_max_angle", "135"),
        ("brim_width", "6"),
        ("seam_slope_type", "none"),
    ];
    set_on_object(&mut model, bed, &own, &mut process)?;
    // One mm a step, plus the first: `std::ceil((end - start) / step) + 1`.
    let height = ((params.end - params.start) / params.step).ceil() + 1.0;
    cut(&mut model, bed, height, Keep::Lower)?;
    Ok((model, process, pairs(&[("slow_down_layer_time", "1")])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: &str) -> Settings {
        Settings::from_json(json).unwrap()
    }

    #[test]
    fn the_temperature_tower_keeps_the_blocks_from_start_to_end() {
        let calib = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/resources/calib");
        let bed = Bed::of([0.0, 0.0, 270.0, 270.0]);
        // 230 to 190: blocks 350 - 5k; upper cut at lround(160 / 5 + 1) =
        // 33 blocks, 330 mm; lower at lround(120 / 5) = 24 blocks, 240 mm:
        // 9 blocks of 10 mm, 230 down to 190.
        let params = CalibParams::new(CalibMode::TemperatureTower, 230.0, 190.0, 5.0);
        let made = temperature(&params, &calib, Some(&bed)).unwrap();
        assert_eq!(made.name, "Nozzle temperature test");
        assert_eq!(
            made.filament,
            pairs(&[
                ("nozzle_temperature_initial_layer", "230"),
                ("nozzle_temperature", "230")
            ])
        );
        let height = height_of(&made.model, Some(&bed)).unwrap();
        assert!((height - 90.0).abs() < 0.01, "{height}");
        let own = made.model.objects()[0].settings();
        assert!(
            own.contains(&("brim_type".into(), "outer_only".into())),
            "{own:?}"
        );
    }

    #[test]
    fn the_pa_speed_is_the_outer_wall_speed_held_to_the_filament_flow() {
        // calib.cpp `find_optimal_PA_speed`, worked by hand: a 0.4 nozzle,
        // auto width 0.45, 0.2 layers: 0.2 * (0.45 - 0.2 * 0.2146) =
        // 0.081416 mm³/mm; 21 mm³/s / that = 257.9 mm/s; the outer wall's
        // 200 is under it and over 100: 200.
        let s = settings(
            r#"{"nozzle_diameter": ["0.4"], "filament_max_volumetric_speed": ["21"],
                "outer_wall_speed": ["200"]}"#,
        );
        assert_eq!(optimal_pa_speed(&s, 0.0, 0.2, 0).unwrap(), 200.0);
        // 12 mm³/s: 12 / 0.081416 = 147.39, rounded down, under the
        // outer wall's 300.
        let s = settings(
            r#"{"nozzle_diameter": ["0.4"], "filament_max_volumetric_speed": ["12"],
                "outer_wall_speed": ["300"]}"#,
        );
        assert_eq!(optimal_pa_speed(&s, 0.0, 0.2, 0).unwrap(), 147.0);
        // A slow outer wall is raised to 100.
        let s = settings(
            r#"{"nozzle_diameter": ["0.4"], "filament_max_volumetric_speed": ["40"],
                "outer_wall_speed": ["60"]}"#,
        );
        assert_eq!(optimal_pa_speed(&s, 0.42, 0.2, 0).unwrap(), 100.0);
    }

    #[test]
    fn a_line_holds_what_the_reference_flow_holds() {
        let area = line_mm3_per_mm(0.45, 0.2);
        assert!((area - 0.081_415_93).abs() < 1e-6, "{area}");
    }

    #[test]
    fn the_pa_tower_is_cut_to_a_mm_a_step_and_set_as_the_reference_sets_it() {
        let calib = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/resources/calib");
        let bed = Bed::of([0.0, 0.0, 270.0, 270.0]);
        let s = settings(
            r#"{"nozzle_diameter": ["0.4"], "filament_max_volumetric_speed": ["21"],
                "outer_wall_speed": ["200"], "line_width": "0.42", "layer_height": "0.2"}"#,
        );
        // 0 to 0.04 by 0.002: 21 mm.
        let params = CalibParams::new(CalibMode::PressureAdvanceTower, 0.0, 0.04, 0.002);
        let made = pressure_advance(&params, &s, &calib, Some(&bed)).unwrap();
        assert_eq!(made.name, "Pressure Advance Test");
        assert_eq!(
            made.process,
            pairs(&[
                ("overhang_reverse", "0"),
                ("max_volumetric_extrusion_rate_slope", "0"),
                // Set on the object by the reference; not followed on an
                // object here, so over the process.
                ("seam_position", "back"),
                ("brim_ears_max_angle", "135"),
                ("seam_slope_type", "none"),
                ("wipe_inward", "0"),
            ])
        );
        assert_eq!(made.filament, pairs(&[("slow_down_layer_time", "1")]));
        assert_eq!(made.printer, pairs(&[("resonance_avoidance", "0")]));
        let object = &made.model.objects()[0];
        let own = object.settings();
        assert!(
            own.contains(&("outer_wall_speed".into(), "200".into())),
            "{own:?}"
        );
        assert!(own.contains(&("brim_type".into(), "brim_ears".into())));
        let copy = object.copies()[0].id();
        let (_, reach) = made.model.in_scene(Some(&bed), copy).unwrap();
        let [low, high] = reach.unwrap();
        assert!((high[2] - low[2] - 21.0).abs() < 1e-3, "{low:?} {high:?}");
        // In the middle of the bed.
        assert!(((low[0] + high[0]) / 2.0 - 135.0).abs() < 1.0);
    }
}
