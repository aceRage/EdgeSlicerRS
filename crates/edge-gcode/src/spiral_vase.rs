//! Spiral vase mode (`spiral_mode`): a layer that is one closed loop is
//! laid as a ramp, its Z rising along the loop's length from the layer
//! below to its own, so the wall goes up in one continuous spiral with no
//! seam.
//!
//! Ported from the reference's post-processor (GCode/SpiralVase.cpp
//! `SpiralVase::process_layer`): it works on each layer's finished G-code,
//! as the reference's does, reading the moves back with the reference's
//! G-code reader (GCodeReader.cpp), in its single precision, so that every
//! Z it writes is the reference's to the last digit. Every layer goes
//! through it, those it leaves alone too, so that it knows where the
//! nozzle is.
//!
//! On a layer it turns into a spiral:
//! - the layer's first Z move goes to the layer below's height instead,
//!   and every extruding move gets a Z of its own, the layer below's plus
//!   the share of the layer's height that the length laid so far is of
//!   the loop's whole length;
//! - travel, retraction and the moves too short to matter (shorter than
//!   twice `resolution`, the reference's "Orca #13517") are left out;
//! - the first spiral layer (the transition from the solid bottom) ramps
//!   the flow up from `spiral_starting_flow_ratio` to all of it along the
//!   loop, and the last layer is laid again after itself, flat, its flow
//!   ramped down from all of it to `spiral_finishing_flow_ratio`;
//! - with `spiral_mode_smooth` each point is pulled towards the nearest
//!   point of the layer below's loop, by less the further along the loop
//!   it is, where that is within `spiral_mode_max_xy_smoothing`, so the
//!   spiral has no step in XY either; the flow is scaled to the move's new
//!   length.
//!
//! The reference tapers the flow only with relative extrusion distances;
//! this writer always writes them, so it always tapers.

/// What a spiral is laid by: the `spiral_*` settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spiral {
    /// `spiral_mode_smooth`.
    pub smooth: bool,
    /// `spiral_mode_max_xy_smoothing`, in mm (a percentage is of the
    /// nozzle's diameter).
    pub max_xy_smoothing: f64,
    /// `spiral_starting_flow_ratio`, from 0 to 1.
    pub starting_flow_ratio: f64,
    /// `spiral_finishing_flow_ratio`, from 0 to 1.
    pub finishing_flow_ratio: f64,
}

/// The reference's axes, in its order (libslic3r.h `Axis`).
const X: usize = 0;
const Y: usize = 1;
const Z: usize = 2;
const E: usize = 3;
const AXES: usize = 8;
/// An axis letter the reader does not know, which it notes all the same.
const UNKNOWN: usize = AXES;

/// One line as the reference's reader holds it (`GCodeReader::GCodeLine`):
/// its text without the line end, and the value of each axis it gives.
#[derive(Clone, Debug, Default)]
struct Line {
    raw: String,
    axis: [f32; AXES],
    mask: u32,
}

fn is_whitespace(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn is_end_of_gcode_line(c: u8) -> bool {
    matches!(c, b';' | b'\r' | b'\n' | 0)
}

fn is_end_of_word(c: u8) -> bool {
    is_whitespace(c) || is_end_of_gcode_line(c)
}

impl Line {
    /// GCodeReader.cpp `parse_line_internal`, for one line's text.
    fn parse(raw: &str) -> Line {
        let bytes = raw.as_bytes();
        let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let mut line = Line {
            raw: raw.to_owned(),
            ..Line::default()
        };
        let mut c = 0;
        while is_whitespace(at(c)) {
            c += 1;
        }
        while !is_end_of_word(at(c)) {
            c += 1;
        }
        while !is_end_of_gcode_line(at(c)) {
            while is_whitespace(at(c)) {
                c += 1;
            }
            if is_end_of_gcode_line(at(c)) {
                break;
            }
            let axis = match at(c) {
                b'X' => Some(X),
                b'Y' => Some(Y),
                b'Z' => Some(Z),
                b'F' => Some(4),
                b'I' => Some(5),
                b'J' => Some(6),
                b'E' => Some(E),
                b'P' => Some(7),
                letter if letter.is_ascii_uppercase() => Some(UNKNOWN),
                _ => None,
            };
            match axis {
                Some(axis) => {
                    c += 1;
                    let start = c;
                    while !is_end_of_word(at(c)) {
                        c += 1;
                    }
                    if let Some(value) = raw
                        .get(start..c)
                        .filter(|word| !word.is_empty() && !word.starts_with('+'))
                        .and_then(|word| word.parse::<f64>().ok())
                    {
                        if axis != UNKNOWN {
                            line.axis[axis] = value as f32;
                        }
                        line.mask |= 1 << axis;
                    }
                }
                None => {
                    while !is_end_of_word(at(c)) {
                        c += 1;
                    }
                }
            }
        }
        line
    }

    fn has(&self, axis: usize) -> bool {
        self.mask & (1 << axis) != 0
    }

    fn has_xy(&self) -> bool {
        self.has(X) || self.has(Y)
    }

    /// The command word.
    fn cmd(&self) -> &str {
        let text = self.raw.trim_start_matches([' ', '\t']);
        let end = text.bytes().position(is_end_of_word).unwrap_or(text.len());
        &text[..end]
    }

    fn is_g1(&self) -> bool {
        self.cmd() == "G1"
    }

    fn dist(&self, axis: usize, reader: &Reader) -> f32 {
        if self.has(axis) {
            self.axis[axis] - reader.position[axis]
        } else {
            0.0
        }
    }

    fn dist_xy(&self, reader: &Reader) -> f32 {
        let (x, y) = (self.dist(X, reader), self.dist(Y, reader));
        (x * x + y * y).sqrt()
    }

    fn extruding(&self, reader: &Reader) -> bool {
        matches!(self.cmd(), "G1" | "G2" | "G3") && self.dist(E, reader) > 0.0
    }

    fn retracting(&self, reader: &Reader) -> bool {
        matches!(self.cmd(), "G1" | "G2" | "G3") && self.dist(E, reader) < 0.0
    }

    /// GCodeReader.cpp `GCodeLine::set`: the axis's value written with
    /// `digits` decimals, in place of the one given or after the command.
    fn set(&mut self, axis: usize, value: f32, digits: usize) {
        let text = format!("{:.*}", digits, f64::from(value));
        let letter = match axis {
            X => 'X',
            Y => 'Y',
            Z => 'Z',
            E => 'E',
            4 => 'F',
            5 => 'I',
            6 => 'J',
            _ => 'P',
        };
        let word = format!(" {letter}");
        if self.has(axis) {
            if let Some(found) = self.raw.find(&word) {
                let from = found + 2;
                let to = self.raw[(from + 1).min(self.raw.len())..]
                    .find(' ')
                    .map_or(self.raw.len(), |at| at + from + 1);
                self.raw.replace_range(from..to, &text);
            }
        } else {
            match self.raw.find(' ') {
                Some(at) => self.raw.insert_str(at, &format!("{word}{text}")),
                None => self.raw.push_str(&format!("{word}{text}")),
            }
        }
        self.axis[axis] = value;
        self.mask |= 1 << axis;
    }
}

/// Where the reference's reader has the nozzle (GCodeReader.cpp
/// `update_coordinates`), with relative extrusion distances.
#[derive(Clone, Debug, Default)]
struct Reader {
    position: [f32; AXES],
}

impl Reader {
    /// Reads `line` before the caller looks at it: with relative
    /// distances a line that extrudes starts from no extrusion.
    fn before(&mut self, line: &Line) {
        if line.has(E) {
            self.position[E] = 0.0;
        }
    }

    /// And moves on to where it leaves the nozzle.
    fn after(&mut self, line: &Line) {
        let cmd = line.cmd();
        if matches!(cmd, "G0" | "G1" | "G2" | "G3" | "G92") {
            for axis in 0..AXES {
                if line.has(axis) {
                    self.position[axis] = line.axis[axis];
                }
            }
        }
    }

    /// Every line of `text` in turn, read before and after `each`.
    fn read(&mut self, text: &str, mut each: impl FnMut(&Reader, &Line)) {
        for raw in lines(text) {
            let line = Line::parse(raw);
            self.before(&line);
            each(self, &line);
            self.after(&line);
        }
    }
}

/// The lines of `text` as the reference's reader splits them: at each line
/// end, `\r\n` as one, without it.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let line = &rest[..end];
        let mut next = end;
        if rest.as_bytes().get(next) == Some(&b'\r') {
            next += 1;
        }
        if rest.as_bytes().get(next) == Some(&b'\n') {
            next += 1;
        }
        rest = &rest[next..];
        Some(line)
    })
}

/// The reference's `SpiralVase`, fed every layer in turn.
#[derive(Clone, Debug)]
pub(crate) struct SpiralVase {
    spiral: Spiral,
    /// Twice `resolution`, at least `EPSILON`: moves shorter are left out.
    min_segment_length: f32,
    reader: Reader,
    enabled: bool,
    /// The first spiral layer, whose flow is ramped up.
    transition_layer: bool,
    /// The points of the last spiral layer laid, for smoothing.
    previous_layer: Option<Vec<[f32; 2]>>,
}

impl SpiralVase {
    pub(crate) fn new(spiral: Spiral, resolution: f64) -> Self {
        Self {
            spiral,
            min_segment_length: (1e-4f32).max(2.0 * resolution as f32),
            reader: Reader::default(),
            enabled: false,
            transition_layer: false,
            previous_layer: None,
        }
    }

    /// SpiralVase.hpp `enable`: whether the next layer is a spiral.
    pub(crate) fn enable(&mut self, on: bool) {
        self.transition_layer = on && !self.enabled;
        self.enabled = on;
    }

    /// SpiralVase.cpp `process_layer`: the layer's G-code, as a spiral when
    /// it is enabled, and how much more filament (mm) it then lays than
    /// before (the flow ramps and smoothing change it).
    pub(crate) fn process_layer(&mut self, gcode: &str, last_layer: bool) -> (String, f64) {
        if !self.enabled {
            self.reader.read(gcode, |_, _| {});
            return (gcode.to_owned(), 0.0);
        }
        // The loop's length, and how far the layer rises.
        let mut total_layer_length = 0f32;
        let mut layer_height = 0f32;
        let mut z = 0f32;
        {
            let mut set_z = false;
            let mut clone = self.reader.clone();
            clone.read(gcode, |reader, line| {
                if line.is_g1() {
                    if line.extruding(reader) {
                        total_layer_length += line.dist_xy(reader);
                    } else if line.has(Z) {
                        layer_height += line.dist(Z, reader);
                        if !set_z {
                            z = line.axis[Z];
                            set_z = true;
                        }
                    }
                }
            });
        }
        // From the layer below.
        z -= layer_height;

        let smooth = self.spiral.smooth;
        let max_xy_dist_for_smoothing = self.spiral.max_xy_smoothing as f32;
        let transition_in = self.transition_layer;
        let transition_out = last_layer;
        let starting_flowrate = self.spiral.starting_flow_ratio as f32;
        let finishing_flowrate = self.spiral.finishing_flow_ratio as f32;
        let min_segment_length = self.min_segment_length;
        let previous_layer = self.previous_layer.take();
        let mut current_layer: Vec<[f32; 2]> = Vec::new();
        let mut new_gcode = String::with_capacity(gcode.len() + gcode.len() / 4);
        let mut transition_gcode = String::new();
        let mut len = 0f32;
        let mut last_point = previous_layer
            .as_ref()
            .and_then(|layer| layer.last().copied())
            .unwrap_or([0.0, 0.0]);
        // Filament laid by moves across, before and after.
        let (mut fed_before, mut fed_after) = (0f64, 0f64);
        let fed = |line: &Line, out: &mut f64| {
            if line.is_g1() && line.has_xy() && line.has(E) && line.axis[E] > 0.0 {
                *out += f64::from(line.axis[E]);
            }
        };
        let mut lines_in = Vec::new();
        self.reader.read(gcode, |reader, line| {
            lines_in.push(line.clone());
            let mut line = line.clone();
            let push = |gcode: &mut String, line: &Line| {
                gcode.push_str(&line.raw);
                gcode.push('\n');
            };
            if line.is_g1() {
                // "Orca: Filter out retractions at layer change", and moves
                // too short to lay anything.
                if line.retracting(reader)
                    || (line.extruding(reader) && line.dist_xy(reader) < min_segment_length)
                {
                    // The feedrate it sets is kept. The reference writes a
                    // move's feedrate on a line of its own before it
                    // (GCodeWriter.cpp `set_speed`), which this filter
                    // leaves; this writer puts it on the move, and without
                    // this the moves after went at the layer change's speed.
                    let feed = feed_only(&line);
                    if line.extruding(reader) && line.has_xy() && !feed.raw.is_empty() {
                        push(&mut new_gcode, &feed);
                    }
                    return;
                }
                if line.has(Z) && !line.has_xy() {
                    // The layer's first Z move goes to the layer below's
                    // height, where the ramp starts.
                    line.set(Z, z, 3);
                    push(&mut new_gcode, &line);
                    return;
                }
                let dist_xy = line.dist_xy(reader);
                if line.has_xy() {
                    if dist_xy > 0.0 && line.extruding(reader) {
                        len += dist_xy;
                        let factor = len / total_layer_length;
                        if transition_in {
                            let starting_e_factor =
                                starting_flowrate + factor * (1.0 - starting_flowrate);
                            let e = line.axis[E] * starting_e_factor;
                            line.set(E, e, 5);
                        } else if transition_out {
                            // The last layer again, flat, its flow ramped
                            // down, after the spiral.
                            let mut flat = line.clone();
                            let finishing_e_factor =
                                finishing_flowrate + (1.0 - factor) * (1.0 - finishing_flowrate);
                            flat.set(E, line.axis[E] * finishing_e_factor, 5);
                            push(&mut transition_gcode, &flat);
                        }
                        // "This line is the core of Spiral Vase mode, ramp
                        // up the Z smoothly."
                        line.set(Z, z + factor * layer_height, 3);
                        if smooth {
                            let p = [line.axis[X], line.axis[Y]];
                            current_layer.push(p);
                            if let Some(previous) = &previous_layer {
                                match nearest_point_on_lines(p, previous) {
                                    Some((nearest, dist)) if dist < max_xy_dist_for_smoothing => {
                                        let target = [
                                            nearest[0] * (1.0 - factor) + p[0] * factor,
                                            nearest[1] * (1.0 - factor) + p[1] * factor,
                                        ];
                                        let modified_dist_xy = distance(last_point, target);
                                        if modified_dist_xy < min_segment_length {
                                            // Its feedrate is kept, as above.
                                            line = feed_only(&line);
                                        } else {
                                            line.set(X, target[0], 3);
                                            line.set(Y, target[1], 3);
                                            let e = line.axis[E] * modified_dist_xy / dist_xy;
                                            line.set(E, e, 5);
                                            last_point = target;
                                        }
                                    }
                                    _ => last_point = p,
                                }
                            }
                        }
                        push(&mut new_gcode, &line);
                    }
                    // Travel is left out: the loop starts where the one
                    // below ended.
                    return;
                }
            }
            push(&mut new_gcode, &line);
            if transition_out {
                push(&mut transition_gcode, &line);
            }
        });
        self.previous_layer = Some(current_layer);
        for line in &lines_in {
            fed(line, &mut fed_before);
        }
        new_gcode.push_str(&transition_gcode);
        for raw in lines(&new_gcode) {
            fed(&Line::parse(raw), &mut fed_after);
        }
        (new_gcode, fed_after - fed_before)
    }
}

/// What of a move left out is kept: its feedrate, as `G1 F…` on a line
/// of its own, or an empty line when it sets none.
fn feed_only(line: &Line) -> Line {
    let raw = (line.raw.split_whitespace())
        .find(|word| word.starts_with('F'))
        .map_or_else(String::new, |word| format!("G1 {word}"));
    Line::parse(&raw)
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// SpiralVase.cpp `nearest_point_on_lines`: the point of the polyline
/// through `points` nearest `p`, and how far it is; none for fewer than
/// two points.
fn nearest_point_on_lines(p: [f32; 2], points: &[[f32; 2]]) -> Option<([f32; 2], f32)> {
    if points.len() < 2 {
        return None;
    }
    let mut min = f32::MAX;
    let mut closest = None;
    for pair in points.windows(2) {
        let (point, dist) = nearest_point_on_line(p, pair[0], pair[1]);
        if dist < min {
            min = dist;
            closest = Some(point);
        }
    }
    closest.map(|point| (point, min))
}

/// SpiralVase.cpp `nearest_point_on_line`.
fn nearest_point_on_line(c: [f32; 2], a: [f32; 2], b: [f32; 2]) -> ([f32; 2], f32) {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ca = [c[0] - a[0], c[1] - a[1]];
    let t = (ca[0] * ab[0] + ca[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1]);
    let t = if t > 1.0 { 1.0 } else { t };
    let t = if t < 0.0 { 0.0 } else { t };
    let closest = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    (closest, distance(c, closest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Spiral {
        Spiral {
            smooth: false,
            max_xy_smoothing: 0.8,
            starting_flow_ratio: 0.0,
            finishing_flow_ratio: 0.0,
        }
    }

    /// A square loop 10 mm a side at `z`, as the writer lays a layer:
    /// the Z move, a travel to the start, a retraction undone, four sides.
    fn square(z: f64) -> String {
        format!(
            ";LAYER_CHANGE\nG1 Z{z:.3} F1200\nG1 X10 Y10 F30000\nG1 E.8 F1800\n;TYPE:Outer wall\n\
             G1 F3000\nG1 X20 Y10 E.4\nG1 X20 Y20 E.4\nG1 X10 Y20 E.4\nG1 X10 Y10 E.4\n"
        )
    }

    #[test]
    fn a_layer_left_alone_is_read_and_kept_as_it_is() {
        let mut vase = SpiralVase::new(plain(), 0.012);
        vase.enable(false);
        let text = square(0.2);
        assert_eq!(vase.process_layer(&text, false), (text.clone(), 0.0));
        assert_eq!(vase.reader.position[Z], 0.2);
        assert_eq!(vase.reader.position[X], 10.0);
    }

    #[test]
    fn a_move_left_out_keeps_the_feedrate_it_sets() {
        let mut vase = SpiralVase::new(plain(), 0.012);
        vase.process_layer(&square(0.2), false);
        vase.enable(true);
        vase.process_layer(&square(0.4), false);
        vase.enable(true);
        // The layer's first move, too short to lay anything, sets the
        // speed of the moves after it.
        let layer = ";LAYER_CHANGE\nG1 Z0.600 F1200\n;TYPE:Outer wall\n\
                     G1 X10.01 Y10 E.001 F3000\nG1 X20 Y10 E.4\nG1 X20 Y20 E.4\n\
                     G1 X10 Y20 E.4\nG1 X10 Y10 E.4\n";
        let (text, _) = vase.process_layer(layer, false);
        assert!(text.contains("\nG1 F3000\nG1 Z"), "{text}");
    }

    #[test]
    fn a_spiral_layer_rises_along_its_length_from_the_layer_below() {
        let mut vase = SpiralVase::new(plain(), 0.012);
        vase.enable(false);
        vase.process_layer(&square(0.2), false);
        vase.enable(true);
        vase.process_layer(&square(0.4), false);
        // The second spiral layer: no transition.
        vase.enable(true);
        let (text, more) = vase.process_layer(&square(0.6), false);
        assert_eq!(
            text,
            ";LAYER_CHANGE\nG1 Z0.400 F1200\n;TYPE:Outer wall\nG1 F3000\n\
             G1 Z0.450 X20 Y10 E.4\nG1 Z0.500 X20 Y20 E.4\nG1 Z0.550 X10 Y20 E.4\n\
             G1 Z0.600 X10 Y10 E.4\n"
        );
        assert_eq!(more, 0.0);
    }

    #[test]
    fn the_first_spiral_layer_ramps_its_flow_up_and_the_last_is_laid_again_flat() {
        let spiral = Spiral {
            starting_flow_ratio: 0.5,
            finishing_flow_ratio: 0.2,
            ..plain()
        };
        let mut vase = SpiralVase::new(spiral, 0.012);
        vase.process_layer(&square(0.2), false);
        vase.enable(true);
        let (text, more) = vase.process_layer(&square(0.4), false);
        // From half the flow to all of it: 0.625, 0.75, 0.875, 1 of 0.4.
        assert!(text.contains("G1 Z0.250 X20 Y10 E0.25000\n"), "{text}");
        assert!(text.contains("G1 Z0.400 X10 Y10 E0.40000\n"), "{text}");
        assert!((more - (0.25 + 0.3 + 0.35 + 0.4 - 1.6)).abs() < 1e-6);
        vase.enable(true);
        let (text, more) = vase.process_layer(&square(0.6), true);
        let (spiral, flat) = text.split_at(text.rfind(";LAYER_CHANGE").unwrap());
        assert!(spiral.contains("G1 Z0.600 X10 Y10 E.4\n"));
        // From all of it down to 0.2: 0.8, 0.6, 0.4 and 0.2 of the flow at
        // the end of each quarter, flat at the layer's height.
        assert_eq!(
            flat,
            ";LAYER_CHANGE\n;TYPE:Outer wall\nG1 F3000\nG1 X20 Y10 E0.32000\nG1 X20 Y20 E0.24000\n\
             G1 X10 Y20 E0.16000\nG1 X10 Y10 E0.08000\n"
        );
        assert!((more - (0.32 + 0.24 + 0.16 + 0.08)).abs() < 1e-6);
    }

    #[test]
    fn a_smooth_spiral_is_pulled_towards_the_loop_below() {
        let spiral = Spiral {
            smooth: true,
            ..plain()
        };
        let mut vase = SpiralVase::new(spiral, 0.012);
        vase.enable(true);
        vase.process_layer(&square(0.2), false);
        vase.enable(true);
        // The same square a little to the right: each corner moves from
        // the loop below by the share of the loop laid.
        let shifted = square(0.4)
            .replace("X20", "X20.4")
            .replace("X10 Y20", "X10.4 Y20");
        let (text, _) = vase.process_layer(&shifted, false);
        // The second corner, half the loop's length round (10.4 + 10 of
        // 40.408 mm), lies about half way between.
        assert!(text.contains("G1 Z0.301 X20.202 Y20.000"), "{text}");
        // The first, a quarter round, a quarter of the way from below.
        assert!(text.contains("G1 Z0.251 X20.103 Y10.000"), "{text}");
        // The last is where it is asked to be.
        assert!(text.contains("G1 Z0.400 X10.000 Y10.000"), "{text}");
    }

    #[test]
    fn the_reader_takes_lines_as_the_reference_does() {
        let line = Line::parse("G1 X1.5 Y-.25 E.0123 F3000 ; comment Z9");
        assert!(line.is_g1() && line.has(X) && line.has(Y) && !line.has(Z));
        assert_eq!(line.axis[Y], -0.25);
        let mut moved = line.clone();
        moved.set(Z, 0.4567, 3);
        moved.set(E, 0.5, 5);
        assert_eq!(
            moved.raw,
            "G1 Z0.457 X1.5 Y-.25 E0.50000 F3000 ; comment Z9"
        );
        assert_eq!(Line::parse("G92 E0").cmd(), "G92");
        assert!(!Line::parse("G10").is_g1());
    }
}

/// The reference's own spiral layers, laid again here: each ramped layer
/// of a file the reference wrote (`EDGE_VASE_REFERENCE`, plain spiral, not
/// smooth) has its Z words taken off and its first Z move put back at the
/// layer's height, which is what its post-processor was given, and must
/// come out as the reference wrote it, move for move.
#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    #[ignore = "needs a file the reference wrote, in EDGE_VASE_REFERENCE"]
    fn the_references_ramps_are_laid_again_move_for_move() {
        let Ok(path) = std::env::var("EDGE_VASE_REFERENCE") else {
            return;
        };
        let text = std::fs::read_to_string(path).unwrap();
        let body = &text[..text.find("; CONFIG_BLOCK_START").unwrap_or(text.len())];
        let mut layers: Vec<&str> = body.split(";LAYER_CHANGE\n").skip(1).collect();
        // The last layer's flat copy comes after it.
        layers.pop();
        let mut vase = SpiralVase::new(
            Spiral {
                smooth: false,
                max_xy_smoothing: 0.8,
                starting_flow_ratio: 0.0,
                finishing_flow_ratio: 0.0,
            },
            0.012,
        );
        let (mut checked, mut moves, mut exact, mut worst) = (0, 0, 0, 0f32);
        for layer in layers {
            let height: f64 = layer
                .lines()
                .find_map(|line| line.strip_prefix(";Z:"))
                .unwrap()
                .parse()
                .unwrap();
            let ramped = layer
                .lines()
                .filter(|line| line.starts_with("G1 Z") && line.contains(" X"))
                .count()
                > 2;
            // A transition layer's flow was ramped too: read, not checked.
            let transition = layer.contains(" E0.0");
            let mut given = String::new();
            let mut first = true;
            for line in layer.lines() {
                if ramped && line.starts_with("G1 Z") && !line.contains(" X") && first {
                    given.push_str(&format!(
                        "G1 Z{height}{}\n",
                        &line[line.find(' ').map_or(0, |at| {
                            line[at + 1..]
                                .find(' ')
                                .map_or(line.len(), |end| at + 1 + end)
                        })..]
                    ));
                    first = false;
                } else if ramped && line.starts_with("G1 Z") {
                    let rest = &line[3..];
                    let after = rest.find(' ').map_or("", |at| &rest[at..]);
                    given.push_str(&format!("G1{after}\n"));
                } else {
                    given.push_str(line);
                    given.push('\n');
                }
            }
            let wanted = layer.to_string();
            vase.enable(ramped);
            let (laid, _) = vase.process_layer(&given, false);
            if ramped && !transition {
                let wanted: Vec<&str> = wanted.lines().filter(|l| l.starts_with("G1 Z")).collect();
                let laid: Vec<&str> = laid.lines().filter(|l| l.starts_with("G1 Z")).collect();
                assert_eq!(laid.len(), wanted.len(), "layer at {height}");
                for (here, there) in laid.iter().zip(&wanted) {
                    let z = |line: &str| Line::parse(line).axis[Z];
                    let rest = |line: &str| {
                        line[line.find(' ').unwrap() + 1..]
                            .split_once(' ')
                            .map(|(_, r)| r.to_owned())
                    };
                    // Everything but Z as it was written.
                    assert_eq!(rest(here), rest(there), "layer at {height}");
                    let dz = (z(here) - z(there)).abs();
                    worst = worst.max(dz);
                    exact += usize::from(dz < 1e-6);
                }
                checked += 1;
                moves += laid.len();
            }
        }
        // A move too short to lay (from the loop's start to the corner
        // beside it) is left out of the file, but the reference counted it
        // in the loop's length: given the file, the length here is short
        // by it, and a Z may come out one in the last digit lower.
        eprintln!(
            "{checked} spiral layers, {moves} moves: {exact} with the reference's Z to the digit, \
             the most apart {worst:.4} mm"
        );
        assert!(checked > 0 && worst < 0.0011);
    }
}
