//! Klipper's planner, ported from klippy/toolhead.py (`Move`,
//! `LookAheadQueue.flush`) and klippy/kinematics/extruder.py
//! (`check_move`, `calc_junction`); Klipper is GPL-3.0.
//!
//! Speeds are kept squared, as there. What Klipper reads from its own
//! configuration and no G-code file carries is taken from the printer
//! profile's machine limits, which is the nearest thing to it a slicer
//! has: the top speed and acceleration of x for `max_velocity` and
//! `max_accel`, those of z and e for the z axis and for moves of the
//! extruder alone.

use crate::{Limits, Planner, reader::Set};

/// The extruder's `instantaneous_corner_velocity`, at Klipper's default.
const INSTANT_CORNER_VELOCITY: f64 = 1.0;
/// Moves are planned from one rest to the next. Should a file go this
/// many moves without one, a rest is put in: it costs a fraction of a
/// second in hours and keeps the queue from holding the whole file.
const LONGEST_RUN: usize = 100_000;

struct Move {
    line: usize,
    move_d: f64,
    accel: f64,
    junction_deviation: f64,
    /// Direction over x, y, z and e, each as a share of `move_d`.
    axes_r: [f64; 4],
    /// False for a move of the extruder alone.
    is_kinematic: bool,
    max_start_v2: f64,
    max_cruise_v2: f64,
    /// The most the squared speed can change over the move.
    delta_v2: f64,
    // The same, under the minimum cruise ratio.
    max_mcr_start_v2: f64,
    mcr_delta_v2: f64,
}

impl Move {
    fn limit_speed(&mut self, speed: f64, accel: f64) {
        self.max_cruise_v2 = self.max_cruise_v2.min(speed * speed);
        self.accel = self.accel.min(accel);
        self.delta_v2 = 2.0 * self.move_d * self.accel;
        self.mcr_delta_v2 = self.mcr_delta_v2.min(self.delta_v2);
    }

    /// The fastest this move may start, given the one before it.
    fn calc_junction(&mut self, prev: &Move) {
        if !self.is_kinematic || !prev.is_kinematic {
            return;
        }
        // The extruder cannot change its share of a move at once.
        let diff_r = self.axes_r[3] - prev.axes_r[3];
        let extruder_v2 = if diff_r != 0.0 {
            (INSTANT_CORNER_VELOCITY / diff_r.abs()).powi(2)
        } else {
            self.max_cruise_v2
        };
        let mut max_start_v2 = self
            .max_cruise_v2
            .min(prev.max_cruise_v2)
            .min(prev.max_start_v2 + prev.delta_v2)
            .min(extruder_v2);
        // The corner as an arc of the "approximated centripetal velocity".
        let cos_theta: f64 = -(0..3).map(|a| self.axes_r[a] * prev.axes_r[a]).sum::<f64>();
        let sin_half = (0.5 * (1.0 - cos_theta)).max(0.0).sqrt();
        let cos_half = (0.5 * (1.0 + cos_theta)).max(0.0).sqrt();
        let one_minus_sin = 1.0 - sin_half;
        if one_minus_sin > 0.0 && cos_half > 0.0 {
            let r_jd = sin_half / one_minus_sin;
            let move_jd_v2 = r_jd * self.junction_deviation * self.accel;
            let prev_jd_v2 = r_jd * prev.junction_deviation * prev.accel;
            // The arc must touch each move no further in than its middle.
            let quarter_tan = 0.25 * sin_half / cos_half;
            max_start_v2 = max_start_v2
                .min(move_jd_v2)
                .min(prev_jd_v2)
                .min(self.delta_v2 * quarter_tan)
                .min(prev.delta_v2 * quarter_tan);
        }
        self.max_start_v2 = max_start_v2;
        self.max_mcr_start_v2 = max_start_v2.min(prev.max_mcr_start_v2 + prev.mcr_delta_v2);
    }

    /// Seconds for the move at these squared speeds (`set_junction`).
    fn time(&self, start_v2: f64, cruise_v2: f64, end_v2: f64) -> f64 {
        let half_inv_accel = 0.5 / self.accel;
        let accel_d = (cruise_v2 - start_v2) * half_inv_accel;
        let decel_d = (cruise_v2 - end_v2) * half_inv_accel;
        let cruise_d = self.move_d - accel_d - decel_d;
        let (start_v, cruise_v, end_v) = (start_v2.sqrt(), cruise_v2.sqrt(), end_v2.sqrt());
        // Each part's distance over its average speed.
        let part = |distance: f64, speed: f64| {
            if distance > 0.0 && speed > 0.0 {
                distance / speed
            } else {
                0.0
            }
        };
        part(accel_d, (start_v + cruise_v) * 0.5)
            + part(cruise_d, cruise_v)
            + part(decel_d, (end_v + cruise_v) * 0.5)
    }
}

pub(crate) struct Klipper {
    max_velocity: f64,
    max_accel: f64,
    square_corner_velocity: f64,
    min_cruise_ratio: f64,
    // The z axis, and the extruder moving alone or backwards.
    max_z_velocity: f64,
    max_z_accel: f64,
    max_e_velocity: f64,
    max_e_accel: f64,
    queue: Vec<Move>,
    times: Vec<f64>,
}

impl Klipper {
    pub(crate) fn new(limits: &Limits) -> Self {
        let or = |limit: f64, default: f64| if limit > 0.0 { limit } else { default };
        Self {
            max_velocity: or(limits.max_speed[0], f64::INFINITY),
            max_accel: or(limits.max_extruding, or(limits.max_acceleration[0], 3000.0)),
            square_corner_velocity: or(limits.jerk[0], 5.0),
            min_cruise_ratio: limits.minimum_cruise_ratio.clamp(0.0, 0.999),
            max_z_velocity: or(limits.max_speed[2], f64::INFINITY),
            max_z_accel: or(limits.max_acceleration[2], f64::INFINITY),
            max_e_velocity: or(limits.max_speed[3], f64::INFINITY),
            max_e_accel: or(
                limits.max_retracting,
                or(limits.max_acceleration[3], f64::INFINITY),
            ),
            queue: Vec::new(),
            times: Vec::new(),
        }
    }

    fn spend(&mut self, line: usize, seconds: f64) {
        if self.times.len() <= line {
            self.times.resize(line + 1, 0.0);
        }
        self.times[line] += seconds;
    }

    /// Plans every queued move, the last of them ending at rest
    /// (`LookAheadQueue.flush`, not lazy).
    fn flush(&mut self) {
        let queue = std::mem::take(&mut self.queue);
        let count = queue.len();
        // From the last move to the first: how fast each may start.
        // (start_v2, cruise_v2 where it is known yet, next move's start_v2)
        let mut junctions: Vec<(f64, Option<f64>, f64)> = vec![(0.0, None, 0.0); count];
        let (mut next_start_v2, mut next_mcr_start_v2, mut peak_cruise_v2) =
            (0.0f64, 0.0f64, 0.0f64);
        let mut pending = 0usize;
        for (i, m) in queue.iter().enumerate().rev() {
            let reachable_start_v2 = next_start_v2 + m.delta_v2;
            let start_v2 = m.max_start_v2.min(reachable_start_v2);
            let mut cruise_v2 = None;
            pending += 1;
            let reach_mcr_start_v2 = next_mcr_start_v2 + m.mcr_delta_v2;
            let mcr_start_v2 = m.max_mcr_start_v2.min(reach_mcr_start_v2);
            if mcr_start_v2 < reach_mcr_start_v2 {
                // This move can accelerate.
                if mcr_start_v2 + m.mcr_delta_v2 > next_mcr_start_v2 || pending > 1 {
                    // It can also slow down, or it is all acceleration
                    // and the next all deceleration: a peak, held to
                    // what the minimum cruise ratio allows.
                    peak_cruise_v2 = (mcr_start_v2 + reach_mcr_start_v2) * 0.5;
                }
                cruise_v2 = Some(
                    ((start_v2 + reachable_start_v2) * 0.5)
                        .min(m.max_cruise_v2)
                        .min(peak_cruise_v2),
                );
                pending = 0;
            }
            junctions[i] = (start_v2, cruise_v2, next_start_v2);
            next_start_v2 = start_v2;
            next_mcr_start_v2 = mcr_start_v2;
        }
        // And forwards, carrying the cruise speed to moves that cannot
        // accelerate.
        let mut prev_cruise_v2 = 0.0f64;
        for (m, (start_v2, cruise_v2, next_start_v2)) in queue.iter().zip(junctions) {
            let cruise_v2 = cruise_v2.unwrap_or_else(|| prev_cruise_v2.min(start_v2));
            let seconds = m.time(
                start_v2.min(cruise_v2),
                cruise_v2,
                next_start_v2.min(cruise_v2),
            );
            self.spend(m.line, seconds);
            prev_cruise_v2 = cruise_v2;
        }
    }
}

impl Planner for Klipper {
    fn go(&mut self, line: usize, delta: [f64; 4], feed: f64) {
        let across = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        let is_kinematic = across >= 1e-9;
        let (move_d, mut accel, velocity) = if is_kinematic {
            (across, self.max_accel, feed.min(self.max_velocity))
        } else {
            // The extruder alone: its own limits, set below.
            (delta[3].abs(), 99_999_999.9, feed)
        };
        if move_d == 0.0 || velocity <= 0.0 {
            return;
        }
        if !is_kinematic {
            accel = accel.min(self.max_e_accel);
        }
        let axes_r: [f64; 4] = if is_kinematic {
            delta.map(|d| d / move_d)
        } else {
            [0.0, 0.0, 0.0, delta[3] / move_d]
        };
        let mcr_pseudo_accel = self.max_accel * (1.0 - self.min_cruise_ratio);
        let mut m = Move {
            line,
            move_d,
            accel,
            junction_deviation: self.square_corner_velocity.powi(2)
                * (std::f64::consts::SQRT_2 - 1.0)
                / self.max_accel,
            axes_r,
            is_kinematic,
            max_start_v2: 0.0,
            max_cruise_v2: velocity * velocity,
            delta_v2: 2.0 * move_d * accel,
            max_mcr_start_v2: 0.0,
            mcr_delta_v2: 2.0 * move_d * mcr_pseudo_accel,
        };
        if !is_kinematic {
            m.limit_speed(self.max_e_velocity, self.max_e_accel);
        } else {
            // A move with any z in it goes no faster than z can.
            if delta[2] != 0.0 {
                let z_ratio = move_d / delta[2].abs();
                m.limit_speed(self.max_z_velocity * z_ratio, self.max_z_accel * z_ratio);
            }
            // Nor one that draws filament back faster than the extruder can.
            if axes_r[3] < 0.0 {
                let inv_extrude_r = 1.0 / axes_r[3].abs();
                m.limit_speed(
                    self.max_e_velocity * inv_extrude_r,
                    self.max_e_accel * inv_extrude_r,
                );
            }
        }
        if let Some(prev) = self.queue.last() {
            m.calc_junction(prev);
        }
        self.queue.push(m);
        if self.queue.len() >= LONGEST_RUN {
            self.flush();
        }
    }

    fn rest(&mut self, line: usize, extra: f64) {
        self.flush();
        self.spend(line, extra);
    }

    fn set(&mut self, what: Set) {
        match what {
            // One acceleration for everything: Klipper has no other.
            Set::Acceleration(value) | Set::TravelAcceleration(value) if value > 0.0 => {
                self.max_accel = value;
            }
            Set::SquareCornerVelocity(value) if value >= 0.0 => {
                self.square_corner_velocity = value;
            }
            Set::CruiseRatio(value) => self.min_cruise_ratio = value.clamp(0.0, 0.999),
            Set::MaxVelocity(value) if value > 0.0 => self.max_velocity = value,
            // Limits a Klipper printer keeps in its own configuration,
            // and Marlin's settings, which it does not have.
            _ => {}
        }
    }

    fn finish(mut self, lines: usize) -> Vec<f64> {
        self.flush();
        self.times.resize(lines.max(self.times.len()), 0.0);
        self.times
    }
}
