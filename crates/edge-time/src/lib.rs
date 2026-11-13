//! How long a G-code file takes to print, with acceleration.
//!
//! The file is read as a printer would read it and its moves are planned
//! the way firmware plans them, in one of two ways:
//!
//! - [`Model::Reference`] is the reference slicer's own estimator
//!   (GCodeProcessor.cpp): Marlin's trapezoid planner, with junction
//!   speeds from the square corner velocity when the firmware is Klipper.
//!   Run on a file the reference wrote, it gives the time the reference
//!   printed in it, which is how the port is checked.
//! - [`Model::Klipper`] is Klipper's planner (klippy/toolhead.py,
//!   GPL-3.0): its junction speeds, and its look-ahead, which holds the
//!   speed a move may reach between an acceleration and a deceleration
//!   to what `minimum_cruise_ratio` allows. The reference's estimator
//!   leaves that out, so it has a Klipper printer going faster through
//!   short zigzags than it does.
//!
//! Neither counts what cannot be known from the file: heating, homing,
//! bed levelling, whatever a printer's own macros do.

mod klipper;
mod marlin;
mod reader;

use edge_config::{ConfigError, Settings};

/// Which planner the moves are timed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// As the reference slicer estimates.
    Reference,
    /// As Klipper plans.
    Klipper,
}

/// The firmware a file is written for, as far as timing cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Klipper,
    /// Marlin 2, which plans corners by junction deviation when one is set.
    Marlin2,
    /// Anything else: corners by per-axis jerk.
    Other,
}

/// Elegoo printers whose filament-change command the reference times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elegoo {
    CentauriCarbon,
    CentauriCarbon2,
}

/// What the machine can do, for x, y, z and e in that order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub flavor: Flavor,
    /// Top speed of each axis in mm/s; zero is no limit.
    pub max_speed: [f64; 4],
    /// Top acceleration of each axis in mm/s²; zero is no limit.
    pub max_acceleration: [f64; 4],
    /// Top acceleration while extruding, retracting and travelling; zero
    /// is no limit.
    pub max_extruding: f64,
    pub max_retracting: f64,
    pub max_travel: f64,
    /// Jerk of each axis in mm/s. For Klipper the first is the square
    /// corner velocity, as the reference keeps it.
    pub jerk: [f64; 4],
    /// Marlin 2's junction deviation in mm; zero plans by jerk.
    pub junction_deviation: f64,
    /// Slowest feed rates the firmware will run, in mm/s.
    pub min_extruding_rate: f64,
    pub min_travel_rate: f64,
    /// Seconds a tool change takes, over and above its moves.
    pub tool_change_time: f64,
    pub load_time: f64,
    pub unload_time: f64,
    /// Klipper's `minimum_cruise_ratio` until a file sets another: its
    /// own default, a half.
    pub minimum_cruise_ratio: f64,
    /// Set for a printer whose `M6211` (swap filament and purge) is timed.
    pub elegoo: Option<Elegoo>,
    /// A Bambu Lab printer, whose end G-code's waits come after the print
    /// and are not counted (GCodeProcessor.cpp `m_skip_end_gcode_delays`,
    /// set by GCode.cpp `_do_export` as the machine's end G-code starts).
    pub bambu: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            flavor: Flavor::Other,
            max_speed: [0.0; 4],
            max_acceleration: [0.0; 4],
            max_extruding: 0.0,
            max_retracting: 0.0,
            max_travel: 0.0,
            jerk: [0.0; 4],
            junction_deviation: 0.0,
            min_extruding_rate: 0.0,
            min_travel_rate: 0.0,
            tool_change_time: 0.0,
            load_time: 0.0,
            unload_time: 0.0,
            minimum_cruise_ratio: 0.5,
            elegoo: None,
            bambu: false,
        }
    }
}

impl Limits {
    /// Reads the machine's limits from a printer profile. Each is a list
    /// for the normal and the quiet mode; the normal mode's is taken.
    pub fn from_settings(settings: &Settings) -> Result<Self, ConfigError> {
        let first = |key: &str| -> Result<f64, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.first().copied().flatten())
                .unwrap_or(0.0))
        };
        let axes = |stem: &str| -> Result<[f64; 4], ConfigError> {
            Ok([
                first(&format!("{stem}_x"))?,
                first(&format!("{stem}_y"))?,
                first(&format!("{stem}_z"))?,
                first(&format!("{stem}_e"))?,
            ])
        };
        let flavor = match settings.text("gcode_flavor") {
            Some("klipper") => Flavor::Klipper,
            Some("marlin2") => Flavor::Marlin2,
            _ => Flavor::Other,
        };
        Ok(Self {
            flavor,
            max_speed: axes("machine_max_speed")?,
            max_acceleration: axes("machine_max_acceleration")?,
            max_extruding: first("machine_max_acceleration_extruding")?,
            max_retracting: first("machine_max_acceleration_retracting")?,
            // The reference has a travel acceleration apart from the
            // printing one only where the firmware can be told one
            // (GCodeWriter::supports_separate_travel_acceleration).
            max_travel: match flavor {
                Flavor::Marlin2 | Flavor::Klipper => first("machine_max_acceleration_travel")?,
                Flavor::Other => 0.0,
            },
            jerk: axes("machine_max_jerk")?,
            junction_deviation: first("machine_max_junction_deviation")?,
            min_extruding_rate: first("machine_min_extruding_rate")?,
            min_travel_rate: first("machine_min_travel_rate")?,
            tool_change_time: first("machine_tool_change_time")?,
            load_time: first("machine_load_filament_time")?,
            unload_time: first("machine_unload_filament_time")?,
            minimum_cruise_ratio: 0.5,
            elegoo: match settings
                .text("printer_model")
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("elegoo centauri carbon" | "elegoo centauri") => Some(Elegoo::CentauriCarbon),
                Some("elegoo centauri carbon 2" | "elegoo centauri 2") => {
                    Some(Elegoo::CentauriCarbon2)
                }
                _ => None,
            },
            bambu: settings
                .text("printer_model")
                .is_some_and(|model| model.starts_with("Bambu Lab")),
        })
    }

    /// Reads the limits from the settings a sliced file lists at its end
    /// (`; key = value`), which is all of them for a file the reference
    /// or this engine wrote. None for a file that lists no firmware.
    pub fn from_gcode(gcode: &str) -> Option<Self> {
        let mut json = serde_json::Map::new();
        for line in gcode.lines() {
            let Some((key, value)) = line
                .strip_prefix("; ")
                .and_then(|setting| setting.split_once(" = "))
            else {
                continue;
            };
            if key == "printer_model" {
                json.insert(key.to_owned(), value.into());
            } else if key == "gcode_flavor"
                || key.starts_with("machine_m")
                || key.starts_with("machine_tool")
                || key.starts_with("machine_load")
                || key.starts_with("machine_unload")
            {
                let values: Vec<serde_json::Value> =
                    value.split(',').map(|part| part.trim().into()).collect();
                let value = match values.as_slice() {
                    [one] => one.clone(),
                    _ => values.into(),
                };
                json.insert(key.to_owned(), value);
            }
        }
        if !json.contains_key("gcode_flavor") {
            return None;
        }
        let text = serde_json::Value::Object(json).to_string();
        Self::from_settings(&Settings::from_json(&text).ok()?).ok()
    }

    /// The planner that times a file as its printer runs it: Klipper's
    /// for Klipper, the reference's for the rest.
    pub fn own_model(&self) -> Model {
        match self.flavor {
            Flavor::Klipper => Model::Klipper,
            Flavor::Marlin2 | Flavor::Other => Model::Reference,
        }
    }
}

/// A file's time.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// Seconds from the first line to the last.
    pub seconds: f64,
    /// Seconds spent on each line of the file, in order.
    pub by_line: Vec<f64>,
}

impl Estimate {
    /// Seconds gone by the end of each line.
    pub fn elapsed(&self) -> Vec<f64> {
        let mut total = 0.0;
        self.by_line
            .iter()
            .map(|seconds| {
                total += seconds;
                total
            })
            .collect()
    }
}

/// Something that plans moves and says how long each line took.
pub(crate) trait Planner {
    /// A move by `delta` (x, y, z, e in mm) at `feed` mm/s, read from `line`.
    fn go(&mut self, line: usize, delta: [f64; 4], feed: f64);
    /// The machine comes to rest, then waits `extra` seconds.
    fn rest(&mut self, line: usize, extra: f64);
    /// A setting the planner keeps: see [`reader::Set`].
    fn set(&mut self, what: reader::Set);
    /// Seconds spent on each of `lines` lines.
    fn finish(self, lines: usize) -> Vec<f64>;
}

/// Times a G-code file.
pub fn estimate(gcode: &str, limits: &Limits, model: Model) -> Estimate {
    let by_line = match model {
        Model::Reference => reader::read(gcode, limits, model, marlin::Marlin::new(limits)),
        Model::Klipper => reader::read(gcode, limits, model, klipper::Klipper::new(limits)),
    };
    Estimate {
        seconds: by_line.iter().sum(),
        by_line,
    }
}

#[cfg(test)]
mod tests;
