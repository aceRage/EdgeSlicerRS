//! Reads G-code as a printer does, far enough to know every move and
//! everything that changes how moves are planned.

use crate::{Elegoo, Limits, Model, Planner};

/// A setting a planner keeps, as a file changes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Set {
    /// Acceleration while printing, in mm/s².
    Acceleration(f64),
    TravelAcceleration(f64),
    RetractAcceleration(f64),
    /// Klipper's square corner velocity, in mm/s.
    SquareCornerVelocity(f64),
    /// Klipper's minimum cruise ratio, from 0 up to but not 1.
    CruiseRatio(f64),
    /// Top speed of any move, in mm/s.
    MaxVelocity(f64),
    /// Top speed, top acceleration and jerk of one axis (x, y, z, e).
    AxisMaxSpeed(usize, f64),
    AxisMaxAcceleration(usize, f64),
    AxisJerk(usize, f64),
    JunctionDeviation(f64),
    MinExtrudingRate(f64),
    MinTravelRate(f64),
    /// What E feed rates are multiplied by (M221).
    ExtrudeFactor(f64),
}

/// Extended commands that do not make Klipper finish the moves it has
/// queued. Any other is taken to be a macro that does.
const QUIET: [&str; 6] = [
    "SET_VELOCITY_LIMIT",
    "SET_PRESSURE_ADVANCE",
    "EXCLUDE_OBJECT_START",
    "EXCLUDE_OBJECT_END",
    "EXCLUDE_OBJECT_DEFINE",
    "EXCLUDE_OBJECT",
];

/// How finely an arc is cut into straight moves, in mm.
const ARC_STEP: f64 = 0.5;

/// Seconds an Elegoo printer takes over `M6211`, which swaps to a filament
/// and purges `length` mm of it, from where the nozzle is. The timings are
/// the firmware's, as the reference has them
/// (ElegooGCodeProcessorHelper.cpp, from OrcaSlicer #14663).
fn elegoo_purge(model: Elegoo, words: &[&str], length: f64, at: [f64; 4]) -> f64 {
    // Seconds to push `length` mm at `feed` mm/min.
    let push = |length: f64, feed: f64| {
        if feed > 0.0 && length > 0.0 {
            length / feed * 60.0
        } else {
            0.0
        }
    };
    let given = |letter: char, default: f64, least: f64, most: f64| {
        word(words, letter).unwrap_or(default).clamp(least, most)
    };
    let flush = length.clamp(10.0, 1000.0);
    match model {
        Elegoo::CentauriCarbon => {
            let cool = given('P', 5000.0, 0.0, 20000.0) / 1e3;
            // To where it purges, at x 256, y 0.
            let travel = (at[1].abs() + (at[0] - 256.0).abs()) / 5000.0 * 60.0;
            // Swapping and heating, then the purge in stretches of 73 mm.
            let mut seconds = 18.2 + travel;
            let mut left = flush;
            while left > 0.0 {
                let stretch = left.min(73.0);
                left -= stretch;
                let pushed = if stretch >= 73.0 {
                    push(30.0, 500.0) + push(35.0, 500.0) + push(10.0, 400.0)
                } else {
                    push(stretch, 500.0)
                };
                seconds += pushed + push(2.0, 400.0) + cool + 2.8;
            }
            seconds
        }
        Elegoo::CentauriCarbon2 => {
            let single = given('K', 75.0, 10.0, 300.0);
            let old_feed = given('M', 300.0, 10.0, 600.0);
            let new_feed = given('N', 300.0, 10.0, 600.0);
            let cool = given('P', 3000.0, 0.0, 20000.0) / 1e3;
            // A purge whose last 10 mm go slower.
            let purge = |length: f64, feed: f64| {
                let tail = length.clamp(0.0, 10.0);
                push((length - tail).max(0.0), feed) + push(tail, 400.0)
            };
            let after_start = (flush - 15.0).max(0.0);
            let times = ((after_start / single).ceil() as usize).max(1);
            let mut seconds = 31.0 + push(flush.min(15.0), old_feed);
            seconds +=
                (times - 1) as f64 * (purge(single, new_feed) + push(6.0, 1800.0) + cool + 5.0);
            seconds += purge(single, new_feed * 0.8) + push(4.0, 1800.0) + cool + 5.0;
            let to = word(words, 'S').unwrap_or(0.0);
            let from = given('R', to + 20.0, 185.0, 350.0);
            let to = given('S', 250.0, 185.0, 350.0);
            if to < from {
                seconds += (from - to) / 1.36;
            }
            seconds
        }
    }
}

/// A letter's value among a command's words: `X12.5` for `'X'`.
fn word(words: &[&str], letter: char) -> Option<f64> {
    words.iter().find_map(|word| {
        let mut letters = word.chars();
        let first = letters.next()?;
        (first.eq_ignore_ascii_case(&letter))
            .then(|| letters.as_str().parse::<f64>().ok())
            .flatten()
    })
}

/// A named value of an extended command: `ACCEL=5000` for `"ACCEL"`.
fn named(words: &[&str], name: &str) -> Option<f64> {
    words.iter().find_map(|word| {
        let (key, value) = word.split_once('=')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.parse::<f64>().ok())
            .flatten()
    })
}

pub(crate) fn read<P: Planner>(
    gcode: &str,
    limits: &Limits,
    model: Model,
    mut planner: P,
) -> Vec<f64> {
    let klipper = model == Model::Klipper;
    let mut position = [0.0f64; 4];
    let (mut absolute, mut absolute_e) = (true, true);
    // mm/s as last commanded, and what Klipper multiplies it by (M220).
    let mut feed = 0.0f64;
    let mut speed_factor = 1.0f64;
    let mut tool = 0usize;
    // Whether an Elegoo printer has loaded its first filament yet.
    let mut loaded = false;
    let mut lines = 0;
    for (line, raw) in gcode.lines().enumerate() {
        lines = line + 1;
        let code = raw.split(';').next().unwrap_or_default().trim();
        if code.is_empty() {
            continue;
        }
        let mut parts = code.split_whitespace();
        let Some(command) = parts.next() else {
            continue;
        };
        let words: Vec<&str> = parts.collect();
        let target = |position: &[f64; 4]| -> [f64; 4] {
            let mut target = *position;
            for (axis, letter) in ['X', 'Y', 'Z', 'E'].into_iter().enumerate() {
                if let Some(value) = word(&words, letter) {
                    let is_absolute = if axis == 3 { absolute_e } else { absolute };
                    target[axis] = if is_absolute {
                        value
                    } else {
                        position[axis] + value
                    };
                }
            }
            target
        };
        match command.to_ascii_uppercase().as_str() {
            "G0" | "G1" | "G00" | "G01" => {
                if let Some(rate) = word(&words, 'F') {
                    feed = rate / 60.0;
                }
                let to = target(&position);
                let delta: [f64; 4] = std::array::from_fn(|axis| to[axis] - position[axis]);
                position = to;
                let speed = if klipper { feed * speed_factor } else { feed };
                if delta.iter().any(|d| *d != 0.0) && speed > 0.0 {
                    planner.go(line, delta, speed);
                }
            }
            turn @ ("G2" | "G3" | "G02" | "G03") => {
                if let Some(rate) = word(&words, 'F') {
                    feed = rate / 60.0;
                }
                let to = target(&position);
                let speed = if klipper { feed * speed_factor } else { feed };
                let centre = [
                    position[0] + word(&words, 'I').unwrap_or(0.0),
                    position[1] + word(&words, 'J').unwrap_or(0.0),
                ];
                let radius = (position[0] - centre[0]).hypot(position[1] - centre[1]);
                let from = (position[1] - centre[1]).atan2(position[0] - centre[0]);
                let until = (to[1] - centre[1]).atan2(to[0] - centre[0]);
                let clockwise = turn.ends_with('2');
                let mut sweep = if clockwise {
                    from - until
                } else {
                    until - from
                };
                if sweep <= 1e-9 {
                    sweep += std::f64::consts::TAU;
                }
                let steps = ((radius * sweep / ARC_STEP).ceil() as usize).max(1);
                if speed > 0.0 && radius > 0.0 {
                    let mut at = position;
                    for step in 1..=steps {
                        let share = step as f64 / steps as f64;
                        let angle = if clockwise {
                            from - sweep * share
                        } else {
                            from + sweep * share
                        };
                        let next = if step == steps {
                            to
                        } else {
                            [
                                centre[0] + radius * angle.cos(),
                                centre[1] + radius * angle.sin(),
                                position[2] + (to[2] - position[2]) * share,
                                position[3] + (to[3] - position[3]) * share,
                            ]
                        };
                        planner.go(line, std::array::from_fn(|a| next[a] - at[a]), speed);
                        at = next;
                    }
                }
                position = to;
            }
            "G4" | "G04" => {
                // Seconds win over milliseconds, as in Marlin.
                let seconds = match word(&words, 'S') {
                    Some(seconds) => seconds.max(0.0),
                    None => word(&words, 'P').unwrap_or(0.0).max(0.0) / 1e3,
                };
                // The reference leaves the queue alone for a wait of
                // nothing; Klipper finishes its moves before any dwell.
                if seconds > 0.0 || klipper {
                    planner.rest(line, seconds);
                }
            }
            "G90" => absolute = true,
            "G91" => absolute = false,
            "M82" => absolute_e = true,
            "M83" => absolute_e = false,
            "G92" => {
                let mut any = false;
                for (axis, letter) in ['X', 'Y', 'Z', 'E'].into_iter().enumerate() {
                    if let Some(value) = word(&words, letter) {
                        position[axis] = value;
                        any |= axis == 3;
                    }
                }
                // GCodeProcessor.cpp `process_G92`: without an E the
                // reference has the machine come to rest.
                if !any && !klipper {
                    planner.rest(line, 0.0);
                }
            }
            "M201" => {
                for (axis, letter) in ['X', 'Y', 'Z', 'E'].into_iter().enumerate() {
                    if let Some(value) = word(&words, letter) {
                        planner.set(Set::AxisMaxAcceleration(axis, value));
                    }
                }
            }
            "M203" => {
                for (axis, letter) in ['X', 'Y', 'Z', 'E'].into_iter().enumerate() {
                    if let Some(value) = word(&words, letter) {
                        planner.set(Set::AxisMaxSpeed(axis, value));
                    }
                }
            }
            "M204" => {
                if let Some(value) = word(&words, 'S') {
                    // The old form: one acceleration for printing and travel.
                    planner.set(Set::Acceleration(value));
                    planner.set(Set::TravelAcceleration(value));
                    if let Some(retract) = word(&words, 'T') {
                        planner.set(Set::RetractAcceleration(retract));
                    }
                } else {
                    if let Some(value) = word(&words, 'P') {
                        planner.set(Set::Acceleration(value));
                    }
                    if let Some(value) = word(&words, 'R') {
                        planner.set(Set::RetractAcceleration(value));
                    }
                    if let Some(value) = word(&words, 'T') {
                        planner.set(Set::TravelAcceleration(value));
                    }
                }
            }
            "M205" => {
                if let Some(value) = word(&words, 'X') {
                    planner.set(Set::AxisJerk(0, value));
                    planner.set(Set::AxisJerk(1, value));
                }
                for (axis, letter) in [(1, 'Y'), (2, 'Z'), (3, 'E')] {
                    if let Some(value) = word(&words, letter) {
                        planner.set(Set::AxisJerk(axis, value));
                    }
                }
                if let Some(value) = word(&words, 'S') {
                    planner.set(Set::MinExtrudingRate(value));
                }
                if let Some(value) = word(&words, 'T') {
                    planner.set(Set::MinTravelRate(value));
                }
                if let Some(value) = word(&words, 'J') {
                    planner.set(Set::JunctionDeviation(value));
                }
            }
            "M220" => {
                if let Some(percent) = word(&words, 'S').filter(|percent| *percent > 0.0) {
                    speed_factor = percent / 100.0;
                }
            }
            "M221" => {
                if let Some(percent) = word(&words, 'S') {
                    planner.set(Set::ExtrudeFactor(percent / 100.0));
                }
            }
            "M400" => {
                let seconds =
                    word(&words, 'S').unwrap_or(0.0) + word(&words, 'P').unwrap_or(0.0) / 1e3;
                if seconds > 0.0 || klipper {
                    planner.rest(line, seconds.max(0.0));
                }
            }
            "M1" | "M01" if !klipper => planner.rest(line, 0.0),
            // Klipper finishes its moves before it waits on a temperature
            // or homes. How long the wait is, no file says.
            "M109" | "M190" | "G28" | "M1" | "M01" if klipper => planner.rest(line, 0.0),
            "M6211" if limits.elegoo.is_some() => {
                let length = word(&words, 'L').filter(|length| *length > 0.0);
                let filament = word(&words, 'T').filter(|filament| *filament >= 0.0);
                if let (Some(model), Some(length), Some(filament)) =
                    (limits.elegoo, length, filament)
                {
                    let seconds = elegoo_purge(model, &words, length, position);
                    let filament = filament.round() as usize;
                    if filament != tool {
                        // The swap takes this in place of the machine's
                        // load and unload times; the T line after it
                        // then picks the filament already in hand.
                        tool = filament;
                        planner.rest(line, seconds + limits.tool_change_time);
                    } else if !loaded {
                        planner.rest(line, seconds);
                    }
                    loaded = true;
                }
            }
            "SET_VELOCITY_LIMIT" => {
                let acceleration = named(&words, "ACCEL");
                if let Some(value) = acceleration {
                    planner.set(Set::Acceleration(value));
                    planner.set(Set::TravelAcceleration(value));
                }
                if let Some(value) = named(&words, "VELOCITY") {
                    planner.set(Set::MaxVelocity(value));
                }
                if let Some(value) = named(&words, "SQUARE_CORNER_VELOCITY") {
                    planner.set(Set::SquareCornerVelocity(value));
                }
                // Klipper before 0.13 and vendor forks are given the
                // limit as ACCEL_TO_DECEL; it is the same thing said as a
                // share of the acceleration set with it.
                let ratio = named(&words, "MINIMUM_CRUISE_RATIO").or_else(|| {
                    let limit = named(&words, "ACCEL_TO_DECEL")?;
                    let acceleration = acceleration.filter(|value| *value > 0.0)?;
                    Some(1.0 - limit / acceleration)
                });
                if let Some(ratio) = ratio {
                    planner.set(Set::CruiseRatio(ratio.clamp(0.0, 0.999)));
                }
            }
            other => {
                // T<number>: another tool, which is not taken up on the fly.
                let picked = other
                    .strip_prefix('T')
                    .and_then(|number| number.parse::<usize>().ok());
                if let Some(picked) = picked {
                    if picked != tool {
                        tool = picked;
                        planner.rest(
                            line,
                            limits.unload_time + limits.load_time + limits.tool_change_time,
                        );
                    } else if klipper {
                        planner.rest(line, 0.0);
                    }
                } else if klipper {
                    let plain = other.len() > 1
                        && matches!(other.as_bytes()[0], b'G' | b'M')
                        && other[1..].bytes().all(|b| b.is_ascii_digit() || b == b'.');
                    if !plain && !QUIET.contains(&other) {
                        planner.rest(line, 0.0);
                    }
                }
            }
        }
    }
    planner.finish(lines)
}
