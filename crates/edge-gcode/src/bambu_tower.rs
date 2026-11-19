//! Bambu's prime tower (the reference's Type 1, WipeTower.cpp) written into
//! the file: one visit of the tower, as its writer wrote it in the tower's
//! own frame (edge-tower `type1::lay`), moved to the tower's place and with
//! the change of filament made where its placeholders say (GCode.cpp
//! `WipeTowerIntegration::append_tcr`, `post_process_wipe_tower_moves`).

use std::fmt::Write as _;
use std::io::{self, Write};

use edge_geom::{Point2, unscaled};

use crate::{GcodeError, Layer, Value, Writer, distance};

/// One visit of Bambu's tower (`ToolChangeResult`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TowerVisit {
    /// What the tower's writer wrote, in its frame: millimetres from the
    /// tower's corner, the rib offset and the layer's y shift applied.
    pub gcode: String,
    /// Where the tower's corner is on the bed, in mm (`m_wipe_tower_pos`).
    pub origin: [f64; 2],
    /// Where the visit starts, where its purge starts after the change,
    /// and where it ends, on the bed.
    pub start: Point2,
    pub change_start: Point2,
    pub end: Point2,
    /// Where the nozzle wipes leaving the tower, on the bed.
    pub wipe_path: Vec<Point2>,
    /// The visit starts with the tower's own lines, not with the change
    /// (`is_finish_first`): the nozzle is taken to it first.
    pub finish_first: bool,
    /// It changes filament.
    pub tool_change: bool,
    /// The change G-code may cool the nozzle before the purge, which then
    /// heats it back up (`reheats_after_cooling`).
    pub reheats_after_cooling: bool,
}

/// A word's number in a G-code line.
fn word(line: &str, letter: char) -> Option<f64> {
    line.split_whitespace()
        .skip(1)
        .find_map(|w| w.strip_prefix(letter).and_then(|n| n.parse().ok()))
}

impl Writer<'_> {
    /// Writes one visit of Bambu's tower for `layer` (GCode.cpp
    /// `append_tcr`): to the tower first where it starts with its own
    /// lines, its lines moved to the tower's place, and at its
    /// placeholders the old filament's end G-code, the change (its
    /// retraction, the change G-code, to where the purge starts and the
    /// filament pushed back) and the new filament's start G-code.
    pub(crate) fn bambu_visit(
        &mut self,
        layer: &Layer<'_>,
        visit: &TowerVisit,
        index: usize,
        low: Option<f64>,
    ) -> Result<(), GcodeError> {
        let from = self.state.tool;
        let to = layer.filament;
        let changes = visit.tool_change && to != from;
        if visit.finish_first {
            if self.state.primed {
                self.retract(false)?;
            }
            self.tower_route(visit.start, self.state.position)?;
            self.travel(visit.start, false, None)?;
            if let Some(tower) = low {
                self.lower_to_tower(tower)?;
            }
            if self.state.primed {
                self.prime()?;
            }
        }
        for line in visit.gcode.lines() {
            match line {
                "[filament_end_gcode]" => {
                    if changes && let Some(template) = &self.all[from].filament_end_gcode {
                        self.custom("filament_end_gcode", template)?;
                    }
                }
                "[restore_layer_z_before_toolchange]" => {
                    if changes {
                        self.leave_tower()?;
                    }
                }
                "[change_filament_gcode]" => {
                    if changes {
                        self.bambu_change(visit, to, index, layer.z, low)?;
                    }
                }
                "[deretraction_from_wipe_tower_generator]" => {
                    if changes && let Some(tower) = low {
                        self.lower_to_tower(tower)?;
                    }
                }
                "[filament_start_gcode]" => {
                    if changes {
                        self.announce(to, index == 0)?;
                    }
                }
                _ => self.tower_text(line, visit.origin)?,
            }
        }
        self.state.position = Some(visit.end);
        self.state.wipe_path = visit.wipe_path.clone();
        Ok(())
    }

    /// The change of filament inside a visit (`append_tcr`, the
    /// `[change_filament_gcode]` placeholder): the filament pulled back by
    /// the change's length, the printer's change G-code, the head to
    /// where the purge starts and the filament pushed on again.
    fn bambu_change(
        &mut self,
        visit: &TowerVisit,
        to: usize,
        layer: usize,
        z: f64,
        low: Option<f64>,
    ) -> Result<(), GcodeError> {
        let from = self.state.tool;
        let s = self.settings;
        if self.state.primed && s.retract_length_toolchange > 0.0 {
            self.retract_by(s.retract_length_toolchange, false)?;
            let more = s.retract_length_toolchange - self.state.retracted_length;
            if more > 0.0 {
                self.feed(s.retraction_speed);
                let feed_word = self.feed_word();
                writeln!(self.out, "G1 E-{more:.5}{feed_word}")?;
                self.state.retracted = true;
                self.state.retracted_length = s.retract_length_toolchange;
                self.spend(more / s.retraction_speed);
            }
        }
        self.out.extend_from_slice(crate::CHANGE_MARK);
        writeln!(self.out, "T{to} S{:.0}", self.temperature(to, layer == 0))?;
        let toolchange_z = low.unwrap_or(z);
        // The change G-code: no cooling before the tower on the first layer
        // or where the tower does not heat back up (`append_tcr`,
        // `bambu_flush_placeholders`), and the head after it where the
        // purge starts (`tool_change_start_pos`).
        let cooling = self
            .variables
            .list("filament_cooling_before_tower")
            .map(<[Value]>::to_vec);
        if let Some(list) = &cooling
            && (layer == 0 || !visit.reheats_after_cooling)
        {
            self.variables.set_list(
                "filament_cooling_before_tower",
                vec![Value::Float(0.0); list.len()],
            );
        }
        self.state.after_change = Some(visit.change_start);
        let swapped = self.swap(Some(from as i64), to, Some(layer), z, toolchange_z, true);
        self.state.after_change = None;
        if let Some(list) = cooling {
            self.variables
                .set_list("filament_cooling_before_tower", list);
        }
        let swapped = swapped?;
        self.state.flushed.select(to, self.all);
        if !s.single_nozzle {
            self.state.parked[from] = (self.state.retracted, self.state.retracted_length);
            (self.state.retracted, self.state.retracted_length) = self.state.parked[to];
        }
        self.state.tool = to;
        self.state.tool_changes += 1;
        self.state.summary.tool_changes += 1;
        self.out.extend_from_slice(crate::RESUME_FAN_MARK);
        if let Some(command) = swapped.command {
            self.out.extend_from_slice(command.as_bytes());
        }
        // To where the purge starts, and the filament pushed back
        // ("Move to start pos", `unretract`).
        self.settings = &self.all[to];
        self.travel(visit.change_start, false, None)?;
        if self.state.primed {
            self.prime()?;
        }
        Ok(())
    }

    /// One line of the tower's writer, moved to the tower's place
    /// (`post_process_wipe_tower_moves`), and what it does counted.
    fn tower_text(&mut self, line: &str, origin: [f64; 2]) -> io::Result<()> {
        let code = line.split(';').next().unwrap_or("").trim();
        let command = code.split_whitespace().next().unwrap_or("");
        if !matches!(command, "G1" | "G2" | "G3") {
            if let Some(feature) = line.strip_prefix("; FEATURE: ") {
                self.state.role =
                    (feature == "Prime tower").then_some(edge_extrusion::Role::PrimeTower);
            } else if let Some(width) = line.strip_prefix("; LINE_WIDTH: ") {
                self.state.width = width.trim().parse().unwrap_or(0.0);
            } else if let Some(height) = line.strip_prefix("; LAYER_HEIGHT: ") {
                self.state.height_tag = height.trim().parse().unwrap_or(0.0);
            } else if command == "G92" {
                // E is relative throughout.
            }
            writeln!(self.out, "{line}")?;
            return Ok(());
        }
        let (x, y) = (word(code, 'X'), word(code, 'Y'));
        let (e, f) = (word(code, 'E'), word(code, 'F'));
        let at = self.state.position;
        let to = if x.is_some() || y.is_some() {
            let here = at.map_or((0.0, 0.0), |p| (unscaled(p.x), unscaled(p.y)));
            Some(Point2::from_mm(
                x.map_or(here.0, |x| x + origin[0]),
                y.map_or(here.1, |y| y + origin[1]),
            ))
        } else {
            None
        };
        let mut out = String::from(command);
        if let Some(to) = to {
            write!(out, " X{:.3} Y{:.3}", self.gcode_x(to), self.gcode_y(to)).ok();
        }
        for w in code.split_whitespace().skip(1) {
            if !(w.starts_with('X') || w.starts_with('Y')) {
                out.push(' ');
                out.push_str(w);
            }
        }
        writeln!(self.out, "{out}")?;
        if let Some(f) = f {
            self.state.feedrate = f;
        }
        let speed = self.state.feedrate.abs() / 60.0;
        let length = match (at, to) {
            (Some(at), Some(to)) => distance(at, to),
            _ => 0.0,
        };
        let seconds = if speed > 0.0 {
            if length > 0.0 {
                length / speed
            } else {
                e.unwrap_or(0.0).abs() / speed
            }
        } else {
            0.0
        };
        match e {
            Some(e) if e > 0.0 && length > 0.0 => {
                if !self.extruding {
                    self.extruding = true;
                    self.timing.fixed = 0.0;
                }
                self.timing.fixed += seconds;
                self.state.summary.seconds += seconds;
                let volume = e * self.filament_area;
                self.state.summary.filament_mm += e;
                self.state.summary.filament_mm3 += volume;
                if let Some(used) = self.state.used.get_mut(self.state.tool) {
                    used.0 += e;
                    used.1 += volume;
                }
                self.state.primed = true;
            }
            _ => {
                if length > 0.0 {
                    self.state.summary.travel_mm += length;
                }
                self.spend(seconds);
            }
        }
        if let Some(to) = to {
            self.state.position = Some(to);
            self.state.left_at = None;
        }
        Ok(())
    }
}
