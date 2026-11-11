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

mod chain;
mod circle;
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
pub use settings::{Lift, OverhangFan, PrintSettings, WallDirection, WallOrder};

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

/// A filament rammed on the prime tower before its tool is put away
/// (WipeTower2.cpp `toolchange_Unload`).
#[derive(Clone, Copy, Debug)]
pub struct Ram<'a> {
    pub lines: &'a [ExtrusionPath],
    /// How fast they are laid, in mm a second.
    pub speed: f64,
    /// Where the tool is taken once it has laid them.
    pub park: Point2,
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
    /// What the tool in use before this one lays on the prime tower
    /// before it is put away, when this part starts with a tool change
    /// and that tool's filament is rammed.
    pub ram: Option<Ram<'a>>,
    /// Where the nozzle goes on its way to the prime tower's lines: in
    /// through the gap left in the tower's wall.
    pub enter: &'a [Point2],
    /// Prime tower lines, printed before anything else.
    pub tower: &'a [ExtrusionPath],
    /// Loops laid before the part on this layer, in order: skirt and brim.
    pub around: &'a [ExtrusionLoop],
    /// Support lines, printed before the part so it has them to rest on.
    pub support: &'a [ExtrusionPath],
    pub loops: &'a [ExtrusionLoop],
    /// Which of `loops` make up each island.
    pub islands: &'a [Range<usize>],
    /// Fill, each line laid with the island it lies in. A line in the
    /// overhang wall's role is a wall over fill that hangs over air, and
    /// is laid ahead of its island's loops; one in a wall's role goes
    /// with the walls, after the loops.
    pub paths: &'a [ExtrusionPath],
    /// Where a move to a fill line may skip retraction if it stays
    /// within one piece: the part of the layer that is neither a top
    /// face nor a bottom one, walls and all, since ooze there is covered
    /// over (RetractWhenCrossingPerimeters.cpp). Pieces of different
    /// filaments are given apart, and a move from one to another retracts.
    pub shelter: &'a [ExPolygon],
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
    /// How long the print takes, in seconds. With a printer profile this
    /// is every move planned as the printer's firmware plans it
    /// (`edge-time`), and `timed` says which way; without one it is
    /// `seconds_at_speeds`. Heating and whatever the printer's own
    /// macros do are not counted.
    pub seconds: f64,
    /// Time for every move at its commanded speed, with no acceleration.
    pub seconds_at_speeds: f64,
    /// The planner `seconds` comes from, when there is a printer profile.
    pub timed: Option<TimeModel>,
    /// Layers slowed down to give them time to cool.
    pub slowed_layers: usize,
    pub tool_changes: usize,
}

pub use edge_time::Model as TimeModel;

/// Starts a line the writer leaves for itself where a tool is changed:
/// the tool taken up and the temperature it prints at.
const CHANGE_MARK: &[u8] = b";_TOOL_CHANGE ";

/// A support wall's hook is six line spacings long, so its two ends are
/// never further apart than a little more than that.
const HOOK_REACH: f64 = 6.5;

/// How fast a tool that has rammed its filament is taken clear of it, in
/// mm a second (WipeTower2.cpp `toolchange_Unload`).
const RAM_PARK_SPEED: f64 = 40.0;

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
            widths: {
                // Above the first layer, the first line of each kind;
                // and whatever the first layer starts with.
                let above = layers.iter().filter(|layer| layer.z > layers[0].z);
                let wall = |role: Role| {
                    above
                        .clone()
                        .flat_map(|layer| layer.loops)
                        .find(|wall| wall.role == role)
                        .map(|wall| wall.flow.width)
                };
                let fill = |role: Role| {
                    above
                        .clone()
                        .flat_map(|layer| layer.paths)
                        .find(|path| path.role == role)
                        .map(|path| path.flow.width)
                };
                let first = layers.first().and_then(|layer| {
                    (layer.loops.first().map(|wall| wall.flow.width))
                        .or(layer.paths.first().map(|path| path.flow.width))
                });
                [
                    ("external perimeters", wall(Role::ExternalPerimeter)),
                    ("perimeters", wall(Role::Perimeter)),
                    ("infill", fill(Role::SparseInfill)),
                    ("solid infill", fill(Role::SolidInfill)),
                    ("top infill", fill(Role::TopSolidInfill)),
                    ("first layer", first),
                ]
                .into_iter()
                .filter_map(|(name, width)| Some((name, width?)))
                .collect()
            },
        },
        below: None,
        inner_wall: None,
        measured: Vec::new(),
        measured_used: 0,
    };
    let several = layers.iter().any(|layer| layer.filament > 0);
    writer.preamble(several)?;
    // The whole file is put together before any of it is handed on, so
    // that progress can be written into it once the time is known. Each
    // entry is a place in the file and the seconds printed by then.
    let mut whole = std::mem::take(&mut writer.out);
    let mut reached: Vec<(usize, f64)> = Vec::with_capacity(groups.len() + 1);
    if let Some(start) = find(&whole, EXECUTABLE_START) {
        reached.push((start + EXECUTABLE_START.len(), 0.0));
    }
    // Tool changes: where in the file, the tool taken up, its temperature.
    let mut changes: Vec<(usize, usize, f64)> = Vec::new();
    let mut second_layer = None;
    for (index, group) in groups.iter().enumerate() {
        writer.out.clear();
        reached.push((whole.len(), writer.state.summary.seconds));
        if index == 1 {
            second_layer = Some(whole.len());
        }
        writer.layer(index, &layers[group.clone()], group.start)?;
        if index == 0 {
            writer.file.first_layer_seconds = writer.state.summary.seconds;
        }
        // The layer's text, less the notes of where a tool was changed.
        for line in writer.out.split_inclusive(|byte| *byte == b'\n') {
            match line.strip_prefix(CHANGE_MARK) {
                Some(noted) => {
                    let noted = String::from_utf8_lossy(noted);
                    let mut words = noted.split_whitespace();
                    let number = |word: Option<&str>| word.and_then(|word| word[1..].parse().ok());
                    let tool: Option<f64> = number(words.next());
                    let temperature: Option<f64> = number(words.next());
                    if let (Some(tool), Some(temperature)) = (tool, temperature) {
                        changes.push((whole.len(), tool as usize, temperature));
                    }
                }
                None => whole.extend_from_slice(line),
            }
        }
    }
    let printed = whole.len();
    writer.out.clear();
    let mut summary = writer.finish()?;
    whole.extend_from_slice(&writer.out);
    summary.seconds_at_speeds = summary.seconds;

    // With a printer profile the file is timed as its firmware will run
    // it. `ends` is where each line ends and `elapsed` the seconds gone
    // by then.
    let limits = settings[first].limits;
    let text = std::str::from_utf8(&whole).ok();
    let timed = limits.zip(text).map(|(limits, text)| {
        let model = limits.own_model();
        let estimate = edge_time::estimate(text, &limits, model);
        let mut at = 0;
        let ends: Vec<usize> = text
            .split_inclusive('\n')
            .map(|line| {
                at += line.len();
                at
            })
            .collect();
        (model, estimate.elapsed(), ends)
    });
    // Seconds gone when the file has been run up to `at`.
    let gone = |at: usize| -> Option<f64> {
        let (_, elapsed, ends) = timed.as_ref()?;
        let lines = ends.partition_point(|end| *end <= at);
        Some(if lines == 0 { 0.0 } else { elapsed[lines - 1] })
    };
    if let Some((model, elapsed, _)) = &timed {
        summary.seconds = elapsed.last().copied().unwrap_or(0.0);
        summary.timed = Some(*model);
        // The first layer, from its start to the start of the next.
        if let (Some(start), Some(end)) = (
            reached.get(1).and_then(|(at, _)| gone(*at)),
            gone(second_layer.unwrap_or(printed)),
        ) {
            writer.file.first_layer_seconds = end - start;
        }
    }

    // What is slipped into the file now that its time is known: (where, text).
    let mut slipped: Vec<(usize, String)> = Vec::new();
    // Progress for the printer's screen: how far through, in percent,
    // and the minutes left, said at the start and at each layer where
    // either has changed (the reference's `M73`). Printers given no
    // profile are not assumed to know the command.
    if settings[first].machine_start_gcode.is_some() && summary.seconds > 0.0 {
        let mut said = None;
        for (at, at_speeds) in &reached {
            let seconds = gone(*at).unwrap_or(*at_speeds);
            let progress = (
                (100.0 * seconds / summary.seconds).floor() as u32,
                ((summary.seconds - seconds) / 60.0).floor() as u32,
            );
            if said != Some(progress) {
                said = Some(progress);
                slipped.push((*at, format!("M73 P{} R{}\n", progress.0, progress.1)));
            }
        }
    }
    // A tool waiting its turn is kept cooler; it is told to warm up
    // `preheat_time` before it is wanted, but never before the last
    // thing said of its temperature, which would undo it.
    if let (Some((_, elapsed, ends)), Some(text)) = (&timed, text) {
        for (change, tool, temperature) in &changes {
            let wanted = &settings[*tool];
            if wanted.standby_temperature_delta.is_none() || wanted.preheat_time <= 0.0 {
                continue;
            }
            let last = ends.partition_point(|end| *end <= *change);
            let mentions = |line: &str| {
                let code = line.split(';').next().unwrap_or_default();
                let mut words = code.split_whitespace();
                matches!(words.next(), Some("M104" | "M109"))
                    && words.any(|word| {
                        word.strip_prefix('T').and_then(|t| t.parse().ok()) == Some(*tool)
                    })
            };
            let line_at = |index: usize| {
                let from = if index == 0 { 0 } else { ends[index - 1] };
                &text[from..ends[index]]
            };
            let earliest = (0..last)
                .rev()
                .find(|index| mentions(line_at(*index)))
                .map_or(0, |index| index + 1);
            let at_change = if last == 0 { 0.0 } else { elapsed[last - 1] };
            let target = at_change - wanted.preheat_time;
            // After the last line that ends before the moment wanted.
            let line = elapsed[..last]
                .partition_point(|seconds| *seconds < target)
                .max(earliest)
                .min(last);
            // Nothing is put before the file's own first lines.
            let executable = reached.first().map_or(0, |(at, _)| *at);
            let at = (if line == 0 { 0 } else { ends[line - 1] }).max(executable);
            let lead = at_change - gone(at).unwrap_or(at_change);
            slipped.push((
                at,
                format!("M104 S{temperature:.0} T{tool} ; preheat T{tool} time: {lead:.0}s\n"),
            ));
        }
    }
    slipped.sort_by_key(|(at, _)| *at);
    let mut from = 0;
    for (at, line) in slipped {
        out.write_all(&whole[from..at])?;
        out.write_all(line.as_bytes())?;
        from = at;
    }
    out.write_all(&whole[from..])?;
    writeln!(
        out,
        "; estimated printing time (normal mode) = {}",
        days_hours_minutes_seconds(summary.seconds)
    )?;
    writeln!(
        out,
        "; estimated first layer printing time (normal mode) = {}",
        days_hours_minutes_seconds(writer.file.first_layer_seconds)
    )?;
    // How the time above was come by, and the plain figure beside it.
    match summary.timed {
        Some(TimeModel::Klipper) => writeln!(
            out,
            "; time planned as Klipper plans, with its minimum cruise ratio; heating is not counted"
        )?,
        Some(TimeModel::Reference) => writeln!(
            out,
            "; time planned with acceleration, as the reference slicer estimates; heating is not counted"
        )?,
        None => {}
    }
    writeln!(
        out,
        "; time at commanded speeds, without acceleration [s] = {:.0}",
        summary.seconds_at_speeds
    )?;
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
    /// What the file's first and last lines say about the print.
    file: FileInfo,
    /// The outline of the layer beneath the one being written.
    below: Option<overhang::Below>,
    /// The line the layer's inner walls are printed with, when it has any.
    inner_wall: Option<Flow>,
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
    /// Line widths in mm as the file's opening lines give them, each
    /// with the name the reference gives it; only those the print has.
    widths: Vec<(&'static str, f64)>,
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
    /// The speed of the wall loop being laid, where it is small enough
    /// to have one of its own (`small_perimeter_speed`).
    small_wall: Option<f64>,
    /// The speed of the wall loop being laid, where it goes round a
    /// circle the auto circle compensation resized (`circle.rs`).
    circle_wall: Option<f64>,
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
        // (utils.cpp `header_gcode_producer`). The version is that of
        // the reference whose settings and file layout these are.
        writeln!(self.out, "; HEADER_BLOCK_START")?;
        write!(
            self.out,
            "; generated by {} (EdgeSlicerRS {})",
            edge_config::reference_producer(),
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
        for (name, width) in &self.file.widths {
            writeln!(self.out, "; {name} extrusion width = {width:.2}mm")?;
        }
        if !self.file.widths.is_empty() {
            writeln!(self.out)?;
        }
        self.out.extend_from_slice(EXECUTABLE_START);
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
        // Select the tool the print starts with, as the reference does
        // when the project has more than one filament to choose from
        // (GCodeWriter.cpp `toolchange`).
        if s.machine_start_gcode.is_some() && s.filaments > 1 {
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

    /// Puts the tool in use away and takes up another, mid-print. `over`
    /// is where on the prime tower the new tool starts: the old one goes
    /// there first, so that it waits over the tower, not over the part,
    /// while the new one comes up to temperature (WipeTower2.cpp
    /// `tool_change`, which unloads on the tower).
    fn change_tool(
        &mut self,
        to: usize,
        layer: usize,
        z: f64,
        cursor: usize,
        over: Option<Point2>,
        ram: Option<Ram<'_>>,
    ) -> Result<(), GcodeError> {
        let from = self.state.tool;
        let s = self.settings;
        match ram.filter(|_| self.state.primed) {
            // A rammed filament is pushed out fast on the tower, and its
            // tool then waits just clear of what it laid.
            Some(ram) => {
                for line in ram.lines.iter().filter(|line| line.points.len() >= 2) {
                    // As fast as the filament is to be pushed out, with
                    // the acceleration any line of the tower has here.
                    let motion = Motion {
                        speed: ram.speed,
                        ..self.motion(line.role, &line.flow, self.state.first_layer)
                    };
                    self.travel(line.points[0], false)?;
                    for point in &line.points[1..] {
                        self.extrude(*point, line.role, &line.flow, motion)?;
                    }
                }
                let length = self.state.position.map_or(0.0, |at| distance(at, ram.park));
                self.go(ram.park, None, RAM_PARK_SPEED, length)?;
            }
            None => {
                if let Some(over) = over.filter(|_| self.state.primed) {
                    self.travel(over, false)?;
                }
            }
        }
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
        // A note of the change, taken out again once the file is whole
        // and timed, when the tool is told to warm up ahead of it.
        self.out.extend_from_slice(CHANGE_MARK);
        writeln!(self.out, "T{to} S{:.0}", self.temperature(to, layer == 0))?;
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

    /// One physical layer: `parts` is what each filament prints on it, in
    /// order, and `cursor` where the first of them comes in the whole print.
    fn layer(
        &mut self,
        index: usize,
        parts: &[Layer<'_>],
        cursor: usize,
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
        self.body(index, parts, cursor)?;
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
                self.body(index, parts, cursor)?;
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
        self.settle_ironing_fan(speed);
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
    fn body(&mut self, index: usize, parts: &[Layer<'_>], cursor: usize) -> Result<(), GcodeError> {
        self.timing = Timing::default();
        self.extruding = false;
        for (offset, layer) in parts.iter().enumerate() {
            if layer.filament != self.state.tool {
                let over = layer.tower.first().and_then(|line| line.points.first());
                self.change_tool(
                    layer.filament,
                    index,
                    layer.z,
                    cursor + offset,
                    over.copied(),
                    layer.ram,
                )?;
                // The new tool comes in through the gap in the tower's wall.
                for point in layer.enter {
                    self.travel(*point, false)?;
                }
            }
            self.inner_wall = layer
                .loops
                .iter()
                .find(|wall| wall.role == Role::Perimeter)
                .map(|wall| wall.flow);
            self.tower_lines(layer.tower)?;
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
            self.lines(&layer.support.iter().collect::<Vec<_>>(), &[])?;
            // Ironing has a fan speed of its own once the fan is on at
            // all (CoolingBuffer.cpp: not on the first layers it is
            // kept off for).
            let ironing_fan = self
                .settings
                .ironing_fan_speed
                .filter(|_| index >= self.settings.close_fan_the_first_x_layers)
                .map(cooling::pwm);
            self.islands(layer, ironing_fan)?;
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

    /// Islands nearest first, each finished before the next is begun: its
    /// walls and the fill that lies in it, in the configured order, and
    /// last its ironing, over all of that (GCode.cpp `process_layer`).
    /// Fill in no island comes last. `ironing_fan` is the fan's speed
    /// while ironing, when ironing has one of its own on this layer.
    fn islands(&mut self, layer: &Layer<'_>, ironing_fan: Option<u8>) -> io::Result<()> {
        let homes = homes(layer);
        let fill = |island: usize| -> Vec<&ExtrusionPath> {
            layer
                .paths
                .iter()
                .zip(&homes)
                .filter(|(_, home)| **home == island)
                .map(|(path, _)| path)
                .collect()
        };
        let mut remaining: Vec<usize> = (0..layer.islands.len()).collect();
        while !remaining.is_empty() {
            let nearest = self.nearest(remaining.iter().map(|&island| {
                layer.loops[layer.islands[island].clone()]
                    .iter()
                    .flat_map(|wall| &wall.polygon.points)
                    .copied()
            }));
            let island = remaining.remove(nearest);
            // Walls laid over fill that hangs over air come ahead of the
            // island's loops (`PerimeterGenerator::apply_extra_perimeters`
            // puts them first among its walls).
            // The walls of thin parts that are not loops follow the
            // loops. Gap fill stays with the fill, as in the reference
            // (Fill.cpp, where `thin_fills` join the fills).
            let (mut over_air, mut with_walls, mut rest) = (Vec::new(), Vec::new(), Vec::new());
            let mut ironing = Vec::new();
            for path in fill(island) {
                match path.role {
                    Role::OverhangPerimeter => over_air.push(path),
                    Role::ExternalPerimeter | Role::Perimeter => with_walls.push(path),
                    Role::Ironing => ironing.push(path),
                    _ => rest.push(path),
                }
            }
            if self.settings.infill_first {
                self.lines(&rest, layer.shelter)?;
            }
            self.lines(&over_air, layer.shelter)?;
            self.walls(layer, &layer.islands[island])?;
            self.lines(&with_walls, layer.shelter)?;
            if !self.settings.infill_first {
                self.lines(&rest, layer.shelter)?;
            }
            self.ironing(&ironing, layer.shelter, ironing_fan)?;
        }
        let (ironing, rest): (Vec<&ExtrusionPath>, Vec<&ExtrusionPath>) = fill(layer.islands.len())
            .into_iter()
            .partition(|path| path.role == Role::Ironing);
        self.lines(&rest, layer.shelter)?;
        self.ironing(&ironing, layer.shelter, ironing_fan)
    }

    /// An island's ironing, once its walls and fill are down (GCode.cpp
    /// `process_layer`, "ironing": `extrude_infill` again, for the
    /// ironing alone). The areas ironed are chained as fill is, and the
    /// lines of each keep the order they were made in. With a fan speed
    /// of its own (`ironing_fan_speed`) the fan is set to it for as long
    /// as the ironing lasts (GCode.cpp `apply_role_based_fan_speed`,
    /// CoolingBuffer.cpp). The reference changes the fan as the first
    /// ironing line begins and back as the next line of another kind
    /// does; here it is changed before the move to the ironing and back
    /// straight after it.
    fn ironing(
        &mut self,
        paths: &[&ExtrusionPath],
        shelter: &[ExPolygon],
        fan: Option<u8>,
    ) -> io::Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let Some(fan) = fan else {
            return self.lines(paths, shelter);
        };
        // What is laid over air is behind: its fan gives way first.
        self.fan_over_air(false)?;
        self.out.extend_from_slice(FAN_IRONING);
        writeln!(self.out, "ON {fan}")?;
        self.lines(paths, shelter)?;
        self.out.extend_from_slice(FAN_IRONING);
        writeln!(self.out, "OFF {fan}")
    }

    /// Turns the layer's ironing fan markers into fan commands, now that
    /// the layer's own fan speed is known: the ironing speed where
    /// ironing starts and `layer` where it ends, or nothing where the
    /// two are the same (CoolingBuffer.cpp skips a command for the speed
    /// the fan already runs at).
    fn settle_ironing_fan(&mut self, layer: u8) {
        if find(&self.out, FAN_IRONING).is_none() {
            return;
        }
        let text = std::mem::take(&mut self.out);
        for line in text.split_inclusive(|byte| *byte == b'\n') {
            let Some(marked) = line.strip_prefix(FAN_IRONING) else {
                self.out.extend_from_slice(line);
                continue;
            };
            let marked = String::from_utf8_lossy(marked);
            let mut words = marked.split_whitespace();
            let on = words.next() == Some("ON");
            let ironing: Option<u8> = words.next().and_then(|word| word.parse().ok());
            if let Some(ironing) = ironing.filter(|ironing| *ironing != layer) {
                let speed = if on { ironing } else { layer };
                self.out
                    .extend_from_slice(format!("M106 S{speed}\n").as_bytes());
            }
        }
    }

    /// One island's walls in the configured order.
    fn walls(&mut self, layer: &Layer<'_>, island: &Range<usize>) -> io::Result<()> {
        let s = self.settings;
        let own = &layer.loops[island.clone()];
        let mut walls: Vec<&ExtrusionLoop> = nested(own).into_iter().map(|at| &own[at]).collect();
        match s.wall_order {
            WallOrder::InnerOuter => {}
            WallOrder::OuterInner => walls.reverse(),
            // "Sandwich mode. Apply after 1st layer."
            WallOrder::InnerOuterInner => {
                if !self.state.first_layer {
                    walls.reverse();
                    walls = sandwich(walls);
                }
            }
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
                    if s.seam == Seam::Random {
                        // By the height the layer was sliced at.
                        let z = layer.z - layer.height / 2.0;
                        return (wall.is_hole, seam::random_point(points, z));
                    }
                    let corner =
                        seam::choose(points, s.seam, s.nozzle_diameter, None, &self.state.seams);
                    (wall.is_hole, points[corner])
                })
                .collect()
        };
        self.state
            .next_seams
            .extend(planned.iter().map(|(_, seam)| *seam));
        // How far each of those walls turns at its seam, for the walls
        // behind that are to start further round: nothing where the seam
        // is not at a corner.
        let seam_turns: Vec<(Point2, f64)> = if s.staggered_inner_seams && s.seam != Seam::Random {
            walls
                .iter()
                .filter(|wall| wall.depth == 0 && wall.polygon.points.len() >= 3)
                .zip(&planned)
                .map(|(wall, (_, seam))| {
                    let points = &wall.polygon.points;
                    let turn = points
                        .iter()
                        .position(|point| point == seam)
                        .map_or(0.0, |corner| {
                            seam::turn_at(points, corner, s.nozzle_diameter)
                        });
                    // Round a hole the walls turn the other way.
                    (*seam, if wall.is_hole { -turn } else { turn })
                })
                .collect()
        } else {
            Vec::new()
        };

        for wall in walls {
            let points = &wall.polygon.points;
            if points.len() < 3 {
                continue;
            }
            // A loop is held counter-clockwise round material and
            // clockwise round a hole; every wall is laid the same way
            // round, which turns one or the other about.
            let clockwise = match s.wall_direction {
                WallDirection::Auto => wall.clockwise,
                WallDirection::CounterClockwise => false,
                WallDirection::Clockwise => true,
            };
            let turned = wall.is_hole != clockwise;
            let (edge, start) = match alongside(points, wall.is_hole, &planned) {
                Some(target) => {
                    let (edge, start) = seam::project(points, target);
                    if s.staggered_inner_seams && wall.depth > 0 {
                        // On round the wall, the way it is laid, by how
                        // deep it lies behind the seam it starts beside
                        // and no less than its own width (SeamPlacer.cpp
                        // `place_seam`, "do the staggering"). At a
                        // corner the distance to that seam is more than
                        // the depth, and is brought back by the corner's
                        // angle; in an inside corner it is less. The
                        // reference also starts an inside corner's walls
                        // nearer the corner before it moves them on,
                        // which is not done here.
                        let behind =
                            unscaled(target.x - start.x).hypot(unscaled(target.y - start.y));
                        let turn = seam_turns
                            .iter()
                            .find(|(seam, _)| *seam == target)
                            .map_or(0.0, |(_, turn)| *turn);
                        let beta = (turn / 2.0).cos();
                        let depth = if turn < -1e-4 && beta > 1e-6 {
                            std::f64::consts::SQRT_2 * behind / beta
                        } else {
                            behind * beta / std::f64::consts::SQRT_2
                        };
                        let depth = wall.flow.width.max(depth);
                        seam::advance(points, edge, start, scaled(depth) as f64, turned)
                    } else {
                        (edge, start)
                    }
                }
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
            // A small loop goes at its own speed (GCode.cpp `extrude_loop`,
            // "check loop length for small perimeter").
            let small = wall.length_mm() <= s.small_perimeter_threshold * std::f64::consts::TAU;
            self.state.small_wall = small.then(|| {
                if s.small_perimeter_speed == FloatOrPercent::Value(0.0) {
                    0.5 * s.outer_wall_speed
                } else {
                    s.small_perimeter_speed.resolve(s.outer_wall_speed)
                }
            });
            self.state.circle_wall = self.circle_wall_speed(wall);
            let mut path = seam::open_at(points, edge, start);
            if turned {
                path.reverse();
            }
            let whole = (!wall.widths.is_empty()).then(|| path.clone());
            seam::clip_end(&mut path, s.seam_gap);
            let Some(whole) = whole else {
                self.path(&path, wall.role, &wall.flow, false, true)?;
                continue;
            };
            // A wall whose width changes is laid from its seam as pieces
            // end to end, each of one width (the reference's
            // `thick_polyline_to_multi_path`).
            let widths: Vec<f64> = wall.widths.iter().map(|&width| width as f64).collect();
            let mut widths = seam::open_widths_at(points, &widths, edge, start);
            if turned {
                widths.reverse();
            }
            let widths = seam::clip_widths(&whole, &widths, &path);
            if path.len() < 2 {
                continue;
            }
            let line = edge_medial::ThickPolyline {
                widths: widths
                    .windows(2)
                    .flat_map(|pair| [pair[0], pair[1]])
                    .collect(),
                points: path,
                open_ends: (true, true),
            };
            for piece in edge_medial::varying_width(&line, wall.role, &wall.flow) {
                self.path(&piece.points, wall.role, &piece.flow, false, true)?;
            }
            // The nozzle is wiped along the whole wall, not its last piece.
            self.state.wipe_path = line.points;
        }
        self.state.small_wall = None;
        self.state.circle_wall = None;
        Ok(())
    }

    /// Open lines, ordered as the reference orders them. Fill comes in
    /// parts, one for each area filled: the parts are chained by their
    /// closest ends from where the nozzle is, and then the lines of each
    /// part the same way, so an area is finished before the next is
    /// begun (GCode.cpp `extrude_infill`, ShortestPath.cpp
    /// `chain_and_reorder_extrusion_entities`). Lines that are no part of
    /// an area are chained kind by kind as they come. Lines laid in turn
    /// keep their order and direction, and a tree branch's wall is never
    /// begun at its far end. Moves that stay inside `shelter` may skip
    /// retraction.
    fn lines(&mut self, paths: &[&ExtrusionPath], shelter: &[ExPolygon]) -> io::Result<()> {
        // Fill of every kind is ordered together; other lines by kind.
        for stage in paths
            .chunk_by(|a, b| (a.part != 0 && b.part != 0) || a.role == b.role && a.part == b.part)
        {
            let mut parts: Vec<Part<'_>> = stage
                .chunk_by(|a, b| a.role == b.role && a.part == b.part && (a.part != 0 || b.in_turn))
                .filter_map(Part::new)
                .collect();
            let ends: Vec<[Point2; 2]> = parts.iter().map(Part::ends).collect();
            let turnable: Vec<bool> = parts.iter().map(Part::reversible).collect();
            let order = chain::chain(&ends, &|part| turnable[part], self.state.position);
            for (part, turned) in order {
                let part = std::mem::take(&mut parts[part]);
                // The lines of an area, chained in their turn from here.
                let members: Vec<(&ExtrusionPath, bool)> = if part.runs.len() == 1 {
                    let reversed = turned && part.reversible();
                    part.runs[0].iter().map(|path| (*path, reversed)).collect()
                } else {
                    let ends: Vec<[Point2; 2]> =
                        part.runs.iter().map(|run| run_ends(run)).collect();
                    let turnable: Vec<bool> =
                        part.runs.iter().map(|run| run_reversible(run)).collect();
                    chain::chain(&ends, &|run| turnable[run], self.state.position)
                        .into_iter()
                        .flat_map(|(run, reversed)| {
                            part.runs[run].iter().map(move |path| (*path, reversed))
                        })
                        .collect()
                };
                for (path, reversed) in members {
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
        }
        Ok(())
    }

    /// The prime tower's lines, nearest end first: they are given in the
    /// order they are meant to go down, and the next is always the nearest.
    fn tower_lines(&mut self, paths: &[ExtrusionPath]) -> io::Result<()> {
        let mut remaining: Vec<&ExtrusionPath> =
            paths.iter().filter(|path| path.points.len() >= 2).collect();
        while !remaining.is_empty() {
            let ends = |path: &ExtrusionPath| [path.points[0], path.points[path.points.len() - 1]];
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
            self.path(&points, path.role, &path.flow, false, false)?;
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
            matches!(
                role,
                Role::BridgeInfill | Role::InternalBridge | Role::OverhangPerimeter
            ) || (fan.at.is_none() && role == Role::ExternalPerimeter)
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
                let mut usual =
                    self.state
                        .small_wall
                        .unwrap_or(if role == Role::ExternalPerimeter {
                            s.outer_wall_speed
                        } else {
                            s.inner_wall_speed
                        });
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
                    // With nothing under it at all a wall goes at the
                    // bridge's speed, or with
                    // `slowdown_for_curled_perimeters` at the last of
                    // the overhang speeds.
                    (
                        1.0,
                        if s.slowdown_for_curled_perimeters {
                            step(s.overhang_speeds[3])
                        } else {
                            s.bridge_speed
                        },
                    ),
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
                // A wall over air is a bridge of the inner walls' width,
                // whichever wall it is (`PerimeterGenerator` `overhang_flow`).
                // At `bridge_line_width` where that is set.
                let bridge = self
                    .inner_wall
                    .unwrap_or(*flow)
                    .bridging_sized(s.bridge_flow, s.thick_bridges, s.bridge_line_width)
                    .map_err(|e| io::Error::other(e.to_string()))?;
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
            // Not as the reference, which gives every line of the first
            // layer but a bottom surface's the first layer's speed
            // outright (GCode.cpp `_extrude`), ironing with the rest: a
            // first layer ironed (`ironing_type` at `solid`) is then
            // ironed at `initial_layer_infill_speed`, 105 mm/s with the
            // U1's presets where ironing is set to 30. Ironing is slow
            // because that is how it smooths, so here it is never laid
            // faster than its own speed, and on the first layer no
            // faster than that layer's fill either.
            Role::Ironing if first_layer && s.initial_layer_infill_speed > 0.0 => {
                s.ironing_speed.min(s.initial_layer_infill_speed)
            }
            Role::Ironing => s.ironing_speed,
            _ if first_layer => s.initial_layer_infill_speed,
            Role::ExternalPerimeter => self.wall_speed(s.outer_wall_speed),
            Role::Perimeter => self.wall_speed(s.inner_wall_speed),
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
            | Role::Ironing
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
            // The limit is said both ways (GCodeWriter.cpp
            // `append_klipper_accel_to_decel`): Klipper before 0.13 and
            // vendor forks read ACCEL_TO_DECEL, current Klipper only
            // MINIMUM_CRUISE_RATIO, which is one less the same share.
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
        let exposed = !sheltered
            || matches!(
                self.state.role,
                Some(Role::ExternalPerimeter | Role::OverhangPerimeter)
            );
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
        let held = role == Role::ExternalPerimeter && s.dont_slow_down_outer_wall
            || self.circle_wall_held(role);
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
        let share = match role {
            Role::TopSolidInfill => s.top_solid_infill_flow_ratio,
            Role::BottomSurface => s.bottom_solid_infill_flow_ratio,
            Role::InternalBridge => s.internal_bridge_flow,
            Role::Brim => s.brim_flow_ratio,
            _ => 1.0,
        };
        let volume =
            length * flow.mm3_per_mm() * s.filament_flow_ratio * s.print_flow_ratio * share;
        let feed = volume / self.filament_area;
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
        if s.machine_start_gcode.is_some() {
            writeln!(self.out, "M73 P100 R0")?;
        }
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
        // The times follow, once the file has been timed.
        Ok(summary)
    }
}

/// Lines laid together: the lines of one filled area, or a line by
/// itself. Its runs are what may be reordered within it: a run is a line,
/// or several laid in turn.
#[derive(Default)]
struct Part<'a> {
    runs: Vec<Vec<&'a ExtrusionPath>>,
}

impl<'a> Part<'a> {
    /// `None` when none of the lines is long enough to lay.
    fn new(paths: &[&'a ExtrusionPath]) -> Option<Self> {
        let runs: Vec<Vec<&ExtrusionPath>> = paths
            .chunk_by(|_, next| next.in_turn)
            .map(|run| {
                run.iter()
                    .copied()
                    .filter(|path| path.points.len() >= 2)
                    .collect::<Vec<_>>()
            })
            .filter(|run| !run.is_empty())
            .collect();
        (!runs.is_empty()).then_some(Self { runs })
    }

    /// Where the part begins and ends as it is given.
    fn ends(&self) -> [Point2; 2] {
        let (first, last) = (&self.runs[0], &self.runs[self.runs.len() - 1]);
        [run_ends(first)[0], run_ends(last)[1]]
    }

    /// Whether the part may be begun at its far end: not when its lines
    /// are laid in turn (the reference's `no_sort`).
    fn reversible(&self) -> bool {
        self.runs.len() > 1 || run_reversible(&self.runs[0])
    }
}

/// Where a run begins and ends.
fn run_ends(run: &[&ExtrusionPath]) -> [Point2; 2] {
    let last = run[run.len() - 1];
    [run[0].points[0], last.points[last.points.len() - 1]]
}

/// Whether a run may be laid from its far end: a single line may, but
/// for a tree branch's wall, which is laid hook first (SupportCommon.cpp
/// `tree_supports_generate_paths`) and is known by its ends: an open
/// support line that comes back to within its hook's length of where it
/// began. Lines laid in turn may not, and nor may an ironing line, even
/// one that is the whole of its area's ironing: it is swept from the side
/// it was made to start from (the reference's collection of an area's
/// ironing is `no_sort`, Fill.cpp `make_ironing`).
fn run_reversible(run: &[&ExtrusionPath]) -> bool {
    let [path] = run else { return false };
    if path.role == Role::Ironing {
        return false;
    }
    let [head, tail] = run_ends(run);
    let hooked = path.role == Role::Support
        && head != tail
        && distance(head, tail) <= HOOK_REACH * path.flow.spacing;
    !hooked
}

/// An island's walls from the inside out, each wall next to the one it
/// lies against: every hole's walls from the one deepest in the part to
/// the one on the hole, then the outline's from the innermost out
/// (PerimeterGenerator.cpp `process_classic` and `traverse_loops`, which
/// nest a hole's wall in the next wall round the same hole, or failing
/// that in the innermost wall of the outline that holds it, and each of
/// the outline's walls in the one outside it). The nozzle then moves a
/// line's width from a wall to the next, where taking all the inner walls
/// before any outer one sends it across the island between them.
fn nested(walls: &[ExtrusionLoop]) -> Vec<usize> {
    let deepest = walls.iter().map(|wall| wall.depth).max().unwrap_or(0);
    let holds = |outer: usize, inner: usize| {
        walls[inner]
            .polygon
            .points
            .first()
            .is_some_and(|&point| inside::within(&walls[outer].polygon, point))
    };
    let around = |inner: usize, is_hole: bool, depth: usize| {
        (0..walls.len()).find(|&outer| {
            outer != inner
                && walls[outer].is_hole == is_hole
                && walls[outer].depth == depth
                && holds(outer, inner)
        })
    };
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); walls.len()];
    let mut roots = Vec::new();
    let mut place = |wall: usize, parent: Option<usize>| match parent {
        Some(parent) => children[parent].push(wall),
        None => roots.push(wall),
    };
    for depth in 0..=deepest {
        for (wall, _) in walls
            .iter()
            .enumerate()
            .filter(|(_, wall)| wall.is_hole && wall.depth == depth)
        {
            let parent = (depth + 1..=deepest)
                .find_map(|further| around(wall, true, further))
                .or_else(|| (0..=deepest).rev().find_map(|at| around(wall, false, at)));
            place(wall, parent);
        }
    }
    for depth in (0..=deepest).rev() {
        for (wall, _) in walls
            .iter()
            .enumerate()
            .filter(|(_, wall)| !wall.is_hole && wall.depth == depth)
        {
            let parent = (0..depth).rev().find_map(|at| around(wall, false, at));
            place(wall, parent);
        }
    }
    // An outline's walls come after what lies inside them; a hole's
    // before the walls nearer the hole.
    fn visit(walls: &[ExtrusionLoop], children: &[Vec<usize>], wall: usize, out: &mut Vec<usize>) {
        if walls[wall].is_hole {
            out.push(wall);
        }
        for &child in &children[wall] {
            visit(walls, children, child, out);
        }
        if !walls[wall].is_hole {
            out.push(wall);
        }
    }
    // Outlines last among those with nothing round them, so a hole's
    // wall that was matched to none still goes down before the outside.
    roots.sort_by_key(|&wall| (!walls[wall].is_hole, std::cmp::Reverse(walls[wall].depth)));
    let mut order = Vec::with_capacity(walls.len());
    for root in roots {
        visit(walls, &children, root, &mut order);
    }
    order
}

/// Which island each of the layer's fill lines lies in, as an index into
/// its islands, or their count for a line in none. An island is known by
/// its outermost walls around material; a line is in it when its first
/// point is, or the middle of its first segment, or the centre of its
/// bounds, and the smallest island that has it wins, so one standing in
/// another's hole is found before the one around it (GCode.cpp
/// `process_layer`).
fn homes(layer: &Layer<'_>) -> Vec<usize> {
    let outlines = |island: &Range<usize>| {
        layer.loops[island.clone()]
            .iter()
            .filter(|wall| wall.depth == 0 && !wall.is_hole)
            .map(|wall| &wall.polygon)
    };
    let bounds = |points: &mut dyn Iterator<Item = Point2>| {
        points.fold(None, |bounds: Option<(Point2, Point2)>, point| {
            Some(match bounds {
                None => (point, point),
                Some((low, high)) => (
                    Point2 {
                        x: low.x.min(point.x),
                        y: low.y.min(point.y),
                    },
                    Point2 {
                        x: high.x.max(point.x),
                        y: high.y.max(point.y),
                    },
                ),
            })
        })
    };
    let mut order: Vec<(i128, usize)> = layer
        .islands
        .iter()
        .enumerate()
        .map(|(index, island)| {
            let size =
                bounds(&mut outlines(island).flat_map(|outline| outline.points.iter().copied()))
                    .map_or(0, |(low, high)| {
                        i128::from(high.x - low.x) * i128::from(high.y - low.y)
                    });
            (size, index)
        })
        .collect();
    order.sort();
    let home = |path: &ExtrusionPath| {
        let Some(&first) = path.points.first() else {
            return layer.islands.len();
        };
        let mut probes = vec![first];
        if let Some(&second) = path.points.get(1) {
            probes.push(Point2 {
                x: (first.x + second.x) / 2,
                y: (first.y + second.y) / 2,
            });
        }
        if let Some((low, high)) = bounds(&mut path.points.iter().copied()) {
            probes.push(Point2 {
                x: (low.x + high.x) / 2,
                y: (low.y + high.y) / 2,
            });
        }
        probes
            .iter()
            .find_map(|&probe| {
                order.iter().map(|&(_, index)| index).find(|&index| {
                    outlines(&layer.islands[index]).any(|outline| inside::within(outline, probe))
                })
            })
            .unwrap_or(layer.islands.len())
    };
    // A path laid in turn goes wherever the one before it went.
    let mut last = layer.islands.len();
    layer
        .paths
        .iter()
        .map(|path| {
            if !path.in_turn {
                last = home(path);
            }
            last
        })
        .collect()
}

/// Walls given outermost first, put in the order that lays the outer
/// wall between the inner ones: for each outer wall with at least two
/// walls behind it, those from the innermost out to the second inner
/// wall, then the outer wall, then the first inner wall. Fewer walls
/// stay outermost first (PerimeterGenerator.cpp `process_classic`,
/// "sandwich mode", followed step by step).
fn sandwich(walls: Vec<&ExtrusionLoop>) -> Vec<&ExtrusionLoop> {
    if walls.len() <= 2 {
        return walls;
    }
    // First the walls two or more deep are held back until the outer
    // wall they are met after, so each first inner wall is ahead of them.
    let mut reordered: Vec<&ExtrusionLoop> = Vec::with_capacity(walls.len());
    let (mut skipped, mut found_second_internal) = (Vec::new(), false);
    for &wall in &walls {
        match wall.depth {
            0 => {
                if found_second_internal {
                    reordered.append(&mut skipped);
                }
                reordered.push(wall);
            }
            1 => reordered.push(wall),
            _ => {
                skipped.push(wall);
                found_second_internal = true;
            }
        }
    }
    reordered.append(&mut skipped);

    let mut ordered = walls;
    let mut position = 0;
    while position < reordered.len() {
        let (mut outer, mut first_internal, mut second_internal) = (None, None, None);
        let mut max_internal = reordered.len() - 1;
        let mut at = position;
        while at < reordered.len() {
            match reordered[at].depth {
                0 => outer = outer.or(Some(at)),
                1 if first_internal.is_none() && outer.is_some_and(|outer| at > outer) => {
                    first_internal = Some(at);
                }
                2 if second_internal.is_none()
                    && outer.is_some()
                    && first_internal.is_none_or(|first| at > first) =>
                {
                    second_internal = Some(at);
                }
                _ => {}
            }
            // Another outer wall with all three found is another island.
            if outer.is_some()
                && first_internal.is_some()
                && second_internal.is_some()
                && reordered[at].depth == 0
            {
                at -= 1;
                max_internal = at;
                break;
            }
            at += 1;
        }
        let (Some(_), Some(_), Some(second_internal)) = (outer, first_internal, second_internal)
        else {
            break;
        };
        let inside_out = (second_internal..=max_internal).rev();
        let outside_in = position..second_internal;
        for (slot, from) in inside_out.chain(outside_in).enumerate() {
            ordered[position + slot] = reordered[from];
        }
        position = at + 1;
    }
    ordered
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

/// The line everything the printer runs comes after.
const EXECUTABLE_START: &[u8] = b"; EXECUTABLE_BLOCK_START\n";

/// Where `needle` first is in `text`.
fn find(text: &[u8], needle: &[u8]) -> Option<usize> {
    text.windows(needle.len())
        .position(|window| window == needle)
}

/// Markers a layer is written with where the fan is to change for an
/// overhang and back, until the layer's own fan speed is known.
const FAN_MARK: &[u8] = b";_FAN_OVER_AIR_";
const FAN_OVER_AIR: &[u8] = b";_FAN_OVER_AIR_ON\n";
const FAN_AS_LAYER: &[u8] = b";_FAN_OVER_AIR_OFF\n";
/// Starts the line a layer is written with where ironing begins and
/// where it ends, when ironing has a fan speed of its own: `ON` or
/// `OFF`, and that speed.
const FAN_IRONING: &[u8] = b";_FAN_IRONING_";

fn distance(a: Point2, b: Point2) -> f64 {
    unscaled(b.x - a.x).hypot(unscaled(b.y - a.y))
}

/// Feature names as the reference writes them (`ExtrusionEntity::role_to_string`),
/// which is what G-code viewers colour by.
pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::ExternalPerimeter => "Outer wall",
        Role::Perimeter => "Inner wall",
        Role::OverhangPerimeter => "Overhang wall",
        Role::SparseInfill => "Sparse infill",
        Role::SolidInfill => "Internal solid infill",
        Role::TopSolidInfill => "Top surface",
        Role::BottomSurface => "Bottom surface",
        Role::Ironing => "Ironing",
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
mod ironing_tests;
#[cfg(test)]
mod tests;
