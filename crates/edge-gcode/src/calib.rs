//! What a calibration print changes from layer to layer, written at each
//! layer change (GCode.cpp `process_layer`, the "Calibration
//! Layer-specific GCode" switch), and the lines of the reference's G-code
//! writer it writes them with (GCodeWriter.cpp).
//!
//! The calibration's model and settings are made in `edge-engine`'s
//! `calib` module; this is only what the writer does with them.

/// The per-layer change a calibration print makes.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub kind: CalibrationKind,
    /// How the printer is told its pressure advance.
    pub firmware: AdvanceFirmware,
    /// `gcode_comments`: the reference's writer comments its own moves.
    pub comments: bool,
    /// `machine_max_jerk_x`, `_y`, `_z`, `_e`, which a jerk the test sets
    /// is held to.
    pub max_jerk: [f64; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub enum CalibrationKind {
    /// `Calib_PA_Tower`: the pressure advance in equal bands of `step`
    /// from `start` to `end`, spread over the layers
    /// ([`stepped_layer_value`]).
    PressureAdvanceTower { start: f64, end: f64, step: f64 },
    /// `Calib_Temp_Tower`: the nozzle 5 °C cooler every 10 mm up from
    /// `start`, set at every layer.
    TemperatureTower { start: f64 },
    /// `Calib_PA_Line`: lines drawn in place of the objects' layers
    /// (`calib_draw.rs`).
    PressureAdvanceLine(Box<crate::calib_draw::PaLine>),
    /// `Calib_PA_Pattern`: what each of the first layers carries in place
    /// of the plate's custom G-code, with the filament it feeds, every
    /// test's together (`calib_draw::pa_pattern`).
    PressureAdvancePattern(Vec<(String, f64)>),
    /// `Calib_Retraction_tower`: the retraction length `start`, `step`
    /// more for each whole mm above 0.4 mm ([`tower_retraction`]).
    RetractionTower { start: f64, step: f64 },
    /// `Calib_Vol_speed_Tower`: the outer wall's speed `start + z × step`
    /// mm/s at a layer whose top is at `z`, rounded (the volumetric
    /// speeds asked for, already divided by the line's mm³ per mm).
    VolumetricSpeedTower { start: f64, step: f64 },
    /// `Calib_VFA_Tower`: the outer wall's speed in equal bands of `step`
    /// from `start` to `end`, spread over the layers
    /// ([`stepped_layer_value`]), rounded.
    VfaTower { start: f64, end: f64, step: f64 },
    /// `Calib_Input_shaping_freq`: the shaper's type and damping on the
    /// second layer, the frequency from `x.0` to `x.1` (and `y.0` to
    /// `y.1`) over the layers ([`spread_layer_value`]).
    InputShapingFrequency {
        firmware: ShaperFirmware,
        damping: f64,
        x: (f64, f64),
        y: (f64, f64),
        shaper: String,
    },
    /// `Calib_Input_shaping_damp`: the shaper's type and each axis's
    /// frequency on the second layer, the damping from `start` to `end`
    /// over the layers.
    InputShapingDamping {
        firmware: ShaperFirmware,
        frequency: (f64, f64),
        start: f64,
        end: f64,
        shaper: String,
    },
}

/// How a firmware is told its input shaper (GCodeWriter.cpp
/// `set_input_shaping`), by `gcode_flavor` alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaperFirmware {
    /// `SET_INPUT_SHAPER`.
    Klipper,
    /// `M593 P"…" F… S…`.
    RepRapFirmware,
    /// Marlin 2: `M593 X|Y F… D…`.
    Marlin,
}

impl ShaperFirmware {
    /// The firmware of a `gcode_flavor`, or the reference's refusal
    /// (GCodeWriter.cpp `set_input_shaping`, which throws).
    pub fn of(flavor: Option<&str>) -> Result<Self, String> {
        match flavor {
            Some("klipper") => Ok(Self::Klipper),
            Some("reprapfirmware") => Ok(Self::RepRapFirmware),
            Some("marlin2") => Ok(Self::Marlin),
            Some("marlin") | None => Err("Input shaping is not supported by Marlin < 2.1.2.\nCheck your firmware version and update your G-code flavor to 'Marlin 2'".to_owned()),
            Some(_) => Err("Input shaping is only supported by Klipper, RepRapFirmware and Marlin 2".to_owned()),
        }
    }
}

/// The line that sets the input shaper (GCodeWriter.cpp
/// `set_input_shaping`): for `axis` X, Y or A (both), `damp` and `freq`
/// in single precision, each written only when over 0, and `shaper` its
/// type ("Default" and "" none; "Disable" turns shaping off, both axes at
/// 0). Nothing when there is nothing to say. The reference throws for a
/// frequency under 0, a damping outside 0 to 1, or another axis.
pub fn input_shaping_line(
    firmware: ShaperFirmware,
    axis: char,
    damp: f32,
    freq: f32,
    shaper: &str,
    comments: bool,
) -> Result<String, String> {
    use std::fmt::Write as _;
    let disable = shaper == "Disable";
    let (axis, damp, freq, shaper) = if disable {
        ('A', 0.0_f32, 0.0_f32, "Default")
    } else {
        if freq < 0.0 || damp < 0.0 || damp > 1.0 || !matches!(axis, 'X' | 'Y' | 'Z' | 'A') {
            return Err(format!(
                "Invalid input shaping parameters: axis={axis}, freq={freq:.6}, damp={damp:.6}"
            ));
        }
        (axis, damp, freq, shaper)
    };
    let mut params = String::new();
    let line = match firmware {
        ShaperFirmware::Klipper => {
            if !shaper.is_empty() && shaper != "Default" {
                let _ = write!(params, " SHAPER_TYPE={shaper}");
            }
            if axis != 'A' {
                if freq > 0.0 {
                    let _ = write!(params, " SHAPER_FREQ_{axis}={freq:.2}");
                }
                if damp > 0.0 {
                    let _ = write!(params, " DAMPING_RATIO_{axis}={damp:.3}");
                }
            } else {
                if freq > 0.0 || disable {
                    let _ = write!(params, " SHAPER_FREQ_X={freq:.2} SHAPER_FREQ_Y={freq:.2}");
                }
                if damp > 0.0 || disable {
                    let _ = write!(
                        params,
                        " DAMPING_RATIO_X={damp:.3} DAMPING_RATIO_Y={damp:.3}"
                    );
                }
            }
            "SET_INPUT_SHAPER"
        }
        ShaperFirmware::RepRapFirmware => {
            if !shaper.is_empty() && shaper != "Default" && shaper != "DAA" {
                let _ = write!(params, " P\"{shaper}\"");
            }
            if freq > 0.0 || disable {
                let _ = write!(params, " F{freq:.2}");
            }
            if damp > 0.0 || disable {
                let _ = write!(params, " S{damp:.3}");
            }
            "M593"
        }
        ShaperFirmware::Marlin => {
            if axis != 'A' {
                let _ = write!(params, " {axis}");
            }
            if freq > 0.0 || disable {
                let _ = write!(params, " F{freq:.2}");
            }
            if damp > 0.0 || disable {
                let _ = write!(params, " D{damp:.3}");
            }
            "M593"
        }
    };
    if params.is_empty() {
        return Ok(String::new());
    }
    let comment = if comments {
        " ; Override input shaping"
    } else {
        ""
    };
    Ok(format!("{line}{params}{comment}\n"))
}

/// A value spread evenly over the layers from the third on (GCode.cpp
/// `interpolate_value_across_layers` without a step): `start` on the
/// second layer (index 1), then `start` to `end` from index 2 to the last,
/// in single precision; index 0 is before the spread, at `start`.
pub fn spread_layer_value(index: usize, count: usize, start: f32, end: f32) -> f32 {
    if index == 1 {
        return start;
    }
    let ratio = ((index as f32 - 2.0) / (count as f32 - 3.0)).clamp(0.0, 1.0);
    start + ratio * (end - start)
}

/// How a firmware is told its pressure advance (GCodeWriter.cpp
/// `set_pressure_advance`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvanceFirmware {
    /// A Bambu Lab printer: `M900 K… L1000 M10`.
    Bambu,
    Klipper,
    /// `gcode_flavor` `reprapfirmware`: `M572 D0 S…`.
    RepRapFirmware,
    /// `M233 X… Y…`.
    Repetier,
    /// Mach3 and Machinekit: `M900 K…`, and a temperature as `M104 P…`.
    Mach3,
    /// Marlin and every other: `M900 K…`.
    Marlin,
}

impl AdvanceFirmware {
    /// By the printer's `gcode_flavor` and whether it is a Bambu Lab
    /// printer (`m_is_bbl_printers`), which wins.
    pub fn of(flavor: Option<&str>, bambu: bool) -> Self {
        if bambu {
            return Self::Bambu;
        }
        match flavor {
            Some("klipper") => Self::Klipper,
            Some("reprapfirmware") => Self::RepRapFirmware,
            Some("repetier") => Self::Repetier,
            Some("mach3" | "machinekit") => Self::Mach3,
            _ => Self::Marlin,
        }
    }
}

/// The line that sets the pressure advance (GCodeWriter.cpp
/// `set_pressure_advance`): nothing for a value below zero, the value to
/// four significant digits (`std::setprecision(4)`).
pub fn pressure_advance_line(advance: f64, firmware: AdvanceFirmware) -> String {
    if advance < 0.0 {
        return String::new();
    }
    let value = general(advance, 4);
    match firmware {
        AdvanceFirmware::Bambu => {
            format!("M900 K{value} L1000 M10 ; Override pressure advance value\n")
        }
        AdvanceFirmware::Klipper => {
            format!("SET_PRESSURE_ADVANCE ADVANCE={value}; Override pressure advance value\n")
        }
        AdvanceFirmware::RepRapFirmware => {
            format!("M572 D0 S{value}; Override pressure advance value\n")
        }
        AdvanceFirmware::Repetier => {
            format!("M233 X{value} Y{value} ; Override pressure advance value\n")
        }
        AdvanceFirmware::Marlin | AdvanceFirmware::Mach3 => {
            format!("M900 K{value}; Override pressure advance value\n")
        }
    }
}

/// The line that sets the nozzle's temperature without waiting
/// (GCodeWriter.cpp `set_temperature`, `wait` false): the tool named when
/// the print has several (`tool`).
pub fn temperature_line(
    temperature: u32,
    tool: Option<usize>,
    firmware: AdvanceFirmware,
) -> String {
    let (code, letter) = match firmware {
        // M104 is deprecated on RepRapFirmware.
        AdvanceFirmware::RepRapFirmware => ("G10", 'S'),
        AdvanceFirmware::Mach3 => ("M104", 'P'),
        _ => ("M104", 'S'),
    };
    let tool = match (tool, firmware) {
        (Some(tool), AdvanceFirmware::RepRapFirmware) => format!(" P{tool}"),
        (Some(tool), _) => format!(" T{tool}"),
        (None, _) => String::new(),
    };
    format!("{code} {letter}{temperature}{tool} ; set nozzle temperature\n")
}

/// The value of a stepped calibration at a layer (calib.cpp
/// `calib_stepped_layer_value`, Orca #14759): `start..=end` split into
/// equal bands of `step` spread evenly over the layers; layers 0 and 1
/// print `start`. Single precision, as the reference works it.
pub fn stepped_layer_value(
    layer_index: usize,
    layer_count: usize,
    start: f32,
    end: f32,
    step: f32,
) -> f32 {
    if layer_index <= 1 || layer_count < 2 {
        return start;
    }
    let ratio = (layer_index as f32 / (layer_count as f32 - 1.0)).min(1.0);
    if step > 0.0 {
        // `std::lround`: halves away from zero, as Rust's `round`.
        let bands = ((end - start).abs() / step).round() as i32 + 1;
        let band = (bands - 1).min((ratio * bands as f32) as i32);
        let sign = if end >= start { 1.0 } else { -1.0 };
        return start + sign * band as f32 * step;
    }
    start + ratio * (end - start)
}

/// What a calibration writes at the change to layer `index` (from 0) of
/// `count`, whose top is at `z` mm: GCode.cpp `process_layer`, written
/// after the layer change G-code and before the second layer's things.
/// `tool` is the tool printing, when the print names its tools.
pub(crate) fn at_layer(
    calibration: &Calibration,
    index: usize,
    count: usize,
    z: f64,
    tool: Option<usize>,
) -> String {
    // The input shaper's values are checked when the calibration is made
    // (`edge_engine::calib`), as the dialogs check them; a line refused
    // here would write nothing.
    let shaping = |firmware, axis, damp: f32, freq: f32, shaper: &str| {
        input_shaping_line(firmware, axis, damp, freq, shaper, calibration.comments)
            .unwrap_or_default()
    };
    match &calibration.kind {
        CalibrationKind::InputShapingFrequency {
            firmware,
            damping,
            x,
            y,
            shaper,
        } => {
            if index == 1 {
                shaping(*firmware, 'A', *damping as f32, 0.0, shaper)
            } else if x == y {
                let freq = spread_layer_value(index, count, x.0 as f32, x.1 as f32);
                shaping(*firmware, 'A', 0.0, freq, "")
            } else {
                let fx = spread_layer_value(index, count, x.0 as f32, x.1 as f32);
                let fy = spread_layer_value(index, count, y.0 as f32, y.1 as f32);
                shaping(*firmware, 'X', 0.0, fx, "") + &shaping(*firmware, 'Y', 0.0, fy, "")
            }
        }
        CalibrationKind::InputShapingDamping {
            firmware,
            frequency,
            start,
            end,
            shaper,
        } => {
            if index == 1 {
                shaping(*firmware, 'X', 0.0, frequency.0 as f32, shaper)
                    + &shaping(*firmware, 'Y', 0.0, frequency.1 as f32, shaper)
            } else {
                let damp = spread_layer_value(index, count, *start as f32, *end as f32);
                shaping(*firmware, 'A', damp, 0.0, "")
            }
        }
        _ => at_layer_plain(calibration, index, count, z, tool),
    }
}

fn at_layer_plain(
    calibration: &Calibration,
    index: usize,
    count: usize,
    z: f64,
    tool: Option<usize>,
) -> String {
    match calibration.kind {
        CalibrationKind::PressureAdvanceTower { start, end, step } => {
            let advance = stepped_layer_value(index, count, start as f32, end as f32, step as f32);
            pressure_advance_line(f64::from(advance), calibration.firmware)
        }
        CalibrationKind::TemperatureTower { start } => {
            temperature_line(tower_temperature(start, z), tool, calibration.firmware)
        }
        CalibrationKind::RetractionTower { start, step } => format!(
            "; Calib_Retraction_tower: Z_HEIGHT: {}, length:{}\n",
            general(z, 6),
            general(tower_retraction(start, step, z), 6)
        ),
        CalibrationKind::PressureAdvanceLine(_)
        | CalibrationKind::PressureAdvancePattern(_)
        | CalibrationKind::VolumetricSpeedTower { .. }
        | CalibrationKind::VfaTower { .. }
        | CalibrationKind::InputShapingFrequency { .. }
        | CalibrationKind::InputShapingDamping { .. } => String::new(),
    }
}

/// The outer wall's speed a calibration sets at the change to layer
/// `index` (from 0) of `count`, whose top is at `z` mm, in place of the
/// process's (GCode.cpp `process_layer`: `m_calib_config`'s
/// `outer_wall_speed`, which `_extrude` lays over each region's settings).
pub(crate) fn outer_wall_speed_at(
    calibration: &Calibration,
    index: usize,
    count: usize,
    z: f64,
) -> Option<f64> {
    match calibration.kind {
        CalibrationKind::VolumetricSpeedTower { start, step } => {
            Some(volumetric_tower_speed(start, step, z))
        }
        CalibrationKind::VfaTower { start, end, step } => {
            Some(vfa_tower_speed(index, count, start, end, step))
        }
        _ => None,
    }
}

/// The volumetric speed tower's outer wall speed at a layer whose top is
/// at `z` mm (GCode.cpp `process_layer`, `Calib_Vol_speed_Tower`): `start +
/// z × step`, `std::round`ed. `start` and `step` are mm/s, the volumetric
/// speeds asked for over the line's mm³ per mm (Plater.cpp
/// `calib_max_vol_speed`).
pub fn volumetric_tower_speed(start: f64, step: f64, z: f64) -> f64 {
    // `std::round`: halves away from zero, as Rust's `round`.
    (start + z * step).round()
}

/// The VFA tower's outer wall speed at layer `index` of `count` (GCode.cpp
/// `process_layer`, `Calib_VFA_Tower`): the stepped value, worked in
/// single precision, `std::round`ed as a float.
pub fn vfa_tower_speed(index: usize, count: usize, start: f64, end: f64, step: f64) -> f64 {
    f64::from(stepped_layer_value(index, count, start as f32, end as f32, step as f32).round())
}

/// The retraction length a calibration sets from the layer whose top is at
/// `z` on, in place of the filament's.
pub(crate) fn retraction_at(calibration: &Calibration, z: f64) -> Option<f64> {
    match calibration.kind {
        CalibrationKind::RetractionTower { start, step } => Some(tower_retraction(start, step, z)),
        _ => None,
    }
}

/// The retraction tower's length at a layer whose top is at `z` mm
/// (GCode.cpp `process_layer`, `Calib_Retraction_tower`): `start`, and
/// `step` more for each whole mm above 0.4 mm, the tower's base.
pub fn tower_retraction(start: f64, step: f64, z: f64) -> f64 {
    start + (z - 0.4).max(0.0).floor() * step
}

/// The temperature tower's temperature at a layer whose top is at `z`
/// mm (GCode.cpp `process_layer`, `Calib_Temp_Tower`): 5 °C less for each
/// whole 10.001 mm below it, the result taken as a whole number.
pub fn tower_temperature(start: f64, z: f64) -> u32 {
    let offset = f64::from((z / 10.001) as u32 * 5);
    (start - offset).max(0.0) as u32
}

/// Plastic laid per mm of a line (Flow.cpp `Flow::mm3_per_mm`): the
/// reference's flow keeps its width and height in single precision and
/// gives the result in single precision too.
pub fn line_mm3_per_mm(width: f64, height: f64) -> f64 {
    let (width, height) = (f64::from(width as f32), f64::from(height as f32));
    f64::from((height * (width - height * (1.0 - 0.25 * std::f64::consts::PI))) as f32)
}

/// The speed to print a pressure advance test at, mm/s (calib.cpp
/// `CalibPressureAdvance::find_optimal_PA_speed`): the outer wall speed,
/// at least 100 mm/s, held to what the filament's maximum volumetric speed
/// allows for a line of this width (0: 1.125 nozzles, Flow.cpp
/// `auto_extrusion_width`) and height, rounded down.
pub fn optimal_pa_speed(
    max_volumetric_speed: f64,
    outer_wall_speed: f64,
    nozzle_diameter: f64,
    line_width: f64,
    layer_height: f64,
) -> f64 {
    let line_width = if line_width <= 0.0 {
        f64::from(1.125_f32 * nozzle_diameter as f32)
    } else {
        line_width
    };
    let speed = 100.0_f64
        .max(outer_wall_speed)
        .min(max_volumetric_speed / line_mm3_per_mm(line_width, layer_height));
    // Returned as a float by the reference.
    f64::from(speed.floor() as f32)
}

/// A number as C++ streams print it with `std::setprecision(digits)`
/// and no format of their own (`%.<digits>g`).
pub fn general(value: f64, digits: usize) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return "0".into();
    }
    let digits = digits.max(1);
    let scientific = format!("{value:.*e}", digits - 1);
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let trim = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            text.to_owned()
        }
    };
    if exponent < -4 || exponent >= digits as i32 {
        let sign = if exponent < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", trim(mantissa), exponent.abs())
    } else {
        let decimals = (digits as i32 - 1 - exponent).max(0) as usize;
        trim(&format!("{value:.decimals$}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_the_reference_streams_them() {
        assert_eq!(general(0.0, 4), "0");
        assert_eq!(general(0.002, 4), "0.002");
        assert_eq!(general(0.0345, 4), "0.0345");
        assert_eq!(general(0.12345, 4), "0.1235");
        assert_eq!(general(1.0, 4), "1");
        assert_eq!(general(0.00001, 4), "1e-05");
        assert_eq!(general(12345.0, 4), "1.234e+04");
        // A float's value carried into a double, as the reference passes
        // the tower's value on.
        assert_eq!(general(f64::from(0.002_f32), 4), "0.002");
        assert_eq!(general(f64::from(0.098_f32), 4), "0.098");
    }

    #[test]
    fn each_firmware_is_told_its_pressure_advance_its_own_way() {
        let lines: Vec<String> = [
            AdvanceFirmware::Bambu,
            AdvanceFirmware::Klipper,
            AdvanceFirmware::RepRapFirmware,
            AdvanceFirmware::Repetier,
            AdvanceFirmware::Marlin,
        ]
        .into_iter()
        .map(|firmware| pressure_advance_line(0.04, firmware))
        .collect();
        assert_eq!(
            lines,
            [
                "M900 K0.04 L1000 M10 ; Override pressure advance value\n",
                "SET_PRESSURE_ADVANCE ADVANCE=0.04; Override pressure advance value\n",
                "M572 D0 S0.04; Override pressure advance value\n",
                "M233 X0.04 Y0.04 ; Override pressure advance value\n",
                "M900 K0.04; Override pressure advance value\n",
            ]
        );
        assert_eq!(pressure_advance_line(-1.0, AdvanceFirmware::Klipper), "");
        assert_eq!(
            AdvanceFirmware::of(Some("klipper"), true),
            AdvanceFirmware::Bambu
        );
        assert_eq!(
            AdvanceFirmware::of(Some("marlin2"), false),
            AdvanceFirmware::Marlin
        );
    }

    #[test]
    fn the_temperature_tower_cools_by_five_every_ten_mm() {
        // GCode.cpp: offset = (unsigned)(print_z / 10.001) * 5.
        assert_eq!(tower_temperature(230.0, 0.2), 230);
        assert_eq!(tower_temperature(230.0, 10.0), 230);
        assert_eq!(tower_temperature(230.0, 10.2), 225);
        assert_eq!(tower_temperature(230.0, 49.8), 210);
        assert_eq!(tower_temperature(230.0, 50.01), 205);
        assert_eq!(
            temperature_line(225, None, AdvanceFirmware::Klipper),
            "M104 S225 ; set nozzle temperature\n"
        );
        assert_eq!(
            temperature_line(225, Some(1), AdvanceFirmware::Marlin),
            "M104 S225 T1 ; set nozzle temperature\n"
        );
        assert_eq!(
            temperature_line(225, Some(1), AdvanceFirmware::RepRapFirmware),
            "G10 S225 P1 ; set nozzle temperature\n"
        );
        assert_eq!(
            temperature_line(225, None, AdvanceFirmware::Mach3),
            "M104 P225 ; set nozzle temperature\n"
        );
    }

    #[test]
    fn the_retraction_tower_lengthens_its_retraction_every_mm() {
        // GCode.cpp: start + floor(max(0, print_z - 0.4)) * step.
        assert_eq!(tower_retraction(0.0, 0.1, 0.2), 0.0);
        assert_eq!(tower_retraction(0.0, 0.1, 1.2), 0.0);
        // 1.4 - 0.4 is just under 1 in double precision, in the
        // reference's arithmetic as here.
        assert_eq!(tower_retraction(0.0, 0.1, 1.4), 0.0);
        assert_eq!(tower_retraction(0.0, 0.1, 1.6), 0.1);
        assert_eq!(tower_retraction(0.5, 0.25, 9.6), 0.5 + 9.0 * 0.25);
        let tower = Calibration {
            kind: CalibrationKind::RetractionTower {
                start: 0.0,
                step: 0.1,
            },
            firmware: AdvanceFirmware::Klipper,
            comments: false,
            max_jerk: [0.0; 4],
        };
        // `%g` of each.
        assert_eq!(
            at_layer(&tower, 12, 100, 2.6, None),
            "; Calib_Retraction_tower: Z_HEIGHT: 2.6, length:0.2\n"
        );
        assert_eq!(retraction_at(&tower, 2.6), Some(0.2));
    }

    #[test]
    fn the_tower_steps_its_value_in_equal_bands_over_the_layers() {
        // Worked from calib.cpp `calib_stepped_layer_value`: 0 to 0.1 by
        // 0.002 is lround(0.1 / 0.002) + 1 = 51 bands; a tower of 255
        // layers (51 mm at 0.2) puts band floor(ratio * 51) on a layer,
        // ratio = index / 254.
        let at = |index| stepped_layer_value(index, 255, 0.0, 0.1, 0.002);
        assert_eq!(at(0), 0.0);
        assert_eq!(at(1), 0.0);
        // 2 / 254 * 51 = 0.40: still band 0.
        assert_eq!(at(2), 0.0);
        // 5 / 254 * 51 = 1.004: band 1.
        assert_eq!(at(5), 0.002);
        assert_eq!(at(4), 0.0);
        // 127 / 254 * 51 = 25.5: band 25.
        assert_eq!(at(127), 25.0 * 0.002_f32);
        // The top layer: ratio 1, band 51 held to the last, 50.
        assert_eq!(at(254), 50.0 * 0.002_f32);
        assert_eq!(general(f64::from(at(254)), 4), "0.1");
        // Downwards, and without a step.
        assert_eq!(
            stepped_layer_value(254, 255, 0.1, 0.0, 0.002),
            0.1 - 50.0 * 0.002
        );
        assert_eq!(stepped_layer_value(127, 255, 0.0, 1.0, 0.0), 0.5);
        // Too few layers to spread over.
        assert_eq!(stepped_layer_value(5, 1, 0.3, 1.0, 0.1), 0.3);
    }
}
