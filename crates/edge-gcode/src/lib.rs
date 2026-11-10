//! Turns each layer's walls and fill into G-code: the order things are
//! printed in, where each wall starts, the travel between them, retraction,
//! wipe and lift, fan and acceleration, how much filament every move
//! pushes, and how much a short layer is slowed so it can cool.
//!
//! With a printer profile, the machine's own start, end and layer-change
//! G-code is expanded from its templates and the output is meant to print.
//! It has not been run on a printer. Without a profile a generic preamble
//! stands in and the output is only good for viewing.
//!
//! Not done: seams placed by visibility, arc fitting, progress (`M73`)
//! and object labels.

mod cooling;
mod inside;
mod overhang;
mod seam;
mod settings;

use std::{
    f64::consts::{PI, TAU},
    io::{self, Write},
    ops::Range,
};

use cooling::{Slowdown, Timing};
use edge_config::FloatOrPercent;
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, scaled, unscaled};
use edge_template::{TemplateError, Value, Variables};

pub use seam::Seam;
pub use settings::{Lift, OverhangFan, PrintSettings, WallOrder};

#[derive(Debug, thiserror::Error)]
pub enum GcodeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The print uses a filament no settings were given for.
    #[error("filament {filament} is used but settings were given for {given}")]
    NoSettings { filament: usize, given: usize },
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
    /// Which filament prints this, counting from 0. A layer printed in
    /// several is given once for each, one after another at the same
    /// height, in the order they are to be printed.
    pub filament: usize,
    /// Prime tower lines, printed before anything else.
    pub tower: &'a [ExtrusionPath],
    /// Loops laid before the part on this layer, in order: skirt and brim.
    pub around: &'a [ExtrusionLoop],
    /// Support lines, printed before the part so it has them to rest on.
    pub support: &'a [ExtrusionPath],
    pub loops: &'a [ExtrusionLoop],
    /// Which of `loops` make up each island.
    pub islands: &'a [Range<usize>],
    pub paths: &'a [ExtrusionPath],
    /// The area inside the walls. Moves between fill lines that stay
    /// within it can skip retraction, since any ooze lands where fill goes.
    pub infill_area: &'a [ExPolygon],
    /// The outline of everything printed on the layer beneath, which
    /// walls are measured against to find where they overhang. Empty on
    /// the first layer, or to leave overhangs alone.
    pub below: &'a [ExPolygon],
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
    /// Layers slowed down to give them time to cool.
    pub slowed_layers: usize,
    pub tool_changes: usize,
}

/// Writes the whole print in one filament and reports what it used.
pub fn write(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    settings: &PrintSettings,
) -> Result<Summary, GcodeError> {
    write_tools(out, layers, std::slice::from_ref(settings))
}

/// Writes the whole print and reports what it used. `settings` has an
/// entry for each filament, which [`Layer::filament`] indexes.
pub fn write_tools(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    settings: &[PrintSettings],
) -> Result<Summary, GcodeError> {
    let first = layers.first().map_or(0, |layer| layer.filament);
    if let Some(layer) = layers.iter().find(|layer| layer.filament >= settings.len()) {
        return Err(GcodeError::NoSettings {
            filament: layer.filament + 1,
            given: settings.len(),
        });
    }
    // One physical layer is every entry at the same height.
    let mut groups: Vec<Range<usize>> = Vec::new();
    for (index, layer) in layers.iter().enumerate() {
        match groups.last_mut() {
            Some(group) if layers[group.start].z == layer.z => group.end = index + 1,
            _ => groups.push(index..index + 1),
        }
    }
    // Where each filament is used for the last time, after which its tool
    // can be switched off.
    let mut last_use = vec![0; settings.len()];
    for (index, layer) in layers.iter().enumerate() {
        last_use[layer.filament] = index;
    }

    let mut variables = settings[first].variables.clone();
    variables.set("total_layer_count", Value::Int(groups.len() as i64));
    variables.set(
        "max_layer_z",
        Value::Float(layers.last().map_or(0.0, |layer| layer.z)),
    );
    variables.set("layer_num", Value::Int(0));
    variables.set("layer_z", Value::Float(0.0));
    for name in [
        "initial_extruder",
        "initial_tool",
        "initial_no_support_extruder",
        "current_extruder",
    ] {
        variables.set(name, Value::Int(first as i64));
    }
    // Where the first layer is, for start G-code that probes or purges near it.
    // "First" being the first layer with anything on it.
    if let Some([min_x, min_y, max_x, max_y]) = layers.iter().find_map(extent) {
        let pair = |x: f64, y: f64| vec![Value::Float(x), Value::Float(y)];
        variables.set_list("first_layer_print_min", pair(min_x, min_y));
        variables.set_list("first_layer_print_max", pair(max_x, max_y));
        variables.set_list("first_layer_print_size", pair(max_x - min_x, max_y - min_y));
    }

    let mut writer = Writer {
        out: Vec::new(),
        all: settings,
        settings: &settings[first],
        variables,
        filament_area: PI * (settings[first].filament_diameter / 2.0).powi(2),
        state: State {
            tool: first,
            parked: vec![(false, 0.0); settings.len()],
            used: vec![(0.0, 0.0); settings.len()],
            ..State::default()
        },
        timing: Timing::default(),
        extruding: false,
        slowdown: Slowdown::None,
        last_use,
        marks: Vec::new(),
        file: FileInfo {
            layers: groups.len(),
            top: layers.last().map_or(0.0, |layer| layer.z),
            first_layer_height: layers.first().map_or(0.0, |layer| layer.height),
            filaments: {
                let mut used: Vec<usize> = layers.iter().map(|layer| layer.filament + 1).collect();
                used.sort_unstable();
                used.dedup();
                used
            },
            first_layer_seconds: 0.0,
        },
        below: None,
        measured: Vec::new(),
        measured_used: 0,
    };
    let several = layers.iter().any(|layer| layer.filament > 0);
    writer.preamble(several)?;
    out.write_all(&writer.out)?;
    for (index, group) in groups.iter().enumerate() {
        writer.out.clear();
        // The tool the next layer starts with, to be warmed up in time.
        let after = groups
            .get(index + 1)
            .map(|next| layers[next.start].filament);
        writer.layer(index, &layers[group.clone()], group.start, after)?;
        if index == 0 {
            writer.file.first_layer_seconds = writer.state.summary.seconds;
        }
        out.write_all(&writer.out)?;
    }
    writer.out.clear();
    let summary = writer.finish()?;
    out.write_all(&writer.out)?;
    Ok(summary)
}

/// The box around everything a layer extrudes, in mm: min x, min y, max x, max y.
fn extent(layer: &Layer<'_>) -> Option<[f64; 4]> {
    let walls = layer
        .around
        .iter()
        .chain(layer.loops)
        .flat_map(|wall| &wall.polygon.points);
    let fills = layer
        .tower
        .iter()
        .chain(layer.support)
        .chain(layer.paths)
        .flat_map(|path| &path.points);
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

struct Writer<'a> {
    /// The text of the part being written; handed on a layer at a time.
    out: Vec<u8>,
    /// The settings for each filament, and those of the one in use.
    all: &'a [PrintSettings],
    settings: &'a PrintSettings,
    /// What templates can refer to; the layer's number and height change as it goes.
    variables: Variables,
    /// Cross-section of the filament in mm².
    filament_area: f64,
    state: State,
    /// How the layer being written spends its time.
    timing: Timing,
    /// Whether the layer has started extruding; time before that is not the layer's.
    extruding: bool,
    /// What the layer's extrusion speeds are held to.
    slowdown: Slowdown,
    /// For each filament, the last entry of the print that uses it.
    last_use: Vec<usize>,
    /// Places in the text of the stretch one tool is printing, with the
    /// time each is reached, where a line can be slipped in to warm the
    /// next tool.
    marks: Vec<(usize, f64)>,
    /// What the file's first and last lines say about the print.
    file: FileInfo,
    /// The outline of the layer beneath the one being written.
    below: Option<overhang::Below>,
    /// Walls measured against it so far this layer, each with what tells
    /// it apart, and how many of them the pass being written has used: a
    /// layer laid out a second time, slower, measures nothing again.
    measured: Vec<(Measured, Vec<overhang::Piece>)>,
    measured_used: usize,
}

/// What the file says about the print outside its G-code.
#[derive(Clone, Debug, Default)]
struct FileInfo {
    layers: usize,
    /// Height of the top of the print, and of its first layer, in mm.
    top: f64,
    first_layer_height: f64,
    /// The filaments used, counting from 1.
    filaments: Vec<usize>,
    /// How long the first layer takes, in seconds.
    first_layer_seconds: f64,
}

/// What tells one measured wall from another: where it starts and ends,
/// how many points it has, its kind and its width.
type Measured = (Point2, Point2, usize, Role, u64);

/// Where the machine stands as the text is written. Kept apart so a layer
/// can be laid out once to see how long it takes and then again, slower.
#[derive(Clone, Debug, Default)]
struct State {
    /// Where the nozzle is, once it has been anywhere.
    position: Option<Point2>,
    /// The current layer's height above the bed.
    z: f64,
    /// How high the nozzle actually is, once a move has said.
    height: Option<f64>,
    first_layer: bool,
    /// The tool in use, and what was left retracted in each tool put away.
    tool: usize,
    parked: Vec<(bool, f64)>,
    tool_changes: usize,
    /// What each filament has had extruded so far, in mm and in mm³.
    used: Vec<(f64, f64)>,
    retracted: bool,
    /// How far the filament is pulled back while `retracted`.
    retracted_length: f64,
    /// Whether the nozzle holds molten plastic that would ooze on a travel:
    /// true once anything is extruded, or once the machine's start G-code
    /// has run, since that purges.
    primed: bool,
    /// A lift decided on at retraction, to be made by the travel that follows.
    pending_lift: Option<Lift>,
    /// What was just printed, in the order the wipe retraces it.
    wipe_path: Vec<Point2>,
    /// Speed of the line just printed, before any slowdown, in mm/s.
    line_speed: f64,
    /// The last feedrate written, in mm/min.
    feedrate: f64,
    /// The last acceleration written, in mm/s².
    acceleration: f64,
    /// The last fan speeds written, 0 to 255.
    fan: Option<u8>,
    /// Whether the fan has been asked to run for an overhang or a bridge.
    over_air: bool,
    auxiliary_fan: Option<u8>,
    role: Option<Role>,
    width: f64,
    /// Seams of the layer below, and of this one so far.
    seams: Vec<Point2>,
    next_seams: Vec<Point2>,
    summary: Summary,
}

impl Writer<'_> {
    fn preamble(&mut self, several: bool) -> Result<(), GcodeError> {
        let s = self.settings;
        let mut started = String::new();
        // The file is laid out as the reference lays its own out
        // (GCode.cpp `_do_export`), since printers and their hosts read
        // it by that layout. The first line names OrcaSlicer because the
        // U1's host accepts files only from a fixed list of slicers, and
        // the reference names itself the same way for the same reason
        // (utils.cpp `header_gcode_producer`).
        writeln!(self.out, "; HEADER_BLOCK_START")?;
        write!(
            self.out,
            "; generated by OrcaSlicer {} (EdgeSlicerRS)",
            env!("CARGO_PKG_VERSION")
        )?;
        match &s.produced_at {
            Some(when) => writeln!(self.out, " on {when}")?,
            None => writeln!(self.out)?,
        }
        writeln!(self.out, "; total layer number: {}", self.file.layers)?;
        for key in ["filament_density", "filament_diameter"] {
            if let Some((_, value)) = s.config.iter().find(|(name, _)| name == key) {
                writeln!(self.out, "; {key}: {value}")?;
            }
        }
        writeln!(self.out, "; max_z_height: {:.2}", self.file.top)?;
        let filaments: Vec<String> = self.file.filaments.iter().map(usize::to_string).collect();
        writeln!(self.out, "; filament: {}", filaments.join(","))?;
        writeln!(self.out, "; support_material_on_wipe_tower: 0")?;
        writeln!(self.out, "; HEADER_BLOCK_END\n")?;
        writeln!(self.out, "; EXECUTABLE_BLOCK_START")?;
        // Both fans off before anything heats, as the reference does when
        // the printer has an auxiliary fan.
        if s.auxiliary_fan && s.close_fan_the_first_x_layers > 0 {
            writeln!(self.out, "M106 S0\nM106 P2 S0")?;
        }
        match &s.machine_start_gcode {
            Some(template) => {
                writeln!(self.out, ";TYPE:Custom")?;
                started = self.custom("machine_start_gcode", template)?;
                self.state.primed = true;
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
        if several && s.machine_start_gcode.is_some() {
            // A print in more than one filament starts by going through
            // the printer's tool-change G-code for its first tool, from
            // whichever tool the start G-code selected last.
            let from = started
                .lines()
                .rev()
                .find_map(|line| line.trim().strip_prefix('T')?.parse::<i64>().ok());
            self.swap(from, self.state.tool, None, 0.0)?;
            // The reference leaves this marker of its own here.
            writeln!(self.out, ";_FORCE_RESUME_FAN_SPEED")?;
            self.announce(self.state.tool, true)?;
            return Ok(());
        }
        if let Some(template) = &s.filament_start_gcode {
            self.custom("filament_start_gcode", template)?;
        }
        self.advance()?;
        // Select the tool the print starts with, as the reference does.
        if s.machine_start_gcode.is_some() {
            writeln!(self.out, "T0")?;
        }
        Ok(())
    }

    /// Sets the filament's pressure advance, when the profile gives one.
    fn advance(&mut self) -> io::Result<()> {
        let s = self.settings;
        if let Some(advance) = s.pressure_advance {
            match s.klipper {
                true => writeln!(
                    self.out,
                    "SET_PRESSURE_ADVANCE ADVANCE={advance}; Override pressure advance value"
                )?,
                false => writeln!(self.out, "M900 K{advance}; Override pressure advance value")?,
            }
        }
        Ok(())
    }

    /// Writes a profile's template, filled in, ending on a line break, and
    /// gives back what was written.
    fn custom(&mut self, name: &'static str, template: &str) -> Result<String, GcodeError> {
        let text = edge_template::expand(template, &self.variables)
            .map_err(|source| GcodeError::Template { name, source })?;
        self.out.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            writeln!(self.out)?;
        }
        Ok(text)
    }

    /// The temperature a tool prints at on the layer being written.
    fn temperature(&self, tool: usize, first_layer: bool) -> f64 {
        let s = &self.all[tool];
        if first_layer || s.nozzle_temperature <= 0.0 {
            s.nozzle_temperature_initial_layer
        } else {
            s.nozzle_temperature
        }
    }

    /// Runs the printer's tool-change G-code from one tool to another.
    /// `layer` is the layer being printed, or none before the first.
    /// Afterwards nothing is known of where the nozzle is.
    fn swap(
        &mut self,
        from: Option<i64>,
        to: usize,
        layer: Option<usize>,
        z: f64,
    ) -> Result<(), GcodeError> {
        let first_layer = layer.is_none_or(|layer| layer == 0);
        let old = from.and_then(|tool| usize::try_from(tool).ok());
        let settings_of = |tool: Option<usize>| tool.and_then(|tool| self.all.get(tool));
        let (old_settings, new_settings) = (settings_of(old), &self.all[to]);
        let area = |s: &PrintSettings| PI * (s.filament_diameter / 2.0).powi(2);
        // How fast a filament can be fed, in mm/min, as the reference
        // offers it to templates.
        let feedrate = |s: Option<&PrintSettings>, missing: i64| match s {
            Some(s) if s.filament_max_volumetric_speed > 0.0 => {
                (60.0 * s.filament_max_volumetric_speed / area(s)) as i64
            }
            Some(_) => 100,
            None => missing,
        };
        let whole = |value: f64| Value::Int(value.round() as i64);
        let position = self
            .state
            .position
            .map_or((0.0, 0.0), |at| (unscaled(at.x), unscaled(at.y)));
        let height = self.state.height.unwrap_or(z);
        let count = self.state.tool_changes as i64 + 1;
        let v = &mut self.variables;
        v.set("previous_extruder", Value::Int(from.unwrap_or(-1)));
        v.set("next_extruder", Value::Int(to as i64));
        v.set("current_filament_id", Value::Int(from.unwrap_or(-1)));
        v.set("next_filament_id", Value::Int(to as i64));
        v.set("current_hotend", Value::Int(-1));
        v.set("next_hotend", Value::Int(-1));
        v.set(
            "layer_num",
            Value::Int(layer.map_or(-1, |layer| layer as i64)),
        );
        v.set("layer_z", Value::Float(z));
        v.set("toolchange_z", Value::Float(z));
        v.set("relative_e_axis", Value::Bool(true));
        v.set("toolchange_count", Value::Int(count));
        v.set("fan_speed", Value::Int(0));
        v.set(
            "old_retract_length",
            Value::Float(old_settings.map_or(0.0, |s| s.retraction_length)),
        );
        v.set(
            "new_retract_length",
            Value::Float(new_settings.retraction_length),
        );
        v.set(
            "old_retract_length_toolchange",
            Value::Float(old_settings.map_or(0.0, |s| s.retract_length_toolchange)),
        );
        v.set(
            "new_retract_length_toolchange",
            Value::Float(new_settings.retract_length_toolchange),
        );
        let temperature = |s: &PrintSettings| {
            if first_layer {
                s.nozzle_temperature_initial_layer
            } else {
                s.nozzle_temperature
            }
        };
        v.set(
            "old_filament_temp",
            whole(old_settings.map_or(0.0, temperature)),
        );
        v.set("new_filament_temp", whole(temperature(new_settings)));
        v.set("x_after_toolchange", Value::Float(position.0));
        v.set("y_after_toolchange", Value::Float(position.1));
        v.set("z_after_toolchange", Value::Float(height));
        // Nothing is flushed between tools that each have a nozzle of their own.
        for name in [
            "flush_length",
            "first_flush_volume",
            "second_flush_volume",
            "flush_length_1",
            "flush_length_2",
            "flush_length_3",
            "flush_length_4",
            "travel_point_1_x",
            "travel_point_1_y",
            "travel_point_2_x",
            "travel_point_2_y",
            "travel_point_3_x",
            "travel_point_3_y",
            "next_wipe_x",
            "next_wipe_y",
        ] {
            v.set(name, Value::Float(0.0));
        }
        v.set(
            "old_filament_e_feedrate",
            Value::Int(feedrate(old_settings, 200)),
        );
        v.set(
            "new_filament_e_feedrate",
            Value::Int(feedrate(Some(new_settings), 100)),
        );
        v.set_list(
            "position",
            vec![
                Value::Float(position.0),
                Value::Float(position.1),
                Value::Float(height),
            ],
        );
        match &new_settings.change_filament_gcode {
            Some(template) => {
                let text =
                    edge_template::expand_mut(template, &mut self.variables).map_err(|source| {
                        GcodeError::Template {
                            name: "change_filament_gcode",
                            source,
                        }
                    })?;
                writeln!(self.out, "; custom gcode start")?;
                self.out.write_all(text.as_bytes())?;
                if !text.ends_with('\n') {
                    writeln!(self.out)?;
                }
                writeln!(self.out, "; custom gcode end")?;
                // The tool is selected by the template, or else here.
                let named = text.lines().any(|line| {
                    line.trim()
                        .strip_prefix('T')
                        .and_then(|rest| rest.split([' ', ';']).next())
                        .is_some_and(|number| number.parse() == Ok(to))
                });
                if !named {
                    writeln!(self.out, "T{to}")?;
                }
            }
            None => writeln!(self.out, "T{to}")?,
        }
        self.variables
            .set("current_extruder", Value::Int(to as i64));
        // The template moves the head as it likes: nothing is assumed
        // about where it left it, and every setting is sent again.
        self.state.position = None;
        self.state.height = None;
        self.state.pending_lift = None;
        self.state.wipe_path.clear();
        self.state.feedrate = 0.0;
        self.state.acceleration = 0.0;
        Ok(())
    }

    /// Makes a tool the one whose settings are used, and tells the printer
    /// what it needs to know about its filament.
    fn announce(&mut self, tool: usize, first_layer: bool) -> Result<(), GcodeError> {
        self.settings = &self.all[tool];
        self.filament_area = PI * (self.settings.filament_diameter / 2.0).powi(2);
        let s = self.settings;
        if let Some(template) = &s.filament_start_gcode {
            self.custom("filament_start_gcode", template)?;
        }
        let temperature = self.temperature(tool, first_layer);
        if s.tool_change_temperature_wait {
            writeln!(
                self.out,
                "M109 S{temperature:.0} T{tool} ; set nozzle temperature and wait for it to be reached"
            )?;
        } else {
            writeln!(
                self.out,
                "M104 S{temperature:.0} T{tool} ; set nozzle temperature"
            )?;
        }
        self.advance()?;
        Ok(())
    }

    /// Puts the tool in use away and takes up another, mid-print.
    fn change_tool(
        &mut self,
        to: usize,
        layer: usize,
        z: f64,
        cursor: usize,
    ) -> Result<(), GcodeError> {
        let from = self.state.tool;
        let s = self.settings;
        // Pull the filament well back, wiping first as for any retraction.
        if self.state.primed && s.retract_length_toolchange > 0.0 {
            self.retract(false)?;
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
        if let Some(template) = &s.filament_end_gcode {
            self.custom("filament_end_gcode", template)?;
        }
        // A tool waiting its turn is kept cooler so it does not ooze; one
        // that is not needed again is switched off.
        if let Some(delta) = s.standby_temperature_delta {
            let standby = if self.last_use[from] < cursor {
                0.0
            } else {
                (self.temperature(from, layer == 0) + delta).max(0.0)
            };
            writeln!(
                self.out,
                "M104 S{standby:.0} T{from} ; set nozzle temperature ;cooldown"
            )?;
        }
        self.swap(Some(from as i64), to, Some(layer), z)?;
        self.state.parked[from] = (self.state.retracted, self.state.retracted_length);
        (self.state.retracted, self.state.retracted_length) = self.state.parked[to];
        self.state.tool = to;
        self.state.tool_changes += 1;
        self.state.summary.tool_changes += 1;
        // The template may have touched the fans.
        if let Some(speed) = self.state.auxiliary_fan {
            writeln!(self.out, "M106 P2 S{speed}")?;
        }
        if let Some(speed) = self.state.fan {
            writeln!(self.out, "M106 S{speed}")?;
        }
        self.announce(to, layer == 0)
    }

    /// Slips in a line that starts heating the tool that is up next, far
    /// enough before the end of the stretch just written for it to be
    /// warm in time: at the last mark that leaves `preheat_time`, or at
    /// the start of the stretch if it is shorter than that.
    fn preheat(&mut self, next: usize, layer: usize, start: (usize, f64)) {
        let s = &self.all[next];
        if s.standby_temperature_delta.is_none() || s.preheat_time <= 0.0 {
            return;
        }
        let end = self.state.summary.seconds;
        let (at, when) = self
            .marks
            .iter()
            .rev()
            .find(|(_, when)| *when <= end - s.preheat_time)
            .copied()
            .unwrap_or(start);
        let temperature = self.temperature(next, layer == 0);
        let line = format!(
            "M104 S{temperature:.0} T{next} ; preheat T{next} time: {:.0}s\n",
            end - when
        );
        self.out.splice(at..at, line.into_bytes());
    }

    /// One physical layer: `parts` is what each filament prints on it, in
    /// order, `cursor` where the first of them comes in the whole print,
    /// and `after` the filament the next layer starts with.
    fn layer(
        &mut self,
        index: usize,
        parts: &[Layer<'_>],
        cursor: usize,
        after: Option<usize>,
    ) -> Result<(), GcodeError> {
        let s = self.settings;
        let layer = &parts[0];
        writeln!(
            self.out,
            ";LAYER_CHANGE\n;Z:{:.3}\n;HEIGHT:{:.3}",
            layer.z, layer.height
        )?;
        // The reference counts layers from 1 before the change and from 0 after it.
        self.variables
            .set("layer_num", Value::Int(index as i64 + 1));
        self.variables.set("layer_z", Value::Float(layer.z));
        if let Some(template) = &s.before_layer_change_gcode {
            self.custom("before_layer_change_gcode", template)?;
        }
        // Stop oozing onto the finished layer while the nozzle moves on.
        if s.retract_when_changing_layer {
            self.retract(true)?;
        }
        self.variables.set("layer_num", Value::Int(index as i64));
        if let Some(template) = &s.layer_change_gcode {
            self.custom("layer_change_gcode", template)?;
        }
        if index == 1 {
            if s.nozzle_temperature != s.nozzle_temperature_initial_layer {
                // With several tools the one in use is named; the others
                // get their temperature when their turn comes.
                if self.all.len() > 1 {
                    writeln!(
                        self.out,
                        "M104 S{:.0} T{}",
                        s.nozzle_temperature, self.state.tool
                    )?;
                } else {
                    writeln!(self.out, "M104 S{:.0}", s.nozzle_temperature)?;
                }
            }
            if let Some(bed) = s
                .bed_temperature
                .filter(|bed| Some(*bed) != s.bed_temperature_initial_layer)
            {
                writeln!(self.out, "M140 S{bed:.0}")?;
            }
        }
        self.state.first_layer = index == 0;
        // The nozzle itself moves up with the layer's first travel.
        self.state.z = layer.z;
        self.below = if index == 0 {
            None
        } else {
            // Nothing a wall is judged by reaches further than its own
            // width from the outline beneath.
            overhang::Below::new(layer.below, (1.5 * s.nozzle_diameter).max(0.6))
        };

        // Lay the layer out to see how long it takes; if that is too short
        // for it to cool, lay it out again, slower.
        let head = self.out.len();
        let before = self.state.clone();
        self.slowdown = Slowdown::None;
        self.measured.clear();
        self.measured_used = 0;
        self.body(index, parts, cursor, after)?;
        if s.slow_down_for_layer_cooling {
            let slowdown = self
                .timing
                .slowdown(s.slow_down_layer_time, s.slow_down_min_speed);
            if slowdown != Slowdown::None {
                self.out.truncate(head);
                self.state = before;
                self.settings = &self.all[self.state.tool];
                self.filament_area = PI * (self.settings.filament_diameter / 2.0).powi(2);
                self.slowdown = slowdown;
                self.measured_used = 0;
                self.body(index, parts, cursor, after)?;
                self.state.summary.slowed_layers += 1;
            }
        }
        let seconds = self.timing.seconds_after(self.slowdown);

        // The fans are set ahead of the layer, where the reference sets them.
        let fan = cooling::Fan {
            min_speed: s.fan_min_speed,
            max_speed: s.fan_max_speed,
            cooling_layer_time: s.fan_cooling_layer_time,
            slow_down_layer_time: s.slow_down_layer_time,
            off_layers: s.close_fan_the_first_x_layers,
            full_speed_layer: s.full_fan_speed_layer,
            keep_running: s.reduce_fan_stop_start_freq,
        };
        let mut fans = String::new();
        let speed = cooling::pwm(fan.percent(index, seconds));
        // Bridges and overhangs get their own fan speed where that is
        // more than the layer's (CoolingBuffer.cpp `overhang_fan_control`).
        let over_air = s
            .overhang_fan
            .and_then(|wanted| fan.overhang_percent(index, wanted.speed))
            .map(cooling::pwm)
            .filter(|over_air| *over_air > speed);
        self.settle_fan(speed, over_air);
        if self.state.fan != Some(speed) {
            fans.push_str(&format!("M106 S{speed}\n"));
            self.state.fan = Some(speed);
        }
        if s.auxiliary_fan {
            let speed = if index >= s.close_fan_the_first_x_layers {
                cooling::auxiliary_pwm(s.additional_cooling_fan_speed)
            } else {
                0
            };
            if self.state.auxiliary_fan != Some(speed) {
                fans.push_str(&format!("M106 P2 S{speed}\n"));
                self.state.auxiliary_fan = Some(speed);
            }
        }
        self.out.splice(0..0, fans.into_bytes());
        Ok(())
    }

    /// Everything the layer prints, each filament's part in turn.
    fn body(
        &mut self,
        index: usize,
        parts: &[Layer<'_>],
        cursor: usize,
        after: Option<usize>,
    ) -> Result<(), GcodeError> {
        self.timing = Timing::default();
        self.extruding = false;
        for (offset, layer) in parts.iter().enumerate() {
            if layer.filament != self.state.tool {
                self.change_tool(layer.filament, index, layer.z, cursor + offset)?;
            }
            let start = (self.out.len(), self.state.summary.seconds);
            self.marks.clear();
            self.lines(layer.tower, &[])?;
            for around in layer.around {
                let points = &around.polygon.points;
                if points.len() < 3 {
                    continue;
                }
                // Skirt and brim start wherever is nearest.
                let (edge, start) = match self.state.position {
                    Some(at) => seam::project(points, at),
                    None => (0, points[0]),
                };
                let mut path = seam::open_at(points, edge, start);
                seam::clip_end(&mut path, self.settings.seam_gap);
                self.path(&path, around.role, &around.flow, false, true)?;
            }
            self.lines(layer.support, &[])?;
            if self.settings.infill_first {
                self.lines(layer.paths, layer.infill_area)?;
                self.walls(layer)?;
            } else {
                self.walls(layer)?;
                self.lines(layer.paths, layer.infill_area)?;
            }
            // Whichever tool is up next starts warming before this one is done.
            let next = parts.get(offset + 1).map(|next| next.filament).or(after);
            if let Some(next) = next.filter(|next| *next != layer.filament) {
                self.preheat(next, index, start);
            }
        }
        self.fan_over_air(false)?;
        self.state.seams = std::mem::take(&mut self.state.next_seams);
        Ok(())
    }

    /// Asks for the overhang fan, or for the layer's own again. What
    /// speeds those are is only known once the layer has been timed, so
    /// a marker is written for [`Self::settle_fan`] to fill in.
    fn fan_over_air(&mut self, wanted: bool) -> io::Result<()> {
        if wanted != self.state.over_air {
            self.state.over_air = wanted;
            self.out
                .extend_from_slice(if wanted { FAN_OVER_AIR } else { FAN_AS_LAYER });
        }
        Ok(())
    }

    /// Turns the layer's fan markers into fan commands: `over_air` where
    /// an overhang starts and `layer` where it ends, or nothing when
    /// overhangs get no fan of their own on this layer.
    fn settle_fan(&mut self, layer: u8, over_air: Option<u8>) {
        let marked = |text: &[u8]| {
            text.windows(FAN_MARK.len())
                .position(|window| window == FAN_MARK)
        };
        if marked(&self.out).is_none() {
            return;
        }
        let mut settled = Vec::with_capacity(self.out.len());
        let mut rest = self.out.as_slice();
        while let Some(at) = marked(rest) {
            settled.extend_from_slice(&rest[..at]);
            let tail = &rest[at..];
            let (length, speed) = if tail.starts_with(FAN_OVER_AIR) {
                (FAN_OVER_AIR.len(), over_air)
            } else if tail.starts_with(FAN_AS_LAYER) {
                (FAN_AS_LAYER.len(), over_air.map(|_| layer))
            } else {
                settled.extend_from_slice(&tail[..FAN_MARK.len()]);
                rest = &tail[FAN_MARK.len()..];
                continue;
            };
            if let Some(speed) = speed {
                settled.extend_from_slice(format!("M106 S{speed}\n").as_bytes());
            }
            rest = &tail[length..];
        }
        settled.extend_from_slice(rest);
        self.out = settled;
    }

    /// Islands nearest first; within an island, walls in the configured order.
    fn walls(&mut self, layer: &Layer<'_>) -> io::Result<()> {
        let s = self.settings;
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
            match s.wall_order {
                WallOrder::InnerOuter => walls.sort_by_key(|wall| std::cmp::Reverse(wall.depth)),
                WallOrder::OuterInner => walls.sort_by_key(|wall| wall.depth),
            }
            // The seams of the island's outermost walls are settled first,
            // so the walls behind them can start alongside.
            let planned: Vec<(bool, Point2)> = if s.seam == Seam::Nearest {
                Vec::new()
            } else {
                walls
                    .iter()
                    .filter(|wall| wall.depth == 0 && wall.polygon.points.len() >= 3)
                    .map(|wall| {
                        let points = &wall.polygon.points;
                        let corner = seam::choose(
                            points,
                            s.seam,
                            s.nozzle_diameter,
                            None,
                            &self.state.seams,
                        );
                        (wall.is_hole, points[corner])
                    })
                    .collect()
            };
            self.state
                .next_seams
                .extend(planned.iter().map(|(_, seam)| *seam));

            for wall in walls {
                let points = &wall.polygon.points;
                if points.len() < 3 {
                    continue;
                }
                let (edge, start) = match alongside(points, wall.is_hole, &planned) {
                    Some(target) => seam::project(points, target),
                    None => {
                        let corner = seam::choose(
                            points,
                            Seam::Nearest,
                            s.nozzle_diameter,
                            self.state.position,
                            &[],
                        );
                        (corner, points[corner])
                    }
                };
                let mut path = seam::open_at(points, edge, start);
                seam::clip_end(&mut path, s.seam_gap);
                self.path(&path, wall.role, &wall.flow, false, true)?;
            }
        }
        Ok(())
    }

    /// Open lines kind by kind as they come, each kind in nearest-end-first
    /// order so the nozzle works across an area instead of jumping about.
    /// Moves that stay inside `shelter` may skip retraction.
    fn lines(&mut self, paths: &[ExtrusionPath], shelter: &[ExPolygon]) -> io::Result<()> {
        for group in paths.chunk_by(|a, b| a.role == b.role) {
            let mut remaining: Vec<&ExtrusionPath> =
                group.iter().filter(|path| path.points.len() >= 2).collect();
            while !remaining.is_empty() {
                let ends =
                    |path: &ExtrusionPath| [path.points[0], path.points[path.points.len() - 1]];
                let nearest = self.nearest(remaining.iter().map(|path| ends(path).into_iter()));
                let path = remaining.remove(nearest);
                let [head, tail] = ends(path);
                let reversed = self
                    .state
                    .position
                    .is_some_and(|at| distance(at, tail) < distance(at, head));
                let mut points: Vec<Point2> = path.points.clone();
                if reversed {
                    points.reverse();
                }
                let sheltered = self.settings.reduce_infill_retraction
                    && self
                        .state
                        .position
                        .is_some_and(|at| inside::stays_inside(shelter, at, points[0]));
                self.path(&points, path.role, &path.flow, sheltered, false)?;
            }
        }
        Ok(())
    }

    /// Travels to the start of a path and extrudes along it. `closed` says
    /// it is a loop, which a wipe carries on around; an open line is wiped
    /// back the way it came.
    fn path(
        &mut self,
        points: &[Point2],
        role: Role,
        flow: &Flow,
        sheltered: bool,
        closed: bool,
    ) -> io::Result<()> {
        self.travel(points[0], sheltered)?;
        let s = self.settings;
        let motion = self.motion(role, flow, self.state.first_layer);
        // Bridges always get the overhang fan, and with no threshold set
        // so does every outer wall (GCode.cpp `overhang_fan_on_for_role`).
        let fan = s.overhang_fan.filter(|_| !self.state.first_layer);
        let cooled = fan.is_some_and(|fan| {
            matches!(role, Role::BridgeInfill | Role::InternalBridge)
                || (fan.at.is_none() && role == Role::ExternalPerimeter)
        });
        let is_wall = matches!(role, Role::ExternalPerimeter | Role::Perimeter);
        let key: Measured = (
            points[0],
            points[points.len() - 1],
            points.len(),
            role,
            flow.width.to_bits(),
        );
        // The same wall, met in the same order, on the layer's second pass.
        let known = self
            .measured
            .get(self.measured_used)
            .filter(|(known, _)| *known == key)
            .map(|(_, pieces)| pieces.clone());
        let measure = is_wall
            && known.is_none()
            && (s.enable_overhang_speed || s.detect_overhang_wall || fan.is_some());
        let measured = measure
            .then_some(self.below.as_ref())
            .flatten()
            .map(|below| {
                // The wall's speed where it is fully supported, and where a
                // quarter, a half, three quarters and nearly all of it hangs
                // over; a bridge's where nothing is under it (GCode.cpp `_extrude`).
                let mut usual = if role == Role::ExternalPerimeter {
                    s.outer_wall_speed
                } else {
                    s.inner_wall_speed
                };
                if s.filament_max_volumetric_speed > 0.0 {
                    usual = usual.min(s.filament_max_volumetric_speed / flow.mm3_per_mm());
                }
                let step = |speed: FloatOrPercent| {
                    let speed = speed.resolve(usual);
                    if speed < 0.5 { usual } else { speed }
                };
                let sections = [
                    (0.1, usual),
                    (0.25, step(s.overhang_speeds[0])),
                    (0.5, step(s.overhang_speeds[1])),
                    (0.75, step(s.overhang_speeds[2])),
                    (0.87, step(s.overhang_speeds[3])),
                    (1.0, s.bridge_speed),
                ];
                let speed = |short: f64| {
                    if s.enable_overhang_speed {
                        overhang::speed_for(&sections, flow.width, motion.speed, short)
                    } else {
                        motion.speed
                    }
                };
                overhang::Judge {
                    below,
                    width: flow.width,
                    clear_beyond: s.detect_overhang_wall.then_some(0.5 * s.nozzle_diameter),
                    speed: &speed,
                    fan_at: fan.and_then(|fan| fan.at),
                }
                .pieces(points)
            });
        let measured = match (known, measured) {
            (Some(pieces), _) => {
                self.measured_used += 1;
                Some(pieces)
            }
            (None, Some(pieces)) => {
                // Anything kept from a pass that went another way is dropped.
                self.measured.truncate(self.measured_used);
                self.measured.push((key, pieces.clone()));
                self.measured_used += 1;
                Some(pieces)
            }
            (None, None) => None,
        };
        match measured {
            Some(pieces) => {
                let bridge = flow.bridging(s.bridge_flow, s.thick_bridges);
                let over_air = self.motion(Role::OverhangPerimeter, &bridge, false);
                for piece in pieces {
                    if piece.clear {
                        self.fan_over_air(fan.is_some())?;
                        let motion = Motion {
                            speed: piece.speed.min(over_air.speed),
                            ..over_air
                        };
                        self.extrude(piece.to, Role::OverhangPerimeter, &bridge, motion)?;
                    } else {
                        self.fan_over_air(cooled || piece.cooled)?;
                        let motion = Motion {
                            speed: piece.speed,
                            ..motion
                        };
                        self.extrude(piece.to, role, flow, motion)?;
                    }
                }
            }
            None => {
                self.fan_over_air(cooled)?;
                for &point in &points[1..] {
                    self.extrude(point, role, flow, motion)?;
                }
            }
        }
        self.state.wipe_path = points.to_vec();
        if !closed {
            self.state.wipe_path.reverse();
        }
        Ok(())
    }

    /// Index of the candidate with a point closest to the nozzle; the first
    /// when the nozzle has not been anywhere yet.
    fn nearest<P: Iterator<Item = Point2>>(&self, candidates: impl Iterator<Item = P>) -> usize {
        let Some(at) = self.state.position else {
            return 0;
        };
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
            // As WipeTower2.cpp has it; a fresh nozzle's first line goes
            // down at a third of the speed.
            Role::PrimeTower | Role::PrimeLine => {
                let usual = if first_layer {
                    s.initial_layer_speed
                } else {
                    s.wipe_tower_max_purge_speed.min(s.sparse_infill_speed)
                };
                if role == Role::PrimeLine {
                    0.33 * usual
                } else {
                    usual
                }
            }
            // As GCode.cpp `_extrude`: the skirt's own speed wins when set.
            Role::Skirt if s.skirt_speed > 0.0 => s.skirt_speed,
            // On the first layer supports go down as carefully as walls
            // (GCode.cpp `_extrude`, where EdgeSlicer departs from Orca).
            _ if first_layer
                && (is_wall
                    || matches!(role, Role::Brim | Role::Support | Role::SupportInterface)) =>
            {
                s.initial_layer_speed
            }
            _ if first_layer => s.initial_layer_infill_speed,
            Role::ExternalPerimeter => s.outer_wall_speed,
            Role::Perimeter => s.inner_wall_speed,
            Role::SparseInfill => s.sparse_infill_speed,
            Role::SolidInfill | Role::BottomSurface => s.internal_solid_infill_speed,
            Role::TopSolidInfill => s.top_surface_speed,
            Role::BridgeInfill | Role::OverhangPerimeter => s.bridge_speed,
            Role::InternalBridge => s.internal_bridge_speed.resolve(s.bridge_speed),
            Role::GapFill => s.gap_infill_speed,
            Role::Skirt | Role::Brim => s.outer_wall_speed,
            Role::Support => s.support_speed,
            Role::SupportInterface => s.support_interface_speed,
        };
        let speed = if s.filament_max_volumetric_speed > 0.0 {
            wanted.min(s.filament_max_volumetric_speed / flow.mm3_per_mm())
        } else {
            wanted
        };
        let own = match role {
            _ if first_layer && s.initial_layer_acceleration > 0.0 => s.initial_layer_acceleration,
            // Anything laid over air, before its kind is looked at.
            Role::BridgeInfill | Role::InternalBridge | Role::OverhangPerimeter
                if s.bridge_acceleration.resolve(s.outer_wall_acceleration) > 0.0 =>
            {
                s.bridge_acceleration.resolve(s.outer_wall_acceleration)
            }
            Role::OverhangPerimeter | Role::InternalBridge => s.default_acceleration,
            Role::ExternalPerimeter => s.outer_wall_acceleration,
            Role::Perimeter => s.inner_wall_acceleration,
            Role::TopSolidInfill => s.top_surface_acceleration,
            Role::SparseInfill => s.sparse_infill_acceleration.resolve(s.default_acceleration),
            Role::SolidInfill | Role::BottomSurface | Role::BridgeInfill => s
                .internal_solid_infill_acceleration
                .resolve(s.default_acceleration),
            Role::Skirt
            | Role::Brim
            | Role::GapFill
            | Role::Support
            | Role::SupportInterface
            | Role::PrimeTower
            | Role::PrimeLine => s.default_acceleration,
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
        if acceleration <= 0.0 || acceleration == self.state.acceleration {
            return Ok(());
        }
        self.state.acceleration = acceleration;
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

    /// `sheltered` says the move may go unretracted however long it is,
    /// unless it leaves an outer wall, which always retracts.
    fn travel(&mut self, to: Point2, sheltered: bool) -> io::Result<()> {
        let s = self.settings;
        let from = self.state.position;
        let length = from.map_or(f64::INFINITY, |from| distance(from, to));
        if length == 0.0 && self.state.height == Some(self.state.z) {
            return Ok(());
        }
        let exposed = !sheltered || self.state.role == Some(Role::ExternalPerimeter);
        let long = length > s.retraction_minimum_travel && exposed;
        if long {
            self.retract(false)?;
        } else {
            // Nothing to wipe once the nozzle has moved off the line.
            self.state.wipe_path.clear();
        }
        // A wipe moves the nozzle, so the travel is measured from where it ends.
        let from = self.state.position;
        let length = from.map_or(f64::INFINITY, |from| distance(from, to));
        // The first layer is gentle throughout. Above it, long moves get the
        // travel acceleration; short hops between neighbouring lines keep
        // whatever is set, rather than switching twice for a millimetre.
        if self.state.first_layer && s.initial_layer_acceleration > 0.0 {
            self.accelerate(s.initial_layer_acceleration)?;
        } else if long {
            let travel = if s.travel_acceleration > 0.0 {
                s.travel_acceleration
            } else {
                s.default_acceleration
            };
            self.accelerate(travel)?;
        }

        let lift = self.state.pending_lift.take();
        let z = self.state.z;
        let top = match (lift, self.state.height) {
            (Some(_), _) => z + s.z_hop,
            // Already up from an earlier lift: stay there.
            (None, Some(height)) => height.max(z),
            (None, None) => z,
        };
        let (Some(from), Some(height)) = (from, self.state.height) else {
            // The first move starts wherever the start G-code left the
            // nozzle, which is not known: across at that height, then to
            // this layer's, as the reference does.
            self.go(to, None, s.travel_speed, 0.0)?;
            self.feed(s.z_speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 Z{top:.3}{feed_word}")?;
            self.state.height = Some(top);
            return Ok(());
        };
        let rise = top - height;
        // A ramp or spiral climbs no faster than the Z axis can.
        let climbing = |run: f64, rise: f64| s.travel_speed.min(s.z_speed * run.hypot(rise) / rise);
        if rise.abs() < 1e-9 {
            self.go(to, None, s.travel_speed, length)?;
        } else if rise < 0.0 {
            self.go(to, Some(top), s.travel_speed, length)?;
        } else {
            match lift {
                Some(Lift::Spiral) if self.spiral_fits(from, to, top - z) => {
                    self.spiral(from, to, top, height)?;
                    self.go(to, Some(top), s.travel_speed, length)?;
                }
                Some(Lift::Normal | Lift::Spiral) => {
                    self.feed(s.z_speed);
                    let feed_word = self.feed_word();
                    writeln!(self.out, "G1 Z{top:.3}{feed_word}")?;
                    self.spend(rise / s.z_speed);
                    self.go(to, None, s.travel_speed, length)?;
                }
                // A ramp along the start of the travel, as steep as
                // `travel_slope`; a travel too short for it climbs all the way.
                _ => {
                    let run = rise / s.travel_slope.to_radians().tan();
                    if run > 0.0 && run < length {
                        let share = run / length;
                        let ramp = Point2 {
                            x: from.x + ((to.x - from.x) as f64 * share).round() as i64,
                            y: from.y + ((to.y - from.y) as f64 * share).round() as i64,
                        };
                        self.go(ramp, Some(top), climbing(run, rise), run)?;
                        self.go(to, Some(top), s.travel_speed, length - run)?;
                    } else {
                        self.go(to, Some(top), climbing(length, rise), length)?;
                    }
                }
            }
        }
        self.state.height = Some(top);
        Ok(())
    }

    /// One move without extruding, to `to` and, if given, height `z`.
    /// `length` is how far that is across the bed, in mm.
    fn go(&mut self, to: Point2, z: Option<f64>, speed: f64, length: f64) -> io::Result<()> {
        self.feed(speed);
        let feed_word = self.feed_word();
        let height = z.map_or(String::new(), |z| format!(" Z{z:.3}"));
        writeln!(
            self.out,
            "G1 X{:.3} Y{:.3}{height}{feed_word}",
            unscaled(to.x),
            unscaled(to.y)
        )?;
        // The very first move has no known start, so it is not counted.
        if length.is_finite() {
            self.state.summary.travel_mm += length;
            self.spend(length / speed);
        }
        self.state.position = Some(to);
        Ok(())
    }

    /// Counts time that passes without extruding.
    fn spend(&mut self, seconds: f64) {
        self.state.summary.seconds += seconds;
        if self.extruding {
            self.timing.fixed += seconds;
        }
    }

    /// Radius of a spiral lift of `hop` mm, as GCodeWriter::travel_to_xyz has it.
    fn spiral_radius(&self, hop: f64) -> f64 {
        hop / (TAU * self.settings.travel_slope.to_radians().atan())
    }

    /// Whether a spiral lift from `from` towards `to` stays over the bed.
    /// Without a known bed it is not risked.
    fn spiral_fits(&self, from: Point2, to: Point2, hop: f64) -> bool {
        let Some([min_x, min_y, max_x, max_y]) = self.settings.bed else {
            return false;
        };
        let length = distance(from, to);
        let radius = self.spiral_radius(hop);
        if length == 0.0 || !(radius.is_finite() && radius > 0.0) {
            return false;
        }
        // The circle's centre lies to the left of the way the travel heads.
        let x = unscaled(from.x) - unscaled(to.y - from.y) / length * radius;
        let y = unscaled(from.y) + unscaled(to.x - from.x) / length * radius;
        x - radius >= min_x && x + radius <= max_x && y - radius >= min_y && y + radius <= max_y
    }

    /// One turn of a rising circle, ending above where it started.
    fn spiral(&mut self, from: Point2, to: Point2, top: f64, height: f64) -> io::Result<()> {
        let s = self.settings;
        let base = self.state.z;
        let hop = top - base;
        let radius = self.spiral_radius(hop);
        let length = distance(from, to);
        let (dx, dy) = (
            unscaled(to.x - from.x) / length,
            unscaled(to.y - from.y) / length,
        );
        let (centre_x, centre_y) = (
            unscaled(from.x) - dy * radius,
            unscaled(from.y) + dx * radius,
        );
        // Enough segments to stay within `resolution` of the circle
        // (ArcWelder.hpp `arc_discretization_steps`), between 8 and 128.
        let steps = if s.resolution <= 0.0 {
            128
        } else if s.resolution >= radius {
            8
        } else {
            ((TAU / (2.0 * (1.0 - s.resolution / radius).acos())).ceil() as usize).clamp(8, 128)
        };
        let chord = 2.0 * radius * (PI / steps as f64).sin();
        let mut below = height;
        for step in 1..=steps {
            let turn = TAU * step as f64 / steps as f64;
            // Start from the centre towards the nozzle, turning to the left.
            let (arm_x, arm_y) = (dy * radius, -dx * radius);
            let point = if step == steps {
                from
            } else {
                Point2 {
                    x: scaled(centre_x + arm_x * turn.cos() - arm_y * turn.sin()),
                    y: scaled(centre_y + arm_x * turn.sin() + arm_y * turn.cos()),
                }
            };
            let z = base + hop * step as f64 / steps as f64;
            let speed = s
                .travel_speed
                .min(s.z_speed * chord.hypot(z - below) / (z - below));
            self.go(point, Some(z), speed, chord)?;
            below = z;
        }
        Ok(())
    }

    fn extrude(&mut self, to: Point2, role: Role, flow: &Flow, motion: Motion) -> io::Result<()> {
        let s = self.settings;
        let from = self
            .state
            .position
            .expect("a travel always comes before extruding");
        let length = distance(from, to);
        if length == 0.0 {
            return Ok(());
        }
        self.prime()?;
        if self.state.role != Some(role) {
            // Two roles may go by one name, which is then said once.
            if self.state.role.map(role_name) != Some(role_name(role)) {
                writeln!(self.out, ";TYPE:{}", role_name(role))?;
            }
            self.state.role = Some(role);
        }
        if self.state.width != flow.width {
            writeln!(self.out, ";WIDTH:{:.3}", flow.width)?;
            self.state.width = flow.width;
        }
        self.accelerate(motion.acceleration)?;
        // A layer's time is counted from its first line, as in the reference.
        if !self.extruding {
            self.extruding = true;
            self.timing.fixed = 0.0;
        }
        let held = role == Role::ExternalPerimeter && s.dont_slow_down_outer_wall;
        let speed = if held {
            self.timing.fixed += length / motion.speed;
            motion.speed
        } else {
            match self.timing.lines.last_mut() {
                Some(line) if line.1 == motion.speed => line.0 += length,
                _ => self.timing.lines.push((length, motion.speed)),
            }
            self.slowdown.apply(motion.speed)
        };
        let volume = length * flow.mm3_per_mm() * s.filament_flow_ratio * s.print_flow_ratio;
        let feed = volume / self.filament_area;
        if self.all.len() > 1 {
            self.marks
                .push((self.out.len(), self.state.summary.seconds));
        }
        self.feed(speed);
        let feed_word = self.feed_word();
        writeln!(
            self.out,
            "G1 X{:.3} Y{:.3} E{feed:.5}{feed_word}",
            unscaled(to.x),
            unscaled(to.y)
        )?;
        self.state.primed = true;
        self.state.line_speed = motion.speed;
        self.state.summary.filament_mm += feed;
        self.state.summary.filament_mm3 += volume;
        if let Some(used) = self.state.used.get_mut(self.state.tool) {
            used.0 += feed;
            used.1 += volume;
        }
        self.state.summary.seconds += length / speed;
        self.state.position = Some(to);
        Ok(())
    }

    /// Pulls the filament back, wiping the nozzle over what it just printed
    /// if the profile asks, and decides whether the next travel lifts.
    /// `layer_change` says the retraction is the one between layers.
    fn retract(&mut self, layer_change: bool) -> io::Result<()> {
        let s = self.settings;
        let path = std::mem::take(&mut self.state.wipe_path);
        if self.state.retracted || !self.state.primed || s.retraction_length <= 0.0 {
            return Ok(());
        }
        let total = s.retraction_length;
        // The wipe sets off from where the nozzle is and follows the path
        // for `wipe_distance` (GCode.cpp `Wipe::wipe`).
        let wipe = match self.state.position {
            Some(at) if s.wipe && s.wipe_distance > 0.0 && path.len() >= 2 => {
                let mut path = path;
                path[0] = at;
                Some(seam::head(&path, s.wipe_distance)).filter(|path| path.len() >= 2)
            }
            _ => None,
        };
        let wipe_speed = s.wipe_speed.unwrap_or(self.state.line_speed).max(10.0);
        let reach: f64 = wipe
            .iter()
            .flat_map(|path| path.windows(2))
            .map(|pair| distance(pair[0], pair[1]))
            .sum();
        // How much is pulled back before, during and after the wipe
        // (`calculateWipeRetractionLengths`).
        let (before, during, after) = if reach > 0.0 {
            let before = total * s.retract_before_wipe;
            let after = (total * s.retract_after_wipe).min(total - before);
            let rest = total - before - after;
            let most = s.retraction_speed * reach / wipe_speed;
            if rest <= 1e-9 {
                (before, 0.0, after)
            } else if most > rest {
                (before, rest, after)
            } else {
                (before + rest - most, most, after)
            }
        } else {
            (total, 0.0, 0.0)
        };
        if before > 0.0 {
            self.feed(s.retraction_speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 E-{before:.5}{feed_word}")?;
        }
        if let Some(path) = wipe.filter(|_| reach > 0.0) {
            self.state.feedrate = (wipe_speed * 60.0).round();
            writeln!(self.out, ";WIPE_START\nG1 F{:.0}", self.state.feedrate)?;
            for pair in path.windows(2) {
                let length = distance(pair[0], pair[1]);
                if length == 0.0 {
                    continue;
                }
                let pulled = if during > 0.0 {
                    format!(" E-{:.5}", during * length / reach)
                } else {
                    String::new()
                };
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3}{pulled}",
                    unscaled(pair[1].x),
                    unscaled(pair[1].y)
                )?;
                self.state.summary.travel_mm += length;
                self.spend(length / wipe_speed);
                self.state.position = Some(pair[1]);
            }
            writeln!(self.out, ";WIPE_END")?;
        }
        if after > 0.0 {
            self.feed(s.retraction_speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 E-{after:.5}{feed_word}")?;
        }
        self.state.retracted = true;
        self.state.retracted_length = total;
        self.state.summary.retractions += 1;
        self.spend((before + after) / s.retraction_speed);

        let z = self.state.z;
        let in_range =
            z >= s.retract_lift_above && (s.retract_lift_below <= 0.0 || z <= s.retract_lift_below);
        if s.z_hop > 0.0 && in_range {
            self.state.pending_lift = Some(match s.lift {
                Lift::Auto if layer_change => Lift::Spiral,
                Lift::Auto => Lift::Slope,
                chosen => chosen,
            });
        }
        Ok(())
    }

    /// Brings the nozzle down to the layer and undoes a retraction, ready
    /// to extrude.
    fn prime(&mut self) -> io::Result<()> {
        let s = self.settings;
        let z = self.state.z;
        if self.state.height != Some(z) {
            self.feed(s.z_speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G1 Z{z:.3}{feed_word}")?;
            if let Some(height) = self.state.height {
                self.spend((height - z).abs() / s.z_speed);
            }
            self.state.height = Some(z);
        }
        self.state.pending_lift = None;
        if self.state.retracted {
            let speed = if s.deretraction_speed > 0.0 {
                s.deretraction_speed
            } else {
                s.retraction_speed
            };
            self.feed(speed);
            let feed_word = self.feed_word();
            // Whatever this tool's filament was last pulled back by.
            let length = self.state.retracted_length;
            writeln!(self.out, "G1 E{length:.5}{feed_word}")?;
            self.state.retracted = false;
            self.state.retracted_length = 0.0;
            self.spend(length / speed);
        }
        Ok(())
    }

    /// Makes `speed` (mm/s) the feedrate of the move about to be written.
    /// `feed_word` then yields the `F` word once, the first time it changes.
    fn feed(&mut self, speed: f64) {
        let feedrate = (speed * 60.0).round();
        if feedrate != self.state.feedrate.abs() {
            // Negative marks a feedrate not yet written.
            self.state.feedrate = -feedrate;
        }
    }

    fn feed_word(&mut self) -> String {
        if self.state.feedrate < 0.0 {
            self.state.feedrate = -self.state.feedrate;
            format!(" F{:.0}", self.state.feedrate)
        } else {
            String::new()
        }
    }

    fn finish(&mut self) -> Result<Summary, GcodeError> {
        let s = self.settings;
        self.retract(false)?;
        writeln!(self.out, "M106 S0")?;
        if s.auxiliary_fan {
            writeln!(self.out, "M106 P2 S0")?;
        }
        if s.filament_end_gcode.is_some() || s.machine_end_gcode.is_some() {
            writeln!(self.out, ";TYPE:Custom")?;
        }
        if let Some(template) = &s.filament_end_gcode {
            self.custom("filament_end_gcode", template)?;
        }
        match &s.machine_end_gcode {
            Some(template) => {
                self.custom("machine_end_gcode", template)?;
            }
            None => writeln!(self.out, "M104 S0\nM140 S0\nM84")?,
        }
        let summary = self.state.summary;
        writeln!(self.out, "; EXECUTABLE_BLOCK_END\n")?;
        // One figure for each filament, as the reference lists them.
        let listed = |figure: &dyn Fn(usize) -> f64| {
            (0..self.state.used.len())
                .map(|filament| format!("{:.2}", figure(filament)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let used = &self.state.used;
        writeln!(
            self.out,
            "; filament used [mm] = {}",
            listed(&|filament| used[filament].0)
        )?;
        writeln!(
            self.out,
            "; filament used [cm3] = {}",
            listed(&|filament| used[filament].1 / 1000.0)
        )?;
        let grams = |filament: usize| {
            used[filament].1 / 1000.0 * self.all[filament].filament_density.unwrap_or(0.0)
        };
        let weighed = self
            .all
            .iter()
            .all(|filament| filament.filament_density.is_some());
        if weighed {
            writeln!(self.out, "; filament used [g] = {}", listed(&grams))?;
        }
        if !s.config.is_empty() {
            writeln!(self.out, "; CONFIG_BLOCK_START")?;
            for (key, value) in &s.config {
                writeln!(self.out, "; {key} = {value}")?;
            }
            // Under the names older hosts look for (GCode.cpp).
            if let Some(bed) = s.bed_temperature_initial_layer {
                writeln!(self.out, "; first_layer_bed_temperature = {bed:.0}")?;
            }
            if let Some((_, area)) = s.config.iter().find(|(name, _)| name == "printable_area") {
                writeln!(self.out, "; bed_shape = {area}")?;
            }
            writeln!(
                self.out,
                "; first_layer_temperature = {:.0}",
                s.nozzle_temperature_initial_layer
            )?;
            writeln!(
                self.out,
                "; first_layer_height = {:.3}",
                self.file.first_layer_height
            )?;
            writeln!(self.out, "; CONFIG_BLOCK_END\n")?;
        }
        if weighed {
            writeln!(
                self.out,
                "; total filament used [g] = {:.2}",
                (0..used.len()).map(grams).sum::<f64>()
            )?;
        }
        if self.state.tool_changes > 0 {
            writeln!(
                self.out,
                "; total filament change = {}",
                self.state.tool_changes
            )?;
        }
        writeln!(self.out, "; total layers count = {}", self.file.layers)?;
        // Both times are at commanded speeds with no acceleration, so the
        // print takes longer than they say.
        writeln!(
            self.out,
            "; estimated printing time (normal mode) = {}",
            days_hours_minutes_seconds(summary.seconds)
        )?;
        writeln!(
            self.out,
            "; estimated first layer printing time (normal mode) = {}",
            days_hours_minutes_seconds(self.file.first_layer_seconds)
        )?;
        writeln!(
            self.out,
            "; time at commanded speeds, without acceleration [s] = {:.0}",
            summary.seconds
        )?;
        Ok(summary)
    }
}

/// The planned seam a wall should start next to: the nearest one, taking
/// those of its own kind (around the part, or around a hole) first. None
/// when no seams were planned.
fn alongside(points: &[Point2], is_hole: bool, planned: &[(bool, Point2)]) -> Option<Point2> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for point in points {
        (min_x, max_x) = (min_x.min(point.x), max_x.max(point.x));
        (min_y, max_y) = (min_y.min(point.y), max_y.max(point.y));
    }
    // How far a seam is from the box around the wall: a cheap lower bound
    // that rules most seams out before the wall itself is measured.
    let outside = |seam: Point2| {
        let dx = (min_x - seam.x).max(seam.x - max_x).max(0);
        let dy = (min_y - seam.y).max(seam.y - max_y).max(0);
        unscaled(dx).hypot(unscaled(dy))
    };
    let same: Vec<Point2> = planned
        .iter()
        .filter(|(hole, _)| *hole == is_hole)
        .map(|(_, seam)| *seam)
        .collect();
    let candidates = if same.is_empty() {
        planned.iter().map(|(_, seam)| *seam).collect()
    } else {
        same
    };
    let nearest_box = candidates
        .iter()
        .map(|seam| outside(*seam))
        .fold(f64::INFINITY, f64::min);
    candidates
        .into_iter()
        .filter(|seam| outside(*seam) <= nearest_box + 2.0)
        .map(|seam| (distance(seam::project(points, seam).1, seam), seam))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, seam)| seam)
}

/// A length of time as the reference writes it: `1d 2h 3m 4s`, without
/// the parts that would lead with a zero (`get_time_dhms`).
fn days_hours_minutes_seconds(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (days, hours, minutes, seconds) = (
        total / 86_400,
        total / 3_600 % 24,
        total / 60 % 60,
        total % 60,
    );
    if days > 0 {
        format!("{days}d {hours}h {minutes}m {seconds}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

/// Markers a layer is written with where the fan is to change for an
/// overhang and back, until the layer's own fan speed is known.
const FAN_MARK: &[u8] = b";_FAN_OVER_AIR_";
const FAN_OVER_AIR: &[u8] = b";_FAN_OVER_AIR_ON\n";
const FAN_AS_LAYER: &[u8] = b";_FAN_OVER_AIR_OFF\n";

fn distance(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// Feature names as the reference writes them (`ExtrusionEntity::role_to_string`),
/// which is what G-code viewers colour by.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::ExternalPerimeter => "Outer wall",
        Role::Perimeter => "Inner wall",
        Role::OverhangPerimeter => "Overhang wall",
        Role::SparseInfill => "Sparse infill",
        Role::SolidInfill => "Internal solid infill",
        Role::TopSolidInfill => "Top surface",
        Role::BottomSurface => "Bottom surface",
        Role::BridgeInfill => "Bridge",
        Role::InternalBridge => "Internal Bridge",
        Role::GapFill => "Gap infill",
        Role::Skirt => "Skirt",
        Role::Brim => "Brim",
        Role::Support => "Support",
        Role::SupportInterface => "Support interface",
        Role::PrimeTower | Role::PrimeLine => "Prime tower",
    }
}

#[cfg(test)]
mod tests;
