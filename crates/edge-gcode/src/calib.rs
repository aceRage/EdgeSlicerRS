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
    match calibration.kind {
        CalibrationKind::PressureAdvanceTower { start, end, step } => {
            let advance = stepped_layer_value(index, count, start as f32, end as f32, step as f32);
            pressure_advance_line(f64::from(advance), calibration.firmware)
        }
        CalibrationKind::TemperatureTower { start } => {
            temperature_line(tower_temperature(start, z), tool, calibration.firmware)
        }
    }
}

/// The temperature tower's temperature at a layer whose top is at `z`
/// mm (GCode.cpp `process_layer`, `Calib_Temp_Tower`): 5 °C less for each
/// whole 10.001 mm below it, the result taken as a whole number.
pub fn tower_temperature(start: f64, z: f64) -> u32 {
    let offset = f64::from((z / 10.001) as u32 * 5);
    (start - offset).max(0.0) as u32
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
