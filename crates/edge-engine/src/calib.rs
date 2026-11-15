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

use crate::{Bed, CutOptions, Failure, Model, Reach, Transform};

/// Which calibration (calib.hpp `CalibMode`), of those built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibMode {
    /// `Calib_PA_Tower`.
    PressureAdvanceTower,
    /// `Calib_Temp_Tower`.
    TemperatureTower,
    /// `Calib_PA_Line`.
    PressureAdvanceLine,
    /// `Calib_PA_Pattern`.
    PressureAdvancePattern,
    /// `Calib_Retraction_tower`.
    RetractionTower,
    /// `Calib_Vol_speed_Tower`: "Max flowrate".
    VolumetricSpeedTower,
    /// `Calib_VFA_Tower`: "VFA".
    VfaTower,
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
    /// "PA Pattern": the handles on the plate being sliced
    /// ([`CalibParams::on_plate`]), each pattern drawn round its own.
    pub handles: Vec<PatternHandle>,
    /// "VFA": the layer height the tower prints at (0: half the nozzle),
    /// which the dialog may lower (`vfa_layer_height`).
    pub vfa_layer_height: f64,
    /// "VFA": the model sized to the nozzle (`nozzle_based_resize`).
    pub nozzle_based_resize: bool,
}

/// A "PA Pattern" handle cube on the plate being sliced: its box across
/// the plate (min x, min y, max x, max y) and the speed and acceleration
/// it carries of its own, when it does.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternHandle {
    pub reach: [f64; 4],
    pub speed: Option<f64>,
    pub acceleration: Option<f64>,
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
            handles: Vec::new(),
            vfa_layer_height: 0.0,
            nozzle_based_resize: true,
        }
    }

    /// The parameters for slicing `plate` of `model`: for "PA Pattern",
    /// with the handles standing on it where they now stand (Plater.cpp
    /// `_calib_pa_pattern_gen_gcode`, run before each slice).
    pub fn on_plate(&self, model: &Model, plate: Option<u32>) -> CalibParams {
        let mut params = self.clone();
        if self.mode != CalibMode::PressureAdvancePattern {
            return params;
        }
        let number = |object: &crate::ModelObject, key: &str| {
            object
                .settings()
                .iter()
                .find(|(own, _)| own == key)
                .and_then(|(_, value)| value.split(',').next()?.trim().parse().ok())
        };
        params.handles = model
            .objects()
            .iter()
            .filter(|object| object.name().starts_with("pa_pattern_"))
            .filter_map(|object| {
                let copy = object.copies().first()?;
                let on = model
                    .plate_of(copy.id())
                    .and_then(|at| model.plates().get(at))
                    .map(|plate| plate.id());
                if plate.is_some() && on != plate {
                    return None;
                }
                let [low, high] = copy.reach()?;
                Some(PatternHandle {
                    reach: [low[0], low[1], high[0], high[1]],
                    speed: number(object, "outer_wall_speed"),
                    acceleration: number(object, "outer_wall_acceleration"),
                })
            })
            .collect();
        params
    }

    /// "PA Pattern": the G-code each of the first layers carries, every
    /// handle's pattern together (Plater.cpp `_calib_pa_pattern_gen_gcode`,
    /// `CalibPressureAdvancePattern::generate_custom_gcodes`).
    pub fn pattern_layers(
        &self,
        settings: &Settings,
        pen: &edge_gcode::Pen,
    ) -> Result<Vec<(String, f64)>, Failure> {
        let nozzle = nozzle_diameter(settings)?;
        let mut layers: Vec<(String, f64)> = Vec::new();
        for handle in &self.handles {
            let speed = match handle.speed {
                Some(speed) => speed,
                None => process_number(settings, "outer_wall_speed")?,
            };
            let acceleration = match handle.acceleration {
                Some(acceleration) => acceleration,
                None => process_number(settings, "outer_wall_acceleration")?,
            };
            let test = pattern_test(self, settings, nozzle, speed, acceleration, handle.reach)?;
            for (at, (text, fed)) in edge_gcode::pa_pattern(&test, pen).into_iter().enumerate() {
                match layers.get_mut(at) {
                    Some(layer) => {
                        layer.0.push_str(&text);
                        layer.1 += fed;
                    }
                    None => layers.push((text, fed)),
                }
            }
        }
        Ok(layers)
    }

    /// What the writer does with it, for a printer of these settings.
    pub fn for_writer(&self, settings: &Settings, bambu: bool) -> Result<Calibration, Failure> {
        let firmware = AdvanceFirmware::of(settings.text("gcode_flavor"), bambu);
        let kind = match self.mode {
            CalibMode::PressureAdvanceTower => CalibrationKind::PressureAdvanceTower {
                start: self.start,
                end: self.end,
                step: self.step,
            },
            CalibMode::TemperatureTower => CalibrationKind::TemperatureTower { start: self.start },
            CalibMode::PressureAdvanceLine => {
                CalibrationKind::PressureAdvanceLine(Box::new(self.pa_line(settings)?))
            }
            // Made by the slice from its objects ([`Self::pa_pattern`]).
            CalibMode::PressureAdvancePattern => {
                CalibrationKind::PressureAdvancePattern(Vec::new())
            }
            CalibMode::RetractionTower => CalibrationKind::RetractionTower {
                start: self.start,
                step: self.step,
            },
            CalibMode::VolumetricSpeedTower => CalibrationKind::VolumetricSpeedTower {
                start: self.start,
                step: self.step,
            },
            CalibMode::VfaTower => CalibrationKind::VfaTower {
                start: self.start,
                end: self.end,
                step: self.step,
            },
        };
        let jerk = |axis: &str| -> Result<f64, Failure> {
            Ok(first_number(settings, &format!("machine_max_jerk_{axis}"))?.unwrap_or(0.0))
        };
        Ok(Calibration {
            kind,
            firmware,
            comments: settings.flag("gcode_comments")?.unwrap_or(false),
            max_jerk: [jerk("x")?, jerk("y")?, jerk("z")?, jerk("e")?],
        })
    }

    /// What "PA Line" draws with (GCode.cpp `_do_export`,
    /// `CalibPressureAdvanceLine`), from the slice's settings and the
    /// first filament's.
    fn pa_line(&self, settings: &Settings) -> Result<edge_gcode::PaLine, Failure> {
        let corners = settings.texts("printable_area").unwrap_or(&[]);
        let mut bed: Option<[f64; 4]> = None;
        for corner in corners {
            let (x, y) = corner
                .split_once('x')
                .ok_or_else(|| format!("printable_area: {corner} is not a corner"))?;
            let (x, y): (f64, f64) = (x.trim().parse()?, y.trim().parse()?);
            let [a, b, c, d] = bed.unwrap_or([x, y, x, y]);
            bed = Some([a.min(x), b.min(y), c.max(x), d.max(y)]);
        }
        let bed = bed.ok_or("PA Line: the printer has no printable_area")?;
        let nozzle = nozzle_diameter(settings)?;
        let layer_height = settings
            .float_or_percent("initial_layer_print_height")?
            .map_or(0.2, |h| h.resolve(nozzle));
        let diameter = filament_number(settings, "filament_diameter", 0)?;
        let filament_flow = filament_number(settings, "filament_flow_ratio", 0)?;
        let print_flow = settings.float("print_flow_ratio")?.unwrap_or(1.0);
        let area = std::f64::consts::PI * (diameter / 2.0).powi(2);
        let mut line = edge_gcode::PaLine {
            start: self.start,
            step: self.step,
            count: ((self.end - self.start) / self.step).ceil().round() as i64 + 1,
            print_numbers: self.print_numbers,
            bed,
            delta: corners.len() > 4,
            nozzle_diameter: nozzle,
            layer_height,
            e_per_mm3: if area > 0.0 {
                print_flow * filament_flow / area
            } else {
                0.0
            },
            fast: 0.0,
            acceleration: first_number(settings, "outer_wall_acceleration")?.unwrap_or(0.0),
            jerk: first_number(settings, "outer_wall_jerk")?.unwrap_or(0.0),
        };
        // Orca #16882: capped by the volumetric speed of the filament the
        // test prints with.
        line.fast = optimal_pa_speed(settings, line.line_width(), layer_height, 0)?;
        Ok(line)
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
    /// What the slice is to do with it: none for a test whose slice is an
    /// ordinary one (the flow rate tiles: Plater.cpp `calib_flowrate`
    /// keeps no parameters on the print).
    pub params: Option<CalibParams>,
    /// What the reference says in its notifications while making it, as
    /// message id and the value it is followed by.
    pub notes: Vec<(String, String)>,
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

pub use edge_gcode::calib::line_mm3_per_mm;

/// The first entry of a filament's list of numbers.
fn filament_number(settings: &Settings, key: &str, filament: usize) -> Result<f64, Failure> {
    Ok(settings
        .floats(key)?
        .and_then(|values| values.get(filament).or(values.last()).copied().flatten())
        .unwrap_or(0.0))
}

/// The speed to print the pressure advance test at
/// ([`edge_gcode::calib::optimal_pa_speed`]) for these settings.
pub fn optimal_pa_speed(
    settings: &Settings,
    line_width: f64,
    layer_height: f64,
    filament: usize,
) -> Result<f64, Failure> {
    Ok(edge_gcode::calib::optimal_pa_speed(
        filament_number(settings, "filament_max_volumetric_speed", filament)?,
        first_number(settings, "outer_wall_speed")?.unwrap_or(0.0),
        nozzle_diameter(settings)?,
        line_width,
        layer_height,
    ))
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
    let mut printer = pairs(&[("resonance_avoidance", "0")]);
    let mut notes = Vec::new();
    let model = match params.mode {
        CalibMode::PressureAdvancePattern => {
            let made = pa_pattern(params, settings, bed)?;
            process.extend(made.process);
            printer.extend(made.printer);
            filament = made.filament;
            notes = made.notes;
            made.model
        }
        CalibMode::TemperatureTower
        | CalibMode::RetractionTower
        | CalibMode::VolumetricSpeedTower
        | CalibMode::VfaTower => {
            return Err("that test is not a pressure advance test".into());
        }
        CalibMode::PressureAdvanceLine => {
            // Plater.cpp `calib_pa`: the model only stands for the test,
            // which GCode.cpp draws in place of its layers.
            filament = Vec::new();
            load(calib_dir, "pressure_advance/pressure_advance_test.stl", bed)?
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
        params: Some(params.clone()),
        notes,
    })
}

/// What "PA Pattern" sets and stands on the plate.
struct Pattern {
    model: Model,
    process: Set,
    filament: Set,
    printer: Set,
    notes: Vec<(String, String)>,
}

/// Whether the printer takes Marlin's junction deviation (Plater.cpp
/// `has_junction_deviation`).
fn junction_deviation(settings: &Settings) -> Result<bool, Failure> {
    Ok(settings.text("gcode_flavor") == Some("marlin2")
        && first_number(settings, "machine_max_junction_deviation")?.unwrap_or(0.0) > 0.0)
}

/// A process setting's first value, or 0.
fn process_number(settings: &Settings, key: &str) -> Result<f64, Failure> {
    Ok(first_number(settings, key)?.unwrap_or(0.0))
}

/// The geometry of one pattern for the settings the test prints with,
/// its handle at `handle` (min x, min y, max x, max y).
#[allow(clippy::too_many_arguments)]
fn pattern_test(
    params: &CalibParams,
    settings: &Settings,
    nozzle: f64,
    speed: f64,
    acceleration: f64,
    handle: [f64; 4],
) -> Result<edge_gcode::PaPattern, Failure> {
    let diameter = filament_number(settings, "filament_diameter", 0)?;
    let flow_ratio = filament_number(settings, "filament_flow_ratio", 0)?;
    let area = std::f64::consts::PI * (diameter / 2.0).powi(2);
    let width = |key: &str| -> Result<f64, Failure> {
        // 0 is "auto" (calib.hpp `line_width`): `frExternalPerimeter`.
        let set = settings
            .float_or_percent(key)?
            .map_or(0.0, |w| w.resolve(nozzle));
        Ok(if set > 0.0 {
            set
        } else {
            f64::from(1.125_f32 * nozzle as f32)
        })
    };
    let height = |key: &str| -> Result<f64, Failure> {
        Ok(settings
            .float_or_percent(key)?
            .map_or(0.2, |h| h.resolve(nozzle)))
    };
    Ok(edge_gcode::PaPattern {
        start: params.start,
        end: params.end,
        step: params.step,
        nozzle_diameter: nozzle,
        line_width: width("line_width")?,
        first_line_width: width("initial_layer_line_width")?,
        layer_height: height("layer_height")?,
        first_layer_height: height("initial_layer_print_height")?,
        walls: settings.integer("wall_loops")?.unwrap_or(3) as i32,
        first_speed: process_number(settings, "initial_layer_speed")?,
        speed,
        acceleration,
        z_hop: first_number(settings, "z_hop")?.unwrap_or(0.0),
        e_per_mm3: if area > 0.0 { flow_ratio / area } else { 0.0 },
        flow_ratio,
        handle,
    })
}

/// Plater.cpp `_calib_pa_pattern`: the settings the pattern prints with,
/// then one 5 mm "handle" cube for each speed and acceleration, standing
/// where the reference's arrangement of the patterns puts it; the slice
/// draws each pattern round its handle ([`CalibParams::pa_pattern`]).
fn pa_pattern(
    params: &CalibParams,
    presets: &Settings,
    bed: Option<&Bed>,
) -> Result<Pattern, Failure> {
    let nozzle = nozzle_diameter(presets)?;
    let mut process: Set = Vec::new();
    let filament = pairs(&[
        ("filament_retract_when_changing_layer", "0"),
        ("filament_wipe", "0"),
    ]);
    let printer = pairs(&[("wipe", "0"), ("retract_when_changing_layer", "0")]);
    let mut notes = Vec::new();
    // The acceleration: the outer wall's, else the inner wall's, else
    // the default; with several asked for, the greatest.
    let mut accel = process_number(presets, "outer_wall_acceleration")?;
    if accel == 0.0 {
        accel = process_number(presets, "inner_wall_acceleration")?;
    }
    if accel == 0.0 {
        accel = process_number(presets, "default_acceleration")?;
    }
    let accels = if params.accelerations.is_empty() {
        notes.push((
            "No accelerations provided for calibration. Use default acceleration value ".to_owned(),
            format!("{}mm/s²", accel.trunc() as i64),
        ));
        vec![accel]
    } else {
        accel = params
            .accelerations
            .iter()
            .copied()
            .fold(f64::MIN, f64::max);
        params.accelerations.clone()
    };
    process.push(("outer_wall_acceleration".to_owned(), written(accel)));
    process.push(("print_sequence".to_owned(), "by layer".to_owned()));
    let deviation = junction_deviation(presets)?;
    if !deviation && process_number(presets, "default_jerk")? > 0.0 {
        let mut jerk = process_number(presets, "outer_wall_jerk")?;
        if jerk == 0.0 {
            jerk = process_number(presets, "inner_wall_jerk")?;
        }
        if jerk == 0.0 {
            jerk = process_number(presets, "default_jerk")?;
        }
        for key in [
            "default_jerk",
            "outer_wall_jerk",
            "inner_wall_jerk",
            "top_surface_jerk",
            "infill_jerk",
            "travel_jerk",
        ] {
            process.push((key.to_owned(), written(jerk)));
        }
    }
    if deviation {
        process.push(("default_junction_deviation".to_owned(), "0".to_owned()));
    }
    // `SuggestedConfigCalibPAPattern`.
    let line_width = nozzle * 112.5 / 100.0;
    process.push(("initial_layer_speed".to_owned(), "30".to_owned()));
    process.push(("line_width".to_owned(), written(line_width)));
    process.push((
        "initial_layer_line_width".to_owned(),
        written(nozzle * 140.0 / 100.0),
    ));
    process.push(("skirt_loops".to_owned(), "0".to_owned()));
    process.push(("wall_loops".to_owned(), "3".to_owned()));
    process.push(("brim_type".to_owned(), "no_brim".to_owned()));
    // The settings as they now stand, for the test's speed and size.
    let mut settings = presets.clone();
    for (key, value) in &process {
        settings.insert(key.clone(), edge_config::Value::One(value.clone()));
    }
    let layer_height = settings
        .float_or_percent("layer_height")?
        .map_or(0.2, |h| h.resolve(nozzle));
    let speeds = if params.speeds.is_empty() {
        let speed = optimal_pa_speed(&settings, line_width, layer_height, 0)?;
        process.push(("outer_wall_speed".to_owned(), written(speed)));
        notes.push((
            "No speeds provided for calibration. Use default optimal speed ".to_owned(),
            format!("{}mm/s", speed.trunc() as i64),
        ));
        vec![speed]
    } else {
        if params.speeds.len() == 1 {
            process.push(("outer_wall_speed".to_owned(), written(params.speeds[0])));
        }
        params.speeds.clone()
    };
    // Each test's place: the patterns, 4 mm apart, arranged on the bed as
    // the reference arranges them, the largest acceleration's size for
    // all (`_calib_pa_pattern`).
    let geometry = pattern_test(params, &settings, nozzle, speeds[0], accel, [0.0; 4])?;
    let tests = speeds.len() * accels.len();
    let (w, d, z) = (
        geometry.print_size_x() + 4.0,
        geometry.print_size_y() + 4.0,
        geometry.max_layer_z(),
    );
    let block = |w: f64, d: f64, name: &str| -> Model {
        let soup = edge_mesh::primitives::cuboid(
            [-(w / 2.0) as f32, -(d / 2.0) as f32, 0.0],
            [(w / 2.0) as f32, (d / 2.0) as f32, z as f32],
        );
        Model::from(crate::load::Opened::mesh(
            name,
            edge_mesh::Mesh::from_triangle_soup(soup),
        ))
    };
    let mut places = Model::new();
    let footprint = block(w, d, "Cube");
    for _ in 0..tests {
        places.import(bed, &footprint, None)?;
    }
    if tests > 1 {
        places.arrange(bed, None)?;
    }
    let offset = geometry.handle_offset();
    let handle = block(
        edge_gcode::PA_HANDLE_SIZE,
        edge_gcode::PA_HANDLE_SIZE,
        "Cube",
    );
    let mut model = Model::new();
    for (index, object) in places.objects().iter().enumerate() {
        let copy = object.copies()[0].id();
        let Some((_, Some([low, high]))) = places.in_scene(bed, copy) else {
            return Err("PA Pattern: a pattern has no place".into());
        };
        let centre = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
        // On the plate the arrangement put it on, made when it is not there yet.
        let plate = match places.plate_of(copy) {
            // The first plate is the one a model's first import makes.
            Some(0) | None => None,
            Some(at) => {
                while model.plates().len() <= at {
                    model.add_plate(bed)?;
                }
                Some(model.plates()[at].id())
            }
        };
        let made = model.import(bed, &handle, plate)?;
        let at = made[0];
        let speed = speeds[index % speeds.len()];
        let accel_here = accels[index / speeds.len()];
        let to = Transform::translation([centre[0] + offset[0], centre[1] + offset[1], 0.0]);
        let scene = model
            .in_scene(bed, at)
            .map(|(scene, _)| scene)
            .unwrap_or(Transform::IDENTITY);
        // Where it stands in the scene, its own turn and size kept.
        let mut standing = scene;
        standing.rows[3] = to.rows[3];
        model.set_transforms(bed, &[(at, standing)])?;
        let object = model
            .objects()
            .last()
            .map(|object| object.id())
            .ok_or("PA Pattern: no handle")?;
        model.rename(
            bed,
            object,
            &format!("pa_pattern_{}_{}", speed as i64, accel_here as i64),
        )?;
        let mut own = Vec::new();
        if speeds.len() > 1 {
            own.push(("outer_wall_speed".to_owned(), Some(written(speed))));
        }
        if accels.len() > 1 {
            own.push((
                "outer_wall_acceleration".to_owned(),
                Some(written(accel_here)),
            ));
        }
        if !own.is_empty() {
            model.set_object_settings(bed, object, &own)?;
        }
    }
    Ok(Pattern {
        model,
        process,
        filament,
        printer,
        notes,
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
    let notes = Vec::new();
    Ok(CalibProject {
        name: "Nozzle temperature test".to_owned(),
        model,
        process,
        filament,
        printer: pairs(&[("resonance_avoidance", "0")]),
        params: Some(params.clone()),
        notes,
    })
}

/// Settings set on one side or another.
type Set = Vec<(String, String)>;

/// "Retraction test" (Plater.cpp `calib_retraction`): the retraction
/// tower, a layer height the nozzle can print (Orca #11835), two walls, no
/// top or fill, cut to `1.4 + (end - start) / step` mm; the slice then
/// lengthens the retraction by `step` every mm above its 0.4 mm base
/// (`edge_gcode::calib::tower_retraction`). `settings` are those the
/// project starts from.
pub fn retraction(
    params: &CalibParams,
    settings: &Settings,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let mut model = load(calib_dir, "retraction/retraction_tower.stl", bed)?;
    let nozzle = nozzle_diameter(settings)?;
    let layer_height = if nozzle <= 0.1 {
        "0.05"
    } else if nozzle <= 0.2 {
        "0.1"
    } else {
        "0.2"
    };
    let height: f64 = layer_height.parse()?;
    let mut process = pairs(&[
        ("wipe_inward", "0"),
        ("initial_layer_print_height", layer_height),
    ]);
    let mut printer = Vec::new();
    // The printer's first extruder's greatest layer height raised to the
    // test's, where it is under it.
    if let Some(most) = settings.floats("max_layer_height")?
        && let Some(Some(first)) = most.first()
        && *first < height
    {
        let mut values: Vec<String> = most
            .iter()
            .map(|value| value.map_or_else(|| "nil".to_owned(), written))
            .collect();
        values[0] = layer_height.to_owned();
        printer.push(("max_layer_height".to_owned(), values.join(",")));
    }
    printer.extend(pairs(&[
        ("resonance_avoidance", "0"),
        ("use_firmware_retraction", "0"),
    ]));
    set_on_object(
        &mut model,
        bed,
        &[
            ("wall_loops", "2"),
            ("top_shell_layers", "0"),
            ("bottom_shell_layers", "3"),
            ("sparse_infill_density", "0%"),
            ("layer_height", layer_height),
            ("alternate_extra_wall", "0"),
            ("seam_position", "aligned"),
            ("wall_sequence", "inner wall/outer wall"),
            ("overhang_reverse", "0"),
            // Ahead of the pin: OrcaSlicer 5302f703c0 (#15543), no scarf
            // seam, whose sloped start hides the retraction's effect.
            ("seam_slope_type", "none"),
        ],
        &mut process,
    )?;
    let cut_at = 1.0 + 0.4 + (params.end - params.start) / params.step;
    if cut_at < height_of(&model, bed)? {
        cut(&mut model, bed, cut_at, Keep::Lower)?;
    }
    Ok(CalibProject {
        name: "Retraction test".to_owned(),
        model,
        process,
        filament: Vec::new(),
        printer,
        params: Some(params.clone()),
        notes: Vec::new(),
    })
}

/// The reference's "Max flowrate" model, `volumetric_speed/
/// SpeedTestStructure.step`, as its STEP reader makes it a mesh: this
/// engine reads no STEP file, so the mesh is kept here, made once by the
/// pin's command line (`--export-stl`, which tessellates with the GUI's
/// defaults, a linear deflection of 0.003 mm and an angle of 0.5,
/// `load_step`): 70036 triangles, 180 × 100 × 280 mm.
const SPEED_TEST_STRUCTURE: &[u8] = include_bytes!("../calib/SpeedTestStructure.stl");

/// Scales the model's one object about its middle, its foot kept on the
/// bed (`ModelObject::scale`, then `ensure_on_bed`).
fn scale_object(model: &mut Model, bed: Option<&Bed>, scale: [f64; 3]) -> Result<(), Failure> {
    let copy = model
        .objects()
        .first()
        .and_then(|object| object.copies().first())
        .map(|copy| copy.id())
        .ok_or("the calibration model has nothing in it")?;
    let Some((scene, Some([low, high]))) = model.in_scene(bed, copy) else {
        return Err("the calibration model has no printed material".into());
    };
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, low[2]];
    let sized = scene
        .then(&Transform::translation([
            -middle[0], -middle[1], -middle[2],
        ]))
        .then(&Transform {
            rows: [
                [scale[0], 0.0, 0.0],
                [0.0, scale[1], 0.0],
                [0.0, 0.0, scale[2]],
                [0.0; 3],
            ],
        })
        .then(&Transform::translation(middle));
    model.set_transforms(bed, &[(copy, sized)])
}

/// The printable area's box (min x, min y, max x, max y).
fn bed_box(settings: &Settings) -> Result<[f64; 4], Failure> {
    let mut bed: Option<[f64; 4]> = None;
    for corner in settings.texts("printable_area").unwrap_or(&[]) {
        let (x, y) = corner
            .split_once('x')
            .ok_or_else(|| format!("printable_area: {corner} is not a corner"))?;
        let (x, y): (f64, f64) = (x.trim().parse()?, y.trim().parse()?);
        let [a, b, c, d] = bed.unwrap_or([x, y, x, y]);
        bed = Some([a.min(x), b.min(y), c.max(x), d.max(y)]);
    }
    Ok(bed.ok_or("the printer has no printable_area")?)
}

/// The printer's first `max_layer_height` raised to `height` where it is
/// under it, as a printer setting; none when it is not.
fn raised_max_layer_height(
    settings: &Settings,
    height: f64,
) -> Result<Option<(String, String)>, Failure> {
    if let Some(most) = settings.floats("max_layer_height")?
        && let Some(Some(first)) = most.first()
        && *first < height
    {
        let mut values: Vec<String> = most
            .iter()
            .map(|value| value.map_or_else(|| "nil".to_owned(), written))
            .collect();
        values[0] = written(height);
        return Ok(Some(("max_layer_height".to_owned(), values.join(","))));
    }
    Ok(None)
}

/// "Max flowrate" (Plater.cpp `calib_max_vol_speed`): the speed test
/// structure, narrowed to the bed when wider, one wall 1.75 nozzles wide in
/// layers of 0.8 nozzle, printed as a vase; cut to `(end - start + 1) /
/// step` mm. The slice then speeds the outer wall up with the height so
/// that it melts `start` mm³/s at the bed and `step` more each mm
/// (`edge_gcode::calib::volumetric_tower_speed`). `settings` are those the
/// project starts from.
pub fn max_flowrate(
    params: &CalibParams,
    settings: &Settings,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let mesh = edge_mesh::stl::parse(SPEED_TEST_STRUCTURE)
        .map_err(|error| format!("the speed test structure could not be read: {error}"))?;
    let mut model = Model::new();
    model.import(
        bed,
        &Model::from(crate::load::Opened::mesh("SpeedTestStructure", mesh)),
        None,
    )?;
    // Narrowed to the bed less 10 mm, in X only.
    let area = bed_box(settings)?;
    let width = {
        let copy = model.objects()[0].copies()[0].id();
        match model.in_scene(bed, copy) {
            Some((_, Some([low, high]))) => high[0] - low[0],
            _ => return Err("the speed test structure has no printed material".into()),
        }
    };
    let narrow = (area[2] - area[0] - 10.0) / width;
    if narrow < 1.0 {
        scale_object(&mut model, bed, [narrow, 1.0, 1.0])?;
    }
    let nozzle = nozzle_diameter(settings)?;
    let line_width = nozzle * 1.75;
    let layer_height = nozzle * 0.8;
    let mut printer = Vec::new();
    printer.extend(raised_max_layer_height(settings, layer_height)?);
    printer.push(("resonance_avoidance".to_owned(), "0".to_owned()));
    // Orca #13658: never lower the filament's own limit.
    let max_volumetric = filament_number(settings, "filament_max_volumetric_speed", 0)?;
    let filament = vec![
        (
            "filament_max_volumetric_speed".to_owned(),
            written(max_volumetric.max(200.0)),
        ),
        ("slow_down_layer_time".to_owned(), "0".to_owned()),
    ];
    let mut process = Vec::new();
    let line_width_text = written(line_width);
    let layer_height_text = written(layer_height);
    set_on_object(
        &mut model,
        bed,
        &[
            ("enable_overhang_speed", "0"),
            ("wall_loops", "1"),
            ("alternate_extra_wall", "0"),
            ("top_shell_layers", "0"),
            ("bottom_shell_layers", "0"),
            ("sparse_infill_density", "0%"),
            ("overhang_reverse", "0"),
            ("outer_wall_line_width", line_width_text.as_str()),
            ("layer_height", layer_height_text.as_str()),
            ("brim_type", "outer_and_inner"),
            ("brim_width", "5"),
            ("brim_object_gap", "0"),
        ],
        &mut process,
    )?;
    process.extend(pairs(&[
        ("timelapse_type", "0"),
        ("spiral_mode", "1"),
        ("max_volumetric_extrusion_rate_slope", "0"),
    ]));
    // Cut the top: `(end - start + 1) / step` mm.
    let height = (params.end - params.start + 1.0) / params.step;
    if height < height_of(&model, bed)? {
        cut(&mut model, bed, height, Keep::Lower)?;
    }
    // The volumetric speeds as the outer wall's speeds: over the plastic
    // the line lays per mm, at the filament's flow ratio.
    let flow_ratio = filament_number(settings, "filament_flow_ratio", 0)?;
    let mm3_per_mm = line_mm3_per_mm(line_width, layer_height) * flow_ratio;
    if mm3_per_mm <= 0.0 {
        return Err("Max flowrate: the line lays no plastic (filament_flow_ratio is 0)".into());
    }
    let mut speeds = params.clone();
    speeds.start = params.start / mm3_per_mm;
    speeds.end = params.end / mm3_per_mm;
    speeds.step = params.step / mm3_per_mm;
    Ok(CalibProject {
        name: "Max volumetric speed test".to_owned(),
        model,
        process,
        filament,
        printer,
        params: Some(speeds),
        notes: Vec::new(),
    })
}

/// The VFA tower's base: blocks of 5 mm, each 25 layers of 0.2 mm, made
/// for a 0.4 mm nozzle (calib.hpp, Orca #14759).
const VFA_LAYERS_PER_BLOCK: f64 = 25.0;
const VFA_BASE_BLOCK_HEIGHT: f64 = 5.0;
const VFA_BASE_NOZZLE_DIAMETER: f64 = 0.4;

/// The fastest outer wall speed, mm/s, the filament's maximum volumetric
/// speed allows a line of this width and layer height (calib.cpp
/// `calib_vfa_speed_limit`).
pub fn vfa_speed_limit(max_volumetric_speed: f64, line_width: f64, layer_height: f64) -> f64 {
    let mm3_per_mm = line_mm3_per_mm(line_width, layer_height);
    if mm3_per_mm > 0.0 {
        max_volumetric_speed / mm3_per_mm
    } else {
        1e9
    }
}

/// What the VFA dialog does when the end speed is over what the filament
/// can melt at the tower's layer height (calib.hpp `CalibVfaPlan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VfaPlan {
    /// The end speed is reached at the default layer height.
    Fits,
    /// This layer height, one the printer's processes use, reaches it.
    LayerHeightReduced(f64),
    /// Even the smallest is too thick: that layer height, and the end
    /// speed lowered to this (a whole number of steps).
    EndSpeedLowered(f64, f64),
    /// No process layer height to take.
    NoCandidates,
}

/// calib.cpp `calib_vfa_plan`: the largest of the process layer heights
/// (within the machine's limits, none above the default) that reaches the
/// end speed; else the smallest, the end speed lowered to the limit there,
/// snapped down to a step (at least `start + step`).
#[allow(clippy::too_many_arguments)]
pub fn vfa_plan(
    start: f64,
    end: f64,
    step: f64,
    default_layer_height: f64,
    mut candidates: Vec<f64>,
    machine_min_layer_height: f64,
    machine_max_layer_height: f64,
    max_volumetric_speed: f64,
    line_width: f64,
    nozzle_diameter: f64,
) -> VfaPlan {
    let limit = |height: f64| vfa_speed_limit(max_volumetric_speed, line_width, height);
    if max_volumetric_speed <= 0.0 || nozzle_diameter <= 0.0 || end <= limit(default_layer_height) {
        return VfaPlan::Fits;
    }
    candidates.retain(|&height| {
        !(height <= 0.0
            || (machine_min_layer_height > 0.0 && height < machine_min_layer_height - 1e-6)
            || (machine_max_layer_height > 0.0 && height > machine_max_layer_height + 1e-6))
    });
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let Some(&smallest) = candidates.first() else {
        return VfaPlan::NoCandidates;
    };
    for &height in candidates.iter().rev() {
        if height > default_layer_height + 1e-6 {
            continue;
        }
        if limit(height) >= end {
            return VfaPlan::LayerHeightReduced(height);
        }
    }
    let mut lowered = if step > 0.0 {
        (limit(smallest) / step).floor() * step
    } else {
        limit(smallest)
    };
    if lowered < start + step {
        lowered = start + step;
    }
    VfaPlan::EndSpeedLowered(smallest, lowered)
}

/// "VFA" (Plater.cpp `calib_VFA`): the VFA tower of 5 mm blocks, cut to
/// one block a step (EPSILON under the block's top), sized to the nozzle
/// with each block 25 layers of the test's layer height when asked
/// (`nozzle_based_resize`); one wall, one bottom layer, vase mode. The
/// slice then steps the outer wall's speed from `start` to `end` over the
/// layers (`edge_gcode::calib::vfa_tower_speed`). `settings` are those the
/// project starts from.
pub fn vfa(
    params: &CalibParams,
    settings: &Settings,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let mut model = load(calib_dir, "vfa/VFA.stl", bed)?;
    let mut nozzle = nozzle_diameter(settings)?;
    if nozzle <= 0.0 {
        nozzle = VFA_BASE_NOZZLE_DIAMETER;
    }
    let layer_height = if params.vfa_layer_height > 0.0 {
        params.vfa_layer_height
    } else {
        nozzle / 2.0
    };
    // Cut on the model as it is, at the base blocks' height; EPSILON under
    // the block's top so the cut misses its flat face.
    let height =
        VFA_BASE_BLOCK_HEIGHT * ((params.end - params.start) / params.step + 1.0) - EPSILON;
    if height < height_of(&model, bed)? {
        cut(&mut model, bed, height, Keep::Lower)?;
    }
    if params.nozzle_based_resize {
        let xy = nozzle / VFA_BASE_NOZZLE_DIAMETER;
        let z = VFA_LAYERS_PER_BLOCK * layer_height / VFA_BASE_BLOCK_HEIGHT;
        if (xy - 1.0).abs() > EPSILON || (z - 1.0).abs() > EPSILON {
            scale_object(&mut model, bed, [xy, xy, z])?;
        }
    }
    let mut process = pairs(&[
        ("enable_overhang_speed", "0"),
        ("timelapse_type", "0"),
        ("wall_loops", "1"),
        ("alternate_extra_wall", "0"),
        ("top_shell_layers", "0"),
        ("bottom_shell_layers", "1"),
        // Ahead of the pin: EdgeSlicer 5c07d87e33 (on the fork's main):
        // with one bottom layer, no bottom thickness either, which would
        // add solid layers under the test (a vase's bottom follows it).
        ("bottom_shell_thickness", "0"),
        ("sparse_infill_density", "0%"),
        ("overhang_reverse", "0"),
        ("detect_thin_wall", "0"),
        ("spiral_mode", "1"),
    ]);
    let layer_height_text = written(layer_height);
    let mut own = vec![
        ("brim_type", "outer_only"),
        ("brim_width", "3"),
        ("brim_object_gap", "0"),
    ];
    if params.nozzle_based_resize {
        process.push((
            "initial_layer_print_height".to_owned(),
            layer_height_text.clone(),
        ));
        own.insert(0, ("layer_height", layer_height_text.as_str()));
    }
    set_on_object(&mut model, bed, &own, &mut process)?;
    let mut made = params.clone();
    made.vfa_layer_height = if params.nozzle_based_resize {
        layer_height
    } else {
        0.0
    };
    Ok(CalibProject {
        name: "VFA test".to_owned(),
        model,
        process,
        filament: pairs(&[("slow_down_layer_time", "0")]),
        printer: pairs(&[("resonance_avoidance", "0")]),
        params: Some(made),
        notes: Vec::new(),
    })
}

/// Which of the Flow rate menu's tests (Plater.cpp `calib_flowrate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowTest {
    /// "Pass 1": tiles from -20 % to +20 % flow.
    Pass1,
    /// "Pass 2": tiles from 0 to -9 %.
    Pass2,
    /// "YOLO (Recommended)": tiles 0.01 of flow ratio apart.
    Yolo,
    /// "YOLO (perfectionist version)": tiles 0.005 apart.
    YoloPerfectionist,
}

impl FlowTest {
    /// Whether the tiles step the flow ratio itself (`is_linear`), and the
    /// pass (1 or 2).
    fn linear_pass(self) -> (bool, i32) {
        match self {
            Self::Pass1 => (false, 1),
            Self::Pass2 => (false, 2),
            Self::Yolo => (true, 1),
            Self::YoloPerfectionist => (true, 2),
        }
    }
}

/// The flow ratio the tile named `flowrate_<modifier>` prints with
/// (Plater.cpp `adjust_settings_for_flowrate_calib`): the modifier is
/// read after the name's 9 letters, an `m` as a minus, as a float (`stof`,
/// 1 when it cannot be read); the YOLO tiles add it to the filament's
/// flow ratio, the passes take it as a percentage.
pub fn tile_flow_ratio(name: &str, linear: bool, filament_flow_ratio: f64) -> f64 {
    let mut modifier = name.get(9..).unwrap_or("").to_owned();
    if modifier.starts_with('m') {
        modifier.replace_range(0..1, "-");
    }
    // `stof` reads the longest number at the start.
    let read = (1..=modifier.len())
        .rev()
        .find_map(|end| modifier.get(..end)?.parse::<f32>().ok())
        .unwrap_or(1.0);
    if linear {
        (filament_flow_ratio + f64::from(read)) / filament_flow_ratio
    } else {
        f64::from(1.0_f32 + read / 100.0)
    }
}

/// The settings Plater.cpp `adjust_settings_for_flowrate_calib` sets on
/// each tile, but the tile's own flow ratio; the speeds are worked out by
/// [`flow_tile_speeds`].
fn flow_tile_settings(nozzle: f32, internal_solid: f64, top: f64) -> Vec<(&'static str, String)> {
    let wide = format!("{}", nozzle * 1.2);
    let mut own: Vec<(&'static str, String)> = [
        ("wall_loops", "1"),
        ("only_one_wall_top", "1"),
        ("thick_internal_bridges", "0"),
        ("enable_extra_bridge_layer", "disabled"),
        ("internal_bridge_density", "100%"),
        ("sparse_infill_density", "35%"),
        ("min_width_top_surface", "100%"),
        ("bottom_shell_layers", "2"),
        ("top_shell_layers", "5"),
        ("top_shell_thickness", "0"),
        ("bottom_shell_thickness", "0"),
        ("detect_thin_wall", "1"),
        ("filter_out_gap_fill", "0"),
        ("sparse_infill_pattern", "rectilinear"),
    ]
    .into_iter()
    .map(|(key, value)| (key, value.to_owned()))
    .collect();
    own.push(("top_surface_line_width", wide.clone()));
    own.push(("internal_solid_infill_line_width", wide));
    for (key, value) in [
        ("top_surface_pattern", "archimedeanchords"),
        // `ipCount`, which this setting writes "default".
        ("undertop_surface_pattern", "default"),
        ("top_solid_infill_flow_ratio", "1"),
        ("infill_direction", "45"),
        ("solid_infill_direction", "135"),
        ("center_of_surface_pattern", "each_surface"),
        ("separated_infills", "0"),
        ("align_infill_direction_to_model", "1"),
        ("ironing_type", "no ironing"),
    ] {
        own.push((key, value.to_owned()));
    }
    own.push(("internal_solid_infill_speed", written(internal_solid)));
    own.push(("top_surface_speed", written(top)));
    for (key, value) in [
        ("seam_slope_type", "none"),
        ("gap_fill_target", "nowhere"),
        ("calib_flowrate_topinfill_special_order", "1"),
        ("top_surface_fill_order", "default"),
    ] {
        own.push((key, value.to_owned()));
    }
    own
}

/// The speeds the tiles' solid fill and top surfaces are held to, mm/s
/// (`adjust_settings_for_flowrate_calib`): what the filament's maximum
/// volumetric speed allows for a line 1.2 nozzles wide at half a nozzle
/// high, at the greatest flow any tile prints with, rounded down.
#[allow(clippy::too_many_arguments)]
pub fn flow_tile_speeds(
    linear: bool,
    pass: i32,
    nozzle: f32,
    layer_height: f64,
    flow_ratio: f64,
    max_volumetric_speed: f64,
    internal_solid_speed: f64,
    top_surface_speed: f64,
) -> (f64, f64) {
    let infill = line_mm3_per_mm(f64::from(nozzle * 1.2), layer_height);
    let fastest = if linear {
        max_volumetric_speed
            / (infill * (flow_ratio + if pass == 2 { 0.035 } else { 0.05 }) / flow_ratio)
    } else {
        max_volumetric_speed / (infill * if pass == 1 { 1.2 } else { 1.0 })
    };
    (
        internal_solid_speed.min(fastest).floor(),
        top_surface_speed.min(fastest).floor(),
    )
}

/// "Flow rate" (Plater.cpp `calib_flowrate`,
/// `adjust_settings_for_flowrate_calib`): the test's tiles, each an
/// object of its own with its own flow ratio, as the file places them in
/// the middle of the bed; sized in height to ten layers of half a nozzle
/// (and across, for a nozzle over 0.72 mm); a top of Archimedean chords
/// whose ridge is read. `settings` are those the project starts from.
///
/// Every tile carries the same settings but its flow ratio. Of those, the
/// ones this engine does not follow set on an object go over the process
/// instead, which for the test's project, of these tiles alone, prints
/// the same; one that differs from tile to tile and is not followed would
/// be an error (none does: `print_flow_ratio` is followed).
pub fn flow_rate(
    test: FlowTest,
    settings: &Settings,
    calib_dir: &Path,
    bed: Option<&Bed>,
) -> Result<CalibProject, Failure> {
    let (linear, pass) = test.linear_pass();
    let (name, file) = match test {
        FlowTest::Pass1 => ("Flowrate Test - Pass1", "flowrate-test-pass1.3mf"),
        FlowTest::Pass2 => ("Flowrate Test - Pass2", "flowrate-test-pass2.3mf"),
        FlowTest::Yolo => ("Orca YOLO Flow Calibration", "Orca-LinearFlow.3mf"),
        FlowTest::YoloPerfectionist => (
            "Orca YOLO Flow Calibration - Perfectionist version",
            "Orca-LinearFlow_fine.3mf",
        ),
    };
    let path = calib_dir.join("filament_flow").join(file);
    let read = Model::open(&path).map_err(|error| -> Failure {
        format!(
            "the calibration model {} could not be read: {error}",
            path.display()
        )
        .into()
    })?;
    // Sized for a 0.6 nozzle across, and ten layers high.
    let nozzle = nozzle_diameter(settings)? as f32;
    let xy_scale = nozzle / 0.6;
    let layer_height = f64::from(nozzle) / 2.0;
    let first_layer_height = settings
        .float("initial_layer_print_height")?
        .unwrap_or(0.2)
        .max(layer_height);
    let z_scale = ((first_layer_height + 9.0 * layer_height) / 2.0) as f32;
    let across = if xy_scale > 1.2 { xy_scale } else { 1.0 };
    let scale = [f64::from(across), f64::from(across), f64::from(z_scale)];

    // The tiles where the file has them, the group in the middle of the
    // bed (Plater.cpp `load_files`, `center_instances_around_point`), then
    // sized about the group's middle.
    let centre = |reach: &Reach| {
        [
            (reach[0][0] + reach[1][0]) / 2.0,
            (reach[0][1] + reach[1][1]) / 2.0,
        ]
    };
    let mut placed: Vec<[f64; 2]> = Vec::new();
    let mut group: Option<Reach> = None;
    for object in read.objects() {
        let reach = object
            .copies()
            .first()
            .and_then(|copy| copy.reach())
            .ok_or("a flow rate tile has no printed material")?;
        placed.push(centre(&reach));
        group = Some(match group {
            None => reach,
            Some([low, high]) => [
                std::array::from_fn(|axis| low[axis].min(reach[0][axis])),
                std::array::from_fn(|axis| high[axis].max(reach[1][axis])),
            ],
        });
    }
    let group = group.ok_or("the flow rate test has no tiles")?;
    let middle = centre(&group);
    let target = match bed {
        Some(bed) => [
            (bed.min[0] + bed.max[0]) / 2.0,
            (bed.min[1] + bed.max[1]) / 2.0,
        ],
        None => middle,
    };
    let mut model = Model::new();
    let copies = model.import(bed, &read, None)?;
    let mut moves = Vec::new();
    for (copy, at) in copies.iter().zip(&placed) {
        let Some((scene, Some(reach))) = model.in_scene(bed, *copy) else {
            return Err("a flow rate tile has no printed material".into());
        };
        let here = centre(&reach);
        let mut sized = scene.then(&Transform::translation([-here[0], -here[1], 0.0]));
        sized = sized.then(&Transform {
            rows: [
                [scale[0], 0.0, 0.0],
                [0.0, scale[1], 0.0],
                [0.0, 0.0, scale[2]],
                [0.0; 3],
            ],
        });
        sized = sized.then(&Transform::translation([
            target[0] + scale[0] * (at[0] - middle[0]),
            target[1] + scale[1] * (at[1] - middle[1]),
            0.0,
        ]));
        moves.push((*copy, sized));
    }
    // Each stood on the bed as it was.
    model.set_transforms(bed, &moves)?;

    // The speeds every tile is held to.
    let flow_ratio = filament_number(settings, "filament_flow_ratio", 0)?;
    let flow_ratio = if flow_ratio > 0.0 { flow_ratio } else { 1.0 };
    let (internal_solid, top) = flow_tile_speeds(
        linear,
        pass,
        nozzle,
        layer_height,
        flow_ratio,
        filament_number(settings, "filament_max_volumetric_speed", 0)?,
        process_number(settings, "internal_solid_infill_speed")?,
        process_number(settings, "top_surface_speed")?,
    );
    let own = flow_tile_settings(nozzle, internal_solid, top);
    let mut process: Set = Vec::new();
    for (key, value) in &own {
        if !crate::settable(key).followed_on_object {
            // The same on every tile: over the process.
            process.push(((*key).to_owned(), value.clone()));
        }
    }
    if !crate::settable("print_flow_ratio").followed_on_object {
        return Err("print_flow_ratio is not followed set on an object, so the tiles cannot each print their own flow".into());
    }
    let objects: Vec<(u64, String)> = model
        .objects()
        .iter()
        .map(|object| (object.id(), object.name().to_owned()))
        .collect();
    for (object, name) in objects {
        let mut on_object: Vec<(String, Option<String>)> = own
            .iter()
            .filter(|(key, _)| crate::settable(key).followed_on_object)
            .map(|(key, value)| ((*key).to_owned(), Some(value.clone())))
            .collect();
        on_object.push((
            "print_flow_ratio".to_owned(),
            Some(written(tile_flow_ratio(&name, linear, flow_ratio))),
        ));
        model.set_object_settings(bed, object, &on_object)?;
    }
    process.extend(pairs(&[("max_volumetric_extrusion_rate_slope", "0")]));
    // Half a single-precision nozzle: written as that float is.
    let half_nozzle = format!("{}", nozzle / 2.0);
    process.push(("layer_height".to_owned(), half_nozzle.clone()));
    process.extend(pairs(&[("alternate_extra_wall", "0")]));
    process.push((
        "initial_layer_print_height".to_owned(),
        if first_layer_height == layer_height {
            half_nozzle
        } else {
            written(first_layer_height)
        },
    ));
    // `spiral_mode` off: the tiles are read by their tops, which vase
    // mode does not print. (Printing them object by object, which the
    // reference then arranges for, is not followed here: the slice says
    // so, and prints them together, as the file places them.)
    process.extend(pairs(&[
        ("reduce_crossing_wall", "1"),
        ("spiral_mode", "0"),
    ]));
    Ok(CalibProject {
        name: name.to_owned(),
        model,
        process,
        filament: Vec::new(),
        printer: pairs(&[("resonance_avoidance", "0")]),
        params: None,
        notes: Vec::new(),
    })
}

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
