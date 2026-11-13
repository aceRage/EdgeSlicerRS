//! Reads G-code as a printer does, far enough to know every move and
//! everything that changes how moves are planned.

use crate::{Elegoo, Flavor, Limits, Model, Planner};

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

/// How long the straight moves are that Klipper cuts an arc into, in mm:
/// the `resolution` of its `[gcode_arcs]` section, at its default. A
/// printer's own configuration may say otherwise, which no file tells.
const KLIPPER_ARC_SEGMENT: f64 = 1.0;

/// The straight moves the reference's estimator cuts an arc into
/// (GCodeProcessor.cpp `process_G2_G3`): where each ends, the last of
/// them at `to`. `from` and `to` are x, y, z and e; `feed` is the feed
/// rate in force, in mm/s. None for a line it does not run: one with no
/// centre (neither `R` nor `I` or `J`), a radius of nothing, or an arc
/// under a micron long.
///
/// It cuts as the firmware it takes the file to be for would: Marlin 2
/// by length, so many moves a second at least (Marlin's `plan_arc`), the
/// rest by how far a chord may stray from the arc, 0.0125 mm (Prusa's
/// `mc_arc`). Like both it turns the radius a step at a time, by a short
/// series for the sine and cosine, and sets it right every 25 steps.
/// Single precision is kept where the reference has it, since it decides
/// how many moves there are.
fn reference_arc(
    words: &[&str],
    marlin: bool,
    from: [f64; 4],
    to: [f64; 4],
    clockwise: bool,
    feed: f64,
) -> Option<Vec<[f64; 4]>> {
    use std::f64::consts::PI;
    let offset: [f32; 2] = if let Some(radius) = word(words, 'R') {
        let radius = radius as f32;
        let (start, end) = (
            [from[0] as f32, from[1] as f32],
            [to[0] as f32, to[1] as f32],
        );
        let v = [end[0] - start[0], end[1] - start[1]];
        let q2 = v[0] * v[0] + v[1] * v[1];
        if radius == 0.0 || q2 == 0.0 {
            return None;
        }
        // ArcWelder.hpp `arc_center`: the centre is to one side of the
        // line between the ends, which side by the turn and the sign of
        // the radius; ends too far apart for the radius get the point
        // midway between them.
        let t2 = radius * radius / q2 - 0.25;
        let t = if t2 > 0.0 { t2.sqrt() } else { 0.0 };
        let mid = [0.5 * (start[0] + end[0]), 0.5 * (start[1] + end[1])];
        let side = [-v[1] * t, v[0] * t];
        let centre = if (radius > 0.0) != clockwise {
            [mid[0] + side[0], mid[1] + side[1]]
        } else {
            [mid[0] - side[0], mid[1] - side[1]]
        };
        [
            (centre[0] as f64 - from[0]) as f32,
            (centre[1] as f64 - from[1]) as f32,
        ]
    } else {
        let (i, j) = (word(words, 'I'), word(words, 'J'));
        if i.is_none() && j.is_none() {
            return None;
        }
        [i.unwrap_or(0.0) as f32, j.unwrap_or(0.0) as f32]
    };
    let centre = [from[0] + offset[0] as f64, from[1] + offset[1] as f64];
    let rel_start = [from[0] - centre[0], from[1] - centre[1]];
    let rel_end = [to[0] - centre[0], to[1] - centre[1]];
    // A move that ends where it starts is a whole turn.
    let full = (to[0] - from[0]).abs() < 1e-4 && (to[1] - from[1]).abs() < 1e-4;
    let angle = if full {
        2.0 * PI
    } else {
        let mut angle = (rel_start[0] * rel_end[1] - rel_start[1] * rel_end[0])
            .atan2(rel_start[0] * rel_end[0] + rel_start[1] * rel_end[1]);
        if angle < 0.0 {
            angle += 2.0 * PI;
        }
        if clockwise {
            angle -= 2.0 * PI;
        }
        angle
    };
    let radius = (rel_start[0] * rel_start[0] + rel_start[1] * rel_start[1]).sqrt();
    let rise = to[2] - from[2];
    let length = ((angle * radius).powi(2) + rise * rise).sqrt();
    if length < 0.001 {
        return None;
    }
    let fed = to[3] - from[3];
    let mut ends = Vec::new();
    if marlin {
        let radius_mm = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
        // A chord no further than 0.02 mm from the arc, at least 50
        // moves a second, and between 0.1 and 2 mm long.
        let segment_mm = (8.0 * radius_mm * 0.02)
            .sqrt()
            .min(feed as f32 * (1.0 / 50.0))
            .clamp(0.1, 2.0);
        let flat_mm = (radius_mm as f64 * angle.abs()) as f32;
        let segments = ((flat_mm / segment_mm + 0.8) as usize).max(1);
        if segments > 1 {
            let inv_segments = 1.0 / segments as f32;
            let theta = angle as f32 * inv_segments;
            let (cos_t, sin_t) = (theta.cos(), theta.sin());
            let z_step = (rise * inv_segments as f64) as f32;
            let e_step = fed as f32 * inv_segments;
            let mut recalc = 25usize;
            let mut arm = [-offset[0], -offset[1]];
            let (mut z, mut e) = (from[2], from[3]);
            for i in 1..segments {
                recalc -= 1;
                if recalc != 0 {
                    let y = arm[0] * sin_t + arm[1] * cos_t;
                    arm[0] = arm[0] * cos_t - arm[1] * sin_t;
                    arm[1] = y;
                } else {
                    recalc = 25;
                    let turned = i as f32 * theta;
                    let (cos_ti, sin_ti) = (turned.cos(), turned.sin());
                    arm[0] = -offset[0] * cos_ti + offset[1] * sin_ti;
                    arm[1] = -offset[0] * sin_ti - offset[1] * cos_ti;
                }
                z += z_step as f64;
                e += e_step as f64;
                ends.push([centre[0] + arm[0] as f64, centre[1] + arm[1] as f64, z, e]);
            }
        }
    } else {
        let segments = arc_discretization_steps(radius, angle.abs(), 0.0125).max(1);
        let inv_segment = 1.0 / segments as f64;
        let theta = angle * inv_segment;
        let (z_step, e_step) = (rise * inv_segment, fed * inv_segment);
        let sq_theta = theta * theta;
        let cos_t = 1.0 - 0.5 * sq_theta;
        let sin_t = theta - sq_theta * theta / 6.0;
        let mut arm = rel_start;
        let mut count = 25usize;
        let (mut z, mut e) = (from[2], from[3]);
        for i in 1..segments {
            if count == 0 {
                let turned = i as f64 * theta;
                let (cos_ti, sin_ti) = (turned.cos(), turned.sin());
                arm[0] = -(offset[0] as f64) * cos_ti + offset[1] as f64 * sin_ti;
                arm[1] = -(offset[0] as f64) * sin_ti - offset[1] as f64 * cos_ti;
                count = 25;
            } else {
                count -= 1;
                // Kept in single precision, as the reference keeps it.
                let y = (arm[0] * sin_t + arm[1] * cos_t) as f32;
                arm[0] = arm[0] * cos_t - arm[1] * sin_t;
                arm[1] = y as f64;
            }
            z += z_step;
            e += e_step;
            ends.push([centre[0] + arm[0], centre[1] + arm[1], z, e]);
        }
    }
    // The last move arrives where the line says.
    ends.push(to);
    Some(ends)
}

/// ArcWelder.hpp `arc_discretization_steps`: how many chords it takes
/// to stay within `deviation` of an arc of `radius` through `angle`.
fn arc_discretization_steps(radius: f64, angle: f64, deviation: f64) -> usize {
    use std::f64::consts::PI;
    let d = radius - deviation;
    if d < 1e-4 {
        // A radius no larger than the deviation: one chord, or two for
        // an arc so far round that its middle is out of reach of one.
        if angle < PI || radius * (1.0 + (PI - 0.5 * angle).cos()) < deviation {
            1
        } else {
            2
        }
    } else {
        (angle / (2.0 * (d / radius).acos())).ceil() as usize
    }
}

/// The straight moves Klipper cuts an arc into (klippy/extras/gcode_arcs.py
/// `planArc`): each [`KLIPPER_ARC_SEGMENT`] long, as near as a whole
/// number of them comes to the arc's length, the last ending at `to`.
/// None for a line Klipper refuses: one given as a radius, one with
/// neither `I` nor `J`, or any arc while moves are relative.
fn klipper_arc(
    words: &[&str],
    absolute: bool,
    from: [f64; 4],
    to: [f64; 4],
    clockwise: bool,
) -> Option<Vec<[f64; 4]>> {
    use std::f64::consts::TAU;
    if !absolute || word(words, 'R').is_some() {
        return None;
    }
    let offset = [
        word(words, 'I').unwrap_or(0.0),
        word(words, 'J').unwrap_or(0.0),
    ];
    if offset == [0.0, 0.0] {
        return None;
    }
    let arm = [-offset[0], -offset[1]];
    let centre = [from[0] - arm[0], from[1] - arm[1]];
    let reach = [to[0] - centre[0], to[1] - centre[1]];
    let mut turn =
        (arm[0] * reach[1] - arm[1] * reach[0]).atan2(arm[0] * reach[0] + arm[1] * reach[1]);
    if turn < 0.0 {
        turn += TAU;
    }
    if clockwise {
        turn -= TAU;
    }
    // A whole turn when it ends where it starts.
    if turn == 0.0 && from[0] == to[0] && from[1] == to[1] {
        turn = TAU;
    }
    let rise = to[2] - from[2];
    let flat = arm[0].hypot(arm[1]) * turn;
    let length = if rise != 0.0 {
        flat.hypot(rise)
    } else {
        flat.abs()
    };
    let segments = (length / KLIPPER_ARC_SEGMENT).floor().max(1.0) as usize;
    let mut ends = Vec::with_capacity(segments);
    for i in 1..segments {
        let share = i as f64 / segments as f64;
        let (sin, cos) = (share * turn).sin_cos();
        ends.push([
            centre[0] - offset[0] * cos + offset[1] * sin,
            centre[1] - offset[0] * sin - offset[1] * cos,
            from[2] + rise * share,
            from[3] + (to[3] - from[3]) * share,
        ]);
    }
    ends.push(to);
    Some(ends)
}

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
    // Whether a Bambu Lab printer's bed levelling has been counted.
    let mut levelled = false;
    let mut lines = 0;
    // On a Bambu Lab printer the waits of the end G-code, from the last
    // custom tag on, are after the print (GCode.cpp `_do_export`,
    // `start_machine_end_gcode`).
    let end_from = if limits.bambu {
        gcode
            .lines()
            .enumerate()
            .filter(|(_, raw)| raw.trim_end() == "; FEATURE: Custom")
            .map(|(line, _)| line)
            .last()
            .unwrap_or(usize::MAX)
    } else {
        usize::MAX
    };
    for (line, raw) in gcode.lines().enumerate() {
        lines = line + 1;
        let code = raw.split(';').next().unwrap_or_default().trim();
        if code.is_empty() {
            // A pause set at a layer: the printer finishes its moves and
            // stops, for as long as no file can say (GCodeProcessor.cpp,
            // the `PAUSE_PRINT` tag, `PAUSE_PRINTING` in a Bambu Lab
            // printer's file: `simulate_st_synchronize`).
            if matches!(raw.trim_end(), ";PAUSE_PRINT" | "; PAUSE_PRINTING") {
                planner.rest(line, 0.0);
            }
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
            // An arc is planned as the straight moves it is run as: the
            // reference's estimator and Klipper each cut it their own way.
            // A line neither would run moves nothing, and what it says of
            // the feed rate goes unheard.
            turn @ ("G2" | "G3" | "G02" | "G03") => {
                let clockwise = turn.ends_with('2');
                let given = word(&words, 'F').map(|rate| rate / 60.0);
                let to = target(&position);
                let ends = if klipper {
                    klipper_arc(&words, absolute, position, to, clockwise)
                } else {
                    reference_arc(
                        &words,
                        limits.flavor == Flavor::Marlin2,
                        position,
                        to,
                        clockwise,
                        given.unwrap_or(feed),
                    )
                };
                if let Some(ends) = ends {
                    feed = given.unwrap_or(feed);
                    let speed = if klipper { feed * speed_factor } else { feed };
                    for end in ends {
                        let delta: [f64; 4] =
                            std::array::from_fn(|axis| end[axis] - position[axis]);
                        position = end;
                        if delta.iter().any(|d| *d != 0.0) && speed > 0.0 {
                            planner.go(line, delta, speed);
                        }
                    }
                }
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
            "M400" if line > end_from && !klipper => {}
            "M400" => {
                let seconds =
                    word(&words, 'S').unwrap_or(0.0) + word(&words, 'P').unwrap_or(0.0) / 1e3;
                if seconds > 0.0 || klipper {
                    planner.rest(line, seconds.max(0.0));
                }
            }
            "M1" | "M01" if !klipper => planner.rest(line, 0.0),
            // Homing as the reference's estimator takes it: a move to 0 on
            // the axes named, or on all three (GCodeProcessor.cpp
            // `process_G28`, through `process_G1`).
            "G28" if !klipper => {
                let named: Vec<usize> = (['X', 'Y', 'Z'].iter().enumerate())
                    .filter(|(_, letter)| words.iter().any(|word| word.starts_with(**letter)))
                    .map(|(axis, _)| axis)
                    .collect();
                let axes = if named.is_empty() {
                    vec![0, 1, 2]
                } else {
                    named
                };
                let mut to = position;
                for axis in axes {
                    to[axis] = if absolute { 0.0 } else { position[axis] };
                }
                let delta: [f64; 4] = std::array::from_fn(|axis| to[axis] - position[axis]);
                position = to;
                if delta.iter().any(|d| *d != 0.0) && feed > 0.0 {
                    planner.go(line, delta, feed);
                }
            }
            // Bambu Lab's bed levelling, 260 s, once a file
            // (GCodeProcessor.cpp `process_G29`).
            "G29" if !klipper && limits.bambu && !levelled => {
                levelled = true;
                planner.rest(line, 260.0);
            }
            // Waiting for the chamber to warm, 720 s above 40 °C
            // (GCodeProcessor.cpp `process_M191`).
            "M191" if !klipper && word(&words, 'S').is_some_and(|heat| heat > 40.0) => {
                planner.rest(line, 720.0);
            }
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
                // Past 254 no tool is taken up, as the reference's
                // estimator reads it: Bambu Lab's own commands (`T255`,
                // `T1000`) cost nothing (GCodeProcessor.cpp `process_T`).
                let picked = other
                    .strip_prefix('T')
                    .and_then(|number| number.parse::<usize>().ok())
                    .filter(|picked| klipper || *picked <= 254);
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
