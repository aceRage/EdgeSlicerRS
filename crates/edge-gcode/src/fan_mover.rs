//! The part cooling fan told earlier than where the cooling sets it, and
//! kick-started at full speed before it settles (`fan_speedup_time`,
//! `fan_speedup_overhangs`, `fan_kickstart`): a port of the reference's
//! post-processor (GCode/FanMover.cpp), which reads each layer's finished
//! text after the cooling and writes it again (GCode.cpp `process_layers`,
//! the `fan_mover` filter).
//!
//! A command that raises the fan is moved back by the time the moves before
//! it take, splitting a `G1` where the moment falls inside it; one that
//! lowers it stays put. Only the part cooling fan is moved (`M106`,
//! `M106 P1`, `M107`); the layer's text is let out whole at its end, so a
//! command never moves into the layer before. The printer's own G-code
//! between "; custom gcode start" and "; custom gcode end" (a tool change) is
//! never split and its fan commands are not moved.
//!
//! Two things are not as the reference has them, on purpose:
//! - the reference counts an arc (`G2`, `G3`) as taking no time, so a fan
//!   command after arcs is moved back much further than asked (measured on
//!   an Elegoo Centauri Carbon file: 0.5 s asked, about 2 s moved); here an
//!   arc takes its length at its speed, as a straight move does;
//! - the reference writes an empty line after the full-speed kick-start
//!   command it adds where the fan is not being moved (its `BufferData` keeps
//!   the command's line end and the output adds another); not here.
//!
//! The reference's other fan commands (`M126`/`M127` of MakerWare and
//! Sailfish, `M106 P` of Mach3) are not written by this engine at all.

use std::collections::VecDeque;

/// The libslic3r `EPSILON` the buffer is let out by.
const EPSILON: f64 = 1e-4;

/// What the printer profile asks of the fan's timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FanTiming {
    /// How many seconds early a rising fan is told (`fan_speedup_time`,
    /// its size: the reference reads a negative time as the same).
    pub speedup: f64,
    /// Only a rise for an overhang wall is moved (`fan_speedup_overhangs`).
    pub only_overhangs: bool,
    /// How long, at most, the fan runs at full speed before it is set to a
    /// higher speed (`fan_kickstart`), for the whole rise from off.
    pub kickstart: f64,
}

impl FanTiming {
    /// The timing asked for, if any (GCode.cpp: the filter runs when
    /// `fan_speedup_time` is not 0 or `fan_kickstart` is above 0).
    pub fn of(speedup: f64, only_overhangs: bool, kickstart: f64) -> Option<Self> {
        (speedup != 0.0 || kickstart > 0.0).then_some(Self {
            speedup: speedup.abs(),
            only_overhangs,
            kickstart,
        })
    }
}

/// A line held back, with how long it takes and, for a move, where it
/// starts and how far it goes.
#[derive(Clone, Debug, Default)]
struct Held {
    raw: String,
    time: f32,
    /// The fan's speed in percent the line sets, or -1.
    fan_speed: i16,
    is_kickstart: bool,
    /// A line of the printer's own G-code, which is never split.
    is_custom: bool,
    x: f32,
    y: f32,
    z: f32,
    dx: f32,
    dy: f32,
    dz: f32,
    de: f32,
}

impl Held {
    fn new(raw: &str, time: f32, fan_speed: i16, is_kickstart: bool) -> Self {
        Self {
            raw: raw.trim_end_matches('\n').to_owned(),
            time,
            fan_speed,
            is_kickstart,
            ..Self::default()
        }
    }
}

/// One line read: its command and the axes it names.
#[derive(Default)]
struct Line<'a> {
    raw: &'a str,
    command: &'a str,
    /// X, Y, Z, E, F, I, J, P.
    axes: [Option<f32>; 8],
}

const X: usize = 0;
const Y: usize = 1;
const Z: usize = 2;
const E: usize = 3;
const F: usize = 4;
const I: usize = 5;
const J: usize = 6;
const P: usize = 7;

impl<'a> Line<'a> {
    /// As GCodeReader.cpp `parse_line_internal` reads a line: the command,
    /// then each word whose letter is an axis and the rest of it a number.
    fn parse(raw: &'a str) -> Self {
        let code = raw.split(';').next().unwrap_or_default();
        let mut words = code.split_whitespace();
        let command = words.next().unwrap_or_default();
        let mut axes = [None; 8];
        for word in words {
            let mut chars = word.chars();
            let axis = match chars.next() {
                Some('X') => X,
                Some('Y') => Y,
                Some('Z') => Z,
                Some('E') => E,
                Some('F') => F,
                Some('I') => I,
                Some('J') => J,
                Some('P') => P,
                _ => continue,
            };
            let number = chars.as_str();
            // The reader's number: no sign of plus, no words.
            if number.starts_with('+')
                || !number
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'e' | b'E'))
            {
                continue;
            }
            if let Ok(value) = number.parse::<f64>() {
                axes[axis] = Some(value as f32);
            }
        }
        Self { raw, command, axes }
    }

    /// How far it goes along an axis from where the reader is.
    fn dist(&self, axis: usize, at: &[f32; 4]) -> f32 {
        self.axes[axis].map_or(0.0, |value| value - at[axis])
    }
}

/// The post-processor, kept from layer to layer.
pub struct FanMover {
    delay: f32,
    only_overhangs: bool,
    kickstart: f32,
    /// Where the reader is: X, Y, Z, E.
    at: [f32; 4],
    /// The moves' speed in mm/s.
    speed: f64,
    overhang: bool,
    in_custom: bool,
    /// The fan's speed where the buffer is let out, and where lines come in.
    front_fan: i32,
    back_fan: i32,
    /// A kick-start's own command, waiting for its time.
    kick: Held,
    buffer: VecDeque<Held>,
    buffer_time: f64,
    output: String,
}

impl FanMover {
    pub fn new(timing: FanTiming) -> Self {
        let delay = timing.speedup as f32;
        Self {
            delay: if delay > 0.0 { delay.max(0.01) } else { 0.0 },
            only_overhangs: timing.only_overhangs,
            kickstart: timing.kickstart as f32,
            at: [0.0; 4],
            speed: 1000.0 / 60.0,
            overhang: false,
            in_custom: false,
            front_fan: 0,
            back_fan: 0,
            kick: Held {
                time: -1.0,
                ..Held::default()
            },
            buffer: VecDeque::new(),
            buffer_time: 0.0,
            output: String::new(),
        }
    }

    /// A layer's text, written again with its fan commands moved; the
    /// whole of it comes out (FanMover.cpp `process_gcode`, `flush`).
    pub fn process_layer(&mut self, text: &str) -> String {
        self.output.clear();
        self.buffer_time = self.buffer.iter().map(|held| f64::from(held.time)).sum();
        for raw in text.lines() {
            self.line(raw);
        }
        while let Some(front) = self.buffer.pop_front() {
            self.output.push_str(&front.raw);
            self.output.push('\n');
            if front.fan_speed >= 0 {
                self.front_fan = i32::from(front.fan_speed);
            }
        }
        self.buffer_time = 0.0;
        std::mem::take(&mut self.output)
    }

    fn put(&mut self, held: Held) {
        self.buffer_time += f64::from(held.time);
        self.buffer.push_back(held);
    }

    fn remove(&mut self, index: usize) {
        if let Some(held) = self.buffer.remove(index) {
            self.buffer_time -= f64::from(held.time);
        }
    }

    /// FanMover.cpp `_set_fan`: GCodeWriter.cpp `set_fan`, in percent.
    fn set_fan(speed: i32) -> String {
        if speed <= 0 {
            "M106 S0".to_owned()
        } else {
            format!("M106 S{}", (255.5 * f64::from(speed) / 100.0) as u32)
        }
    }

    /// FanMover.cpp `_process_gcode_line`.
    fn line(&mut self, raw: &str) {
        let line = Line::parse(raw);
        let mut need_flush = false;
        let mut time: f64 = 0.0;
        let mut fan_speed: i16 = -1;
        let command = line.command;
        if command.len() > 1 {
            if let Some(feed) = line.axes[F] {
                self.speed = f64::from(feed / 60.0);
            }
            match command.as_bytes()[0].to_ascii_uppercase() {
                b'G' => {
                    let number = &command[1..];
                    if matches!(number, "0" | "1" | "00" | "01") {
                        let dx = f64::from(line.dist(X, &self.at));
                        let dy = f64::from(line.dist(Y, &self.at));
                        let dz = f64::from(line.dist(Z, &self.at));
                        let dist = dx * dx + dy * dy + dz * dz;
                        if dist > 0.0 {
                            time = dist.sqrt() / self.speed;
                        }
                    } else if matches!(number, "2" | "3" | "02" | "03") {
                        // Not as the reference, which counts no time for
                        // an arc (see the module's note).
                        time = self.arc_length(&line, number.ends_with('2')) / self.speed;
                    }
                }
                b'M' => {
                    if let Some(speed) = fan_command(raw) {
                        self.fan(&line, speed, &mut time, &mut need_flush);
                        fan_speed = speed;
                    }
                }
                _ => {}
            }
        } else if raw.starts_with(';') {
            if raw.len() > 10
                && let Some(role) = raw.strip_prefix(";TYPE:")
            {
                self.overhang = role == "Overhang wall";
            }
            if raw.starts_with(";_FAN_MOVER_SKIP_") {
                self.in_custom = raw.starts_with(";_FAN_MOVER_SKIP_START");
                time = -1.0;
            }
            if raw.len() > 16 && raw.starts_with("; custom gcode") {
                self.in_custom = !raw.starts_with("; custom gcode end");
            }
        }

        if time >= 0.0 {
            let mut held = Held::new(raw, time as f32, fan_speed, false);
            held.is_custom = self.in_custom;
            if line.axes[X].is_some() {
                held.x = self.at[X];
                held.dx = line.dist(X, &self.at);
            }
            if line.axes[Y].is_some() {
                held.y = self.at[Y];
                held.dy = line.dist(Y, &self.at);
            }
            if line.axes[Z].is_some() {
                held.z = self.at[Z];
                held.dz = line.dist(Z, &self.at);
            }
            if let Some(e) = line.axes[E] {
                // This engine's extrusion is relative.
                held.de = e;
            }
            self.put(held);
            if self.kick.time > 0.0 && time > 0.0 {
                self.kick.time = (f64::from(self.kick.time) - time) as f32;
                if self.kick.time < 0.0 {
                    let last = self.buffer.len() - 1;
                    let at = (time + f64::from(self.kick.time)) as f32;
                    let kick = Held::new(&self.kick.raw, 0.0, self.kick.fan_speed, true);
                    self.put_in_middle(last, at, kick);
                }
            }
            // Let out what is more than the delay behind.
            while let Some(front) = self.buffer.front() {
                let over =
                    self.buffer_time - f64::from(front.time) > f64::from(self.delay) - EPSILON;
                if !need_flush && !over {
                    break;
                }
                let Some(front) = self.buffer.pop_front() else {
                    break;
                };
                self.buffer_time -= f64::from(front.time);
                // A fan command repeating the speed the fan is at is dropped.
                if front.fan_speed < 0
                    || i32::from(front.fan_speed) != self.front_fan
                    || front.is_kickstart
                {
                    if front.is_kickstart && i32::from(front.fan_speed) < self.front_fan {
                        // Slower than the fan is running: written as a slow-down.
                        self.output
                            .push_str(&Self::set_fan(i32::from(front.fan_speed)));
                        self.output.push('\n');
                        self.front_fan = i32::from(front.fan_speed);
                    } else {
                        self.output.push_str(&front.raw);
                        self.output.push('\n');
                        if front.fan_speed >= 0 {
                            self.front_fan = i32::from(front.fan_speed);
                        }
                    }
                }
            }
        }
        // GCodeReader.cpp `update_coordinates`: moves and G92 set where
        // the reader is.
        if matches!(command, "G0" | "G1" | "G2" | "G3" | "G92") {
            for axis in [X, Y, Z, E] {
                if let Some(value) = line.axes[axis] {
                    self.at[axis] = value;
                }
            }
        }
    }

    /// A fan command as it comes in (FanMover.cpp, the `'M'` case).
    fn fan(&mut self, line: &Line<'_>, speed: i16, time: &mut f64, need_flush: &mut bool) {
        let speed = i32::from(speed);
        // Any speed change stops a kick-start being undone.
        self.kick.time = -1.0;
        if self.in_custom {
            // Kept in its place: everything before it goes out.
            *need_flush = true;
            return;
        }
        let raw = line.raw;
        if self.back_fan < speed {
            if self.delay > 0.0 && (!self.only_overhangs || self.overhang) {
                // This command goes back in time.
                *time = -1.0;
                if self.kickstart > 0.0 && speed > self.front_fan {
                    self.remove_slow_fan(speed, (self.buffer_time + 1.0) as f32);
                    self.remove_slow_fan(255, self.kickstart);
                    self.write_back(&Self::set_fan(100));
                    // The speed asked for, after the kick-start.
                    let duration = self.kickstart * (speed - self.front_fan) as f32 / 100.0;
                    let mut count = duration;
                    let mut index = 0;
                    while index < self.buffer.len() && count > 0.0 {
                        count -= self.buffer[index].time;
                        if count < 0.0 {
                            let at = self.buffer[index].time + count;
                            self.put_in_middle(index, at, Held::new(raw, 0.0, speed as i16, true));
                            break;
                        }
                        index += 1;
                    }
                    if count > 0.0 {
                        // Past what is held: the lines still to come.
                        self.kick = Held::new(raw, count, speed as i16, false);
                    }
                    self.front_fan = speed;
                } else {
                    self.remove_slow_fan(speed, (self.buffer_time + 1.0) as f32);
                    self.write_back(raw);
                    self.front_fan = speed;
                }
            } else if self.kickstart <= 0.0 {
                // Left where it is.
            } else if self.kick.time > 0.0 {
                // Unreachable: the kick-start was stopped above. Kept as
                // the reference has it.
                if self.back_fan >= speed {
                    self.kick.time = -1.0;
                } else {
                    let duration = self.kickstart * (speed - self.back_fan) as f32 / 100.0;
                    self.kick.fan_speed = speed as i16;
                    self.kick.time += duration;
                    raw.clone_into(&mut self.kick.raw);
                    *time = -1.0;
                }
            } else if self.back_fan < speed - 10 {
                // Full speed first, then the speed asked for.
                *time = -1.0;
                let duration = self.kickstart * (speed - self.back_fan) as f32 / 100.0;
                self.put(Held::new(&Self::set_fan(100), 0.0, speed as i16, true));
                self.kick = Held::new(raw, duration, speed as i16, false);
            }
        }
        self.back_fan = speed;
    }

    /// A command moved back `delay` seconds from the newest line held: into
    /// the oldest line held, where that is far enough back, else before
    /// everything held (FanMover.cpp, `_print_in_middle_G1` and after).
    fn write_back(&mut self, command: &str) {
        let delay = f64::from(self.delay);
        match self.buffer.front() {
            Some(front) if self.buffer_time - f64::from(front.time) * 0.1 > delay => {
                let at = (self.buffer_time - delay) as f32;
                let Some(front) = self.buffer.pop_front() else {
                    return;
                };
                self.buffer_time -= f64::from(front.time);
                self.print_in_middle(&front, at, command);
            }
            _ => {
                self.output.push_str(command);
                self.output.push('\n');
            }
        }
    }

    /// FanMover.cpp `_remove_slow_fan`: the commands held for a lower speed
    /// in the oldest `past` seconds are dropped, so a rise is not undone.
    fn remove_slow_fan(&mut self, min_speed: i32, mut past: f32) {
        let mut index = 0;
        while index < self.buffer.len() && past > 0.0 {
            past -= self.buffer[index].time;
            let speed = i32::from(self.buffer[index].fan_speed);
            if speed >= 0 && speed < min_speed {
                self.remove(index);
            } else {
                index += 1;
            }
        }
    }

    /// FanMover.cpp `_put_in_middle_G1`: a line put `seconds` into a held
    /// line, splitting it where it is a `G1` and the moment is well inside.
    fn put_in_middle(&mut self, index: usize, seconds: f32, line: Held) {
        let item = &self.buffer[index];
        if seconds > item.time * 0.9 {
            self.buffer.insert(index + 1, line);
        } else if seconds < item.time * 0.1 || item.is_custom || !item.raw.starts_with("G1 ") {
            self.buffer.insert(index, line);
        } else {
            let item = &mut self.buffer[index];
            let percent = seconds / item.time;
            let mut before = item.clone();
            before.time *= percent;
            item.time *= 1.0 - percent;
            if item.dx != 0.0 {
                before.dx = item.dx * percent;
                item.x += before.dx;
                item.dx *= 1.0 - percent;
                change_axis_value(&mut before.raw, 'X', before.x + before.dx, 3);
            }
            if item.dy != 0.0 {
                before.dy = item.dy * percent;
                item.y += before.dy;
                item.dy *= 1.0 - percent;
                change_axis_value(&mut before.raw, 'Y', before.y + before.dy, 3);
            }
            if item.dz != 0.0 {
                before.dz = item.dz * percent;
                item.z += before.dz;
                item.dz *= 1.0 - percent;
                change_axis_value(&mut before.raw, 'Z', before.z + before.dz, 3);
            }
            if item.de != 0.0 {
                before.de = item.de * percent;
                change_axis_value(&mut before.raw, 'E', before.de, 5);
                item.de *= 1.0 - percent;
                let de = item.de;
                change_axis_value(&mut item.raw, 'E', de, 5);
            }
            self.buffer.insert(index, line);
            self.buffer.insert(index, before);
        }
    }

    /// FanMover.cpp `_print_in_middle_G1`: a line let out with a command
    /// written `seconds` into it.
    fn print_in_middle(&mut self, item: &Held, seconds: f32, command: &str) {
        let command = command.trim_end_matches('\n');
        if seconds < item.time * 0.1 {
            self.output.push_str(command);
            self.output.push('\n');
            self.output.push_str(&item.raw);
            self.output.push('\n');
        } else if seconds > item.time * 0.9 {
            self.output.push_str(&item.raw);
            self.output.push('\n');
            self.output.push_str(command);
            self.output.push('\n');
        } else if !item.is_custom && item.raw.starts_with("G1 ") {
            let percent = seconds / item.time;
            let mut before = item.raw.clone();
            let mut after = item.raw.clone();
            if item.dx != 0.0 {
                change_axis_value(&mut before, 'X', item.x + item.dx * percent, 3);
            }
            if item.dy != 0.0 {
                change_axis_value(&mut before, 'Y', item.y + item.dy * percent, 3);
            }
            if item.dz != 0.0 {
                change_axis_value(&mut before, 'Z', item.z + item.dz * percent, 3);
            }
            if item.de != 0.0 {
                change_axis_value(&mut before, 'E', item.de * percent, 5);
                change_axis_value(&mut after, 'E', item.de * (1.0 - percent), 5);
            }
            for text in [&before, command, &after] {
                self.output.push_str(text);
                self.output.push('\n');
            }
        } else {
            self.output.push_str(command);
            self.output.push('\n');
            self.output.push_str(&item.raw);
            self.output.push('\n');
        }
    }

    /// How long an arc is, from where the reader is: its turn about its
    /// centre (`I`, `J`), whole turns where it ends where it starts, and
    /// any rise.
    fn arc_length(&self, line: &Line<'_>, clockwise: bool) -> f64 {
        let (Some(i), Some(j)) = (line.axes[I], line.axes[J]) else {
            return 0.0;
        };
        let [x, y, z, _] = self.at.map(f64::from);
        let (i, j) = (f64::from(i), f64::from(j));
        let end_x = line.axes[X].map_or(x, f64::from);
        let end_y = line.axes[Y].map_or(y, f64::from);
        let end_z = line.axes[Z].map_or(z, f64::from);
        let (cx, cy) = (x + i, y + j);
        let radius = i.hypot(j);
        let start = (y - cy).atan2(x - cx);
        let end = (end_y - cy).atan2(end_x - cx);
        let mut turn = if clockwise { start - end } else { end - start };
        if turn <= 1e-9 {
            turn += std::f64::consts::TAU;
        }
        let along = radius * turn;
        along.hypot(end_z - z)
    }
}

/// FanMover.cpp `get_fan_speed`, for this engine's commands: the part
/// cooling fan's speed in percent an `M106` (without `P`, or `P1`) or
/// `M107` sets, or none.
fn fan_command(raw: &str) -> Option<i16> {
    let pwm = if raw.starts_with("M106") {
        if axis_value(raw, 'P').is_some_and(|fan| fan != 1.0) {
            return None;
        }
        // A command with no speed reads as 0, as the reference's cast of
        // "not a number" comes out on this machine.
        axis_value(raw, 'S').map_or(0, |speed| speed as i16)
    } else if raw.starts_with("M107") || raw.starts_with("M127") {
        0
    } else {
        return None;
    };
    // FanMover.cpp: `100 * fan_speed / fan_baseline`, kept whole.
    Some((f64::from(100 * i32::from(pwm)) / 255.0) as i16)
}

/// FanMover.cpp `get_axis_value`: the number after the first " X" in the
/// line, 0 where none can be read there.
fn axis_value(line: &str, axis: char) -> Option<f32> {
    let at = line.find(&format!(" {axis}"))? + 2;
    let rest = &line[at..];
    let end = rest
        .char_indices()
        .take_while(|(index, c)| {
            c.is_ascii_digit() || *c == '.' || ((*c == '-' || *c == '+') && *index == 0)
        })
        .last()
        .map_or(0, |(index, c)| index + c.len_utf8());
    Some(rest[..end].parse::<f64>().unwrap_or(0.0) as f32)
}

/// FanMover.cpp `change_axis_value`: an axis's number in a line replaced,
/// written with this many decimals.
fn change_axis_value(line: &mut String, axis: char, value: f32, decimals: usize) {
    let Some(found) = line.find(&format!(" {axis}")) else {
        return;
    };
    let at = found + 2;
    // Looked for from the second character on, as the reference does.
    let end = [
        line.get(at + 1..).and_then(|rest| rest.find(' ')),
        line.get(at + 1..).and_then(|rest| rest.find(';')),
    ]
    .into_iter()
    .flatten()
    .min()
    .map_or(line.len(), |offset| at + 1 + offset);
    line.replace_range(at..end, &format!("{value:.decimals$}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(timing: FanTiming, layers: &[&str]) -> Vec<String> {
        let mut mover = FanMover::new(timing);
        layers
            .iter()
            .map(|text| mover.process_layer(text))
            .collect()
    }

    #[test]
    fn a_rising_fan_is_told_early_by_splitting_a_move() {
        let timing = FanTiming::of(0.5, false, 0.0).unwrap();
        // 10 mm at 10 mm/s each: the fan is wanted at 2 s, told at 1.5 s.
        let layer = "G1 X0 Y0 F600\nG1 X10 E.5\nG1 X20 E.5\nM106 S255\nG1 X30 E.5\n";
        let out = moved(timing, &[layer]);
        assert_eq!(
            out[0],
            "G1 X0 Y0 F600\nG1 X10 E.5\nG1 X15.000 E0.25000\nM106 S255\nG1 X20 E0.25000\nG1 X30 E.5\n"
        );
        // A fan slowing down is left where it is.
        let down = "M106 S255\nG1 X10 F600 E.5\nG1 X20 E.5\nM106 S100\nG1 X30 E.5\n";
        assert_eq!(moved(timing, &[down])[0], down);
    }

    #[test]
    fn only_overhangs_moves_the_fan_for_an_overhang_wall() {
        let timing = FanTiming::of(0.5, true, 0.0).unwrap();
        let wall = "G1 X0 Y0 F600\nG1 X10 E.5\n;TYPE:Outer wall\nM106 S255\nG1 X20 E.5\n";
        assert_eq!(moved(timing, &[wall])[0], wall);
        let overhang = "G1 X0 Y0 F600\nG1 X10 E.5\n;TYPE:Overhang wall\nM106 S255\nG1 X20 E.5\n";
        assert_eq!(
            moved(timing, &[overhang])[0],
            "G1 X0 Y0 F600\nG1 X5.000 E0.25000\nM106 S255\nG1 X10 E0.25000\n;TYPE:Overhang wall\nG1 X20 E.5\n"
        );
    }

    #[test]
    fn printer_gcode_and_other_fans_stay_where_they_are() {
        let timing = FanTiming::of(0.5, false, 0.0).unwrap();
        let layer = "G1 X0 Y0 F600\nG1 X10 E.5\n; custom gcode start\nG1 X20 E.5\nM106 S255\n\
                     ; custom gcode end\nM106 P2 S178\nG1 X30 E.5\n";
        assert_eq!(moved(timing, &[layer])[0], layer);
    }

    #[test]
    fn a_repeated_fan_command_is_dropped() {
        let timing = FanTiming::of(0.5, false, 0.0).unwrap();
        let out = moved(
            timing,
            &[
                "M106 S255\nG1 X10 F600 E.5\n",
                "M106 S255\nG1 X20 E.5\nG1 X30 E.5\n",
            ],
        );
        assert_eq!(out[1], "G1 X20 E.5\nG1 X30 E.5\n");
    }

    #[test]
    fn a_kick_start_runs_the_fan_flat_out_first() {
        // No speed-up: full speed where the fan comes on, then the speed
        // asked for after the kick-start's share of its time.
        let timing = FanTiming::of(0.0, true, 1.0).unwrap();
        let out = moved(
            timing,
            &["G1 X0 Y0 F600\nM106 S127\nG1 X10 E.5\nG1 X20 E.5\n"],
        );
        // 49 % of 1 s: 4.9 mm into the first move.
        assert_eq!(
            out[0],
            "G1 X0 Y0 F600\nM106 S255\nG1 X4.900 E0.24500\nM106 S127\nG1 X10 E0.25500\nG1 X20 E.5\n"
        );
    }

    #[test]
    fn an_arc_takes_its_time() {
        // A half circle of radius 10 at 10 mm/s takes 3.14 s: a fan wanted
        // at its end is told where it starts (an arc is not split), not in
        // the move before it, as it would be were the arc to take no time.
        let timing = FanTiming::of(0.5, false, 0.0).unwrap();
        let layer = "G1 X0 Y0 F600\nG1 X10 E.5\nG2 X30 Y0 I10 J0 E1\nM106 S255\n";
        assert_eq!(
            moved(timing, &[layer])[0],
            "G1 X0 Y0 F600\nG1 X10 E.5\nM106 S255\nG2 X30 Y0 I10 J0 E1\n"
        );
        assert!(
            (FanMover::new(timing).arc_length(&Line::parse("G2 X20 Y0 I10 J0"), true) - 31.4159)
                .abs()
                < 1e-3
        );
    }
}
