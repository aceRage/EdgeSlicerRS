//! Turns each layer's walls and fill into G-code: the order things are
//! printed in, the travel between them, retraction, fan and acceleration,
//! and how much filament every move pushes.
//!
//! With a printer profile, the machine's own start, end and layer-change
//! G-code is expanded from its templates and the output is meant to print.
//! It has not been run on a printer. Without a profile a generic preamble
//! stands in and the output is only good for viewing.
//!
//! Not done: tool changes and multiple extruders, slowing down short
//! layers, the auxiliary fan, overhang and bridge fan speeds, seam
//! placement, wipe, arc fitting, skirt and brim, progress (`M73`) and
//! object labels. Coordinates are the project's own, which for a 3MF's
//! first plate are the bed's.

mod inside;
mod settings;

use std::{
    f64::consts::PI,
    io::{self, Write},
    ops::Range,
};

use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, unscaled};
use edge_template::{TemplateError, Value, Variables};

pub use settings::{PrintSettings, WallOrder};

#[derive(Debug, thiserror::Error)]
pub enum GcodeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A G-code template from the profile could not be filled in.
    #[error("{name}, {source}")]
    Template {
        /// The setting holding the template.
        name: &'static str,
        #[source]
        source: TemplateError,
    },
}

/// One layer's toolpaths.
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    /// Height of the top of the layer above the bed, in mm.
    pub z: f64,
    /// Thickness of the layer in mm.
    pub height: f64,
    pub loops: &'a [ExtrusionLoop],
    /// Which of `loops` make up each island.
    pub islands: &'a [Range<usize>],
    pub paths: &'a [ExtrusionPath],
    /// The area inside the walls. Moves between fill lines that stay
    /// within it can skip retraction, since any ooze lands where fill goes.
    pub infill_area: &'a [ExPolygon],
}

/// What a print comes to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Summary {
    /// Filament fed by the layers, in mm, not counting retraction or
    /// whatever the machine's start G-code purges.
    pub filament_mm: f64,
    /// The same as a volume, in mm³.
    pub filament_mm3: f64,
    /// Distance moved without extruding, in mm.
    pub travel_mm: f64,
    pub retractions: usize,
    /// Time for every move at its commanded speed, in seconds. Real prints
    /// take longer: acceleration and heating are not counted.
    pub seconds: f64,
}

/// Writes the whole print and reports what it used.
pub fn write(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    settings: &PrintSettings,
) -> Result<Summary, GcodeError> {
    let mut variables = settings.variables.clone();
    variables.set("total_layer_count", Value::Int(layers.len() as i64));
    variables.set(
        "max_layer_z",
        Value::Float(layers.last().map_or(0.0, |layer| layer.z)),
    );
    variables.set("layer_num", Value::Int(0));
    variables.set("layer_z", Value::Float(0.0));
    // Where the first layer is, for start G-code that probes or purges near it.
    if let Some([min_x, min_y, max_x, max_y]) = layers.first().and_then(extent) {
        let pair = |x: f64, y: f64| vec![Value::Float(x), Value::Float(y)];
        variables.set_list("first_layer_print_min", pair(min_x, min_y));
        variables.set_list("first_layer_print_max", pair(max_x, max_y));
        variables.set_list("first_layer_print_size", pair(max_x - min_x, max_y - min_y));
    }

    let mut writer = Writer {
        out,
        settings,
        variables,
        filament_area: PI * (settings.filament_diameter / 2.0).powi(2),
        position: None,
        z: 0.0,
        first_layer: true,
        lifted: false,
        retracted: false,
        primed: false,
        feedrate: 0.0,
        acceleration: 0.0,
        fan: None,
        role: None,
        width: 0.0,
        summary: Summary::default(),
    };
    writer.preamble()?;
    for (index, layer) in layers.iter().enumerate() {
        writer.layer(index, layer)?;
    }
    writer.finish()
}

/// The box around everything a layer extrudes, in mm: min x, min y, max x, max y.
fn extent(layer: &Layer<'_>) -> Option<[f64; 4]> {
    let walls = layer.loops.iter().flat_map(|wall| &wall.polygon.points);
    let fills = layer.paths.iter().flat_map(|path| &path.points);
    walls
        .chain(fills)
        .map(|p| (unscaled(p.x), unscaled(p.y)))
        .fold(None, |extent, (x, y)| {
            let [min_x, min_y, max_x, max_y] = extent.unwrap_or([x, y, x, y]);
            Some([min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y)])
        })
}

/// How fast a line is laid and how hard the printer may accelerate into it.
#[derive(Clone, Copy)]
struct Motion {
    /// In mm/s.
    speed: f64,
    /// In mm/s²; zero leaves the printer's setting alone.
    acceleration: f64,
}

struct Writer<'a, W> {
    out: &'a mut W,
    settings: &'a PrintSettings,
    /// What templates can refer to; the layer's number and height change as it goes.
    variables: Variables,
    /// Cross-section of the filament in mm².
    filament_area: f64,
    /// Where the nozzle is, once it has been anywhere.
    position: Option<Point2>,
    /// The current layer's height above the bed.
    z: f64,
    first_layer: bool,
    /// Whether the nozzle is raised above the layer for a travel.
    lifted: bool,
    retracted: bool,
    /// Whether the nozzle holds molten plastic that would ooze on a travel:
    /// true once anything is extruded, or once the machine's start G-code
    /// has run, since that purges.
    primed: bool,
    /// The last feedrate written, in mm/min.
    feedrate: f64,
    /// The last acceleration written, in mm/s².
    acceleration: f64,
    /// The last fan speed written, 0 to 255.
    fan: Option<u8>,
    role: Option<Role>,
    width: f64,
    summary: Summary,
}

impl<W: Write> Writer<'_, W> {
    fn preamble(&mut self) -> Result<(), GcodeError> {
        let s = self.settings;
        writeln!(
            self.out,
            "; generated by EdgeSlicerRS {}",
            env!("CARGO_PKG_VERSION")
        )?;
        match &s.machine_start_gcode {
            Some(template) => {
                writeln!(self.out, ";TYPE:Custom")?;
                self.custom("machine_start_gcode", template)?;
                self.primed = true;
            }
            None => {
                writeln!(
                    self.out,
                    "; NOT PRINTER-READY: no printer profile, so a generic preamble follows"
                )?;
                if let Some(bed) = s.bed_temperature_initial_layer.or(s.bed_temperature) {
                    writeln!(self.out, "M140 S{bed:.0}")?;
                }
                writeln!(self.out, "M104 S{:.0}", s.nozzle_temperature_initial_layer)?;
                writeln!(self.out, "G28")?;
                if let Some(bed) = s.bed_temperature_initial_layer.or(s.bed_temperature) {
                    writeln!(self.out, "M190 S{bed:.0}")?;
                }
                writeln!(self.out, "M109 S{:.0}", s.nozzle_temperature_initial_layer)?;
            }
        }
        // Absolute positions, millimetres, relative extrusion: what every
        // move below assumes, whatever the start G-code left set. Written
        // as the reference writes it, so the two files can be compared.
        writeln!(
            self.out,
            "G90\nG21\nM83 ; use relative distances for extrusion"
        )?;
        if s.machine_start_gcode.is_none() {
            writeln!(self.out, "G92 E0")?;
        }
        if let Some(template) = &s.filament_start_gcode {
            self.custom("filament_start_gcode", template)?;
        }
        if let Some(advance) = s.pressure_advance {
            match s.klipper {
                true => writeln!(
                    self.out,
                    "SET_PRESSURE_ADVANCE ADVANCE={advance}; Override pressure advance value"
                )?,
                false => writeln!(self.out, "M900 K{advance}; Override pressure advance value")?,
            }
        }
        // Select the tool the print starts with, as the reference does.
        if s.machine_start_gcode.is_some() {
            writeln!(self.out, "T0")?;
        }
        Ok(())
    }

    /// Writes a profile's template, filled in, ending on a line break.
    fn custom(&mut self, name: &'static str, template: &str) -> Result<(), GcodeError> {
        let text = edge_template::expand(template, &self.variables)
            .map_err(|source| GcodeError::Template { name, source })?;
        self.out.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            writeln!(self.out)?;
        }
        Ok(())
    }

    fn layer(&mut self, index: usize, layer: &Layer<'_>) -> Result<(), GcodeError> {
        let s = self.settings;
        writeln!(
            self.out,
            ";LAYER_CHANGE\n;Z:{:.3}\n;HEIGHT:{:.3}",
            layer.z, layer.height
        )?;
        self.variables.set("layer_num", Value::Int(index as i64));
        self.variables.set("layer_z", Value::Float(layer.z));
        if let Some(template) = &s.before_layer_change_gcode {
            self.custom("before_layer_change_gcode", template)?;
        }
        if index == 1 {
            if s.nozzle_temperature != s.nozzle_temperature_initial_layer {
                writeln!(self.out, "M104 S{:.0}", s.nozzle_temperature)?;
            }
            if let Some(bed) = s
                .bed_temperature
                .filter(|bed| Some(*bed) != s.bed_temperature_initial_layer)
            {
                writeln!(self.out, "M140 S{bed:.0}")?;
            }
        }
        // Stop oozing onto the finished layer while the nozzle rises.
        self.retract()?;
        if let Some(template) = &s.layer_change_gcode {
            self.custom("layer_change_gcode", template)?;
        }

        self.first_layer = index == 0;
        let fan = (self.fan_percent(index, layer).clamp(0.0, 100.0) * 2.55).round() as u8;
        if self.fan != Some(fan) {
            writeln!(self.out, "M106 S{fan}")?;
            self.fan = Some(fan);
        }

        // Rise above the new layer before moving to its first point, so the
        // nozzle clears what is printed and, on the first layer, whatever
        // the start G-code laid down. It comes down when extrusion starts.
        self.z = layer.z;
        self.lifted = s.z_hop > 0.0;
        self.feed(s.z_speed);
        let feed_word = self.feed_word();
        writeln!(self.out, "G1 Z{:.3}{feed_word}", layer.z + s.z_hop.max(0.0))?;

        if s.infill_first {
            self.fills(layer)?;
            self.walls(layer)?;
        } else {
            self.walls(layer)?;
            self.fills(layer)?;
        }
        Ok(())
    }

    /// Fan speed for a layer, as a percentage: off at the start, then
    /// faster the less time the layer takes and so the less it cools by
    /// itself. The reference also slows short layers down; this does not.
    fn fan_percent(&self, index: usize, layer: &Layer<'_>) -> f64 {
        let s = self.settings;
        if index < s.close_fan_the_first_x_layers {
            return 0.0;
        }
        let first_layer = index == 0;
        let walls = layer
            .loops
            .iter()
            .map(|wall| wall.length_mm() / self.motion(wall.role, &wall.flow, first_layer).speed);
        let fills = layer
            .paths
            .iter()
            .map(|path| path.length_mm() / self.motion(path.role, &path.flow, first_layer).speed);
        let seconds: f64 = walls.chain(fills).sum();
        if seconds >= s.fan_cooling_layer_time {
            s.fan_min_speed
        } else if seconds > s.slow_down_layer_time {
            let share = (seconds - s.slow_down_layer_time)
                / (s.fan_cooling_layer_time - s.slow_down_layer_time);
            s.fan_max_speed - (s.fan_max_speed - s.fan_min_speed) * share
        } else {
            s.fan_max_speed
        }
    }

    /// Islands nearest first; within an island, walls in the configured order.
    fn walls(&mut self, layer: &Layer<'_>) -> Result<(), GcodeError> {
        let mut remaining: Vec<&Range<usize>> = layer.islands.iter().collect();
        while !remaining.is_empty() {
            let nearest = self.nearest(remaining.iter().map(|island| {
                layer.loops[(*island).clone()]
                    .iter()
                    .flat_map(|wall| &wall.polygon.points)
                    .copied()
            }));
            let island = remaining.remove(nearest);
            let mut walls: Vec<&ExtrusionLoop> = layer.loops[island.clone()].iter().collect();
            // Stable, so each depth keeps its outer boundaries before its holes.
            match self.settings.wall_order {
                WallOrder::InnerOuter => walls.sort_by_key(|wall| std::cmp::Reverse(wall.depth)),
                WallOrder::OuterInner => walls.sort_by_key(|wall| wall.depth),
            }
            for wall in walls {
                let points = &wall.polygon.points;
                if points.len() < 3 {
                    continue;
                }
                // Start at the corner nearest the nozzle and go once round.
                let start = self.nearest(points.iter().map(|point| std::iter::once(*point)));
                self.travel(points[start], false)?;
                let motion = self.motion(wall.role, &wall.flow, self.first_layer);
                for step in 1..=points.len() {
                    self.extrude(
                        points[(start + step) % points.len()],
                        wall.role,
                        &wall.flow,
                        motion,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Fill lines kind by kind as they come, each kind in nearest-end-first
    /// order so the nozzle works across an area instead of jumping about.
    fn fills(&mut self, layer: &Layer<'_>) -> Result<(), GcodeError> {
        for group in layer.paths.chunk_by(|a, b| a.role == b.role) {
            let mut remaining: Vec<&ExtrusionPath> =
                group.iter().filter(|path| path.points.len() >= 2).collect();
            while !remaining.is_empty() {
                let ends =
                    |path: &ExtrusionPath| [path.points[0], path.points[path.points.len() - 1]];
                let nearest = self.nearest(remaining.iter().map(|path| ends(path).into_iter()));
                let path = remaining.remove(nearest);
                let [head, tail] = ends(path);
                let reversed = self
                    .position
                    .is_some_and(|at| distance(at, tail) < distance(at, head));
                let mut points: Vec<Point2> = path.points.clone();
                if reversed {
                    points.reverse();
                }
                let sheltered = self.settings.reduce_infill_retraction
                    && self
                        .position
                        .is_some_and(|at| inside::stays_inside(layer.infill_area, at, points[0]));
                self.travel(points[0], sheltered)?;
                let motion = self.motion(path.role, &path.flow, self.first_layer);
                for &point in &points[1..] {
                    self.extrude(point, path.role, &path.flow, motion)?;
                }
            }
        }
        Ok(())
    }

    /// Index of the candidate with a point closest to the nozzle; the first
    /// when the nozzle has not been anywhere yet.
    fn nearest<P: Iterator<Item = Point2>>(&self, candidates: impl Iterator<Item = P>) -> usize {
        let Some(at) = self.position else { return 0 };
        candidates
            .map(|points| {
                points
                    .map(|point| distance(at, point))
                    .fold(f64::INFINITY, f64::min)
            })
            .enumerate()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0, |(index, _)| index)
    }

    /// Speed and acceleration for a line. The speed is held to what the hot
    /// end can melt.
    fn motion(&self, role: Role, flow: &Flow, first_layer: bool) -> Motion {
        let s = self.settings;
        let is_wall = matches!(role, Role::ExternalPerimeter | Role::Perimeter);
        let wanted = match role {
            _ if first_layer && is_wall => s.initial_layer_speed,
            _ if first_layer => s.initial_layer_infill_speed,
            Role::ExternalPerimeter => s.outer_wall_speed,
            Role::Perimeter => s.inner_wall_speed,
            Role::SparseInfill => s.sparse_infill_speed,
            Role::SolidInfill | Role::BottomSurface => s.internal_solid_infill_speed,
            Role::TopSolidInfill => s.top_surface_speed,
            Role::BridgeInfill => s.bridge_speed,
        };
        let speed = if s.filament_max_volumetric_speed > 0.0 {
            wanted.min(s.filament_max_volumetric_speed / flow.mm3_per_mm())
        } else {
            wanted
        };
        let own = match role {
            _ if first_layer && s.initial_layer_acceleration > 0.0 => s.initial_layer_acceleration,
            Role::ExternalPerimeter => s.outer_wall_acceleration,
            Role::Perimeter => s.inner_wall_acceleration,
            Role::TopSolidInfill => s.top_surface_acceleration,
            Role::SparseInfill => s.sparse_infill_acceleration.resolve(s.default_acceleration),
            Role::SolidInfill | Role::BottomSurface | Role::BridgeInfill => s
                .internal_solid_infill_acceleration
                .resolve(s.default_acceleration),
        };
        Motion {
            speed,
            acceleration: if own > 0.0 {
                own
            } else {
                s.default_acceleration
            },
        }
    }

    /// Tells the printer a new acceleration limit, if it is a change.
    fn accelerate(&mut self, acceleration: f64) -> io::Result<()> {
        if acceleration <= 0.0 || acceleration == self.acceleration {
            return Ok(());
        }
        self.acceleration = acceleration;
        match (self.settings.klipper, self.settings.accel_to_decel_factor) {
            (true, Some(factor)) => writeln!(
                self.out,
                "SET_VELOCITY_LIMIT ACCEL={acceleration:.0} ACCEL_TO_DECEL={:.0}",
                acceleration * factor
            ),
            (true, None) => writeln!(self.out, "SET_VELOCITY_LIMIT ACCEL={acceleration:.0}"),
            (false, _) => writeln!(self.out, "M204 S{acceleration:.0}"),
        }
    }

    /// `sheltered` says the move may go unretracted however long it is.
    fn travel(&mut self, to: Point2, sheltered: bool) -> io::Result<()> {
        let s = self.settings;
        let length = self
            .position
            .map_or(f64::INFINITY, |from| distance(from, to));
        if length == 0.0 {
            return Ok(());
        }
        if length > s.retraction_minimum_travel && !sheltered {
            self.retract()?;
            if s.z_hop > 0.0 && !self.lifted {
                self.feed(s.z_speed);
                let feed_word = self.feed_word();
                writeln!(self.out, "G1 Z{:.3}{feed_word}", self.z + s.z_hop)?;
                self.lifted = true;
            }
        }
        // The first layer is gentle throughout. Above it, long moves get the
        // travel acceleration; short hops between neighbouring lines keep
        // whatever is set, rather than switching twice for a millimetre.
        if self.first_layer && s.initial_layer_acceleration > 0.0 {
            self.accelerate(s.initial_layer_acceleration)?;
        } else if length > s.retraction_minimum_travel && !sheltered {
            let travel = if s.travel_acceleration > 0.0 {
                s.travel_acceleration
            } else {
                s.default_acceleration
            };
            self.accelerate(travel)?;
        }
        self.feed(s.travel_speed);
        let feed_word = self.feed_word();
        writeln!(
            self.out,
            "G1 X{:.3} Y{:.3}{feed_word}",
            unscaled(to.x),
            unscaled(to.y)
        )?;
        // The very first move starts from wherever the start G-code left
        // the nozzle, which is not known, so it is not counted.
        if length.is_finite() {
            self.summary.travel_mm += length;
            self.summary.seconds += length / s.travel_speed;
        }
        self.position = Some(to);
        Ok(())
    }

    fn extrude(&mut self, to: Point2, role: Role, flow: &Flow, motion: Motion) -> io::Result<()> {
        let from = self
            .position
            .expect("a travel always comes before extruding");
        let length = distance(from, to);
        if length == 0.0 {
            return Ok(());
        }
        self.prime()?;
        if self.role != Some(role) {
            writeln!(self.out, ";TYPE:{}", role_name(role))?;
            self.role = Some(role);
        }
        if self.width != flow.width {
            writeln!(self.out, ";WIDTH:{:.3}", flow.width)?;
            self.width = flow.width;
        }
        self.accelerate(motion.acceleration)?;
        let volume = length
            * flow.mm3_per_mm()
            * self.settings.filament_flow_ratio
            * self.settings.print_flow_ratio;
        let feed = volume / self.filament_area;
        self.feed(motion.speed);
        let feed_word = self.feed_word();
        writeln!(
            self.out,
            "G1 X{:.3} Y{:.3} E{feed:.5}{feed_word}",
            unscaled(to.x),
            unscaled(to.y)
        )?;
        self.primed = true;
        self.summary.filament_mm += feed;
        self.summary.filament_mm3 += volume;
        self.summary.seconds += length / motion.speed;
        self.position = Some(to);
        Ok(())
    }

    fn retract(&mut self) -> io::Result<()> {
        let s = self.settings;
        if self.retracted || !self.primed || s.retraction_length <= 0.0 {
            return Ok(());
        }
        self.feed(s.retraction_speed);
        let feed_word = self.feed_word();
        writeln!(self.out, "G1 E-{:.5}{feed_word}", s.retraction_length)?;
        self.retracted = true;
        self.summary.retractions += 1;
        self.summary.seconds += s.retraction_length / s.retraction_speed;
        Ok(())
    }

    /// Undoes a lift and a retraction, ready to extrude.
    fn prime(&mut self) -> io::Result<()> {
        let s = self.settings;
        if self.lifted {
            self.feed(s.z_speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 Z{:.3}{feed_word}", self.z)?;
            self.lifted = false;
        }
        if self.retracted {
            let speed = if s.deretraction_speed > 0.0 {
                s.deretraction_speed
            } else {
                s.retraction_speed
            };
            self.feed(speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 E{:.5}{feed_word}", s.retraction_length)?;
            self.retracted = false;
            self.summary.seconds += s.retraction_length / speed;
        }
        Ok(())
    }

    /// Makes `speed` (mm/s) the feedrate of the move about to be written.
    /// `feed_word` then yields the `F` word once, the first time it changes.
    fn feed(&mut self, speed: f64) {
        let feedrate = (speed * 60.0).round();
        if feedrate != self.feedrate.abs() {
            // Negative marks a feedrate not yet written.
            self.feedrate = -feedrate;
        }
    }

    fn feed_word(&mut self) -> String {
        if self.feedrate < 0.0 {
            self.feedrate = -self.feedrate;
            format!(" F{:.0}", self.feedrate)
        } else {
            String::new()
        }
    }

    fn finish(mut self) -> Result<Summary, GcodeError> {
        let s = self.settings;
        self.retract()?;
        if self.fan.is_some_and(|fan| fan > 0) {
            writeln!(self.out, "M106 S0")?;
        }
        if s.filament_end_gcode.is_some() || s.machine_end_gcode.is_some() {
            writeln!(self.out, ";TYPE:Custom")?;
        }
        if let Some(template) = &s.filament_end_gcode {
            self.custom("filament_end_gcode", template)?;
        }
        match &s.machine_end_gcode {
            Some(template) => self.custom("machine_end_gcode", template)?,
            None => writeln!(self.out, "M104 S0\nM140 S0\nM84")?,
        }
        let summary = self.summary;
        writeln!(
            self.out,
            "; filament used [mm] = {:.2}",
            summary.filament_mm
        )?;
        writeln!(
            self.out,
            "; filament used [cm3] = {:.2}",
            summary.filament_mm3 / 1000.0
        )?;
        if let Some(density) = s.filament_density {
            writeln!(
                self.out,
                "; filament used [g] = {:.2}",
                summary.filament_mm3 / 1000.0 * density
            )?;
        }
        writeln!(
            self.out,
            "; time at commanded speeds, without acceleration [s] = {:.0}",
            summary.seconds
        )?;
        Ok(summary)
    }
}

fn distance(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// Feature names as the reference writes them (`ExtrusionEntity::role_to_string`),
/// which is what G-code viewers colour by.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::ExternalPerimeter => "Outer wall",
        Role::Perimeter => "Inner wall",
        Role::SparseInfill => "Sparse infill",
        Role::SolidInfill => "Internal solid infill",
        Role::TopSolidInfill => "Top surface",
        Role::BottomSurface => "Bottom surface",
        Role::BridgeInfill => "Bridge",
    }
}

#[cfg(test)]
mod tests;
