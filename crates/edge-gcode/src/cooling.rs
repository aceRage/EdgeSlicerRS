//! Slowing a layer down so it has time to cool, and choosing its fan speed.
//!
//! After the reference's CoolingBuffer.cpp, for one extruder: a layer that
//! would finish sooner than `slow_down_layer_time` has its fastest lines
//! slowed first, level by level, until it takes that long or every line is
//! down to the slowest speed allowed.

/// What to do to a layer's extrusion speeds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slowdown {
    None,
    /// No line goes faster than this, in mm/s.
    Cap(f64),
    /// Every line takes this many times as long.
    Stretch(f64),
}

impl Slowdown {
    pub fn apply(self, speed: f64) -> f64 {
        match self {
            Self::None => speed,
            Self::Cap(cap) => speed.min(cap),
            Self::Stretch(rate) => speed / rate,
        }
    }
}

/// How a layer spends its time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timing {
    /// Each extruded line's length in mm and speed in mm/s.
    pub lines: Vec<(f64, f64)>,
    /// Seconds spent travelling, lifting and retracting once the first line
    /// has started; what comes before it is not counted, as in the reference.
    pub fixed: f64,
}

impl Timing {
    pub fn seconds(&self) -> f64 {
        self.fixed
            + self
                .lines
                .iter()
                .map(|(length, speed)| length / speed)
                .sum::<f64>()
    }

    /// How long the layer takes once slowed.
    pub fn seconds_after(&self, slowdown: Slowdown) -> f64 {
        self.fixed
            + self
                .lines
                .iter()
                .map(|(length, speed)| length / slowdown.apply(*speed))
                .sum::<f64>()
    }

    /// The slowdown that brings the layer up to `layer_time` seconds, as far
    /// as `min_speed` (mm/s) allows; zero for no floor.
    pub fn slowdown(&self, layer_time: f64, min_speed: f64) -> Slowdown {
        // The reference aims a hair past the limit so rounding cannot leave
        // the layer just short of it.
        let target = layer_time * 1.001;
        let total = self.seconds();
        if self.lines.is_empty() || total > target {
            return Slowdown::None;
        }
        let mut stretch = target - total;

        // Lines already at or under the floor cannot be slowed.
        let mut lines: Vec<(f64, f64)> = self
            .lines
            .iter()
            .copied()
            .filter(|(length, speed)| *length > 0.0 && *speed > min_speed)
            .collect();
        if lines.is_empty() {
            return Slowdown::None;
        }
        if min_speed <= 0.0 {
            let adjustable: f64 = lines.iter().map(|(length, speed)| length / speed).sum();
            return Slowdown::Stretch((adjustable + stretch) / adjustable);
        }

        // Fastest first; lines at one speed are slowed together, each level
        // down to the next until the time is made up.
        lines.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut current = lines[0].1;
        let mut length = 0.0;
        let mut next = 0;
        loop {
            while next < lines.len() && lines[next].1 >= current {
                length += lines[next].0;
                next += 1;
            }
            let limit = lines
                .get(next)
                .map_or(min_speed, |line| line.1.max(min_speed));
            let gained = length / limit - length / current;
            if gained >= stretch {
                return Slowdown::Cap(length / (length / current + stretch));
            }
            stretch -= gained;
            current = limit;
            if next == lines.len() || limit <= min_speed {
                return Slowdown::Cap(current);
            }
        }
    }
}

/// The part cooling fan's settings, as percentages and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fan {
    pub min_speed: f64,
    pub max_speed: f64,
    /// Layers at least this long get the minimum speed, or none at all.
    pub cooling_layer_time: f64,
    /// Layers this short or shorter get the maximum.
    pub slow_down_layer_time: f64,
    /// The fan stays off for this many layers at the start.
    pub off_layers: usize,
    /// The layer the fan reaches its full speed on, ramping up to it; zero
    /// for no ramp.
    pub full_speed_layer: usize,
    /// Keep the fan at its minimum on long layers instead of stopping it.
    pub keep_running: bool,
}

impl Fan {
    /// Fan speed in percent for the layer with this index that takes
    /// `seconds`, after CoolingBuffer::change_extruder_set_fan.
    pub fn percent(&self, layer: usize, seconds: f64) -> f64 {
        if layer < self.off_layers {
            return 0.0;
        }
        let mut speed = if seconds < self.slow_down_layer_time {
            self.max_speed
        } else if seconds < self.cooling_layer_time {
            let share = (seconds - self.slow_down_layer_time)
                / (self.cooling_layer_time - self.slow_down_layer_time);
            (share * self.min_speed + (1.0 - share) * self.max_speed).floor()
        } else if self.keep_running {
            self.min_speed
        } else {
            0.0
        };
        if layer + 1 < self.full_speed_layer {
            let factor = (layer + 1 - self.off_layers) as f64
                / (self.full_speed_layer - self.off_layers) as f64;
            speed = (speed * factor + 0.5).floor();
        }
        speed.clamp(0.0, 100.0)
    }
}

/// The value `M106 S` takes for a percentage, as GCodeWriter::set_fan computes it.
pub fn pwm(percent: f64) -> u8 {
    (255.5 * percent / 100.0) as u8
}

/// The same for the auxiliary fan, which the reference rounds differently.
pub fn auxiliary_pwm(percent: f64) -> u8 {
    (255.0 * percent / 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds_after(timing: &Timing, slowdown: Slowdown) -> f64 {
        timing.fixed
            + timing
                .lines
                .iter()
                .map(|(length, speed)| length / slowdown.apply(*speed))
                .sum::<f64>()
    }

    #[test]
    fn long_layers_are_left_alone() {
        let timing = Timing {
            lines: vec![(100.0, 50.0), (300.0, 100.0)],
            fixed: 1.0,
        };
        assert_eq!(timing.seconds(), 6.0);
        assert_eq!(timing.slowdown(5.0, 10.0), Slowdown::None);
        assert_eq!(Timing::default().slowdown(5.0, 10.0), Slowdown::None);
    }

    #[test]
    fn the_fastest_lines_are_slowed_first() {
        // 2 s at 100 mm/s and 2 s at 50 mm/s. Needing 5 s, only the fast
        // lines give way: 200 mm in 3 s.
        let timing = Timing {
            lines: vec![(100.0, 50.0), (200.0, 100.0)],
            fixed: 0.0,
        };
        let slowdown = timing.slowdown(5.0, 10.0);
        let Slowdown::Cap(cap) = slowdown else {
            panic!("expected a cap, got {slowdown:?}")
        };
        assert!(cap > 50.0 && cap < 100.0, "cap {cap}");
        assert!((seconds_after(&timing, slowdown) - 5.005).abs() < 1e-9);

        // Needing 10 s, both end up at one speed: 300 mm in 10 s.
        let slowdown = timing.slowdown(10.0, 10.0);
        assert!((seconds_after(&timing, slowdown) - 10.01).abs() < 1e-9);
        let Slowdown::Cap(cap) = slowdown else {
            panic!()
        };
        assert!((cap - 300.0 / 10.01).abs() < 1e-9);
    }

    #[test]
    fn time_spent_travelling_counts_but_cannot_be_stretched() {
        let timing = Timing {
            lines: vec![(100.0, 100.0)],
            fixed: 3.0,
        };
        let slowdown = timing.slowdown(8.0, 10.0);
        assert!((seconds_after(&timing, slowdown) - 8.008).abs() < 1e-9);
    }

    #[test]
    fn nothing_goes_below_the_slowest_speed_allowed() {
        let timing = Timing {
            lines: vec![(10.0, 100.0), (10.0, 20.0), (5.0, 8.0)],
            fixed: 0.0,
        };
        // 60 s is out of reach: everything faster than the floor drops to it.
        assert_eq!(timing.slowdown(60.0, 10.0), Slowdown::Cap(10.0));
        // A line already slower than the floor keeps its speed.
        assert_eq!(Slowdown::Cap(10.0).apply(8.0), 8.0);
        // With every line under the floor there is nothing to do.
        let slow = Timing {
            lines: vec![(5.0, 8.0)],
            fixed: 0.0,
        };
        assert_eq!(slow.slowdown(60.0, 10.0), Slowdown::None);
    }

    #[test]
    fn without_a_floor_every_line_stretches_alike() {
        let timing = Timing {
            lines: vec![(100.0, 100.0), (100.0, 50.0)],
            fixed: 1.0,
        };
        let slowdown = timing.slowdown(10.0, 0.0);
        assert!(matches!(slowdown, Slowdown::Stretch(_)));
        assert!((seconds_after(&timing, slowdown) - 10.01).abs() < 1e-9);
    }

    #[test]
    fn fan_speed_follows_layer_time() {
        let fan = Fan {
            min_speed: 60.0,
            max_speed: 100.0,
            cooling_layer_time: 100.0,
            slow_down_layer_time: 8.0,
            off_layers: 1,
            full_speed_layer: 0,
            keep_running: true,
        };
        assert_eq!(fan.percent(0, 3.0), 0.0);
        assert_eq!(fan.percent(1, 3.0), 100.0);
        assert_eq!(fan.percent(1, 200.0), 60.0);
        // Halfway between the two times, halfway between the two speeds.
        assert_eq!(fan.percent(1, 54.0), 80.0);
        // In between, the reference rounds down.
        assert_eq!(fan.percent(1, 55.0), 79.0);
        let stopping = Fan {
            keep_running: false,
            ..fan
        };
        assert_eq!(stopping.percent(1, 200.0), 0.0);

        // Ramping up to full speed on the fifth layer.
        let ramp = Fan {
            full_speed_layer: 5,
            ..fan
        };
        assert_eq!(ramp.percent(1, 3.0), 25.0);
        assert_eq!(ramp.percent(3, 3.0), 75.0);
        assert_eq!(ramp.percent(4, 3.0), 100.0);

        assert_eq!((pwm(100.0), pwm(89.0), pwm(0.0)), (255, 227, 0));
        assert_eq!((auxiliary_pwm(50.0), auxiliary_pwm(100.0)), (127, 255));
    }
}
