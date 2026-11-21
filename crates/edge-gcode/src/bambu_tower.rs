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
    /// Where the purge starts, in mm, worked in single precision as the
    /// reference's `tool_change_start_pos` is (for the H2C's heating point).
    pub change_start_mm: [f64; 2],
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
    /// The change is on a contact layer of its block (`is_contact`): with
    /// Bambu's interface features a prime tower interface.
    pub is_contact: bool,
    /// Onto another nozzle: the outgoing filament's ramming, laid before
    /// the change (`nozzle_change_result`).
    pub nozzle_change: Option<TowerRamming>,
    /// Where the change G-code takes the head beside the tower on its way
    /// back, in mm on the bed (`wipe_avoid_pos_x`: 3 mm right of the
    /// tower's first layer, else left of it, within X 100 to 250, else
    /// 110).
    pub wipe_avoid_x: f64,
}

/// The ramming of the filament put away before a change onto another
/// nozzle (WipeTower.cpp `ramming`, `NozzleChangeResult`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TowerRamming {
    /// What the tower's writer wrote, in its frame.
    pub gcode: String,
    /// Where it starts and ends on the bed.
    pub start: Point2,
    pub end: Point2,
    /// Where the nozzle wipes after it, on the bed.
    pub wipe_path: Vec<Point2>,
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
                    if changes && let Some(ramming) = &visit.nozzle_change {
                        self.bambu_ramming(ramming, visit.origin, index, low)?;
                    }
                    if changes && let Some(template) = &self.all[from].filament_end_gcode {
                        // Told which filament it ends (`append_tcr`).
                        self.variables
                            .set("filament_extruder_id", Value::Int(from as i64));
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

    /// The outgoing filament's ramming before a change onto another nozzle
    /// (GCode.cpp `append_tcr`, from Bambu Studio 2.9.1): retracted at the
    /// model, to the ramming's start, pushed back, laid at the first
    /// layer's or the default acceleration, then the tool change's
    /// retraction with a wipe back along the last ramming line.
    fn bambu_ramming(
        &mut self,
        ramming: &TowerRamming,
        origin: [f64; 2],
        layer: usize,
        low: Option<f64>,
    ) -> Result<(), GcodeError> {
        if self.state.primed {
            self.retract(false)?;
            // `GCode::retract`'s own lift: straight up.
            if self.state.pending_lift.is_some() {
                self.state.pending_lift = Some(crate::Lift::Normal);
            }
        }
        self.tower_route(ramming.start, self.state.position)?;
        self.travel(ramming.start, false, None)?;
        if let Some(tower) = low {
            self.lower_to_tower(tower)?;
        }
        if self.state.primed {
            self.prime()?;
        }
        // The ramming rows are print moves: Bambu Studio's tower writer
        // sets the acceleration round them (Edge, review S2/N5).
        let s = self.settings;
        if s.default_acceleration > 0.0 {
            let acceleration = if layer == 0 && s.initial_layer_acceleration > 0.0 {
                s.initial_layer_acceleration
            } else {
                s.default_acceleration
            };
            self.accelerate((acceleration + 0.5).floor())?;
        }
        for line in ramming.gcode.lines() {
            self.tower_text(line, origin)?;
        }
        self.state.position = Some(ramming.end);
        self.state.wipe_path = ramming.wipe_path.clone();
        let toolchange = self.settings.retract_length_toolchange;
        self.retract_by(toolchange, false)?;
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
        // The head keeps clear of the tower's wall on its way back where
        // the wall is left open (`wipe_avoid_perimeter`), beside the tower
        // (`wipe_avoid_pos_x`); and with Bambu's interface features a
        // change on a contact layer is a prime tower interface, which the
        // change G-code purges for (`append_tcr`).
        let interface = self.all[to].tower_interface;
        let mut kept = Vec::new();
        let mut set = |writer: &mut Self, name: &str, value: Value| {
            kept.push((name.to_owned(), writer.variables.value(name)));
            writer.variables.set(name, value);
        };
        set(self, "wipe_avoid_perimeter", Value::Bool(s.tower_wall_gap));
        set(self, "wipe_avoid_pos_x", Value::Float(visit.wipe_avoid_x));
        // The point the nozzle heats at for this change: 2 mm out from
        // where the purge starts (GCode.cpp `append_tcr`), for this change
        // only.
        if let Some(boxes) =
            crate::bambu_vars::HeatingBoxes::new(&self.variables, self.settings.tower_mesh_bottom)
        {
            let [x, y] = boxes.at_change(visit.change_start_mm);
            set(self, "wipe_tower_center_pos_valid", Value::Bool(true));
            set(self, "wipe_tower_center_pos_x", Value::Float(x));
            set(self, "wipe_tower_center_pos_y", Value::Float(y));
        }
        if interface.features {
            set(
                self,
                "is_prime_tower_interface",
                Value::Bool(visit.is_contact),
            );
            set(
                self,
                "filament_tower_interface_purge_volume",
                Value::Float(interface.purge_volume),
            );
            set(
                self,
                "filament_tower_interface_print_temp",
                Value::Int(interface.print_temperature as i64),
            );
        }
        self.state.after_change = Some(visit.change_start);
        let swapped = self.swap(Some(from as i64), to, Some(layer), z, toolchange_z, true);
        self.state.after_change = None;
        for (name, value) in kept.into_iter().rev() {
            match value {
                Some(value) => self.variables.set(&name, value),
                None => self.variables.remove(&name),
            }
        }
        if let Some(list) = cooling {
            self.variables
                .set_list("filament_cooling_before_tower", list);
        }
        let swapped = swapped?;
        self.state.flushed.select(to, self.all);
        self.state.tool = to;
        self.state.tool_changes += 1;
        self.state.summary.tool_changes += 1;
        self.out.extend_from_slice(crate::RESUME_FAN_MARK);
        // Two nozzles, the farthest point the other nozzle's: the photo
        // just after the change from the photo nozzle onto it, from the
        // safe spot, before the travel to the tower (GCode.cpp
        // `append_tcr`, "case B"); after the tool command where the
        // template does not change the tool itself. Unless one was taken in
        // passing or here this layer (`inserted_this_layer`): a photo
        // placed before an earlier change does not count, so such a layer
        // gets two, as in Bambu Studio's `append_tcr`.
        let case_b = self.photos.as_ref().is_some_and(|photos| {
            let photo = photos.photo_extruder(self.current_run);
            photos.farthest
                && !photos.far_on_photo_head
                && !photos.inserted
                && photos.physical(photos.map.extruder(from)) == photos.physical(photo)
                && photos.physical(photos.map.extruder(to)) != photos.physical(photo)
        });
        if case_b && swapped.templated {
            self.nozzle_photo(z, Some(to), false)?;
            self.inserted_photo();
        }
        if let Some(command) = swapped.command {
            self.out.extend_from_slice(command.as_bytes());
        }
        if case_b && !swapped.templated {
            self.nozzle_photo(z, Some(to), false)?;
            self.inserted_photo();
        }
        // To where the purge starts, and the filament pushed back
        // ("Move to start pos", `unretract`).
        self.settings = &self.all[to];
        self.travel(visit.change_start, false, None)?;
        // A contact change with the interface features pushes out
        // `filament_tower_interface_pre_extrusion_length` more at its start
        // outside the tower.
        let extra = if interface.features && visit.is_contact {
            interface.pre_extrusion_length
        } else {
            0.0
        };
        if self.state.primed {
            if extra > 0.0 {
                self.state.retracted = true;
                self.state.retracted_length += extra;
            }
            self.prime()?;
        }
        // Each nozzle keeps its own filament's retraction: the reference
        // takes the new filament's up only after the travel and the push
        // back, which are the outgoing one's (`append_tcr` asks the writer
        // for the tool command last).
        if !s.single_nozzle {
            self.state.parked[from] = (self.state.retracted, self.state.retracted_length);
            (self.state.retracted, self.state.retracted_length) = self.state.parked[to];
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
