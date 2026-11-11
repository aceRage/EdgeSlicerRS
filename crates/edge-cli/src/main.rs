use std::{fmt::Write as _, fs, path::PathBuf, process::ExitCode, time::Instant};

use clap::{Args, Parser, Subcommand};
use edge_3mf::{PartKind, Project};
use edge_config::{Kind, Vendor};
use edge_engine::{BedType, Chosen, Overrides, Unwatched};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Role};
use edge_geom::{ExPolygon, unscaled};
use edge_infill::SurfaceKind;
use edge_perimeter::Perimeters;
use edge_slice::LayerRegions;

type Failure = edge_engine::Failure;

#[derive(Parser)]
#[command(name = "edge", version, about = "EdgeSlicer engine command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cut a model into layer contours and report what was found and how long it took.
    Slice(Box<SliceArgs>),
    /// Show what a 3MF project contains.
    Inspect {
        /// 3MF file.
        project: PathBuf,
    },
    /// Print a preset with everything it inherits, as JSON.
    Preset(PresetArgs),
}

#[derive(Args)]
struct ProfilesArgs {
    /// Directory holding the vendor profiles, e.g. EdgeSlicer's resources/profiles.
    #[arg(long, env = "EDGE_PROFILES")]
    profiles: Option<PathBuf>,
}

#[derive(Args)]
struct SliceArgs {
    /// STL (binary or ASCII) or 3MF file.
    model: PathBuf,
    /// Plate number to slice from a 3MF; defaults to plate 1 when the file has plates.
    #[arg(long)]
    plate: Option<u32>,
    /// Layer height in mm. Overrides the process preset and the 3MF's own settings.
    #[arg(long)]
    layer_height: Option<f64>,
    /// First layer height in mm.
    #[arg(long)]
    first_layer_height: Option<f64>,
    /// Fill cracks narrower than twice this many mm when resolving a layer's
    /// regions (the slice_closing_radius setting); 0 turns it off.
    #[arg(long)]
    closing_radius: Option<f64>,
    /// Vendor whose process preset supplies the slicing settings; needs --process.
    #[arg(long)]
    vendor: Option<String>,
    /// Process preset name within --vendor.
    #[arg(long, requires = "vendor")]
    process: Option<String>,
    /// Machine preset name within --vendor; supplies the nozzle, the bed and
    /// the printer's own start and end G-code.
    #[arg(long, requires = "vendor")]
    machine: Option<String>,
    /// Filament preset name within --vendor; supplies temperatures, cooling
    /// and retraction. Given once it serves every filament the model
    /// prints in; given again, the second is for filament 2, and so on.
    #[arg(long, requires = "vendor")]
    filament: Vec<String>,
    /// Which build plate is on the printer, e.g. "Textured PEI Plate"; it
    /// picks the bed temperature. Defaults to the 3MF's, or else to the
    /// reference's default, "Cool Plate".
    #[arg(long)]
    bed_type: Option<String>,
    #[command(flatten)]
    profiles: ProfilesArgs,
    /// Number of walls; overrides the presets and the 3MF's own settings.
    #[arg(long)]
    wall_loops: Option<usize>,
    /// Worker threads; defaults to all cores.
    #[arg(long)]
    threads: Option<usize>,
    /// Write one layer's contours to this SVG file.
    #[arg(long)]
    svg: Option<PathBuf>,
    /// Layer index for --svg; defaults to the middle layer.
    #[arg(long, requires = "svg")]
    svg_layer: Option<usize>,
    /// Write the print as G-code to this file. With a printer profile (from
    /// the 3MF or --machine) its start and end G-code is used; without one a
    /// generic preamble is written and the file is only good for viewing.
    #[arg(long)]
    gcode: Option<PathBuf>,
}

#[derive(Args)]
struct PresetArgs {
    /// Vendor name, e.g. Snapmaker.
    vendor: String,
    /// machine-model, machine, process or filament.
    kind: Kind,
    /// Preset name; a name the preset used to have also works.
    name: String,
    /// Print only this setting instead of the whole preset.
    #[arg(long)]
    key: Option<String>,
    #[command(flatten)]
    profiles: ProfilesArgs,
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Slice(args) => slice(*args),
        Command::Inspect { project } => inspect(project),
        Command::Preset(args) => preset(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

impl ProfilesArgs {
    fn vendor(&self, name: &str) -> Result<Vendor, Failure> {
        let dir = self
            .profiles
            .as_ref()
            .ok_or("no profiles directory; pass --profiles or set EDGE_PROFILES")?;
        Ok(Vendor::open(dir, name)?)
    }
}

fn preset(args: PresetArgs) -> Result<(), Failure> {
    let settings = args
        .profiles
        .vendor(&args.vendor)?
        .resolve(args.kind, &args.name)?;
    match args.key {
        None => println!("{}", settings.to_json()),
        Some(key) => match settings.texts(&key) {
            Some(values) => println!("{}", values.join("\n")),
            None => return Err(format!("{} has no setting {key}", args.name).into()),
        },
    }
    Ok(())
}

fn inspect(path: PathBuf) -> Result<(), Failure> {
    let project = Project::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    println!(
        "{}: {} objects, {} plates",
        path.display(),
        project.objects.len(),
        project.plates.len()
    );
    for object in &project.objects {
        let name = object.name.as_deref().unwrap_or("(unnamed)");
        let triangles: usize = object
            .parts
            .iter()
            .map(|part| part.mesh.triangles.len())
            .sum();
        println!(
            "  object {} {name}: {} instances, {} parts, {triangles} triangles",
            object.id,
            object.instances.len(),
            object.parts.len(),
        );
        for part in object
            .parts
            .iter()
            .filter(|part| part.kind != PartKind::Normal || object.parts.len() > 1)
        {
            println!(
                "    {:?} {}",
                part.kind,
                part.name.as_deref().unwrap_or("(unnamed)")
            );
        }
    }
    for plate in &project.plates {
        let name = if plate.name.is_empty() {
            String::new()
        } else {
            format!(" \"{}\"", plate.name)
        };
        println!(
            "  plate {}{name}: {} instances",
            plate.id,
            plate.instances.len()
        );
    }
    match &project.settings {
        None => println!("  no project settings"),
        Some(settings) => {
            println!("  {} project settings", settings.len());
            for key in [
                "printer_settings_id",
                "print_settings_id",
                "filament_settings_id",
                "layer_height",
            ] {
                if let Some(values) = settings.texts(key) {
                    println!("    {key}: {}", values.join(", "));
                }
            }
        }
    }
    Ok(())
}

/// The presets named on the command line, resolved.
fn presets(args: &SliceArgs) -> Result<Chosen, Failure> {
    let Some(vendor) = &args.vendor else {
        return Ok(Chosen::default());
    };
    let vendor = args.profiles.vendor(vendor)?;
    let resolve = |kind, name: &Option<String>| {
        name.as_ref()
            .map(|name| {
                vendor
                    .resolve(kind, name)
                    .map(|settings| (name.clone(), settings))
            })
            .transpose()
    };
    Ok(Chosen {
        machine: resolve(Kind::Machine, &args.machine)?,
        process: resolve(Kind::Process, &args.process)?,
        filaments: args
            .filament
            .iter()
            .map(|name| Ok((name.clone(), vendor.resolve(Kind::Filament, name)?)))
            .collect::<Result<_, Failure>>()?,
    })
}

fn slice(args: SliceArgs) -> Result<(), Failure> {
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let started = Instant::now();
    let opened = edge_engine::open(&args.model, args.plate)?;
    if let Some((id, count)) = opened.plate() {
        println!("plate {id} of {count}");
    }
    let loaded = opened.place()?;
    let load_time = started.elapsed();

    let chosen = presets(&args)?;
    let overrides = Overrides {
        layer_height: args.layer_height,
        first_layer_height: args.first_layer_height,
        closing_radius: args.closing_radius,
        wall_loops: args.wall_loops,
        bed_type: args.bed_type.clone(),
    };
    let job = edge_engine::prepare(loaded, &chosen, &overrides)?;
    match &job.bed_type {
        BedType::Settled => {}
        BedType::PrinterDefault(plate) => println!(
            "bed type not given; the printer's default, {plate}, is used (--bed-type changes it)"
        ),
        BedType::Assumed => println!(
            "bed type not given; the default, Cool Plate, is assumed (--bed-type changes it)"
        ),
    }
    let (
        (layer_height, layer_source),
        (first_layer_height, first_source),
        (closing_radius, closing_source),
    ) = (job.layer_height, job.first_layer_height, job.closing_radius);
    let sliced = edge_engine::slice(job, &Unwatched)?;
    let (min, max) = sliced.bounds;
    let (bodies, layers) = (&sliced.bodies, &sliced.layers);
    let (walls, infill, support, adhesion) = (
        &sliced.walls,
        &sliced.infill,
        &sliced.support,
        &sliced.adhesion,
    );
    let (perimeters, fills) = sliced.combined();
    let surfaces: Vec<&edge_infill::Surface> =
        sliced.surfaces_by.iter().flatten().flatten().collect();

    let carved = bodies.iter().filter(|body| body.negative.is_some()).count();
    println!(
        "{}: {} triangles, {:.3} x {:.3} x {:.3} mm{}",
        args.model.display(),
        bodies
            .iter()
            .map(|body| body.solid.triangles.len())
            .sum::<usize>(),
        max[0] - min[0],
        max[1] - min[1],
        max[2] - min[2],
        if carved > 0 {
            format!(", {carved} bodies with negative parts")
        } else {
            String::new()
        },
    );
    println!(
        "layer height {layer_height} mm ({layer_source}), first layer {first_layer_height} mm ({first_source}), \
         closing radius {closing_radius} mm ({closing_source})"
    );
    if sliced.elephant_layers > 0 {
        println!(
            "elephant foot compensation {} mm over the first {} layer(s)",
            sliced.elephant_foot, sliced.elephant_layers
        );
    }

    let regions = || layers.iter().flat_map(|layer| &layer.regions);
    println!(
        "{} layers, {} regions, {} holes, {} points, {:.1} mm² in all",
        layers.len(),
        regions().count(),
        regions().map(|region| region.holes.len()).sum::<usize>(),
        regions()
            .flat_map(ExPolygon::rings)
            .map(|ring| ring.points.len())
            .sum::<usize>(),
        regions().map(ExPolygon::area_mm2).sum::<f64>(),
    );
    let leaky = layers.iter().filter(|l| l.open_chains > 0).count();
    if leaky > 0 {
        println!("warning: {leaky} layers have open contours; the mesh is not watertight there");
    }

    let sample = walls.params(layer_height, false)?;
    let loops = || perimeters.iter().flat_map(|layer| &layer.loops);
    let external = loops().filter(|wall| wall.role == Role::ExternalPerimeter);
    println!(
        "{} walls of {:.3} mm outside and {:.3} mm inside on a {} mm nozzle: {} loops ({} external), \
         {:.2} m, {:.2} cm³; {} infill areas",
        walls.wall_loops,
        sample.outer.width,
        sample.inner.width,
        walls.nozzle_diameter,
        loops().count(),
        external.count(),
        loops().map(ExtrusionLoop::length_mm).sum::<f64>() / 1e3,
        loops().map(ExtrusionLoop::volume_mm3).sum::<f64>() / 1e3,
        perimeters
            .iter()
            .map(|layer| layer.infill.len())
            .sum::<usize>(),
    );
    let area_of = |kind: SurfaceKind| {
        surfaces
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.area.area_mm2())
            .sum::<f64>()
            / 100.0
    };
    // Ironing is counted by itself, further down.
    let paths = || {
        fills
            .iter()
            .flatten()
            .filter(|line| line.role != Role::Ironing)
    };
    println!(
        "infill at {}% {:?}: {:.1} cm² top, {:.1} bottom, {:.1} bridge, {:.1} solid, {:.1} bridged \
         over infill, {:.1} sparse; {} lines, {:.2} m, {:.2} cm³",
        infill.sparse_infill_density_percent,
        infill.pattern,
        area_of(SurfaceKind::Top),
        area_of(SurfaceKind::Bottom),
        area_of(SurfaceKind::Bridge),
        area_of(SurfaceKind::InternalSolid),
        area_of(SurfaceKind::InternalBridge),
        area_of(SurfaceKind::Sparse),
        paths().count(),
        paths().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
        paths().map(ExtrusionPath::volume_mm3).sum::<f64>() / 1e3,
    );
    if support.enabled {
        let lines = || sliced.supports.iter().flatten();
        let dense = lines().filter(|line| line.role == Role::SupportInterface);
        println!(
            "supports on {} layer(s): {} lines, {:.2} m, of which {:.2} m in the dense tops; {:.2} cm³",
            sliced
                .supports
                .iter()
                .filter(|layer| !layer.is_empty())
                .count(),
            lines().count(),
            lines().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
            dense.map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
            lines().map(ExtrusionPath::volume_mm3).sum::<f64>() / 1e3,
        );
    }
    let skirt_loops = sliced.skirt_loops;
    if let Some(first) = sliced.around.first().filter(|first| !first.is_empty()) {
        println!(
            "{skirt_loops} skirt loop(s) on {} layer(s), {} brim loop(s): {:.2} m on the first layer",
            adhesion.skirt_height.min(layers.len()),
            first.len() - skirt_loops,
            first.iter().map(ExtrusionLoop::length_mm).sum::<f64>() / 1e3,
        );
    }
    if let Some((x, y)) = sliced.centred {
        println!("placed in the middle of the bed, at {x} x {y} mm");
    }
    let gap_lines = || {
        fills
            .iter()
            .flatten()
            .filter(|line| line.role == Role::GapFill)
    };
    if gap_lines().next().is_some() {
        println!(
            "gaps between walls filled with {} line(s), {:.2} m",
            gap_lines().count(),
            gap_lines().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
        );
    }
    let ironed = |layer: &&Vec<ExtrusionPath>| layer.iter().any(|line| line.role == Role::Ironing);
    let ironing = || {
        fills
            .iter()
            .flatten()
            .filter(|line| line.role == Role::Ironing)
    };
    if ironing().next().is_some() {
        println!(
            "ironing on {} layer(s): {} line(s), {:.2} m, {:.3} cm³",
            fills.iter().filter(ironed).count(),
            ironing().count(),
            ironing().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
            ironing().map(ExtrusionPath::volume_mm3).sum::<f64>() / 1e3,
        );
    }
    let gaps_skipped: usize = sliced
        .walls_by
        .iter()
        .flatten()
        .map(|layer| layer.gaps_skipped)
        .sum();
    if gaps_skipped > 0 {
        println!(
            "note: {gaps_skipped} gap(s) between walls were left empty; their centre line could \
             not be worked out"
        );
    }
    if let Some(pattern) = &infill.unsupported_pattern {
        println!(
            "note: sparse pattern \"{pattern}\" is not implemented; grid was used in its place"
        );
    }
    let timings = sliced.timings;
    println!(
        "load {:.1} ms, slice {:.1} ms, perimeters {:.1} ms, supports {:.1} ms, surfaces {:.1} ms, \
         fill {:.1} ms on {} threads",
        load_time.as_secs_f64() * 1e3,
        timings.slice.as_secs_f64() * 1e3,
        timings.perimeters.as_secs_f64() * 1e3,
        timings.supports.as_secs_f64() * 1e3,
        timings.surfaces.as_secs_f64() * 1e3,
        timings.fill.as_secs_f64() * 1e3,
        rayon::current_num_threads(),
    );

    if let Some(path) = &args.gcode {
        let started = Instant::now();
        let print = &sliced.print;
        if let Some([a, b, c, d]) = sliced.tower.footprint {
            println!(
                "prime tower from {a:.1} x {b:.1} to {c:.1} x {d:.1} mm, on {} layer(s), for {} tool change(s)",
                sliced
                    .tower
                    .layers
                    .iter()
                    .filter(|layer| !layer.finish.is_empty())
                    .count(),
                sliced.changes.iter().sum::<usize>(),
            );
        }
        // Written whole or not at all: a file cut short by an error must
        // never be left where it could be sent to a printer.
        let gcode = sliced.gcode(&edge_engine::timestamp())?;
        let summary = gcode.summary;
        fs::write(path, &gcode.text)?;
        let weight = match print.filament_density {
            Some(density) => format!(", {:.1} g", summary.filament_mm3 / 1000.0 * density),
            None => String::new(),
        };
        let timed = match summary.timed {
            Some(edge_gcode::TimeModel::Klipper) => "as Klipper plans it (heating not counted)",
            Some(edge_gcode::TimeModel::Reference) => "with acceleration (heating not counted)",
            None => "at commanded speeds (no acceleration)",
        };
        println!(
            "G-code written to {} in {:.1} ms: {:.2} m of filament{weight}, {:.1} m of travel, {} retractions, \
             {:.0} min {timed}",
            path.display(),
            started.elapsed().as_secs_f64() * 1e3,
            summary.filament_mm / 1e3,
            summary.travel_mm / 1e3,
            summary.retractions,
            summary.seconds / 60.0,
        );
        if print.machine_start_gcode.is_some() {
            println!(
                "note: start, end and layer-change G-code come from the printer profile; not yet run on a printer"
            );
        } else {
            println!(
                "note: no printer profile, so this G-code has a generic preamble and is not printer-ready"
            );
        }
        for note in sliced.remarks.iter().chain(sliced.notes()) {
            println!("note: {note}");
        }
        // A filament with no tool of its own is written as the reference
        // writes it, and said out loud: the printer may well refuse it.
        for filament in sliced.without_a_tool() {
            println!(
                "warning: filament {filament} is used, but the printer has {} tools; T{} is written, as the \
                 C++ slicer writes it, and the printer may not accept it",
                print.tools,
                filament - 1
            );
        }
        if summary.tool_changes > 0 {
            println!(
                "{} tool change(s) between {} filaments",
                summary.tool_changes,
                sliced.used.len()
            );
        }
        if summary.slowed_layers > 0 {
            println!(
                "{} layer(s) were slowed down to give them time to cool",
                summary.slowed_layers
            );
        }
    }

    if let Some(path) = args.svg {
        let index = args.svg_layer.unwrap_or(layers.len() / 2);
        let layer = layers.get(index).ok_or_else(|| {
            format!(
                "--svg-layer {index} is out of range; there are {} layers",
                layers.len()
            )
        })?;
        fs::write(&path, svg(layer, &perimeters[index], &fills[index]))?;
        println!(
            "layer {index} (z {:.3} mm) written to {}",
            layer.z,
            path.display()
        );
    }
    Ok(())
}

/// Renders a layer in millimetres with +y up: the sliced outline as a thin
/// line, the infill area tinted, and every fill line and wall stroked at
/// its real width so gaps and overlaps show as they would print.
fn svg(layer: &LayerRegions, perimeters: &Perimeters, fills: &[ExtrusionPath]) -> String {
    fn path(
        out: &mut String,
        rings: impl Iterator<Item = impl std::borrow::Borrow<edge_geom::Polygon>>,
    ) {
        for ring in rings {
            for (i, p) in ring.borrow().points.iter().enumerate() {
                let command = if i == 0 { 'M' } else { 'L' };
                write!(out, "{command}{:.4} {:.4}", unscaled(p.x), -unscaled(p.y)).unwrap();
            }
            out.push('Z');
        }
    }

    let points = || {
        layer
            .regions
            .iter()
            .flat_map(ExPolygon::rings)
            .flat_map(|p| &p.points)
            .map(|p| (unscaled(p.x), unscaled(p.y)))
    };
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in points() {
        (min_x, max_x) = (min_x.min(x), max_x.max(x));
        (min_y, max_y) = (min_y.min(y), max_y.max(y));
    }
    if min_x > max_x {
        (min_x, min_y, max_x, max_y) = (0.0, 0.0, 1.0, 1.0);
    }
    let (width, height) = (max_x - min_x, max_y - min_y);
    let margin = width.max(height) * 0.05;

    let mut body = String::new();
    body.push_str("<g fill=\"#faf3e0\" fill-rule=\"evenodd\">");
    for area in &perimeters.infill {
        body.push_str("<path d=\"");
        path(&mut body, area.rings());
        body.push_str("\"/>");
    }
    body.push_str("</g><g fill=\"none\" stroke-linecap=\"round\" stroke-opacity=\"0.8\">");
    for line in fills {
        let colour = match line.role {
            Role::TopSolidInfill => "#2a9d8f",
            Role::BottomSurface => "#8d6e63",
            Role::BridgeInfill => "#d62828",
            Role::InternalBridge => "#e76f51",
            Role::SolidInfill => "#b5651d",
            Role::GapFill => "#f4f1de",
            // The reference's own colour for it.
            Role::Ironing => "#ff8c69",
            _ => "#e09f3e",
        };
        write!(
            body,
            "<path stroke=\"{colour}\" stroke-width=\"{:.4}\" d=\"",
            line.flow.width
        )
        .unwrap();
        for (i, p) in line.points.iter().enumerate() {
            let command = if i == 0 { 'M' } else { 'L' };
            write!(body, "{command}{:.4} {:.4}", unscaled(p.x), -unscaled(p.y)).unwrap();
        }
        body.push_str("\"/>");
    }
    body.push_str("</g><g fill=\"none\" stroke-linejoin=\"round\" stroke-opacity=\"0.75\">");
    for wall in &perimeters.loops {
        let colour = if wall.role == Role::ExternalPerimeter {
            "#1d3557"
        } else {
            "#5b8cc0"
        };
        write!(
            body,
            "<path stroke=\"{colour}\" stroke-width=\"{:.4}\" d=\"",
            wall.flow.width
        )
        .unwrap();
        path(&mut body, std::iter::once(&wall.polygon));
        body.push_str("\"/>");
    }
    write!(
        body,
        "</g><g fill=\"none\" stroke=\"#c1121f\" stroke-width=\"{:.4}\"><path d=\"",
        width.max(height) / 2000.0
    )
    .unwrap();
    path(&mut body, layer.regions.iter().flat_map(ExPolygon::rings));
    body.push_str("\"/></g>");

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{:.4} {:.4} {:.4} {:.4}\">{body}</svg>\n",
        min_x - margin,
        -max_y - margin,
        width + 2.0 * margin,
        height + 2.0 * margin,
    )
}
