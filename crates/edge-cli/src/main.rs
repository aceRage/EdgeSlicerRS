use std::{error::Error, fmt::Write as _, fs, path::PathBuf, process::ExitCode, time::Instant};

use clap::{Args, Parser, Subcommand};
use edge_3mf::{Body, PartKind, Project};
use edge_config::{Kind, Settings, Vendor};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Role};
use edge_gcode::PrintSettings;
use edge_geom::{ExPolygon, scaled, unscaled};
use edge_infill::{InfillSettings, SurfaceKind};
use edge_perimeter::{Perimeters, WallSettings};
use edge_slice::LayerRegions;
use rayon::prelude::*;

type Failure = Box<dyn Error>;

#[derive(Parser)]
#[command(name = "edge", version, about = "EdgeSlicer engine command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Cut a model into layer contours and report what was found and how long it took.
    Slice(SliceArgs),
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
    /// Machine preset name within --vendor; supplies the nozzle diameter.
    #[arg(long, requires = "vendor")]
    machine: Option<String>,
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
    /// Write the print as G-code to this file. Not printer-ready yet: the
    /// machine's start and end G-code is replaced by a generic preamble.
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
        Command::Slice(args) => slice(args),
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

/// The bodies to slice and, for a project, the settings it was saved with.
fn load(args: &SliceArgs) -> Result<(Vec<Body>, Option<Settings>), Failure> {
    let path = &args.model;
    let named = |e: &dyn Error| format!("{}: {e}", path.display());
    let is_3mf = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"));
    if !is_3mf {
        if args.plate.is_some() {
            return Err("--plate only applies to 3MF files".into());
        }
        let solid = edge_mesh::stl::load(path).map_err(|e| named(&e))?;
        return Ok((
            vec![Body {
                solid,
                negative: None,
            }],
            None,
        ));
    }

    let project = Project::open(path).map_err(|e| named(&e))?;
    let plate = match args.plate {
        Some(id) => Some(project.plate_index(id).ok_or_else(|| {
            let ids: Vec<String> = project
                .plates
                .iter()
                .map(|plate| plate.id.to_string())
                .collect();
            match ids.as_slice() {
                [] => format!(
                    "{}: no plate {id}; the file defines no plates",
                    path.display()
                ),
                _ => format!(
                    "{}: no plate {id}; it has plates {}",
                    path.display(),
                    ids.join(", ")
                ),
            }
        })?),
        None => project.plate_index(1),
    };
    if let Some(index) = plate {
        println!(
            "plate {} of {}",
            project.plates[index].id,
            project.plates.len()
        );
    }
    Ok((project.bodies(plate), project.settings))
}

/// The presets named on the command line, resolved.
struct Presets {
    machine: Option<Settings>,
    process: Option<Settings>,
}

fn presets(args: &SliceArgs) -> Result<Presets, Failure> {
    let Some(vendor) = &args.vendor else {
        return Ok(Presets {
            machine: None,
            process: None,
        });
    };
    let vendor = args.profiles.vendor(vendor)?;
    let resolve = |kind, name: &Option<String>| {
        name.as_ref()
            .map(|name| vendor.resolve(kind, name))
            .transpose()
    };
    Ok(Presets {
        machine: resolve(Kind::Machine, &args.machine)?,
        process: resolve(Kind::Process, &args.process)?,
    })
}

/// A slicing setting's value and where it came from.
type Sourced = (f64, &'static str);

/// Layer height, first layer height and closing radius, each from the first
/// of: command line, process preset, the project's settings, default.
fn slice_settings(
    args: &SliceArgs,
    preset: Option<&Settings>,
    project: Option<&Settings>,
) -> Result<[Sourced; 3], Failure> {
    let pick = |flag: Option<f64>, key: &str| -> Result<Option<Sourced>, Failure> {
        if let Some(value) = flag {
            return Ok(Some((value, "command line")));
        }
        for (settings, source) in [(preset, "process preset"), (project, "project settings")] {
            if let Some(value) = settings
                .map(|settings| settings.float(key))
                .transpose()?
                .flatten()
            {
                return Ok(Some((value, source)));
            }
        }
        Ok(None)
    };
    let layer = pick(args.layer_height, "layer_height")?.unwrap_or((0.2, "default"));
    let first = pick(args.first_layer_height, "initial_layer_print_height")?
        .unwrap_or((layer.0, "layer height"));
    // 0.049 is the reference's default for slice_closing_radius.
    let closing = pick(args.closing_radius, "slice_closing_radius")?.unwrap_or((0.049, "default"));
    if !(layer.0 > 0.0 && first.0 > 0.0) {
        return Err("layer heights must be positive".into());
    }
    if closing.0.is_nan() || closing.0 < 0.0 {
        return Err("closing radius cannot be negative".into());
    }
    Ok([layer, first, closing])
}

fn slice(args: SliceArgs) -> Result<(), Failure> {
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let started = Instant::now();
    let (bodies, project_settings) = load(&args)?;
    let load_time = started.elapsed();
    let bounds =
        bodies
            .iter()
            .filter_map(|body| body.solid.bounds())
            .reduce(|(lo, hi), (min, max)| {
                (
                    std::array::from_fn(|i| lo[i].min(min[i])),
                    std::array::from_fn(|i| hi[i].max(max[i])),
                )
            });
    let Some((min, max)) = bounds else {
        return Err(format!("{}: nothing printable to slice", args.model.display()).into());
    };

    let presets = presets(&args)?;
    let [
        (layer_height, layer_source),
        (first_layer_height, first_source),
        (closing_radius, closing_source),
    ] = slice_settings(&args, presets.process.as_ref(), project_settings.as_ref())?;
    // Later sources replace earlier ones: project, machine preset, process preset.
    let mut settings = project_settings.unwrap_or_default();
    for preset in [&presets.machine, &presets.process].into_iter().flatten() {
        settings.overlay(preset);
    }
    let mut walls = WallSettings::from_settings(&settings)?;
    if let Some(count) = args.wall_loops {
        walls.wall_loops = count;
    }

    let started = Instant::now();
    let planes = edge_slice::layer_planes(
        min[2].into(),
        max[2].into(),
        first_layer_height,
        layer_height,
    );
    let borrowed: Vec<edge_slice::Body<'_>> = bodies
        .iter()
        .map(|body| edge_slice::Body {
            solid: &body.solid,
            negative: body.negative.as_ref(),
        })
        .collect();
    let layers = edge_slice::slice_regions(&borrowed, &planes, scaled(closing_radius))?;
    let slice_time = started.elapsed();

    let started = Instant::now();
    let perimeters = layers
        .par_iter()
        .enumerate()
        .map(|(index, layer)| {
            let height = if index == 0 {
                first_layer_height
            } else {
                layer_height
            };
            let params = walls
                .params(height, index == 0)
                .map_err(|e| e.to_string())?;
            edge_perimeter::classic(&layer.regions, &params).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<Perimeters>, String>>()?;
    let perimeter_time = started.elapsed();

    let started = Instant::now();
    let infill = InfillSettings::from_settings(&settings)?;
    let slices: Vec<&[ExPolygon]> = layers
        .iter()
        .map(|layer| layer.regions.as_slice())
        .collect();
    let inside_walls: Vec<&[ExPolygon]> = perimeters
        .iter()
        .map(|layer| layer.infill.as_slice())
        .collect();
    let surfaces = edge_infill::classify(&slices, &inside_walls, &infill.shells(layer_height)?)?;
    let classify_time = started.elapsed();
    let fills = surfaces
        .par_iter()
        .enumerate()
        .map(|(index, surfaces)| {
            let height = if index == 0 {
                first_layer_height
            } else {
                layer_height
            };
            let params = infill
                .params(height, index == 0)
                .map_err(|e| e.to_string())?;
            edge_infill::fill(surfaces, index, &params).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<Vec<ExtrusionPath>>, String>>()?;
    let infill_time = started.elapsed();

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
            .flatten()
            .filter(|s| s.kind == kind)
            .map(|s| s.area.area_mm2())
            .sum::<f64>()
            / 100.0
    };
    let paths = || fills.iter().flatten();
    println!(
        "infill at {}% {:?}: {:.1} cm² top, {:.1} bottom, {:.1} bridge, {:.1} solid, {:.1} sparse; \
         {} lines, {:.2} m, {:.2} cm³",
        infill.sparse_infill_density_percent,
        infill.pattern,
        area_of(SurfaceKind::Top),
        area_of(SurfaceKind::Bottom),
        area_of(SurfaceKind::Bridge),
        area_of(SurfaceKind::InternalSolid),
        area_of(SurfaceKind::Sparse),
        paths().count(),
        paths().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
        paths().map(ExtrusionPath::volume_mm3).sum::<f64>() / 1e3,
    );
    if let Some(pattern) = &infill.unsupported_pattern {
        println!(
            "note: sparse pattern \"{pattern}\" is not implemented; grid was used in its place"
        );
    }
    println!(
        "load {:.1} ms, slice {:.1} ms, perimeters {:.1} ms, surfaces {:.1} ms, fill {:.1} ms on {} threads",
        load_time.as_secs_f64() * 1e3,
        slice_time.as_secs_f64() * 1e3,
        perimeter_time.as_secs_f64() * 1e3,
        classify_time.as_secs_f64() * 1e3,
        (infill_time - classify_time).as_secs_f64() * 1e3,
        rayon::current_num_threads(),
    );

    if let Some(path) = &args.gcode {
        let started = Instant::now();
        let print = PrintSettings::from_settings(&settings)?;
        // Layer tops, measured from the bottom of the model as the bed.
        let plan: Vec<edge_gcode::Layer<'_>> = (0..layers.len())
            .map(|index| edge_gcode::Layer {
                z: first_layer_height + index as f64 * layer_height,
                height: if index == 0 {
                    first_layer_height
                } else {
                    layer_height
                },
                loops: &perimeters[index].loops,
                islands: &perimeters[index].islands,
                paths: &fills[index],
                infill_area: &perimeters[index].infill,
            })
            .collect();
        let mut file = std::io::BufWriter::new(fs::File::create(path)?);
        let summary = edge_gcode::write(&mut file, &plan, &print)?;
        std::io::Write::flush(&mut file)?;
        let weight = match print.filament_density {
            Some(density) => format!(", {:.1} g", summary.filament_mm3 / 1000.0 * density),
            None => String::new(),
        };
        println!(
            "G-code written to {} in {:.1} ms: {:.2} m of filament{weight}, {:.1} m of travel, {} retractions, \
             {:.0} min at commanded speeds (no acceleration)",
            path.display(),
            started.elapsed().as_secs_f64() * 1e3,
            summary.filament_mm / 1e3,
            summary.travel_mm / 1e3,
            summary.retractions,
            summary.seconds / 60.0,
        );
        println!(
            "note: this G-code is not printer-ready; machine start and end G-code is not expanded yet"
        );
        for note in &print.unsupported {
            println!("note: {note}");
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
            Role::SolidInfill => "#b5651d",
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
