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
//! Not done: progress (`M73`) and object labels.

mod arc;
mod avoid;
mod chain;
mod circle;
mod contour;
mod cooling;
mod inside;
mod overhang;
mod scarf;
mod seam;
mod seam_align;
mod seam_plan;
mod settings;
mod simplify;
mod small_area;
mod unsupported;
mod wipe;

use std::{
    collections::HashMap,
    f64::consts::{PI, TAU},
    io::{self, Write},
    ops::Range,
};

use cooling::{Slowdown, Timing};
use edge_avoid::Kind;
use edge_config::FloatOrPercent;
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Flow, Role};
use edge_geom::{ExPolygon, Point2, scaled, unscaled};
use edge_template::{TemplateError, Value, Variables};
use rayon::prelude::*;

pub use contour::Contours;
pub use edge_avoid::{Detour, Slices};
pub use scarf::{Scarf, ScarfSeam};
pub use seam::Seam;
pub use seam_plan::{SeamMesh, SeamObject, SeamPart, SeamScene};
pub use settings::{Lift, OverhangFan, PrintSettings, WallDirection, WallOrder, gcode_template};

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
    /// An arc fitted to a path bulges past the edge of the bed, though
    /// the path's own points are on it.
    #[error(
        "an arc reaches from {:.3} x {:.3} to {:.3} x {:.3} mm, off the bed, which runs from \
         {} x {} to {} x {} mm; no G-code was written",
        reach[0], reach[1], reach[2], reach[3], bed[0], bed[1], bed[2], bed[3]
    )]
    OffBed {
        /// The box round the arc in mm: min x, min y, max x, max y.
        reach: [f64; 4],
        bed: [f64; 4],
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

/// G-code set at a layer by the person (a pause, a custom line, the
/// printer's template), written at the start of the layer after its layer
/// change, as the reference writes it (GCode.cpp
/// `ProcessLayer::emit_custom_gcode_per_print_z`).
#[derive(Clone, Debug, PartialEq)]
pub enum LayerCode {
    /// A pause: the printer's `machine_pause_gcode`, as a template.
    Pause(String),
    /// The printer's `template_custom_gcode`, as a template.
    Template(String),
    /// G-code written as it is, placeholders and all.
    Custom(String),
}

/// A [`LayerCode`] and the layer it is written at, by the layer's top.
#[derive(Clone, Debug, PartialEq)]
pub struct AtLayer {
    /// The top of the layer, as [`Layer::z`] has it.
    pub z: f64,
    pub code: LayerCode,
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

/// How many layers have their arcs fitted at once: enough to keep every
/// thread busy, few enough that what is fitted is not held for the whole
/// print.
const FITTED_AHEAD: usize = 64;

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
    write_routed(out, layers, &[], settings)
}

/// [`write_tools`], with what travel is routed by where the settings
/// ask for it to go round the inside of the walls
/// (`reduce_crossing_wall`): `slices` has an entry for each of `layers`,
/// or none at all, and travel then goes straight whatever the settings
/// say. They are given beside the layers and not in them so that
/// whatever makes layers without them still does.
pub fn write_routed(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    slices: &[Slices<'_>],
    settings: &[PrintSettings],
) -> Result<Summary, GcodeError> {
    write_placed(out, layers, slices, &SeamScene::default(), settings)
}

/// [`write_routed`], with what seams are placed by beside the walls
/// themselves: the objects on the plate and each layer's outline. They
/// too are given beside the layers, and without them the plate is taken
/// for one object and no point for hidden inside the print.
pub fn write_placed(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    slices: &[Slices<'_>],
    scene: &SeamScene<'_>,
    settings: &[PrintSettings],
) -> Result<Summary, GcodeError> {
    write_contoured(out, layers, slices, scene, &Contours::default(), settings)
}

/// [`write_placed`], with the lines contoured in height laid by their
/// contours (`zaa_enabled`, `contour.rs`). They too are given beside the
/// layers: `contours.layers` has an entry for each of `layers`, or none.
pub fn write_contoured(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    slices: &[Slices<'_>],
    scene: &SeamScene<'_>,
    contours: &Contours<'_>,
    settings: &[PrintSettings],
) -> Result<Summary, GcodeError> {
    write_coded(out, layers, slices, scene, contours, &[], settings)
}

/// [`write_contoured`], with G-code set at layers (`codes`): each written
/// at the start of the layer whose top it names. One that names no layer
/// is not written; what is given is matched to the layers beforehand.
pub fn write_coded(
    out: &mut impl Write,
    layers: &[Layer<'_>],
    slices: &[Slices<'_>],
    scene: &SeamScene<'_>,
    contours: &Contours<'_>,
    codes: &[AtLayer],
    settings: &[PrintSettings],
) -> Result<Summary, GcodeError> {
    let profiles = if contours.layers.len() == layers.len() {
        contours.layers
    } else {
        &[]
    };
    let slices = if slices.len() == layers.len() {
        slices
    } else {
        &[]
    };
    let first = layers.first().map_or(0, |layer| layer.filament);
    if let Some(layer) = layers.iter().find(|layer| layer.filament >= settings.len()) {
        return Err(GcodeError::NoSettings {
            filament: layer.filament + 1,
            given: settings.len(),
        });
    }
    // Without arcs the walls are thinned to `resolution` before anything
    // else, for the whole print at once: the seams are planned on the
    // loops as they will be written, the points they are chosen among
    // being those left (`simplify.rs`), and the writer goes by the same
    // loops, which the plan knows by where they are kept.
    let walls = simplify::Thinned::walls(layers, settings, profiles);
    let layers = &walls.layers(layers, 0)[..];
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
        seams: seam_plan::Plan::new(layers, &groups, settings, scene),
        inner_wall: None,
        measured: Vec::new(),
        measured_used: 0,
        fits: HashMap::new(),
        off_bed: None,
        slices,
        routes: Vec::new(),
        routes_from: 0,
        ways: HashMap::new(),
        shelters: HashMap::new(),
        contoured: HashMap::new(),
        contour_speed: contours.speed_scaling,
        codes,
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
    // The stretch of layers in hand with their paths thinned, and the
    // entry it starts at.
    let mut thinned = simplify::Thinned::default();
    let mut thinned_from = 0;
    for (index, group) in groups.iter().enumerate() {
        writer.out.clear();
        reached.push((whole.len(), writer.state.summary.seconds));
        if index == 1 {
            second_layer = Some(whole.len());
        }
        // Arcs are fitted a stretch of layers at a time, ahead of
        // laying them; without arcs the paths are thinned to
        // `resolution` instead, as the reference does one or the other
        // before it writes anything (PrintObject.cpp
        // `simplify_extrusion_path`). The walls were thinned already.
        if index % FITTED_AHEAD == 0 {
            let ahead = &groups[index..(index + FITTED_AHEAD).min(groups.len())];
            let until = groups[(index + FITTED_AHEAD).min(groups.len()) - 1].end;
            writer.prefit(&layers[group.start..until]);
            thinned = simplify::Thinned::lines(
                &layers[group.start..until],
                settings,
                profiles.get(group.start..until).unwrap_or(&[]),
            );
            thinned_from = group.start;
            // And so are the layers made ready to route travel on.
            writer.preroute(layers, ahead, index)?;
        }
        let parts = thinned.layers(&layers[group.clone()], group.start - thinned_from);
        // Which of the lines about to be laid are contoured, by where
        // they are kept now.
        writer.contoured = contour::kept(&parts, profiles.get(group.clone()).unwrap_or(&[]));
        writer.layer(index, &parts, group.start)?;
        // Whole or not at all, and never off the bed: an arc counts by
        // its bulge, which the points it was fitted to do not show.
        if let (Some(reach), Some(bed)) = (writer.off_bed, writer.settings.bed) {
            return Err(GcodeError::OffBed { reach, bed });
        }
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
    /// G-code set at layers, by the layer's top.
    codes: &'a [AtLayer],
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
    /// What each outer wall's seam is chosen by (`seam_plan.rs`).
    seams: seam_plan::Plan,
    /// The line the layer's inner walls are printed with, when it has any.
    inner_wall: Option<Flow>,
    /// Walls measured against it so far this layer, each with what tells
    /// it apart, and how many of them the pass being written has used: a
    /// layer laid out a second time, slower, measures nothing again.
    measured: Vec<(Measured, Vec<Run>)>,
    measured_used: usize,
    /// The arcs of the layers being written, fitted ahead of laying
    /// them: for each wall loop and line, by where it is kept and, for a
    /// loop, whether it is laid the other way round from how its points
    /// run. A layer laid out a second time, slower, fits nothing again.
    fits: HashMap<(usize, bool), arc::Fitted>,
    /// The box round an arc that was found to leave the bed, in mm; the
    /// file is then not written.
    off_bed: Option<[f64; 4]>,
    /// What each entry's travel is routed by, where travel is to go
    /// round the inside of the walls; none where it goes straight.
    slices: &'a [Slices<'a>],
    /// The layers being written made ready to route on, the first of
    /// them layer `routes_from` of the print (`avoid.rs`).
    routes: Vec<edge_avoid::Layer>,
    routes_from: usize,
    /// The ways found so far on the layer being written, by where a
    /// move starts and ends and what kind it is.
    ways: HashMap<(Point2, Point2, Kind), Vec<Point2>>,
    /// The areas a way round has been asked to stay within on the layer
    /// being written, by where they are kept.
    shelters: HashMap<(usize, usize), edge_avoid::Shelter>,
    /// The lines of the layer being written that are contoured in
    /// height, by where they are kept, and whether a contoured move is
    /// slowed for its height (`contour.rs`).
    contoured: HashMap<usize, std::sync::Arc<edge_contour::Profile>>,
    contour_speed: bool,
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

/// A stretch of a wall measured against the layer beneath, as it is to
/// be laid.
#[derive(Clone, Debug)]
struct Run {
    points: Vec<Point2>,
    /// Whether it hangs clear of the layer beneath, and so is laid as an
    /// overhang wall.
    clear: bool,
    /// The points it is written by, with their speeds and cover, where
    /// its speed or its fan changes on the way; empty where it is laid
    /// at one speed along its own points (GCode.cpp `_extrude`,
    /// `variable_speed`).
    readings: Vec<overhang::Reading>,
    /// For a stretch laid at one speed, whether so little of it is
    /// supported that it gets the overhang fan
    /// (`path_overhang_fan_active`).
    cooled: bool,
}

/// How a wall measured against the layer beneath is laid: as itself
/// where it is supported, and as a bridge where it hangs clear.
#[derive(Clone, Copy)]
struct Laid<'a> {
    role: Role,
    flow: &'a Flow,
    motion: Motion,
    /// The line and motion of a stretch that hangs clear.
    bridge: &'a Flow,
    over_air: Motion,
    /// Whether the wall gets the overhang fan wherever it goes, and
    /// whether there is an overhang fan at all.
    cooled: bool,
    fan: bool,
    /// The fan comes on where no more than this share of the wall's
    /// width is supported; none when it does not go by that.
    fan_at: Option<f32>,
}

impl<'a> Laid<'a> {
    /// What a stretch is laid as, with what line and how fast.
    fn of(&self, clear: bool) -> (Role, &'a Flow, Motion) {
        if clear {
            (Role::OverhangPerimeter, self.bridge, self.over_air)
        } else {
            (self.role, self.flow, self.motion)
        }
    }

    /// Whether the overhang fan is on at a point of a stretch with this
    /// much of its width supported (GCode.cpp `overhang_fan_on_at`).
    fn fan_on(&self, clear: bool, cover: f32) -> bool {
        if clear {
            self.fan
        } else {
            self.cooled || self.fan_at.is_some_and(|at| cover <= at)
        }
    }
}

/// Where the machine stands as the text is written. Kept apart so a layer
/// can be laid out once to see how long it takes and then again, slower.
#[derive(Clone, Debug, Default)]
struct State {
    /// Where the nozzle is, once it has been anywhere.
    position: Option<Point2>,
    /// Which layer of the print is being written, and how its next
    /// travel is routed.
    layer: usize,
    routing: avoid::Routing,
    /// The current layer's height above the bed.
    z: f64,
    /// How high the nozzle actually is, once a move has said.
    height: Option<f64>,
    /// The stretch of a scarf seam being laid, and how far along it the
    /// nozzle is (`scarf.rs`).
    slope: Option<scarf::Slope>,
    /// The line contoured in height being laid (`contour.rs`), and the
    /// heights of the one `wipe_path` is the end of, where it was.
    contour: Option<contour::Laying>,
    wipe_heights: Option<std::sync::Arc<edge_contour::Profile>>,
    first_layer: bool,
    /// The speed of the wall loop being laid, where it is small enough
    /// to have one of its own (`small_perimeter_speed`).
    small_wall: Option<f64>,
    /// The speed of the wall loop being laid, where it goes round a
    /// circle the auto circle compensation resized (`circle.rs`).
    circle_wall: Option<f64>,
    /// Whether the fill line being laid has its short moves thinned
    /// (`small_area.rs`).
    short_lines: bool,
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

    /// G-code set at a layer, with the tag the reference's G-code viewer
    /// and time estimator know it by (GCode.cpp
    /// `emit_custom_gcode_per_print_z`): a pause's G-code and a template
    /// are filled in like any of the printer's; custom G-code is written
    /// as it is. Each ends with a new line, as there.
    ///
    /// The two templates are given what the reference gives them: not the
    /// layer's own number and height, which it sets for the layer-change
    /// G-code alone, so a template that names them is refused here as it
    /// is there.
    fn layer_code(&mut self, code: &LayerCode) -> Result<(), GcodeError> {
        let expand = |name: &'static str, template: &str, variables: &Variables| {
            let mut variables = variables.clone();
            for local in ["layer_num", "layer_z", "max_layer_z"] {
                variables.remove(local);
            }
            edge_template::expand(template, &variables)
                .map_err(|source| GcodeError::Template { name, source })
        };
        match code {
            LayerCode::Pause(template) => {
                let text = expand("machine_pause_gcode", template, &self.variables)?;
                write!(self.out, ";PAUSE_PRINT\n{text}\n")?;
            }
            LayerCode::Template(template) => {
                let text = expand("template_custom_gcode", template, &self.variables)?;
                write!(self.out, ";CUSTOM_GCODE\n{text}")?;
            }
            LayerCode::Custom(text) => write!(self.out, ";CUSTOM_GCODE\n{text}")?,
        }
        writeln!(self.out)?;
        Ok(())
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
        // What is flushed: on a printer of one nozzle, the project's
        // flushing volume from the filament put away to the new one, as a
        // length of filament, which the printer's own G-code may purge
        // (GCode.cpp `set_extruder`: `flush_length`, half of it each as
        // `first_flush_volume` and `second_flush_volume`, and in chunks of
        // at most 135 mm³, at most four, as `flush_length_1` to `_4`).
        // Nothing is flushed between tools that each have a nozzle of
        // their own; the reference gives their templates the volume all
        // the same, which none of theirs reads.
        let flushed = match (old, self.all[to].tools) {
            (Some(old), 1) => {
                let count = (new_settings.flush_volumes.len() as f64).sqrt().round() as usize;
                new_settings
                    .flush_volumes
                    .get(old * count + to)
                    .copied()
                    .filter(|_| old < count && to < count)
                    .unwrap_or(0.0)
            }
            _ => 0.0,
        };
        let length = (flushed as f32 / area(new_settings) as f32) as f64;
        v.set("flush_length", Value::Float(length));
        v.set("first_flush_volume", Value::Float(length / 2.0));
        v.set("second_flush_volume", Value::Float(length / 2.0));
        let chunks = ((flushed as f32 / 135.0).round() as usize).clamp(1, 4);
        for chunk in 1..=4 {
            let each = if chunk <= chunks {
                length / chunks as f64
            } else {
                0.0
            };
            v.set(format!("flush_length_{chunk}"), Value::Float(each));
        }
        for name in [
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
                    self.travel(line.points[0], false, None)?;
                    for point in &line.points[1..] {
                        self.extrude(*point, line.role, &line.flow, motion)?;
                    }
                }
                let length = self.state.position.map_or(0.0, |at| distance(at, ram.park));
                self.go(ram.park, None, RAM_PARK_SPEED, length)?;
            }
            None => {
                if let Some(over) = over.filter(|_| self.state.primed) {
                    self.travel(over, false, None)?;
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
        // What the person set at this layer, after the layer change and
        // whatever the second layer resets, before anything is printed.
        if let Some(code) = self.codes.iter().find(|code| code.z == layer.z) {
            self.layer_code(&code.code)?;
        }
        self.state.first_layer = index == 0;
        self.state.layer = index;
        self.ways.clear();
        self.shelters.clear();
        // The nozzle itself moves up with the layer's first travel.
        self.state.z = layer.z;
        self.below = if index == 0 {
            None
        } else {
            // What is asked of it most often is within a wall's width
            // of the outline beneath, and the corners of that outline
            // grown by half a nozzle reach three times as far as that:
            // by the widest nozzle a wall of this print may be laid
            // with.
            let widest = (self.all.iter())
                .map(|filament| filament.nozzle_diameter)
                .fold(s.nozzle_diameter, f64::max);
            overhang::Below::new(layer.below, (1.5 * widest).max(0.6))
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
            // To a prime tower, on it and away from it are moves outside
            // the part (GCode.cpp `append_tcr`: `use_external_mp_once`
            // before the move to the tower and after the tower). The
            // first move with a tool just taken up starts from nowhere
            // known and goes straight, here as there.
            let towered = layer.filament != self.state.tool || !layer.tower.is_empty();
            if towered {
                self.state.routing.kind = Kind::Outside;
            }
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
                    self.travel(*point, false, None)?;
                }
            }
            self.inner_wall = layer
                .loops
                .iter()
                .find(|wall| wall.role == Role::Perimeter)
                .map(|wall| wall.flow);
            self.tower_lines(layer.tower)?;
            self.state.routing.outside_once |= towered;
            // So are the moves to and between skirt and brim (GCode.cpp
            // `use_external_mp` round each). From the first layer's
            // skirt, and from a brim, the move to whatever is printed
            // next goes straight (`disable_once`).
            let mut before = None;
            for around in layer.around {
                let points = &around.polygon.points;
                if points.len() < 3 {
                    continue;
                }
                self.state.routing.kind = Kind::Outside;
                if before == Some(Role::Skirt) && around.role != Role::Skirt {
                    self.state.routing.straight_once |= self.state.first_layer;
                }
                before = Some(around.role);
                // Skirt and brim start wherever is nearest.
                let (edge, start) = match self.state.position {
                    Some(at) => seam::project(points, at),
                    None => (0, points[0]),
                };
                let mut path = seam::open_at(points, edge, start);
                seam::clip_end(&mut path, self.settings.seam_gap);
                self.path(&path, around.role, &around.flow, false, true)?;
            }
            self.state.routing.straight_once |= match before {
                Some(Role::Skirt) => self.state.first_layer,
                Some(_) => true,
                None => false,
            };
            // Supports stand outside the part, and their moves are
            // routed by its outline wherever they go (the reference's
            // `is_support_layer`).
            self.state.routing.kind = Kind::Support;
            self.lines(&layer.support.iter().collect::<Vec<_>>(), &[])?;
            self.state.routing.kind = Kind::Part;
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
            // Loops wholly over air, with `unsupported_wall_last`.
            let held = self.held(&layer.loops[layer.islands[island].clone()], &with_walls);
            if self.settings.infill_first {
                self.lines(&rest, layer.shelter)?;
            }
            self.lines(&over_air, layer.shelter)?;
            self.walls(layer, &layer.islands[island], held.as_ref(), false)?;
            self.lines(&with_walls, layer.shelter)?;
            if !self.settings.infill_first {
                self.lines(&rest, layer.shelter)?;
                // Then the loops left hanging in mid air, now that the
                // fill can anchor them (GCode.cpp `process_layer`, the
                // second `extrude_perimeters`). With the fill laid first
                // nothing is held back: the fill is down already.
                if let Some(held) = held.as_ref().filter(|held| held.any_after_fill()) {
                    self.walls(layer, &layer.islands[island], Some(held), true)?;
                }
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

    /// Which of an island's loops are over air and which of those wait
    /// for its fill, where `unsupported_wall_last` asks and overhangs are
    /// told apart on this layer; none otherwise, or where no loop is over
    /// air. `thin` are the island's walls that are no loops.
    fn held(&self, own: &[ExtrusionLoop], thin: &[&ExtrusionPath]) -> Option<unsupported::Held> {
        if !self.settings.unsupported_wall_last {
            return None;
        }
        let (below, by) = self.clear_of()?;
        let over_air: Vec<bool> = (own.iter())
            .map(|wall| unsupported::over_air(below, by, wall))
            .collect();
        if !over_air.contains(&true) {
            return None;
        }
        Some(unsupported::held(
            own,
            over_air,
            thin,
            unsupported::reach(own),
        ))
    }

    /// One island's walls in the configured order. With `held`, the loops
    /// over air go after the rest, innermost first, and those that wait
    /// for the fill are left out, or with `after_fill` are all that is laid.
    fn walls(
        &mut self,
        layer: &Layer<'_>,
        island: &Range<usize>,
        held: Option<&unsupported::Held>,
        after_fill: bool,
    ) -> io::Result<()> {
        let s = self.settings;
        let own = &layer.loops[island.clone()];
        let mut order = nested(own);
        match s.wall_order {
            WallOrder::InnerOuter => {}
            WallOrder::OuterInner => order.reverse(),
            // "Sandwich mode. Apply after 1st layer."
            WallOrder::InnerOuterInner => {
                if !self.state.first_layer {
                    order.reverse();
                    let walls: Vec<&ExtrusionLoop> = order.iter().map(|&at| &own[at]).collect();
                    let sandwiched = sandwich(walls);
                    order = sandwiched
                        .into_iter()
                        .map(|wall| {
                            own.iter()
                                .position(|other| std::ptr::eq(other, wall))
                                .expect("a wall of the island")
                        })
                        .collect();
                }
            }
        }
        // Loops over air last (PerimeterGenerator.cpp
        // `defer_unsupported_loops`, after the wall order is settled).
        if let Some(held) = held {
            order = unsupported::reorder(own, order, held);
            let hold = !s.infill_first;
            order.retain(|&at| !hold || held.after_fill[at] == after_fill);
        }
        let walls: Vec<&ExtrusionLoop> = order.into_iter().map(|at| &own[at]).collect();
        // The walls laid so far, for a wipe over them (`wipe.rs`).
        let mut printed = wipe::Printed::default();
        for wall in walls {
            let points = &wall.polygon.points;
            if points.len() < 3 {
                continue;
            }
            // Contoured in height, where it is (`contour.rs`).
            self.contour(Some(address(wall)));
            // A loop is held counter-clockwise round material and
            // clockwise round a hole; every wall is laid the same way
            // round, which turns one or the other about.
            let clockwise = match s.wall_direction {
                WallDirection::Auto => wall.clockwise,
                WallDirection::CounterClockwise => false,
                WallDirection::Clockwise => true,
            };
            let turned = wall.is_hole != clockwise;
            // An outermost wall starts at its seam and a wall behind
            // one beside it (`seam_plan.rs`).
            let placed = self.seams.place(
                self.state.layer,
                wall,
                turned,
                self.state.position,
                s.staggered_inner_seams,
            );
            let (edge, start) = match placed {
                Some(placed) => (placed.edge, placed.start),
                // With no outer wall on the layer to go by, at the
                // point nearest the nozzle.
                None => seam::project(points, self.state.position.unwrap_or(points[0])),
            };
            let unsupported = placed.map(|placed| placed.unsupported);
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
            // Joined with a scarf, the loop is laid whole and its start
            // on a slope (`scarf.rs`); otherwise it stops short of it.
            if self.scarf_wall(layer, own, wall, turned, &path, (edge, start), unsupported)? {
                self.wipe_after_wall(wall, turned, wall.flow.width, &mut printed)?;
                continue;
            }
            let whole = (!wall.widths.is_empty()).then(|| path.clone());
            seam::clip_end(&mut path, s.seam_gap);
            let Some(whole) = whole else {
                let fitted = self.fit_loop(wall, turned, path[0]);
                let laid = fitted.as_ref().map_or(&path, |fitted| &fitted.points);
                self.wipe_before_wall(own, wall, turned, laid, wall.flow.width)?;
                match fitted {
                    Some(fitted) => self.path_parts(
                        &fitted.points,
                        Some(&fitted.parts),
                        wall.role,
                        &wall.flow,
                        false,
                        true,
                    )?,
                    None => self.path(&path, wall.role, &wall.flow, false, true)?,
                }
                self.wipe_after_wall(wall, turned, wall.flow.width, &mut printed)?;
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
            let pieces = edge_medial::varying_width(&line, wall.role, &wall.flow);
            let width = pieces
                .first()
                .map_or(wall.flow.width, |piece| piece.flow.width);
            self.wipe_before_wall(own, wall, turned, &line.points, width)?;
            for piece in pieces {
                self.curved(&piece.points, wall.role, &piece.flow, false, true)?;
            }
            // The nozzle is wiped along the whole wall, not its last piece.
            self.state.wipe_path = line.points;
            self.wipe_after_wall(wall, turned, width, &mut printed)?;
        }
        self.state.small_wall = None;
        self.state.circle_wall = None;
        self.contour(None);
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
        // A line of gap fill between walls is a filled area of its own
        // there, chained with the rest (Fill.cpp `make_fills` gives each
        // of a region's `thin_fills` a collection of its own among its
        // `fills`, which `extrude_infill` chains together).
        let fill = |path: &ExtrusionPath| path.part != 0 || path.role == Role::GapFill;
        for stage in
            paths.chunk_by(|a, b| (fill(a) && fill(b)) || a.role == b.role && a.part == b.part)
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
                    // By the way the move takes, where it is routed
                    // (GCode.cpp `travel_to` asks `needs_retraction`
                    // again of the travel it planned).
                    let mut sheltered = false;
                    if self.settings.reduce_infill_retraction
                        && let Some(at) = self.state.position
                    {
                        let way = self.way(points[0]);
                        sheltered = match way[..] {
                            [to] => inside::stays_inside(shelter, at, to),
                            _ => self.way_sheltered(shelter, at, &way),
                        };
                    }
                    // A short move of solid fill gets less plastic
                    // (`small_area_infill_flow_compensation`).
                    let thinned = self.settings.small_area.as_ref();
                    self.state.short_lines = thinned.is_some_and(|model| model.applies(path));
                    self.contour(Some(address(path)));
                    // Arcs are fitted to the line as it was made and turned
                    // about with it, as in the reference, where fitting
                    // comes before the order lines are laid in is chosen.
                    match self.fit_line(path) {
                        Some(mut fitted) => {
                            if reversed {
                                fitted.reverse();
                            }
                            self.path_parts(
                                &fitted.points,
                                Some(&fitted.parts),
                                path.role,
                                &path.flow,
                                sheltered,
                                false,
                            )?;
                        }
                        None => self.path(&points, path.role, &path.flow, sheltered, false)?,
                    }
                    self.state.short_lines = false;
                    self.contour(None);
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
        self.path_parts(points, None, role, flow, sheltered, closed)
    }

    /// Fits the arcs of some layers ahead of laying them, every loop and
    /// line at once on as many threads as there are: fitting is most of
    /// the work of writing a curved part, and what is fitted to a path
    /// does not depend on when its turn comes. Whatever is not found
    /// here when it is wanted is fitted then.
    fn prefit(&mut self, parts: &[Layer<'_>]) {
        self.fits.clear();
        let mut loops: Vec<(&PrintSettings, &ExtrusionLoop)> = Vec::new();
        let mut lines: Vec<(&PrintSettings, &ExtrusionPath)> = Vec::new();
        for part in parts {
            let s = &self.all[part.filament];
            if !s.arc_fitting {
                continue;
            }
            // A loop whose width changes is fitted piece by piece as it
            // is laid.
            loops.extend(
                (part.loops.iter())
                    .filter(|wall| wall.widths.is_empty() && wall.polygon.points.len() >= 3)
                    .map(|wall| (s, wall)),
            );
            lines.extend(
                (part.support.iter().chain(part.paths))
                    .filter(|path| path.points.len() >= 3)
                    .map(|path| (s, path)),
            );
        }
        let loops = loops.par_iter().filter_map(|(s, wall)| {
            let turned = wall_turned(s, wall);
            let fitted = fit_closed(s, &wall.polygon.points, turned, wall.role)?;
            Some(((address(*wall), turned), fitted))
        });
        let lines = lines.par_iter().filter_map(|(s, path)| {
            Some(((address(*path), false), fit(s, &path.points, path.role)?))
        });
        self.fits = loops.chain(lines).collect();
    }

    /// The arcs in a line as it was made: fitted ahead with its layer,
    /// or now. None when arcs are not asked for, or it is too short for
    /// one.
    fn fit_line(&self, path: &ExtrusionPath) -> Option<arc::Fitted> {
        // "ZAA: a contoured path carries per-point Z, which an arc move
        // cannot express."
        if self.contouring() {
            return None;
        }
        match self.fits.get(&(address(path), false)) {
            Some(fitted) => Some(fitted.clone()),
            None => fit(self.settings, &path.points, path.role),
        }
    }

    /// A wall loop with its arcs, once round from `seam` and stopped
    /// short of it by the seam gap; `turned` lays it the other way round
    /// from how its points run. As in the reference, the arcs are fitted
    /// to the whole loop from its own first point (PrintObject.cpp
    /// `simplify_extrusion_path`) before it is opened at the seam
    /// (GCode.cpp `extrude_loop`), which cuts the arc the seam is on in
    /// two. None when arcs are not asked for or the gap leaves nothing.
    fn fit_loop(&self, wall: &ExtrusionLoop, turned: bool, seam: Point2) -> Option<arc::Fitted> {
        if self.contouring() {
            return None;
        }
        let mut fitted = match self.fits.get(&(address(wall), turned)) {
            Some(fitted) => fitted.clone(),
            None => fit_closed(self.settings, &wall.polygon.points, turned, wall.role)?,
        };
        fitted.start_at(seam);
        fitted.clip_end(arc::tolerance(self.settings.seam_gap));
        (fitted.points.len() >= 2).then_some(fitted)
    }

    /// [`Self::path`], with whatever arcs fit the points when arcs are
    /// asked for.
    fn curved(
        &mut self,
        points: &[Point2],
        role: Role,
        flow: &Flow,
        sheltered: bool,
        closed: bool,
    ) -> io::Result<()> {
        // A piece of a contoured wall is laid as it is, neither fitted
        // nor thinned.
        if self.contouring() {
            return self.path(points, role, flow, sheltered, closed);
        }
        match fit(self.settings, points, role) {
            Some(fitted) => self.path_parts(
                &fitted.points,
                Some(&fitted.parts),
                role,
                flow,
                sheltered,
                closed,
            ),
            // With no arcs a piece is thinned by itself, as the
            // reference thins each path of a loop (LayerRegion.cpp
            // `simplify_loop`).
            None => {
                let points = simplify::piece(self.settings, points);
                self.path(&points, role, flow, sheltered, closed)
            }
        }
    }

    /// [`Self::path`] for a path that may have arcs in it: `parts` says
    /// how each stretch of `points` is laid, or is none for a path that
    /// is straight from point to point and was never asked about.
    fn path_parts(
        &mut self,
        points: &[Point2],
        parts: Option<&[arc::Part]>,
        role: Role,
        flow: &Flow,
        sheltered: bool,
        closed: bool,
    ) -> io::Result<()> {
        self.travel(points[0], sheltered, Some(role))?;
        self.lay_path(points, parts, role, flow, closed)
    }

    /// Extrudes along a path the nozzle is at the start of; the rest is
    /// as [`Self::path_parts`].
    fn lay_path(
        &mut self,
        points: &[Point2],
        parts: Option<&[arc::Part]>,
        role: Role,
        flow: &Flow,
        closed: bool,
    ) -> io::Result<()> {
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
            .map(|(_, runs)| runs.clone());
        let measure = is_wall
            && self.below.is_some()
            && (s.enable_overhang_speed || s.detect_overhang_wall || fan.is_some());
        if known.is_none() && !measure {
            self.fan_over_air(cooled)?;
            match parts {
                Some(parts) => self.lay_parts(points, parts, role, flow, motion)?,
                None => {
                    for &point in &points[1..] {
                        self.extrude(point, role, flow, motion)?;
                    }
                }
            }
        } else {
            // A wall over air is a bridge of the inner walls' width,
            // whichever wall it is (`PerimeterGenerator` `overhang_flow`).
            // At `bridge_line_width` where that is set.
            let bridge = self
                .inner_wall
                .unwrap_or(*flow)
                .bridging_sized(s.bridge_flow, s.thick_bridges, s.bridge_line_width)
                .map_err(|e| io::Error::other(e.to_string()))?;
            let laid = Laid {
                role,
                flow,
                motion,
                bridge: &bridge,
                over_air: self.motion(Role::OverhangPerimeter, &bridge, false),
                cooled,
                fan: fan.is_some(),
                fan_at: fan.and_then(|fan| fan.at).map(|at| at as f32),
            };
            let runs = match known {
                Some(runs) => runs,
                None => {
                    let runs = self.measure(points, &laid);
                    // Anything kept from a pass that went another way is dropped.
                    self.measured.truncate(self.measured_used);
                    self.measured.push((key, runs.clone()));
                    runs
                }
            };
            self.measured_used += 1;
            self.lay_runs(points, parts, &runs, &laid)?;
        }
        self.state.wipe_path = points.to_vec();
        if !closed {
            self.state.wipe_path.reverse();
        }
        self.state.wipe_heights = self.contour_profile();
        Ok(())
    }

    /// The layer beneath and how far beyond it a line's middle has to be
    /// for the line to hang clear, half the nozzle's width, where such
    /// walls are told apart (`detect_overhang_wall`) and there is a layer
    /// beneath: what [`overhang::stretches`] and [`overhang::Below::clear`]
    /// go by.
    pub(crate) fn clear_of(&self) -> Option<(&overhang::Below, f64)> {
        let s = self.settings;
        let below = self.below.as_ref()?;
        (s.detect_overhang_wall && !self.state.first_layer)
            .then_some((below, 0.5 * s.nozzle_diameter))
    }

    /// A wall measured against the layer beneath: cut into the stretches
    /// that rest on it and those that hang clear, and each measured
    /// along its length as a path of its own, as the reference has them
    /// (GCode.cpp `_extrude`, from `need_overhang_detection` to
    /// `variable_speed`).
    fn measure(&self, points: &[Point2], laid: &Laid<'_>) -> Vec<Run> {
        let s = self.settings;
        // Each stretch with how far its points are from the outline
        // beneath, where cutting it has found that out already.
        let stretches: Vec<(overhang::Stretch, Option<Vec<f64>>)> = match self.clear_of() {
            Some((below, by)) => (overhang::stretches_measured(below, by, points).into_iter())
                .map(|(stretch, known)| (stretch, Some(known)))
                .collect(),
            None => vec![(
                overhang::Stretch {
                    points: points.to_vec(),
                    clear: false,
                },
                None,
            )],
        };
        let Some(below) = self
            .below
            .as_ref()
            .filter(|_| s.enable_overhang_speed || laid.fan)
        else {
            return (stretches.into_iter())
                .map(|(stretch, _)| Run {
                    points: stretch.points,
                    clear: stretch.clear,
                    readings: Vec::new(),
                    cooled: false,
                })
                .collect();
        };
        (stretches.into_iter())
            .map(|(stretch, known)| {
                let (role, flow, motion) = laid.of(stretch.clear);
                // The speed the overhang speeds are shares of: the
                // wall's own setting, the inner walls' for a stretch
                // over air, held to what the hot end can melt and on a
                // scarf to the scarf's (`ref_speed`). Not the speed of
                // a small loop or of anything else the wall is slowed
                // by: that is `motion.speed`, which no step exceeds.
                let own = if role == Role::ExternalPerimeter {
                    s.outer_wall_speed
                } else {
                    s.inner_wall_speed
                };
                let melts = s.filament_max_volumetric_speed / self.melted(role, flow);
                let reference = if s.filament_max_volumetric_speed <= 0.0 {
                    own
                } else if own == 0.0 {
                    melts
                } else {
                    own.min(melts)
                };
                let reference = self.scarf_speed(reference, reference);
                // A step set to nothing leaves the wall at its own speed.
                let step = |speed: FloatOrPercent| {
                    let speed = speed.resolve(reference);
                    (speed >= 0.5).then_some(speed)
                };
                let speeds = overhang::Speeds::new(
                    flow.width as f32,
                    reference,
                    [
                        None,
                        step(s.overhang_speeds[0]),
                        step(s.overhang_speeds[1]),
                        step(s.overhang_speeds[2]),
                        step(s.overhang_speeds[3]),
                        // With nothing under it at all a wall goes at
                        // the bridge's speed, or with
                        // `slowdown_for_curled_perimeters` at the last
                        // of the overhang speeds.
                        if s.slowdown_for_curled_perimeters {
                            step(s.overhang_speeds[3])
                        } else {
                            Some(s.bridge_speed)
                        },
                    ],
                    motion.speed,
                );
                // An overhang wall is cooled whatever is under it, so
                // nothing along it turns on how much is.
                let fan_at = laid.fan_at.filter(|_| !stretch.clear);
                let mut readings = overhang::Gauge {
                    below,
                    width: flow.width as f32,
                    speeds: &speeds,
                    fan_at,
                }
                .readings(&stretch.points, known.as_deref());
                // "with the slowdown off the points only carry the
                // overhang fan transitions"
                if !s.enable_overhang_speed {
                    for reading in &mut readings {
                        reading.speed = motion.speed as f32;
                    }
                }
                // "Ignore small speed variations (under 1mm/sec)"
                let mut varies = readings
                    .iter()
                    .any(|reading| (f64::from(reading.speed) - motion.speed).abs() > 1.0);
                let mut cooled = false;
                if let (Some(at), Some(first)) = (fan_at, readings.first()) {
                    cooled = first.cover <= at;
                    varies |= readings
                        .windows(2)
                        .any(|pair| (pair[0].cover <= at) != (pair[1].cover <= at));
                }
                if !varies {
                    readings.clear();
                }
                Run {
                    points: stretch.points,
                    clear: stretch.clear,
                    readings,
                    cooled,
                }
            })
            .collect()
    }

    /// Lays a measured wall stretch by stretch. As in the reference
    /// (GCode.cpp `_extrude`), a stretch laid one way throughout, at one
    /// speed to within 1 mm/s of its own and with the overhang fan
    /// either on all along or off, is laid along its own points and
    /// keeps its arcs; one whose speed or fan changes on the way is laid
    /// as straight moves between the points it was measured at.
    fn lay_runs(
        &mut self,
        points: &[Point2],
        parts: Option<&[arc::Part]>,
        runs: &[Run],
        laid: &Laid<'_>,
    ) -> io::Result<()> {
        for run in runs {
            let (role, flow, motion) = laid.of(run.clear);
            if !run.readings.is_empty() {
                self.lay_readings(run, laid)?;
                continue;
            }
            self.fan_over_air(if run.clear {
                laid.fan
            } else {
                laid.cooled || run.cooled
            })?;
            // Part of a wall keeps the arcs of that stretch by itself:
            // the reference makes separate paths of the stretches
            // before it fits arcs to any.
            let fitted = match parts {
                Some(_) if runs.len() > 1 => fit(self.settings, &run.points, laid.role),
                _ => None,
            };
            match (parts, fitted) {
                (Some(parts), _) if runs.len() == 1 => {
                    self.lay_parts(points, parts, role, flow, motion)?;
                }
                (_, Some(fitted)) => {
                    self.lay_parts(&fitted.points, &fitted.parts, role, flow, motion)?;
                }
                _ => {
                    for &point in &run.points[1..] {
                        self.extrude(point, role, flow, motion)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// A stretch of wall whose speed or fan changes on the way, one
    /// straight move to each point it was measured at (GCode.cpp
    /// `_extrude`, the branch taken with `variable_speed`).
    fn lay_readings(&mut self, run: &Run, laid: &Laid<'_>) -> io::Result<()> {
        let (role, flow, motion) = laid.of(run.clear);
        let readings = &run.readings;
        // In mm/min, as the reference compares them.
        let own = motion.speed * 60.0;
        let mut set = f64::from(readings[0].speed) * 60.0;
        let mut fan_before = laid.fan_on(run.clear, readings[0].cover);
        for index in 1..readings.len() {
            // A point's cover is that of the line leaving it, so the fan
            // is on over a line that is itself short of support and
            // followed by another that is, or that ends the path so.
            let fan_now = laid.fan_on(run.clear, readings[index].cover);
            let fan = fan_before && fan_now;
            fan_before = fan_now;
            let to = readings[index].at;
            // "if (line_length < EPSILON) continue": a move that comes
            // to nothing as it is written goes with the next.
            let written = |point: Point2| {
                (
                    (unscaled(point.x) * 1000.0).round(),
                    (unscaled(point.y) * 1000.0).round(),
                )
            };
            if self.state.position.map(written) == Some(written(to)) {
                continue;
            }
            // "Ignore small speed variations - emit speed change if the
            // delta between current and new is greater than 60mm/min /
            // 1mm/sec. Reset speed to F if delta to F is less than
            // 1mm/sec". The line goes at the speed of the point it
            // leaves.
            //
            // The reference asks the two the other way round, and so
            // comes to the second only when the speed has changed by
            // 1 mm/s or less: the first move back at full speed after
            // an overhang is written at the whole number of mm/s the
            // measuring rounds to (F9720 for a wall at 162.36 mm/s) and
            // only the one after it at the wall's own (F9741.893), or
            // none if the path ends there. Here a move within 1 mm/s
            // of the wall's own speed goes at that speed at once.
            let wanted = f64::from(readings[index - 1].speed) * 60.0;
            if (own - wanted).abs() <= 60.0 {
                set = own;
            } else if (set - wanted).abs() > 60.0 {
                set = wanted;
            }
            // A wall's own speed as it is (GCode.cpp `_extrude`,
            // `F = speed * 60`).
            let speed = if set == own { motion.speed } else { set / 60.0 };
            self.fan_over_air(fan)?;
            self.extrude(to, role, flow, Motion { speed, ..motion })?;
        }
        Ok(())
    }

    /// Extrudes along a path stretch by stretch: straight from point to
    /// point, or round an arc (GCode.cpp `_extrude`, "generate gcode from
    /// arc fitting data").
    fn lay_parts(
        &mut self,
        points: &[Point2],
        parts: &[arc::Part],
        role: Role,
        flow: &Flow,
        motion: Motion,
    ) -> io::Result<()> {
        for part in parts {
            match &part.kind {
                arc::Kind::Line => {
                    for pair in points[part.start..=part.end].windows(2) {
                        // A move of next to no length is left out, as
                        // the reference leaves it out; cutting a loop at
                        // its seam can leave one.
                        if distance(pair[0], pair[1]) >= 1e-4 {
                            self.extrude(pair[1], role, flow, motion)?;
                        }
                    }
                }
                arc::Kind::Arc(arc) => self.lay(arc.end, Some(arc), role, flow, motion)?,
            }
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
            wanted.min(s.filament_max_volumetric_speed / self.melted(role, flow))
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

    /// What a line lays per mm where the hot end's limit is worked out,
    /// in mm³: its own cross-section with the flow ratios it is laid
    /// with, the filament's, the print's and its kind's (GCode.cpp
    /// `_extrude`, `_mm3_per_mm`, which the speed is capped by), or on a
    /// scarf the scarf's. The prime tower's lines are not laid by
    /// `_extrude` in the reference, and keep their cross-section alone.
    fn melted(&self, role: Role, flow: &Flow) -> f64 {
        let s = self.settings;
        let share = match role {
            Role::PrimeTower | Role::PrimeLine => return flow.mm3_per_mm() * self.scarf_flow(),
            Role::TopSolidInfill => s.top_solid_infill_flow_ratio,
            Role::BottomSurface => s.bottom_solid_infill_flow_ratio,
            Role::InternalBridge => s.internal_bridge_flow,
            Role::Brim => s.brim_flow_ratio,
            _ => self.scarf_flow(),
        };
        flow.mm3_per_mm() * s.filament_flow_ratio * s.print_flow_ratio * share
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
    /// unless it leaves an outer wall, which always retracts. `role` is
    /// what the move is to, where it goes to a line.
    fn travel(&mut self, to: Point2, sheltered: bool, role: Option<Role>) -> io::Result<()> {
        let s = self.settings;
        let from = self.state.position;
        // Round the inside of the walls where the move would cross
        // them, and it is as long as the way round is.
        let mut way = self.way(to);
        let along = |from: Point2, way: &[Point2]| {
            let mut at = from;
            way.iter()
                .map(|&point| distance(std::mem::replace(&mut at, point), point))
                .sum::<f64>()
        };
        let length = from.map_or(f64::INFINITY, |from| along(from, &way));
        // The height the line it goes to is laid from: the layer's, or a
        // contoured line's own where it starts (`contour.rs`).
        let arrival = self.arrival_height(to);
        if length == 0.0 && self.state.height == Some(arrival) {
            self.state.routing.passed();
            return Ok(());
        }
        // Every travel goes at the travel acceleration, set before the
        // retraction and so the wipe too, but for a short one into an
        // outer wall or an overhang wall, which keeps that wall's, so
        // the nozzle does not slow down just before the wall it is about
        // to lay (GCode.cpp `travel_to`). On the first layer, the first
        // layer's. Short is shorter than `retraction_minimum_travel` in a
        // straight line.
        let straight = from.map_or(f64::INFINITY, |from| distance(from, to));
        let short = straight < s.retraction_minimum_travel;
        if self.state.first_layer && s.initial_layer_acceleration > 0.0 {
            self.accelerate(s.initial_layer_acceleration)?;
        } else if s.default_acceleration > 0.0 {
            let bridge = s.bridge_acceleration.resolve(s.outer_wall_acceleration);
            let wanted = match role {
                Some(Role::OverhangPerimeter) if short => bridge,
                Some(Role::ExternalPerimeter) if short => s.outer_wall_acceleration,
                _ => s.travel_acceleration,
            };
            self.accelerate(wanted)?;
        }
        let exposed = !sheltered
            || matches!(
                self.state.role,
                Some(Role::ExternalPerimeter | Role::OverhangPerimeter)
            );
        let long = length > s.retraction_minimum_travel && exposed;
        if long {
            self.retract(false)?;
            // A wipe moves the nozzle, and the way is found again from
            // where it ends (GCode.cpp `travel_to`). Not as the
            // reference, on purpose: by then it has forgotten that the
            // move was to go straight, and routes it after all.
            if self.state.position != from {
                way = self.way(to);
            }
        } else {
            // Nothing to wipe once the nozzle has moved off the line.
            self.state.wipe_path.clear();
        }
        self.state.routing.passed();
        // The lift is made over the first stretch of the way and the
        // rest is gone along at that height (GCode.cpp `travel_to`,
        // "lift the z to normal height at the beginning").
        let rest = way.split_off(1);
        let to = way[0];
        // A wipe moves the nozzle, so the travel is measured from where it ends.
        let from = self.state.position;
        let length = from.map_or(f64::INFINITY, |from| distance(from, to));
        let lift = self.state.pending_lift.take();
        let z = self.state.z;
        let top = match (lift, self.state.height) {
            (Some(_), _) => z + s.z_hop,
            // Already up from an earlier lift: stay there. Never down
            // on the way: a contoured line that ends lower than the next
            // starts goes up to it as it travels, and one that ends
            // higher comes down once there. Not as the reference, which
            // travels straight from the one height to the other
            // (GCode.cpp `travel_to` to `zaa_first_z`), and from a
            // contoured line to a flat one at the height it ended at,
            // down in the layer, before it puts it back ("reset Z after
            // contouring").
            (None, Some(height)) => height.max(arrival),
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
        let mut at = to;
        for point in rest {
            let length = distance(std::mem::replace(&mut at, point), point);
            self.go(point, None, s.travel_speed, length)?;
        }
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
        // With arc fitting the turn is one counter-clockwise arc
        // (GCodeWriter.cpp `_spiral_travel_to_z`): it ends where it
        // starts, so it has no X or Y, and `P1` says it is a whole turn
        // to firmware that would otherwise take it for none. Its speed
        // is held to what the Z axis can climb at over the turn, the
        // radius being the one the three places of I and J give.
        if s.arc_fitting {
            let (i, j) = (thousandths(-dy * radius), thousandths(dx * radius));
            let around = TAU * i.hypot(j);
            let rise = top - height;
            let speed = s.travel_speed.min(s.z_speed * around.hypot(rise) / rise);
            self.feed(speed);
            let feed_word = self.feed_word();
            writeln!(self.out, "G17\nG3 Z{top:.3} I{i:.3} J{j:.3} P1{feed_word}")?;
            self.state.summary.travel_mm += around;
            self.spend(around / speed);
            return Ok(());
        }
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
        self.lay(to, None, role, flow, motion)
    }

    /// One extruding move to `to`: straight, or round `turn`, an arc
    /// that starts where the nozzle is and ends there.
    fn lay(
        &mut self,
        to: Point2,
        turn: Option<&arc::Arc>,
        role: Role,
        flow: &Flow,
        motion: Motion,
    ) -> io::Result<()> {
        let s = self.settings;
        let from = self
            .state
            .position
            .expect("a travel always comes before extruding");
        let length = match turn {
            Some(arc) => {
                // An arc of next to no length is left out, as in the
                // reference (GCode.cpp `_extrude`, `arc_length < EPSILON`).
                if arc.length_mm() < 1e-4 {
                    return Ok(());
                }
                // Its bulge has to be over the bed as well as its ends.
                if let Some([min_x, min_y, max_x, max_y]) = s.bed {
                    let [a, b, c, d] = arc.bounds().map(|units| units / edge_geom::UNITS_PER_MM);
                    if a < min_x || b < min_y || c > max_x || d > max_y {
                        self.off_bed.get_or_insert([a, b, c, d]);
                    }
                }
                arc.length_mm()
            }
            None => distance(from, to),
        };
        if length == 0.0 {
            return Ok(());
        }
        self.contour_role(role);
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
        // On a scarf seam the move climbs as it goes, and lays a share
        // of the line (`scarf.rs`).
        let sloping = self.slope_on(length);
        let share = match role {
            Role::TopSolidInfill => s.top_solid_infill_flow_ratio,
            Role::BottomSurface => s.bottom_solid_infill_flow_ratio,
            Role::InternalBridge => s.internal_bridge_flow,
            Role::Brim => s.brim_flow_ratio,
            _ => sloping.as_ref().map_or(1.0, |sloping| sloping.flow),
        };
        // On a line contoured in height the move climbs or falls as it
        // goes, lays as much as the line is tall, and may be slowed for
        // it (`contour.rs`).
        let laid = flow.mm3_per_mm() * s.filament_flow_ratio * s.print_flow_ratio * share;
        let contoured = self.contour_on((from, to), role, flow, motion.speed, length, laid);
        let moved = match (&sloping, &contoured) {
            (Some(sloping), _) => sloping.length,
            (None, Some(contoured)) => contoured.length,
            (None, None) => length,
        };
        let own = contoured
            .as_ref()
            .and_then(|contoured| contoured.speed)
            .unwrap_or(motion.speed);
        // What a layer slowed to cool leaves at its speed, its time
        // counted: the outer wall where asked, a compensated circle's
        // wall, and the prime tower, whose lines the reference writes
        // without the marks that make a line adjustable (WipeTower2.cpp
        // writes them itself, not GCode.cpp `_extrude`, and
        // CoolingBuffer.cpp slows only what is marked
        // `;_EXTRUDE_SET_SPEED`).
        let held = role == Role::ExternalPerimeter && s.dont_slow_down_outer_wall
            || matches!(role, Role::PrimeTower | Role::PrimeLine)
            || self.circle_wall_held(role);
        let speed = if held {
            self.timing.fixed += moved / own;
            own
        } else {
            match self.timing.lines.last_mut() {
                Some(line) if line.1 == own => line.0 += moved,
                _ => self.timing.lines.push((moved, own)),
            }
            self.slowdown.apply(own)
        };
        let short = s.small_area.as_ref().filter(|_| self.state.short_lines);
        let share = share
            * short.map_or(1.0, |model| model.share(length))
            * contoured.as_ref().map_or(1.0, |contoured| contoured.flow);
        let volume =
            length * flow.mm3_per_mm() * s.filament_flow_ratio * s.print_flow_ratio * share;
        let feed = volume / self.filament_area;
        self.feed(speed);
        let feed_word = self.feed_word();
        match (turn, &sloping) {
            // GCodeWriter.cpp `extrude_to_xyz`: every move of a scarf
            // says its height, the way down too, which stays level.
            // So does every move of a contoured line.
            (None, None) if let Some(contoured) = &contoured => {
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3} Z{:.3} E{feed:.5}{feed_word}",
                    unscaled(to.x),
                    unscaled(to.y),
                    contoured.z
                )?;
                self.state.height = Some(contoured.z);
            }
            (None, Some(sloping)) => {
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3} Z{:.3} E{feed:.5}{feed_word}",
                    unscaled(to.x),
                    unscaled(to.y),
                    sloping.z
                )?;
                self.state.height = Some(sloping.z);
            }
            (None, None) => writeln!(
                self.out,
                "G1 X{:.3} Y{:.3} E{feed:.5}{feed_word}",
                unscaled(to.x),
                unscaled(to.y)
            )?,
            // GCodeWriter.cpp `extrude_arc_to_xy`: G3 goes round
            // counter-clockwise and G2 clockwise, and I and J say where
            // the centre is from where the move starts, to the same
            // three places as X and Y.
            (Some(arc), _) => writeln!(
                self.out,
                "G{} X{:.3} Y{:.3} I{:.3} J{:.3} E{feed:.5}{feed_word}",
                if arc.ccw { 3 } else { 2 },
                unscaled(to.x),
                unscaled(to.y),
                thousandths(unscaled(arc.center.x) - unscaled(arc.start.x)),
                thousandths(unscaled(arc.center.y) - unscaled(arc.start.y)),
            )?,
        }
        self.state.primed = true;
        self.state.line_speed = motion.speed;
        self.state.summary.filament_mm += feed;
        self.state.summary.filament_mm3 += volume;
        if let Some(used) = self.state.used.get_mut(self.state.tool) {
            used.0 += feed;
            used.1 += volume;
        }
        self.state.summary.seconds += moved / speed;
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
        // Over a line contoured in height the wipe keeps to the line's
        // own height (`contour.rs`).
        let heights = self.state.wipe_heights.take();
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
                let height = self.wipe_height(heights.as_deref(), pair[1]);
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3}{height}{pulled}",
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
        // The layer's height, or the foot of a scarf seam.
        let z = self.laying_height();
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

/// The arcs in a path, when arcs are asked for and it has the three
/// points one takes: the path within the tolerance of its kind, and how
/// each stretch of it is laid. Sparse infill and supports are held less
/// closely than the rest (LayerRegion.cpp `simplify_path`, Layer.cpp
/// `simplify_support_path`). Skirt, brim and prime tower are never asked
/// about: the reference fits nothing to the first two, and the tower's
/// rounded ribs, which it does fit, are not done here.
fn fit(s: &PrintSettings, points: &[Point2], role: Role) -> Option<arc::Fitted> {
    if !s.arc_fitting || points.len() < 3 {
        return None;
    }
    let within = match role {
        Role::SparseInfill => arc::SPARSE_INFILL_RESOLUTION,
        Role::Support | Role::SupportInterface => arc::SUPPORT_RESOLUTION,
        _ => s.resolution,
    };
    Some(arc::Fitted::new(points, arc::tolerance(within)))
}

/// The arcs in a loop taken once round from its first point, the other
/// way round from how `points` run if `turned`.
fn fit_closed(
    s: &PrintSettings,
    points: &[Point2],
    turned: bool,
    role: Role,
) -> Option<arc::Fitted> {
    if !s.arc_fitting {
        return None;
    }
    let mut closed = Vec::with_capacity(points.len() + 1);
    closed.extend_from_slice(points);
    closed.push(points[0]);
    if turned {
        closed.reverse();
    }
    fit(s, &closed, role)
}

/// Whether a wall is laid the other way round from how its points run,
/// as [`Writer::walls`] has it. Only what is fitted ahead of a layer
/// goes by this: a wall laid another way after all is fitted when it is
/// laid.
fn wall_turned(s: &PrintSettings, wall: &ExtrusionLoop) -> bool {
    let clockwise = match s.wall_direction {
        WallDirection::Auto => wall.clockwise,
        WallDirection::CounterClockwise => false,
        WallDirection::Clockwise => true,
    };
    wall.is_hole != clockwise
}

/// Where something is kept, to know it by while its layer is written.
fn address<T>(kept: &T) -> usize {
    std::ptr::from_ref(kept) as usize
}

/// A length in mm to the three places a coordinate is written with,
/// rounded as the reference rounds what it writes (GCodeWriter.cpp
/// `emit_axis`), and never a zero with a minus sign.
fn thousandths(mm: f64) -> f64 {
    let rounded = (mm * 1000.0).round() / 1000.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

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
mod arc_tests;
#[cfg(test)]
mod avoid_tests;
#[cfg(test)]
mod contour_tests;
#[cfg(test)]
mod ironing_tests;
#[cfg(test)]
mod scarf_tests;
#[cfg(test)]
mod tests;
