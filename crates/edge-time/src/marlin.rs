//! The reference slicer's estimator, ported from GCodeProcessor.cpp
//! (`process_G1`'s time section, `TimeBlock`, `TimeMachine::calculate_time`
//! and the planner kernels it has from Marlin by way of PrusaSlicer).

use crate::{Flavor, Limits, Planner, reader::Set};

// What the reference assumes of a machine that gives no limit.
const DEFAULT_ACCELERATION: f64 = 1500.0;
const DEFAULT_RETRACT_ACCELERATION: f64 = 1500.0;
const DEFAULT_TRAVEL_ACCELERATION: f64 = 1250.0;
/// Blocks the firmware keeps planning, and how many gather before the
/// oldest are settled (`TimeProcessor::Planner`).
const QUEUE: usize = 64;
const REFRESH: usize = QUEUE * 4;

#[derive(Clone, Copy, Default)]
struct State {
    feedrate: f64,
    safe_feedrate: f64,
    axis_feedrate: [f64; 4],
    abs_axis_feedrate: [f64; 4],
    /// Direction over x, y and z; the reference keeps one for the way in
    /// and one for the way out, which differ only for arcs it does not
    /// cut up. Here every move is straight.
    direction: [f64; 3],
    /// Direction over x, y, z and e.
    unit: [f64; 4],
}

#[derive(Clone, Copy)]
struct Block {
    line: usize,
    distance: f64,
    acceleration: f64,
    entry: f64,
    cruise: f64,
    exit: f64,
    max_entry: f64,
    safe: f64,
    nominal_length: bool,
    recalculate: bool,
    // The trapezoid: how far the block accelerates, where it starts to
    // slow, and the speed it holds between.
    accelerate_until: f64,
    decelerate_after: f64,
    cruise_feedrate: f64,
}

fn sqr(value: f64) -> f64 {
    value * value
}

fn estimated_acceleration_distance(initial: f64, target: f64, acceleration: f64) -> f64 {
    if acceleration == 0.0 {
        0.0
    } else {
        (sqr(target) - sqr(initial)) / (2.0 * acceleration)
    }
}

fn intersection_distance(initial: f64, last: f64, acceleration: f64, distance: f64) -> f64 {
    if acceleration == 0.0 {
        0.0
    } else {
        (2.0 * acceleration * distance - sqr(initial) + sqr(last)) / (4.0 * acceleration)
    }
}

fn speed_from_distance(initial: f64, distance: f64, acceleration: f64) -> f64 {
    (sqr(initial) + 2.0 * acceleration * distance)
        .max(0.0)
        .sqrt()
}

/// The fastest a move may start and still reach `target` within `distance`.
fn max_allowable_speed(acceleration: f64, target: f64, distance: f64) -> f64 {
    (sqr(target) - 2.0 * acceleration * distance)
        .max(0.0)
        .sqrt()
}

fn acceleration_time_from_distance(initial: f64, distance: f64, acceleration: f64) -> f64 {
    if acceleration != 0.0 {
        (speed_from_distance(initial, distance, acceleration) - initial) / acceleration
    } else {
        0.0
    }
}

impl Block {
    fn calculate_trapezoid(&mut self) {
        let mut accelerate =
            estimated_acceleration_distance(self.entry, self.cruise, self.acceleration).max(0.0);
        let decelerate =
            estimated_acceleration_distance(self.cruise, self.exit, -self.acceleration).max(0.0);
        let mut cruise = self.distance - accelerate - decelerate;
        // Not enough room to reach the speed asked for: accelerate up to
        // the point from which the exit speed can still be met.
        if cruise < 0.0 {
            accelerate =
                intersection_distance(self.entry, self.exit, self.acceleration, self.distance)
                    .clamp(0.0, self.distance);
            cruise = 0.0;
            self.cruise_feedrate = speed_from_distance(self.entry, accelerate, self.acceleration);
        } else {
            self.cruise_feedrate = self.cruise;
        }
        self.accelerate_until = accelerate;
        self.decelerate_after = accelerate + cruise;
    }

    fn time(&self) -> f64 {
        let cruise = if self.cruise_feedrate != 0.0 {
            (self.decelerate_after - self.accelerate_until) / self.cruise_feedrate
        } else {
            0.0
        };
        acceleration_time_from_distance(self.entry, self.accelerate_until, self.acceleration)
            + cruise
            + acceleration_time_from_distance(
                self.cruise_feedrate,
                self.distance - self.decelerate_after,
                -self.acceleration,
            )
    }
}

pub(crate) struct Marlin {
    limits: Limits,
    acceleration: f64,
    travel_acceleration: f64,
    retract_acceleration: f64,
    extrude_factor: f64,
    prev: State,
    blocks: Vec<Block>,
    times: Vec<f64>,
}

impl Marlin {
    pub(crate) fn new(limits: &Limits) -> Self {
        let or = |limit: f64, default: f64| if limit > 0.0 { limit } else { default };
        Self {
            limits: *limits,
            acceleration: or(limits.max_extruding, DEFAULT_ACCELERATION),
            travel_acceleration: or(limits.max_travel, DEFAULT_TRAVEL_ACCELERATION),
            retract_acceleration: or(limits.max_retracting, DEFAULT_RETRACT_ACCELERATION),
            extrude_factor: 1.0,
            prev: State::default(),
            blocks: Vec::new(),
            times: Vec::new(),
        }
    }

    fn spend(&mut self, line: usize, seconds: f64) {
        if self.times.len() <= line {
            self.times.resize(line + 1, 0.0);
        }
        self.times[line] += seconds;
    }

    /// Junction deviation for a block of this acceleration; zero where
    /// corners are planned by jerk (`get_junction_deviation`).
    fn junction_deviation(&self, acceleration: f64) -> f64 {
        match self.limits.flavor {
            // Klipper has no jerk: jd = scv² (√2 - 1) / accel, and the
            // square corner velocity is kept where x's jerk is.
            Flavor::Klipper => {
                let scv = self.limits.jerk[0];
                if scv <= 0.0 || acceleration <= 0.0 {
                    0.0
                } else {
                    sqr(scv) * (std::f64::consts::SQRT_2 - 1.0) / acceleration
                }
            }
            Flavor::Marlin2 => self.limits.junction_deviation,
            Flavor::Other => 0.0,
        }
    }

    /// An axis's jerk, which Marlin 2 with a junction deviation set
    /// derives from it (`get_axis_max_jerk`).
    fn axis_jerk(&self, axis: usize) -> f64 {
        let jd = self.limits.junction_deviation;
        if self.limits.flavor == Flavor::Marlin2 && jd > 0.0 {
            let mut effective = self.acceleration;
            let limit = self.limits.max_acceleration[axis];
            if limit > 0.0 {
                effective = if effective > 0.0 {
                    effective.min(limit)
                } else {
                    limit
                };
            }
            if effective <= 0.0 {
                return 0.0;
            }
            return (jd * effective * 2.5).sqrt();
        }
        self.limits.jerk[axis]
    }

    /// The speed two moves may share where they meet, by junction
    /// deviation; none where the machine plans by jerk
    /// (`calc_vmax_junction_deviation`).
    fn junction_speed(&self, block: &Block, curr: &State, has_prev: bool) -> Option<f64> {
        let jd = self.junction_deviation(block.acceleration);
        if jd <= 0.0 {
            return None;
        }
        if !has_prev {
            return Some(0.0);
        }
        let prev = &self.prev;
        // -1 straight on, +1 straight back.
        let cos_theta: f64 = -(0..4).map(|a| prev.unit[a] * curr.unit[a]).sum::<f64>();
        if cos_theta > 0.999999 {
            return Some(0.0);
        }
        let cos_theta = cos_theta.max(-0.999999);
        let sin_half = (0.5 * (1.0 - cos_theta)).sqrt();
        let across: [f64; 4] = std::array::from_fn(|a| curr.unit[a] - prev.unit[a]);
        let length = across.iter().map(|v| v * v).sum::<f64>().sqrt();
        let mut acceleration = block.acceleration;
        if length > 0.0 {
            for (axis, part) in across.iter().enumerate() {
                let along = part / length;
                let limit = self.limits.max_acceleration[axis];
                if along != 0.0 && limit > 0.0 {
                    acceleration = acceleration.min((limit / along).abs());
                }
            }
        }
        let mut speed_sqr = acceleration * jd * sin_half / (1.0 - sin_half);
        // Marlin treats a short move through a shallow corner as an arc.
        if self.limits.flavor != Flavor::Klipper
            && block.distance < 1.0
            && cos_theta < -std::f64::consts::FRAC_1_SQRT_2
        {
            let neg = if cos_theta < 0.0 { -1.0 } else { 1.0 };
            let t = neg * cos_theta;
            let asin = 0.032843707
                + t * (-1.451838349
                    + t * (29.66153956
                        + t * (-131.1123477
                            + t * (262.8130562 + t * (-242.7199627 + t * 84.31466202)))));
            let theta = std::f64::consts::FRAC_PI_2 + neg * asin;
            speed_sqr = speed_sqr.min(block.distance * acceleration / theta);
        }
        speed_sqr = speed_sqr.min(sqr(block.cruise).min(sqr(prev.feedrate)));
        Some(speed_sqr.sqrt())
    }

    /// The speed two moves may share where they meet, by jerk: the
    /// classic planner's rule.
    fn jerk_speed(&self, block: &Block, curr: &State) -> f64 {
        let prev = &self.prev;
        let prev_larger = prev.feedrate > block.cruise;
        let smaller_factor = if prev_larger {
            block.cruise / prev.feedrate
        } else {
            prev.feedrate / block.cruise
        };
        let mut junction = if prev_larger {
            block.cruise
        } else {
            prev.feedrate
        };
        let mut factor = 1.0;
        let mut limited = false;
        // x, y and z together, by how much each changes.
        let mut jerk: [f64; 3] = std::array::from_fn(|a| {
            let mut exit = prev.feedrate * prev.direction[a];
            if prev_larger {
                exit *= smaller_factor;
            }
            (block.cruise * curr.direction[a] - exit).abs()
        });
        for axis in 0..3 {
            let limit = self.axis_jerk(axis);
            if jerk[axis] > limit {
                factor *= limit / jerk[axis];
                for value in &mut jerk {
                    *value *= factor;
                }
                limited = true;
            }
        }
        // Then the extruder.
        let (mut exit, mut entry) = (prev.axis_feedrate[3], curr.axis_feedrate[3]);
        if prev_larger {
            exit *= smaller_factor;
        }
        if limited {
            exit *= factor;
            entry *= factor;
        }
        // Coasting on in the same direction, or turning round.
        let change = if exit > entry {
            if entry > 0.0 || exit < 0.0 {
                exit - entry
            } else {
                exit.max(-entry)
            }
        } else if entry < 0.0 || exit > 0.0 {
            entry - exit
        } else {
            (-exit).max(entry)
        };
        let limit = self.axis_jerk(3);
        if change > limit {
            factor *= limit / change;
            limited = true;
        }
        if limited {
            junction *= factor;
        }
        // Not coasting: the machine stops and starts anyway.
        let threshold = junction * 0.99;
        if prev.safe_feedrate > threshold && curr.safe_feedrate > threshold {
            junction = curr.safe_feedrate;
        }
        junction
    }

    /// Plans what is queued and settles all but the last `keep` blocks
    /// (`TimeMachine::calculate_time`).
    fn calculate(&mut self, keep: usize) {
        let count = self.blocks.len();
        // Backwards: no block enters faster than it can slow from.
        for i in (1..count).rev() {
            let next = self.blocks[i];
            let curr = &mut self.blocks[i - 1];
            if curr.entry != curr.max_entry || next.recalculate {
                let entry = if curr.nominal_length {
                    curr.max_entry
                } else {
                    curr.max_entry.min(max_allowable_speed(
                        -curr.acceleration,
                        next.entry,
                        curr.distance,
                    ))
                };
                if curr.entry != entry {
                    curr.entry = entry;
                    curr.recalculate = true;
                }
            }
        }
        // Forwards: nor faster than the one before can bring it to.
        for i in 0..count.saturating_sub(1) {
            let prev = self.blocks[i];
            let curr = &mut self.blocks[i + 1];
            if !prev.nominal_length && prev.entry < curr.entry {
                let entry = max_allowable_speed(-prev.acceleration, prev.entry, prev.distance);
                if entry < curr.entry {
                    curr.entry = entry;
                    curr.recalculate = true;
                }
            }
        }
        for i in 1..count {
            let next = self.blocks[i];
            let curr = &mut self.blocks[i - 1];
            if curr.recalculate || next.recalculate {
                curr.exit = next.entry;
                curr.calculate_trapezoid();
                curr.recalculate = false;
            }
        }
        if let Some(last) = self.blocks.last_mut() {
            last.exit = last.safe;
            last.calculate_trapezoid();
            last.recalculate = false;
        }
        let settled = count.saturating_sub(keep);
        let done: Vec<(usize, f64)> = self.blocks[..settled]
            .iter()
            .map(|block| (block.line, block.time()))
            .collect();
        self.blocks.drain(..settled);
        for (line, seconds) in done {
            self.spend(line, seconds);
        }
    }
}

impl Planner for Marlin {
    fn go(&mut self, line: usize, delta: [f64; 4], feed: f64) {
        let across = sqr(delta[0]) + sqr(delta[1]) + sqr(delta[2]);
        let distance = if across > 0.0 {
            across.sqrt()
        } else {
            delta[3].abs()
        };
        if distance == 0.0 {
            return;
        }
        let extrusion_only = across == 0.0;
        let travel = delta[3] == 0.0;
        let mut curr = State {
            feedrate: if travel {
                feed.max(self.limits.min_travel_rate)
            } else {
                feed.max(self.limits.min_extruding_rate)
            },
            ..State::default()
        };
        if !extrusion_only {
            let length = across.sqrt();
            curr.direction = [delta[0] / length, delta[1] / length, delta[2] / length];
        }
        let whole = (across + sqr(delta[3])).sqrt();
        curr.unit = delta.map(|d| d / whole);

        // A gentle bend is taken no faster than its radius allows.
        let prev = self.prev;
        let flat = |d: [f64; 3]| d[0] != 0.0 || d[1] != 0.0;
        if flat(prev.direction) && flat(curr.direction) {
            let unit = |d: [f64; 3]| {
                let length = d[0].hypot(d[1]);
                [d[0] / length, d[1] / length]
            };
            let (v1, v2) = (unit(prev.direction), unit(curr.direction));
            let turn = (v2[0] - v1[0]).hypot(v2[1] - v1[1]);
            if turn < 0.5 && turn > 0.00001 {
                let dot = v1[0] * v2[0] + v1[1] * v2[1];
                let cross = v1[0] * v2[1] - v1[1] * v2[0];
                let angle = cross.atan2(dot);
                let sin_half = ((1.0 - angle.cos()) * 0.5).sqrt();
                let radius = delta[0].hypot(delta[1]) * 0.5 / sin_half;
                curr.feedrate = curr.feedrate.min((self.acceleration * radius).sqrt());
            }
        }

        // No axis faster than it can go.
        let mut slowest = 1.0f64;
        for (axis, change) in delta.iter().enumerate() {
            curr.axis_feedrate[axis] = curr.feedrate * change / distance;
            if axis == 3 {
                curr.axis_feedrate[axis] *= self.extrude_factor;
            }
            curr.abs_axis_feedrate[axis] = curr.axis_feedrate[axis].abs();
            let limit = self.limits.max_speed[axis];
            if curr.abs_axis_feedrate[axis] != 0.0 && limit != 0.0 {
                slowest = slowest.min(limit / curr.abs_axis_feedrate[axis]);
            }
        }
        curr.feedrate *= slowest;
        if slowest < 1.0 {
            for axis in 0..4 {
                curr.axis_feedrate[axis] *= slowest;
                curr.abs_axis_feedrate[axis] *= slowest;
            }
        }

        let mut acceleration = if travel {
            self.travel_acceleration
        } else if extrusion_only {
            self.retract_acceleration
        } else {
            self.acceleration
        };
        for (axis, change) in delta.iter().enumerate() {
            let share = change.abs() / distance;
            let limit = self.limits.max_acceleration[axis];
            // A machine that gives no limit has none.
            if limit > 0.0 && acceleration * share > limit {
                acceleration = limit / share;
            }
        }

        let mut block = Block {
            line,
            distance,
            acceleration,
            entry: 0.0,
            cruise: curr.feedrate,
            exit: 0.0,
            max_entry: 0.0,
            safe: 0.0,
            nominal_length: false,
            recalculate: true,
            accelerate_until: 0.0,
            decelerate_after: 0.0,
            cruise_feedrate: 0.0,
        };
        let has_prev = !self.blocks.is_empty() && prev.feedrate > 0.0001;
        let by_deviation = self.junction_speed(&block, &curr, has_prev);
        // By junction deviation a move is free to start from rest; by
        // jerk it may start at whatever its jerk allows.
        curr.safe_feedrate = match by_deviation {
            Some(_) => 0.0,
            None => {
                let mut safe = block.cruise;
                for axis in 0..4 {
                    let jerk = self.axis_jerk(axis);
                    if curr.abs_axis_feedrate[axis] > jerk {
                        safe = safe.min(jerk);
                    }
                }
                safe
            }
        };
        block.exit = curr.safe_feedrate;
        let junction = match by_deviation {
            Some(speed) => speed,
            None if has_prev => self.jerk_speed(&block, &curr),
            None => curr.safe_feedrate,
        };
        let allowable = max_allowable_speed(-acceleration, curr.safe_feedrate, distance);
        block.entry = junction.min(allowable);
        block.max_entry = junction;
        block.nominal_length = block.cruise <= allowable;
        block.safe = curr.safe_feedrate;
        block.calculate_trapezoid();
        self.prev = curr;
        self.blocks.push(block);
        if self.blocks.len() > REFRESH {
            self.calculate(QUEUE);
        }
    }

    fn rest(&mut self, line: usize, extra: f64) {
        // With fewer than two blocks queued the reference only notes the
        // wait: the block stays to be planned with what follows.
        if self.blocks.len() >= 2 {
            self.calculate(0);
        }
        self.spend(line, extra);
    }

    fn set(&mut self, what: Set) {
        // A limit of zero is none; a value set is held to the limit.
        let held = |value: f64, limit: f64| {
            if limit == 0.0 {
                value
            } else {
                value.min(limit)
            }
        };
        match what {
            Set::Acceleration(value) => {
                self.acceleration = held(value, self.limits.max_extruding);
            }
            Set::TravelAcceleration(value) => {
                self.travel_acceleration = held(value, self.limits.max_travel);
            }
            Set::RetractAcceleration(value) => {
                self.retract_acceleration = held(value, self.limits.max_retracting);
            }
            // Kept where x's and y's jerk are, as the reference keeps it.
            Set::SquareCornerVelocity(value) => {
                self.limits.jerk[0] = value;
                self.limits.jerk[1] = value;
            }
            // The reference's estimator knows neither.
            Set::CruiseRatio(_) | Set::MaxVelocity(_) => {}
            Set::AxisMaxSpeed(axis, value) => self.limits.max_speed[axis] = value,
            Set::AxisMaxAcceleration(axis, value) => self.limits.max_acceleration[axis] = value,
            Set::AxisJerk(axis, value) => self.limits.jerk[axis] = value,
            Set::JunctionDeviation(value) => self.limits.junction_deviation = value,
            Set::MinExtrudingRate(value) => self.limits.min_extruding_rate = value,
            Set::MinTravelRate(value) => self.limits.min_travel_rate = value,
            Set::ExtrudeFactor(value) => self.extrude_factor = value,
        }
    }

    fn finish(mut self, lines: usize) -> Vec<f64> {
        self.calculate(0);
        self.times.resize(lines.max(self.times.len()), 0.0);
        self.times
    }
}
