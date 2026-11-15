//! The pressure advance tests that draw their own moves: "PA Line" and
//! "PA Pattern" (calib.cpp `CalibPressureAdvance`,
//! `CalibPressureAdvanceLine`, `CalibPressureAdvancePattern`), and the
//! part of the reference's G-code writer they draw with (GCodeWriter.cpp:
//! `travel_to_xy`, `travel_to_z`, `extrude_to_xy`, `set_speed`, `retract`,
//! `unretract`, `reset_e`, `set_print_acceleration`, `set_jerk_xy`).
//!
//! The numbers are written as this crate's writer writes them (X, Y and Z
//! to the thousandth, E to five places, F whole), the reference's own
//! formatter trimming their zeros: the values are the reference's.
//! Extrusion is relative, the only way this crate writes it.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::calib::{AdvanceFirmware, general, pressure_advance_line};

/// What the drawing writer is told of the printer and the filament.
#[derive(Clone, Debug, PartialEq)]
pub struct Pen {
    pub firmware: AdvanceFirmware,
    pub bambu: bool,
    pub klipper: bool,
    /// Klipper's deceleration share, as the writer has it.
    pub accel_to_decel_factor: Option<f64>,
    /// `gcode_comments`: the writer's comments on its moves.
    pub comments: bool,
    pub retraction_length: f64,
    pub retraction_speed: f64,
    pub deretraction_speed: f64,
    pub travel_speed: f64,
    pub z_speed: f64,
    /// `machine_max_jerk_x`, `_y`, `_z`, `_e`: the jerk set is held to x
    /// and y; a Bambu Lab printer is told z and e beside it.
    pub max_jerk: [f64; 4],
    /// Where the tool's nozzle is off the first (`extruder_offset`).
    pub offset: [f64; 2],
}

impl Pen {
    /// The pen of a filament's settings, for a calibration.
    pub fn of(s: &crate::PrintSettings, calibration: &crate::Calibration) -> Self {
        Self {
            firmware: calibration.firmware,
            bambu: s.bambu,
            klipper: s.klipper,
            accel_to_decel_factor: s.accel_to_decel_factor,
            comments: calibration.comments,
            retraction_length: s.retraction_length,
            retraction_speed: s.retraction_speed,
            deretraction_speed: s.deretraction_speed,
            travel_speed: s.travel_speed,
            z_speed: s.z_speed,
            max_jerk: calibration.max_jerk,
            offset: s.extruder_offset,
        }
    }
}

/// The writer's state while it draws (GCodeWriter.cpp, Extruder.cpp).
pub(crate) struct Drawing<'a> {
    pen: &'a Pen,
    pub out: String,
    /// `CalibPressureAdvance::m_last_pos`.
    last: [f64; 2],
    /// The filament pulled back and not yet pushed out again.
    pub retracted: f64,
    /// Filament fed in all, in mm, retraction left out.
    pub fed: f64,
    pub acceleration: Option<f64>,
    jerk: Option<f64>,
}

/// How a number is drawn (calib.hpp `DrawDigitMode`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigitMode {
    LeftToRight,
    BottomToTop,
}

/// calib.hpp's constants.
const DIGIT_SEGMENT: f64 = 2.0;
const DIGIT_GAP: f64 = 1.0;
const MAX_NUMBER_LEN: usize = 5;
const ENCROACHMENT: f64 = 1.0 / 3.0;

/// `DrawBoxOptArgs`.
#[derive(Clone, Copy)]
pub(crate) struct BoxArgs {
    pub filled: bool,
    pub perimeters: i32,
    pub height: f64,
    pub line_width: f64,
    /// A feedrate, mm/min.
    pub speed: f64,
    /// 0: worked out from the line (`draw_line`).
    pub e_per_mm: f64,
}

fn radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

/// `CalibPressureAdvance::convert_number_to_string`: to `precision`
/// significant digits less one for the point below 1000 (none: six).
pub(crate) fn number_text(value: f64, precision: usize) -> String {
    if precision == 0 {
        return general(value, 6);
    }
    general(
        value,
        if value >= 1000.0 {
            precision
        } else {
            precision - 1
        },
    )
}

/// `number_spacing`.
pub(crate) fn number_spacing() -> f64 {
    DIGIT_SEGMENT + DIGIT_GAP
}

impl<'a> Drawing<'a> {
    pub(crate) fn new(pen: &'a Pen, acceleration: Option<f64>) -> Self {
        Self {
            pen,
            out: String::new(),
            last: [0.0, 0.0],
            retracted: 0.0,
            fed: 0.0,
            acceleration,
            jerk: None,
        }
    }

    fn comment(&self, words: &str) -> String {
        if self.pen.comments && !words.is_empty() {
            format!(" ; {words}")
        } else {
            String::new()
        }
    }

    fn x(&self, x: f64) -> f64 {
        x - self.pen.offset[0]
    }

    fn y(&self, y: f64) -> f64 {
        y - self.pen.offset[1]
    }

    /// A feature tag (`reserved_tag(Role)`).
    pub(crate) fn role(&mut self, role: &str) {
        let tag = if self.pen.bambu {
            "; FEATURE: "
        } else {
            ";TYPE:"
        };
        let _ = writeln!(self.out, "{tag}{role}");
    }

    pub(crate) fn height_tag(&mut self, height: f64) {
        let tag = if self.pen.bambu {
            "; LAYER_HEIGHT: "
        } else {
            ";HEIGHT:"
        };
        let _ = writeln!(self.out, "{tag}{}", general(height, 6));
    }

    pub(crate) fn layer_change_tag(&mut self) {
        let tag = if self.pen.bambu {
            "; CHANGE_LAYER"
        } else {
            ";LAYER_CHANGE"
        };
        let _ = writeln!(self.out, "{tag}");
    }

    /// `GCodeWriter::retract`: the retraction length less what is pulled
    /// back already.
    pub(crate) fn retract(&mut self) {
        let length = self.pen.retraction_length;
        if self.retracted < length {
            let pull = length - self.retracted;
            self.retracted = length;
            let comment = self.comment("retract");
            let _ = writeln!(
                self.out,
                "G1 E-{pull:.5} F{:.0}{comment}",
                (self.pen.retraction_speed * 60.0).round()
            );
        }
    }

    /// `GCodeWriter::unretract`.
    pub(crate) fn unretract(&mut self) {
        if self.retracted > 0.0 {
            let push = self.retracted;
            self.retracted = 0.0;
            let speed = if self.pen.deretraction_speed > 0.0 {
                self.pen.deretraction_speed
            } else {
                self.pen.retraction_speed
            };
            let comment = self.comment("unretract");
            let _ = writeln!(
                self.out,
                "G1 E{push:.5} F{:.0}{comment}",
                (speed * 60.0).round()
            );
        }
    }

    /// `GCodeWriter::reset_e`: nothing with relative extrusion.
    pub(crate) fn reset_e(&mut self) {}

    pub(crate) fn travel_to_xy(&mut self, to: [f64; 2], comment: &str) {
        let comment = self.comment(comment);
        let _ = writeln!(
            self.out,
            "G1 X{:.3} Y{:.3} F{:.0}{comment}",
            self.x(to[0]),
            self.y(to[1]),
            (self.pen.travel_speed * 60.0).round()
        );
    }

    pub(crate) fn travel_to_z(&mut self, z: f64, comment: &str) {
        let comment = self.comment(comment);
        let _ = writeln!(
            self.out,
            "G1 Z{z:.3} F{:.0}{comment}",
            (self.pen.z_speed * 60.0).round()
        );
    }

    /// `GCodeWriter::extrude_to_xy`: no E for an amount next to nothing.
    pub(crate) fn extrude_to_xy(&mut self, to: [f64; 2], fed: f64, comment: &str) {
        let comment = self.comment(comment);
        if fed.abs() <= f64::EPSILON {
            let _ = writeln!(
                self.out,
                "G1 X{:.3} Y{:.3}{comment}",
                self.x(to[0]),
                self.y(to[1])
            );
        } else {
            self.fed += fed;
            let _ = writeln!(
                self.out,
                "G1 X{:.3} Y{:.3} E{fed:.5}{comment}",
                self.x(to[0]),
                self.y(to[1])
            );
        }
    }

    /// `GCodeWriter::set_speed`, a feedrate in mm/min.
    pub(crate) fn set_speed(&mut self, feedrate: f64) {
        let _ = writeln!(self.out, "G1 F{:.0}", feedrate.round());
    }

    pub(crate) fn set_pressure_advance(&mut self, advance: f64) {
        let text = pressure_advance_line(advance, self.pen.firmware);
        self.out.push_str(&text);
    }

    /// `GCodeWriter::set_print_acceleration`: nothing for 0 or the value
    /// in force; written as this crate's writer writes an acceleration.
    pub(crate) fn set_print_acceleration(&mut self, acceleration: f64) {
        if acceleration <= 0.0 || Some(acceleration) == self.acceleration {
            return;
        }
        self.acceleration = Some(acceleration);
        let _ = match (self.pen.klipper, self.pen.accel_to_decel_factor) {
            (true, Some(factor)) if factor > 0.0 => {
                let factor = factor.min(1.0);
                let ratio = format!("{:.6}", 1.0 - factor);
                let ratio = ratio.trim_end_matches('0').trim_end_matches('.');
                writeln!(
                    self.out,
                    "SET_VELOCITY_LIMIT ACCEL={acceleration:.0} ACCEL_TO_DECEL={:.0} MINIMUM_CRUISE_RATIO={ratio}",
                    acceleration * factor
                )
            }
            (true, _) => writeln!(self.out, "SET_VELOCITY_LIMIT ACCEL={acceleration:.0}"),
            (false, _) => writeln!(self.out, "M204 S{acceleration:.0}"),
        };
    }

    /// `GCodeWriter::set_jerk_xy`: held to the machine's x and y jerk.
    pub(crate) fn set_jerk_xy(&mut self, jerk: f64) {
        if jerk < 0.01 || self.jerk.is_some_and(|last| (last - jerk).abs() < 1e-6) {
            return;
        }
        self.jerk = Some(jerk);
        let [max_x, max_y, max_z, max_e] = self.pen.max_jerk;
        let held = |limit: f64, jerk: f64| {
            if limit > 0.0 && jerk > limit {
                limit
            } else {
                jerk
            }
        };
        let mut text = if self.pen.klipper {
            format!(
                "SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY={}",
                general(held(max_y, held(max_x, jerk)), 6)
            )
        } else if self.pen.firmware == AdvanceFirmware::Repetier {
            format!("M207 X{}", general(held(max_y, held(max_x, jerk)), 6))
        } else {
            format!(
                "M205 X{} Y{}",
                general(held(max_x, jerk), 6),
                general(held(max_y, jerk), 6)
            )
        };
        if self.pen.bambu {
            let _ = write!(text, " Z{} E{}", general(max_z, 2), general(max_e, 2));
        }
        text.push_str(&self.comment("adjust jerk"));
        text.push('\n');
        self.out.push_str(&text);
    }

    /// `CalibPressureAdvance::move_to`: pulled back, to the point (lifted
    /// to `lift` and back down to `layer` when given), pushed out again.
    pub(crate) fn move_to(&mut self, to: [f64; 2], comment: &str, lift: Option<(f64, f64)>) {
        self.retract();
        match lift {
            Some((z, layer)) if z > 1e-4 && layer >= 0.0 => {
                self.travel_to_z(z, "z-hop");
                self.travel_to_xy(to, comment);
                self.travel_to_z(layer, "undo z-hop");
            }
            _ => self.travel_to_xy(to, comment),
        }
        self.unretract();
        self.last = to;
    }

    /// `CalibPressureAdvance::draw_line`: `e_per_mm` of 0 is worked out
    /// by `e_per_mm` for the line.
    pub(crate) fn draw_line(&mut self, to: [f64; 2], e_per_mm: f64, feedrate: f64, comment: &str) {
        let length = (to[0] - self.last[0]).hypot(to[1] - self.last[1]);
        self.set_speed(feedrate);
        self.extrude_to_xy(to, e_per_mm * length, comment);
        self.last = to;
    }

    /// `CalibPressureAdvance::draw_box`.
    pub(crate) fn draw_box(
        &mut self,
        min_x: f64,
        min_y: f64,
        size_x: f64,
        size_y: f64,
        args: BoxArgs,
    ) {
        let (mut x, mut y) = (min_x, min_y);
        let max_x = min_x + size_x;
        let max_y = min_y + size_y;
        let spacing = args.line_width - args.height * (1.0 - PI / 4.0);
        let sin45 = radians(45.0).sin();
        // As many perimeters as fit, at most (concentric fill's count).
        let most = ((size_x * sin45).floor() / (spacing / sin45))
            .min((size_y * sin45).floor() / (spacing / sin45));
        let perimeters = args.perimeters.min(most as i32);
        let e = args.e_per_mm;
        let speed = args.speed;
        self.move_to([min_x, min_y], "Move to box start", None);
        for i in 0..perimeters {
            let i = f64::from(i);
            if i != 0.0 {
                x += spacing;
                y += spacing;
                self.move_to([x, y], "Step inwards to print next perimeter", None);
            }
            y += size_y - i * spacing * 2.0;
            self.draw_line([x, y], e, speed, "Draw perimeter (up)");
            x += size_x - i * spacing * 2.0;
            self.draw_line([x, y], e, speed, "Draw perimeter (right)");
            y -= size_y - i * spacing * 2.0;
            self.draw_line([x, y], e, speed, "Draw perimeter (down)");
            x -= size_x - i * spacing * 2.0;
            self.draw_line([x, y], e, speed, "Draw perimeter (left)");
        }
        if !args.filled {
            return;
        }
        // The infill, at 45°.
        let spacing_45 = spacing / sin45;
        let bound = spacing * f64::from(perimeters - 1) + args.line_width * (1.0 - ENCROACHMENT);
        let (x_min, x_max) = (min_x + bound, max_x - bound);
        let (y_min, y_max) = (min_y + bound, max_y - bound);
        let x_count = ((x_max - x_min) / spacing_45).floor() as i32;
        let y_count = ((y_max - y_min) / spacing_45).floor() as i32;
        let mut x_rest = (x_max - x_min) % spacing_45;
        let mut y_rest = (y_max - y_min) % spacing_45;
        x = x_min;
        y = y_min;
        self.move_to([x, y], "Move to fill start", None);
        let lines = x_count + y_count + i32::from(x_rest + y_rest >= spacing_45);
        for i in 0..lines {
            if i < x_count.min(y_count) {
                if i % 2 == 0 {
                    x += spacing_45;
                    y = y_min;
                    self.move_to([x, y], "Fill: Step right", None);
                    y += x - x_min;
                    x = x_min;
                    self.draw_line([x, y], e, speed, "Fill: Print up/left");
                } else {
                    y += spacing_45;
                    x = x_min;
                    self.move_to([x, y], "Fill: Step up", None);
                    x += y - y_min;
                    y = y_min;
                    self.draw_line([x, y], e, speed, "Fill: Print down/right");
                }
            } else if i < x_count.max(y_count) {
                if x_count > y_count {
                    // Wider than tall.
                    if i % 2 == 0 {
                        x += spacing_45;
                        y = y_min;
                        self.move_to([x, y], "Fill: Step right", None);
                        x -= y_max - y_min;
                        y = y_max;
                        self.draw_line([x, y], e, speed, "Fill: Print up/left");
                    } else {
                        if i == y_count {
                            x += spacing_45 - y_rest;
                            y_rest = 0.0;
                        } else {
                            x += spacing_45;
                        }
                        y = y_max;
                        self.move_to([x, y], "Fill: Step right", None);
                        x += y_max - y_min;
                        y = y_min;
                        self.draw_line([x, y], e, speed, "Fill: Print down/right");
                    }
                } else if i % 2 == 0 {
                    // Taller than wide.
                    x = x_max;
                    if i == x_count {
                        y += spacing_45 - x_rest;
                        x_rest = 0.0;
                    } else {
                        y += spacing_45;
                    }
                    self.move_to([x, y], "Fill: Step up", None);
                    x = x_min;
                    y += x_max - x_min;
                    self.draw_line([x, y], e, speed, "Fill: Print up/left");
                } else {
                    x = x_min;
                    y += spacing_45;
                    self.move_to([x, y], "Fill: Step up", None);
                    x = x_max;
                    y -= x_max - x_min;
                    self.draw_line([x, y], e, speed, "Fill: Print down/right");
                }
            } else if i % 2 == 0 {
                x = x_max;
                if i == x_count {
                    y += spacing_45 - x_rest;
                } else {
                    y += spacing_45;
                }
                self.move_to([x, y], "Fill: Step up", None);
                x -= y_max - y;
                y = y_max;
                self.draw_line([x, y], e, speed, "Fill: Print up/left");
            } else {
                if i == y_count {
                    x += spacing_45 - y_rest;
                } else {
                    x += spacing_45;
                }
                y = y_max;
                self.move_to([x, y], "Fill: Step right", None);
                y -= x_max - x;
                x = x_max;
                self.draw_line([x, y], e, speed, "Fill: Print down/right");
            }
        }
    }

    /// `CalibPressureAdvance::draw_digit`.
    fn draw_digit(
        &mut self,
        start: [f64; 2],
        c: char,
        mode: DigitMode,
        line_width: f64,
        e_per_mm: f64,
    ) {
        let len = DIGIT_SEGMENT;
        let gap = line_width / 2.0;
        let de = e_per_mm * len;
        let two = de * 2.0;
        let [sx, sy] = start;
        let add = |a: [f64; 2], b: [f64; 2]| [a[0] + b[0], a[1] + b[1]];
        let (p0, p0_5, p1, p2, p3, p4, p4_5, p5, gap03, gap23, dot) = match mode {
            //  1-------2-------5
            //  |       |       |
            //  0-------3-------4
            DigitMode::BottomToTop => {
                let p0 = [sx, sy];
                let p2 = [sx + len, sy + len];
                (
                    p0,
                    [sx, sy + len / 2.0],
                    [sx, sy + len],
                    p2,
                    [sx + len, sy],
                    [sx + len * 2.0, sy],
                    [sx + len * 2.0, sy + len / 2.0],
                    [sx + len * 2.0, sy + len],
                    add(p0, [gap, 0.0]),
                    add(p2, [0.0, gap]),
                    [-len / 2.0, 0.0],
                )
            }
            //  0-------1
            //  |       |
            //  3-------2
            //  |       |
            //  4-------5
            DigitMode::LeftToRight => {
                let p0 = [sx, sy];
                let p2 = [sx + len, sy - len];
                (
                    p0,
                    [sx + len / 2.0, sy],
                    [sx + len, sy],
                    p2,
                    [sx, sy - len],
                    [sx, sy - len * 2.0],
                    [sx + len / 2.0, sy - len * 2.0],
                    [sx + len, sy - len * 2.0],
                    add(p0, [0.0, -gap]),
                    add(p2, [-gap, 0.0]),
                    [0.0, len / 2.0],
                )
            }
        };
        match c {
            '0' => {
                self.move_to(p0, "Glyph: 0", None);
                self.extrude_to_xy(p1, de, "");
                self.extrude_to_xy(p5, two, "");
                self.extrude_to_xy(p4, de, "");
                self.extrude_to_xy(gap03, two, "");
            }
            '1' => {
                self.move_to(p0_5, "Glyph: 1", None);
                self.extrude_to_xy(p4_5, two, "");
            }
            '2' => {
                self.move_to(p0, "Glyph: 2", None);
                for to in [p1, p2, p3, p4, p5] {
                    self.extrude_to_xy(to, de, "");
                }
            }
            '3' => {
                self.move_to(p0, "Glyph: 3", None);
                self.extrude_to_xy(p1, de, "");
                self.extrude_to_xy(p5, two, "");
                self.extrude_to_xy(p4, de, "");
                self.move_to(gap23, "", None);
                self.extrude_to_xy(p3, de, "");
            }
            '4' => {
                self.move_to(p0, "Glyph: 4", None);
                self.extrude_to_xy(p3, de, "");
                self.extrude_to_xy(p2, de, "");
                self.move_to(p1, "", None);
                self.extrude_to_xy(p5, two, "");
            }
            '5' => {
                self.move_to(p1, "Glyph: 5", None);
                for to in [p0, p3, p2, p5, p4] {
                    self.extrude_to_xy(to, de, "");
                }
            }
            '6' => {
                self.move_to(p1, "Glyph: 6", None);
                self.extrude_to_xy(p0, de, "");
                self.extrude_to_xy(p4, two, "");
                for to in [p5, p2, p3] {
                    self.extrude_to_xy(to, de, "");
                }
            }
            '7' => {
                self.move_to(p0, "Glyph: 7", None);
                self.extrude_to_xy(p1, de, "");
                self.extrude_to_xy(p5, two, "");
            }
            '8' => {
                self.move_to(p2, "Glyph: 8", None);
                self.extrude_to_xy(p3, de, "");
                self.extrude_to_xy(p4, de, "");
                self.extrude_to_xy(p5, de, "");
                self.extrude_to_xy(p1, two, "");
                self.extrude_to_xy(p0, de, "");
                self.extrude_to_xy(p3, de, "");
            }
            '9' => {
                self.move_to(p5, "Glyph: 9", None);
                self.extrude_to_xy(p1, two, "");
                for to in [p0, p3, p2] {
                    self.extrude_to_xy(to, de, "");
                }
            }
            '.' => {
                self.move_to(p4_5, "Glyph: .", None);
                self.extrude_to_xy(add(p4_5, dot), de, "");
            }
            _ => {}
        }
    }

    /// `CalibPressureAdvance::draw_number`: at most `length` glyphs.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_number(
        &mut self,
        start: [f64; 2],
        value: f64,
        length: usize,
        mode: DigitMode,
        line_width: f64,
        e_per_mm: f64,
        feedrate: f64,
    ) {
        let text = number_text(value, length);
        self.set_speed(feedrate);
        for (i, c) in text.chars().enumerate().take(length) {
            let along = i as f64 * number_spacing();
            let at = match mode {
                DigitMode::BottomToTop => [start[0], start[1] + along],
                DigitMode::LeftToRight => [start[0] + along, start[1]],
            };
            self.draw_digit(at, c, mode, line_width, e_per_mm);
        }
    }
}

/// What "PA Line" draws with, from the slice's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct PaLine {
    pub start: f64,
    pub step: f64,
    /// `std::llround(std::ceil((end - start) / step)) + 1`.
    pub count: i64,
    pub print_numbers: bool,
    /// The bed's box (`printable_area`) and whether it is round (more
    /// than four corners: a delta).
    pub bed: [f64; 4],
    pub delta: bool,
    pub nozzle_diameter: f64,
    /// `initial_layer_print_height`.
    pub layer_height: f64,
    /// Filament fed for one mm³ (the filament's flow ratio over its
    /// section, times the print's flow ratio).
    pub e_per_mm3: f64,
    /// `find_optimal_PA_speed` for the line, mm/s.
    pub fast: f64,
    /// `outer_wall_acceleration` and `outer_wall_jerk`.
    pub acceleration: f64,
    pub jerk: f64,
}

impl PaLine {
    /// `m_line_width`: 1.5 nozzles up to 0.51 mm, 1.05 above.
    pub fn line_width(&self) -> f64 {
        if self.nozzle_diameter < 0.51 {
            self.nozzle_diameter * 1.5
        } else {
            self.nozzle_diameter * 1.05
        }
    }

    fn bed_ext(&self) -> [f64; 4] {
        let [a, b, c, d] = self.bed;
        if self.delta {
            // `delta_scale_bed_ext`: about the origin.
            // The reference's own figure, not √2.
            #[allow(clippy::approx_constant)]
            let f = 1.0 / 1.41421;
            [a * f, b * f, c * f, d * f]
        } else {
            [a, b, c, d]
        }
    }

    /// `CalibPressureAdvanceLine::print_extents` (Orca #14440), with the
    /// bed's box offset by -25 as GCode.cpp gives it to the start G-code
    /// (`first_layer_print_min`, `_max`): min x, min y, max x, max y.
    pub fn first_layer_box(&self) -> [f64; 4] {
        let [min_x, _, max_x, _] = self.bed_ext();
        let width = max_x - min_x;
        let long = 40.0 + (width - 120.0).min(0.0);
        let start_x = min_x + (width - 2.0 * 20.0 - long - 20.0) / 2.0;
        let numbers = if self.print_numbers {
            number_spacing() * 8.0
        } else {
            0.0
        };
        let [a, b, c, d] = self.bed;
        [
            start_x.max(a + 25.0),
            b + 25.0,
            (start_x + 20.0 * 2.0 + long + self.line_width() + numbers).min(c - 25.0),
            d - 25.0,
        ]
    }

    /// The speeds, mm/s: `find_optimal_PA_speed`'s, and a tenth of it (at
    /// least 10), five apart at least (GCode.cpp `_do_export`).
    fn speeds(&self) -> (f64, f64) {
        let fast = self.fast;
        let slow = (fast / 10.0).max(10.0);
        (if fast < slow + 5.0 { slow + 5.0 } else { fast }, slow)
    }

    fn e_per_mm(&self, width: f64) -> f64 {
        crate::calib::line_mm3_per_mm(width, self.layer_height) * self.e_per_mm3
    }
}

/// GCode.cpp `_do_export` for `Calib_PA_Line`, and
/// `CalibPressureAdvanceLine::generate_test` / `print_pa_lines`: the
/// lines in place of the objects' layers. Answers the text and the
/// filament it feeds.
pub(crate) fn pa_line(
    test: &PaLine,
    pen: &Pen,
    acceleration: Option<f64>,
) -> (String, f64, f64, f64) {
    let mut d = Drawing::new(pen, acceleration);
    if test.acceleration > 0.0 {
        d.set_print_acceleration((test.acceleration + 0.5).floor());
    }
    if test.jerk > 0.0 {
        d.set_jerk_xy(test.jerk);
    }
    let [min_x, min_y, max_x, max_y] = test.bed_ext();
    let (w, h) = (max_x - min_x, max_y - min_y);
    const SPACE_Y: f64 = 3.5;
    const SHORT: f64 = 20.0;
    let count = test.count.min(((h - 10.0) / SPACE_Y) as i64).max(0) as i32;
    let long = 40.0 + (w - 120.0).min(0.0);
    let start_x = min_x + (w - SHORT * 2.0 - long - 20.0) / 2.0;
    let start_y = min_y + (h - f64::from(count) * SPACE_Y) / 2.0;

    let line_width = test.line_width();
    let thin = test.nozzle_diameter;
    let e = test.e_per_mm(line_width);
    let number_e = test.e_per_mm(thin);
    let (fast, slow) = test.speeds();
    let (fast, slow) = (fast * 60.0, slow * 60.0);
    let layer = test.layer_height;
    let num = f64::from(count);

    d.travel_to_z(layer, "");
    d.height_tag(layer);
    let y_pos = start_y;
    // The anchor: a line along the test's left side.
    d.role("Custom");
    d.set_pressure_advance(0.0);
    d.move_to([start_x, y_pos + num * SPACE_Y], "", None);
    d.set_speed(slow);
    d.extrude_to_xy([start_x, y_pos], e * SPACE_Y * num * 1.2, "");
    for i in 0..count {
        let fi = f64::from(i);
        let y = y_pos + fi * SPACE_Y;
        d.set_pressure_advance(test.start + fi * test.step);
        d.move_to([start_x, y], "", None);
        d.role("Custom");
        d.set_speed(slow);
        d.extrude_to_xy([start_x + SHORT, y], e * SHORT, "");
        d.role("Outer wall");
        d.set_speed(fast);
        d.extrude_to_xy([start_x + SHORT + long, y], e * long, "");
        d.role("Custom");
        d.set_speed(slow);
        d.extrude_to_xy([start_x + SHORT + long + SHORT, y], e * SHORT, "");
        if i == 0 {
            // The anchor on the right.
            d.set_pressure_advance(0.0);
            d.extrude_to_xy(
                [start_x + SHORT + long + SHORT, y_pos + num * SPACE_Y],
                e * SPACE_Y * num * 1.2,
                "",
            );
        }
    }
    d.set_pressure_advance(0.0);
    let mut top = layer;
    if test.print_numbers {
        let box_x = start_x + SHORT + long + SHORT + line_width;
        d.role("Bottom surface");
        d.draw_box(
            box_x,
            start_y - SPACE_Y,
            number_spacing() * 8.0,
            (num + 1.0) * SPACE_Y,
            BoxArgs {
                filled: true,
                perimeters: 2,
                height: layer,
                line_width,
                speed: fast,
                e_per_mm: e,
            },
        );
        // The numbers print on the next layer.
        d.layer_change_tag();
        d.height_tag(layer * 2.0);
        d.role("Top surface");
        d.travel_to_z(layer * 2.0, "");
        top = layer * 2.0;
        for i in (0..count).step_by(2) {
            let fi = f64::from(i);
            d.draw_number(
                [
                    box_x + 3.0 + line_width,
                    y_pos + fi * SPACE_Y + SPACE_Y / 2.0,
                ],
                test.start + fi * test.step,
                MAX_NUMBER_LEN,
                DigitMode::LeftToRight,
                thin,
                number_e,
                3600.0,
            );
        }
    }
    (d.out, d.fed, d.retracted, top)
}

/// One "PA Pattern" test (`CalibPressureAdvancePattern`), as its handle
/// cube's settings and place give it.
#[derive(Clone, Debug, PartialEq)]
pub struct PaPattern {
    pub start: f64,
    pub end: f64,
    pub step: f64,
    pub nozzle_diameter: f64,
    /// `line_width` and `initial_layer_line_width`, in mm.
    pub line_width: f64,
    pub first_line_width: f64,
    pub layer_height: f64,
    pub first_layer_height: f64,
    /// `wall_loops`.
    pub walls: i32,
    /// `initial_layer_speed` and `outer_wall_speed` (the object's), mm/s.
    pub first_speed: f64,
    pub speed: f64,
    /// `outer_wall_acceleration` (the object's).
    pub acceleration: f64,
    /// The first `z_hop`.
    pub z_hop: f64,
    /// Filament fed for one mm³: the filament's flow ratio over its
    /// section (`CalibPressureAdvance::e_per_mm`, no print flow ratio).
    pub e_per_mm3: f64,
    /// The filament's flow ratio, for the flow printed with the numbers.
    pub flow_ratio: f64,
    /// The handle cube's box across the bed: min x, min y, max x, max y.
    pub handle: [f64; 4],
}

const WALL_SIDE: f64 = 30.0;
const CORNER_ANGLE: f64 = 90.0;
const PATTERN_SPACING: f64 = 2.0;
const GLYPH_PADDING_HORIZONTAL: f64 = 1.0;
const GLYPH_PADDING_VERTICAL: f64 = 1.0;
const LAYERS: usize = 4;
/// `m_handle_xy_size`, `m_handle_spacing`.
pub const HANDLE_SIZE: f64 = 5.0;
pub const HANDLE_SPACING: f64 = 1.2;

impl PaPattern {
    fn half_angle(&self) -> f64 {
        radians(CORNER_ANGLE) / 2.0
    }
    pub fn patterns(&self) -> i32 {
        ((self.end - self.start) / self.step + 1.0).ceil() as i32
    }
    fn spacing(&self) -> f64 {
        self.line_width - self.layer_height * (1.0 - PI / 4.0)
    }
    fn first_spacing(&self) -> f64 {
        self.first_line_width - self.first_layer_height * (1.0 - PI / 4.0)
    }
    fn angle_spacing(&self) -> f64 {
        self.spacing() / self.half_angle().sin()
    }
    fn frame_size_y(&self) -> f64 {
        self.half_angle().sin() * WALL_SIDE * 2.0
    }
    fn object_size_x(&self) -> f64 {
        let n = f64::from(self.patterns());
        n * (f64::from(self.walls - 1) * self.angle_spacing())
            + (n - 1.0) * (PATTERN_SPACING + self.line_width)
            + self.half_angle().cos() * WALL_SIDE
            + self.first_spacing() * f64::from(self.walls)
    }
    pub fn pattern_shift(&self) -> f64 {
        f64::from(self.walls - 1) * self.first_spacing()
            + self.first_line_width
            + GLYPH_PADDING_HORIZONTAL
    }
    pub fn print_size_x(&self) -> f64 {
        self.object_size_x() + self.pattern_shift()
    }
    pub fn print_size_y(&self) -> f64 {
        2.0 * (self.half_angle().sin() * WALL_SIDE)
            + self.numbering_height()
            + GLYPH_PADDING_VERTICAL * 2.0
            + self.first_line_width
    }
    pub fn max_layer_z(&self) -> f64 {
        self.first_layer_height + (LAYERS as f64 - 1.0) * self.layer_height
    }
    /// `handle_pos_offset`: where the handle stands from the middle of the
    /// pattern's place.
    pub fn handle_offset(&self) -> [f64; 2] {
        [
            -self.print_size_x() / 2.0 + HANDLE_SIZE / 2.0 + HANDLE_SPACING,
            -self.numbering_height() / 2.0 - GLYPH_PADDING_VERTICAL,
        ]
    }
    /// `max_numbering_length`: the longest number drawn, the
    /// acceleration's included, at most five glyphs.
    pub fn numbering_length(&self) -> usize {
        let mut most = (0..self.patterns())
            .step_by(2)
            .map(|i| number_text(self.start + f64::from(i) * self.step, 0).len())
            .max()
            .unwrap_or(0);
        most = most.max(number_text(self.acceleration, 0).len());
        most.min(MAX_NUMBER_LEN)
    }
    fn numbering_height(&self) -> f64 {
        let n = self.numbering_length() as f64;
        n * DIGIT_SEGMENT + (n - 1.0) * DIGIT_GAP
    }
    /// `flow_val`: the volumetric speed of the pattern's walls.
    fn flow(&self) -> f64 {
        self.speed
            * crate::calib::line_mm3_per_mm(self.line_width, self.layer_height)
            * self.flow_ratio
    }
    fn e_per_mm(&self, width: f64, height: f64) -> f64 {
        crate::calib::line_mm3_per_mm(width, height) * self.e_per_mm3
    }
    /// `_refresh_starting_point`.
    fn start_point(&self) -> [f64; 2] {
        let [min_x, min_y, _, max_y] = self.handle;
        [
            min_x - HANDLE_SPACING,
            max_y - self.half_angle().sin() * WALL_SIDE - (max_y - min_y) / 2.0,
        ]
    }
    fn glyph_length_x(&self) -> f64 {
        self.line_width + 2.0 * DIGIT_SEGMENT
    }
    fn glyph_start_x(&self, i: i32) -> f64 {
        let i = f64::from(i);
        self.start_point()[0]
            + self.pattern_shift()
            + i * f64::from(self.walls - 1) * self.angle_spacing()
            + i * self.line_width
            + i * PATTERN_SPACING
            + f64::from(self.walls) * self.angle_spacing() / 2.0
            - self.glyph_length_x() / 2.0
    }
}

/// `CalibPressureAdvancePattern::generate_custom_gcodes`: the G-code each
/// of the four layers carries, for one test.
pub fn pa_pattern(test: &PaPattern, pen: &Pen) -> Vec<(String, f64)> {
    let mut d = Drawing::new(pen, None);
    let mut layers = Vec::with_capacity(LAYERS);
    let start = test.start_point();
    let walls = test.walls;
    let (h1, h) = (test.first_layer_height, test.layer_height);
    let (lw1, lw) = (test.first_line_width, test.line_width);
    let first_feed = test.first_speed * 60.0;
    let half = test.half_angle();
    let patterns = test.patterns();

    d.out
        .push_str("; start pressure advance pattern for layer\n");
    d.move_to(start, "Move to start XY position", None);
    d.travel_to_z(h1, "Move to start Z position");
    d.set_pressure_advance(test.start);
    // The anchoring frame, at the outer wall's width and speed.
    let frame = BoxArgs {
        filled: false,
        perimeters: walls,
        height: h1,
        line_width: lw1,
        speed: first_feed,
        e_per_mm: test.e_per_mm(lw1, h1),
    };
    d.role("Outer wall");
    d.draw_box(
        start[0],
        start[1],
        test.print_size_x(),
        test.frame_size_y(),
        frame,
    );
    // The tab the numbers stand on, a bottom surface.
    d.role("Bottom surface");
    d.draw_box(
        start[0],
        start[1] + test.frame_size_y() + test.first_spacing(),
        test.print_size_x(),
        test.numbering_height() + test.first_spacing() + GLYPH_PADDING_VERTICAL * 2.0,
        BoxArgs {
            filled: true,
            ..frame
        },
    );
    let lift = test.z_hop;
    let acceleration = test.acceleration;
    d.role("Outer wall");
    for i in 0..LAYERS {
        let z = h1 + i as f64 * h;
        let hop = Some((z + lift, z));
        if i > 0 {
            d.out.push_str("; end pressure advance pattern for layer\n");
            layers.push((std::mem::take(&mut d.out), std::mem::take(&mut d.fed)));
            d.out
                .push_str("; start pressure advance pattern for layer\n");
            d.travel_to_z(z, "Move to layer height");
            d.reset_e();
        }
        if i == 1 {
            let length = test.numbering_length();
            d.set_pressure_advance(test.start);
            let number_e = test.e_per_mm(lw, h);
            d.role("Outer wall");
            let y = start[1] + test.frame_size_y() + GLYPH_PADDING_VERTICAL + lw;
            // As the reference: a speed in mm/s where a feedrate is
            // meant, which each glyph's travel then overrides.
            for j in (0..patterns).step_by(2) {
                d.draw_number(
                    [test.glyph_start_x(j), y],
                    test.start + f64::from(j) * test.step,
                    length,
                    DigitMode::BottomToTop,
                    lw,
                    number_e,
                    test.first_speed,
                );
            }
            d.draw_number(
                [test.glyph_start_x(patterns + 2), y],
                test.flow(),
                length,
                DigitMode::BottomToTop,
                lw,
                number_e,
                test.first_speed,
            );
            d.draw_number(
                [test.glyph_start_x(patterns + 4), y],
                acceleration,
                length,
                DigitMode::BottomToTop,
                lw,
                number_e,
                test.first_speed,
            );
        }
        let mut to_x = start[0] + test.pattern_shift();
        let mut to_y = start[1];
        let mut side = WALL_SIDE;
        if i == 0 {
            // The first layer shrunk inside the frame.
            let shrink = (test.first_spacing() * f64::from(walls - 1) + lw1 * (1.0 - ENCROACHMENT))
                / half.sin();
            side = WALL_SIDE - shrink;
            to_x += shrink * (radians(90.0) - half).sin();
            to_y += test.first_spacing() * f64::from(walls - 1) + lw1 * (1.0 - ENCROACHMENT);
        } else {
            // A line a little slower, so the acceleration and speed are
            // written again whatever the tests beside it left.
            d.set_print_acceleration(f64::from((acceleration as i32 - 1).max(1)));
            d.move_to(start, "Move to starting point", hop);
            let trick = f64::from((test.speed as i32 - 1).max(1)) * 60.0;
            d.draw_line(
                [start[0], start[1] + test.frame_size_y()],
                test.e_per_mm(lw, h),
                trick,
                "Accel/flow trick line",
            );
            d.set_print_acceleration(acceleration);
        }
        let (first_x, first_y) = (to_x, to_y);
        d.move_to([to_x, to_y], "Move to pattern start", hop);
        let height = if i == 0 { h1 } else { h };
        let feed = if i == 0 {
            first_feed
        } else {
            test.speed * 60.0
        };
        // The same width on every layer, so the results compare.
        let e = test.e_per_mm(lw, height);
        for j in 0..patterns {
            d.set_pressure_advance(test.start + f64::from(j) * test.step);
            d.role("Outer wall");
            for k in 0..walls {
                to_x += half.cos() * side;
                to_y += half.sin() * side;
                d.draw_line([to_x, to_y], e, feed, "Print pattern wall");
                to_x -= half.cos() * side;
                to_y += half.sin() * side;
                d.draw_line([to_x, to_y], e, feed, "Print pattern wall");
                to_y = first_y;
                if k != walls - 1 {
                    to_x += test.angle_spacing();
                    d.move_to([to_x, to_y], "Move to start next pattern wall", hop);
                } else if j != patterns - 1 {
                    to_x += PATTERN_SPACING + lw;
                    d.move_to([to_x, to_y], "Move to next pattern", hop);
                } else if i != LAYERS - 1 {
                    to_x = first_x;
                    d.move_to([to_x, to_y], "Move back to start position", hop);
                    d.reset_e();
                }
            }
        }
    }
    // Ahead of the pin: from OrcaSlicer dfb93e0332 (#14473), the
    // extrusion reset before the pressure advance is set back, so absolute
    // E does not over-retract after the pattern. (Nothing to write here,
    // where extrusion is relative.)
    d.reset_e();
    d.set_pressure_advance(test.start);
    d.out.push_str("; end pressure advance pattern for layer\n");
    layers.push((d.out, d.fed));
    layers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pen() -> Pen {
        Pen {
            firmware: AdvanceFirmware::Klipper,
            bambu: false,
            klipper: true,
            accel_to_decel_factor: None,
            comments: false,
            retraction_length: 0.8,
            retraction_speed: 30.0,
            deretraction_speed: 0.0,
            travel_speed: 200.0,
            z_speed: 10.0,
            max_jerk: [9.0, 9.0, 0.4, 2.5],
            offset: [0.0, 0.0],
        }
    }

    #[test]
    fn numbers_are_drawn_to_the_reference_s_digits() {
        // `convert_number_to_string(num, 5)`: four significant digits
        // below 1000.
        assert_eq!(number_text(0.002, 5), "0.002");
        assert_eq!(number_text(3.0 * 0.002, 5), "0.006");
        assert_eq!(number_text(0.1, 5), "0.1");
        assert_eq!(number_text(12345.0, 5), "12345");
        assert_eq!(number_text(1500.0, 0), "1500");
    }

    #[test]
    fn a_move_pulls_back_travels_and_pushes_out_once() {
        let pen = pen();
        let mut d = Drawing::new(&pen, None);
        d.move_to([10.0, 20.0], "", None);
        d.move_to([12.0, 20.0], "", Some((0.6, 0.2)));
        assert_eq!(
            d.out,
            "G1 E-0.80000 F1800\nG1 X10.000 Y20.000 F12000\nG1 E0.80000 F1800\n\
             G1 E-0.80000 F1800\nG1 Z0.600 F600\nG1 X12.000 Y20.000 F12000\nG1 Z0.200 F600\nG1 E0.80000 F1800\n"
        );
    }

    #[test]
    fn a_box_of_two_perimeters_is_drawn_round_then_inwards() {
        let pen = pen();
        let mut d = Drawing::new(&pen, None);
        let args = BoxArgs {
            filled: false,
            perimeters: 2,
            height: 0.2,
            line_width: 0.6,
            speed: 6000.0,
            e_per_mm: 0.05,
        };
        d.draw_box(0.0, 0.0, 10.0, 5.0, args);
        let lines: Vec<&str> = d.out.lines().filter(|line| line.contains(" E0.")).collect();
        // Four sides each, the second loop one spacing in (0.6 - 0.2 ×
        // 0.2146 = 0.55708): 5 mm up is 0.25 of filament.
        assert_eq!(lines.len(), 8 + 2, "{}", d.out);
        assert!(d.out.contains("G1 X0.000 Y5.000 E0.25000"));
        assert!(d.out.contains("G1 X0.557 Y0.557 F12000"));
        assert!(d.out.contains("G1 X0.557 Y4.443 E0.19429"));
    }

    #[test]
    fn jerk_is_held_to_the_machine_and_said_once() {
        let pen = pen();
        let mut d = Drawing::new(&pen, None);
        d.set_jerk_xy(12.0);
        d.set_jerk_xy(12.0);
        assert_eq!(d.out, "SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY=9\n");
        let marlin = Pen {
            klipper: false,
            firmware: AdvanceFirmware::Marlin,
            ..pen
        };
        let mut d = Drawing::new(&marlin, None);
        d.set_jerk_xy(8.0);
        assert_eq!(d.out, "M205 X8 Y8\n");
    }

    fn pattern() -> PaPattern {
        PaPattern {
            start: 0.0,
            end: 0.08,
            step: 0.005,
            nozzle_diameter: 0.4,
            line_width: 0.45,
            first_line_width: 0.56,
            layer_height: 0.2,
            first_layer_height: 0.2,
            walls: 3,
            first_speed: 30.0,
            speed: 150.0,
            acceleration: 10000.0,
            z_hop: 0.4,
            e_per_mm3: 1.0 / (PI * 0.875 * 0.875),
            flow_ratio: 1.0,
            handle: [100.0, 100.0, 105.0, 105.0],
        }
    }

    #[test]
    fn the_pattern_is_sized_as_the_reference_sizes_it() {
        // calib.hpp/.cpp worked by hand: 17 patterns; spacing 0.45 - 0.2 ×
        // (1 - π/4) = 0.407080, first layer 0.517080, along the 45° walls
        // 0.575696; object_size_x = 17 × 2 × 0.575696 + 16 × 2.45 + 30 ×
        // cos 45° + 3 × 0.517080 = 81.5382; pattern_shift = 2 × 0.517080
        // + 0.56 + 1 = 2.59416.
        let p = pattern();
        assert_eq!(p.patterns(), 17);
        assert!(
            (p.pattern_shift() - 2.594159).abs() < 1e-5,
            "{}",
            p.pattern_shift()
        );
        assert!(
            (p.print_size_x() - 84.1323).abs() < 1e-4,
            "{}",
            p.print_size_x()
        );
        // Numbers of up to five glyphs ("10000"): 5 × 2 + 4 × 1 = 14 mm.
        assert_eq!(p.numbering_length(), 5);
        assert!(
            (p.print_size_y() - (2.0 * 30.0 * (PI / 4.0).sin() + 14.0 + 2.0 + 0.56)).abs() < 1e-9
        );
        assert!((p.max_layer_z() - 0.8).abs() < 1e-9);
        // The handle 1.2 mm right of the pattern's left edge, its numbers'
        // half height and a padding below the middle.
        let [x, y] = p.handle_offset();
        assert!((x - (-84.1323 / 2.0 + 2.5 + 1.2)).abs() < 1e-4);
        assert!((y - (-7.0 - 1.0)).abs() < 1e-9);
    }

    #[test]
    fn the_pattern_steps_its_advance_block_by_block_and_layer_by_layer() {
        let layers = pa_pattern(&pattern(), &pen());
        assert_eq!(layers.len(), 4);
        for (index, (text, fed)) in layers.iter().enumerate() {
            assert!(
                text.starts_with("; start pressure advance pattern for layer\n"),
                "{index}"
            );
            assert!(
                text.ends_with("; end pressure advance pattern for layer\n"),
                "{index}"
            );
            assert!(*fed > 0.0);
            // One value for each block, start to end.
            for j in 0..17 {
                let value = general(0.0 + f64::from(j) * 0.005, 4);
                assert!(
                    text.contains(&format!("SET_PRESSURE_ADVANCE ADVANCE={value};")),
                    "layer {index}, block {j}"
                );
            }
        }
        // From the second layer the trick line sets the acceleration again.
        assert!(!layers[0].0.contains("SET_VELOCITY_LIMIT ACCEL"));
        assert!(layers[1].0.contains("SET_VELOCITY_LIMIT ACCEL=9999\n"));
        assert!(layers[1].0.contains("SET_VELOCITY_LIMIT ACCEL=10000\n"));
        // The trick line at 149 mm/s, the walls at 150.
        assert!(layers[2].0.contains("G1 F8940\n"));
        assert!(layers[2].0.contains("G1 F9000\n"));
        // Lifted between walls by the z hop: 0.6 + 0.4 on the third layer.
        assert!(layers[2].0.contains("G1 Z1.000 F600\n"));
        // The last layer ends back at the start's advance.
        assert!(layers[3].0.ends_with("SET_PRESSURE_ADVANCE ADVANCE=0; Override pressure advance value\n; end pressure advance pattern for layer\n"));
    }

    #[test]
    fn the_line_test_is_drawn_and_numbered() {
        let test = PaLine {
            start: 0.0,
            step: 0.002,
            count: 51,
            print_numbers: true,
            bed: [0.0, 0.0, 270.0, 270.0],
            delta: false,
            nozzle_diameter: 0.4,
            layer_height: 0.2,
            e_per_mm3: 1.0 / (PI * 0.875 * 0.875),
            fast: 120.0,
            acceleration: 5000.0,
            jerk: 0.0,
        };
        let (text, fed, _, top) = pa_line(&test, &pen(), None);
        assert!(fed > 0.0);
        assert!((top - 0.4).abs() < 1e-9);
        // GCodeWriter: the acceleration, then the first layer.
        assert!(text.starts_with(
            "SET_VELOCITY_LIMIT ACCEL=5000\nG1 Z0.200 F600\n;HEIGHT:0.2\n;TYPE:Custom\n"
        ));
        // 51 lines, each its own advance, then 0 again.
        for i in 0..51 {
            let value = general(f64::from(i) * 0.002, 4);
            assert!(text.contains(&format!("ADVANCE={value};")), "{i}");
        }
        // The long middle at 120 mm/s, the ends at 12.
        assert!(text.contains(";TYPE:Outer wall\nG1 F7200\n"));
        assert!(text.contains("G1 F720\n"));
        // Lines 3.5 mm apart from y = (270 - 51 × 3.5) / 2 = 45.75; x from
        // (270 - 40 - 40 - 20) / 2 = 85.
        assert!(text.contains("G1 X85.000 Y45.750 F12000"));
        assert!(text.contains("G1 X105.000 Y45.750 E"));
        // The numbers on the second layer.
        assert!(text.contains(";LAYER_CHANGE\n;HEIGHT:0.4\n;TYPE:Top surface\nG1 Z0.400 F600\n"));
        assert_eq!(
            test.first_layer_box(),
            [85.0, 25.0, 85.0 + 80.0 + 0.6 + 24.0, 245.0]
        );
    }
}
