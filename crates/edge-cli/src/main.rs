use std::{error::Error, fmt::Write as _, fs, path::PathBuf, process::ExitCode, time::Instant};

use clap::{Args, Parser, Subcommand};
use edge_3mf::{Body, PartKind, Project};
use edge_adhesion::{AdhesionSettings, BrimKind, Footing};
use edge_config::{Kind, Settings, Vendor};
use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Role};
use edge_gcode::PrintSettings;
use edge_geom::{ExPolygon, Point2, scaled, unscaled};
use edge_infill::{InfillSettings, SurfaceKind};
use edge_mesh::Mesh;
use edge_perimeter::{Perimeters, WallSettings};
use edge_slice::LayerRegions;
use edge_support::SupportSettings;
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
    /// and retraction.
    #[arg(long, requires = "vendor")]
    filament: Option<String>,
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

/// Moves bodies across the bed by `(x, y)` mm.
fn shift(bodies: &mut [Body], x: f64, y: f64) {
    let meshes = bodies
        .iter_mut()
        .flat_map(|body| std::iter::once(&mut body.solid).chain(body.negative.as_mut()));
    shift_meshes(meshes, x, y);
}

fn shift_meshes<'a>(meshes: impl Iterator<Item = &'a mut Mesh>, x: f64, y: f64) {
    for mesh in meshes {
        for vertex in &mut mesh.vertices {
            vertex[0] = (f64::from(vertex[0]) + x) as f32;
            vertex[1] = (f64::from(vertex[1]) + y) as f32;
        }
    }
}

/// Where plate `index` of `count` sits in a project's own coordinates. The
/// plates are laid out in a grid a fifth of a bed apart, rows running
/// towards -Y (PartPlate.cpp `compute_origin`); a part's position on its
/// plate, which is what the printer needs, is measured from here. The bed
/// size is the project's own `printable_area`, whole millimetres as the
/// reference takes it.
fn plate_origin(index: usize, count: usize, settings: Option<&Settings>) -> Option<(f64, f64)> {
    if index == 0 {
        return Some((0.0, 0.0));
    }
    let [min_x, min_y, max_x, max_y] = PrintSettings::from_settings(settings?).ok()?.bed?;
    let (width, depth) = ((max_x - min_x).trunc(), (max_y - min_y).trunc());
    let columns = (count as f64).sqrt().ceil() as usize;
    let (row, column) = (index / columns, index % columns);
    Some((column as f64 * width * 1.2, -(row as f64) * depth * 1.2))
}

/// What a model file holds.
struct Loaded {
    /// What to slice.
    bodies: Vec<Body>,
    /// For a project, the settings it was saved with.
    settings: Option<Settings>,
    /// For a project, each object on the plate by itself, for what depends
    /// on which object is which. Empty for a bare model, which is one object.
    objects: Vec<Mesh>,
}

fn load(args: &SliceArgs) -> Result<Loaded, Failure> {
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
        return Ok(Loaded {
            bodies: vec![Body {
                solid,
                negative: None,
            }],
            settings: None,
            objects: Vec::new(),
        });
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
    let mut bodies = project.bodies(plate);
    let mut objects = project.instance_meshes(plate);
    if let Some(index) = plate {
        let (x, y) = plate_origin(index, project.plates.len(), project.settings.as_ref())
            .ok_or_else(|| {
                format!(
                    "{}: plate {} cannot be placed: the project does not give the bed's size",
                    path.display(),
                    project.plates[index].id
                )
            })?;
        shift(&mut bodies, -x, -y);
        shift_meshes(objects.iter_mut(), -x, -y);
    }
    Ok(Loaded {
        bodies,
        settings: project.settings,
        objects,
    })
}

/// The presets named on the command line, resolved.
struct Presets {
    machine: Option<Settings>,
    process: Option<Settings>,
    filament: Option<Settings>,
}

fn presets(args: &SliceArgs) -> Result<Presets, Failure> {
    let Some(vendor) = &args.vendor else {
        return Ok(Presets {
            machine: None,
            process: None,
            filament: None,
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
        filament: resolve(Kind::Filament, &args.filament)?,
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
    let Loaded {
        mut bodies,
        settings: project_settings,
        objects,
    } = load(&args)?;
    let load_time = started.elapsed();
    let is_project = project_settings.is_some();
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
    // Later sources replace earlier ones: project, then the presets named.
    let mut settings = project_settings.unwrap_or_default();
    for preset in [&presets.machine, &presets.filament, &presets.process]
        .into_iter()
        .flatten()
    {
        settings.overlay(preset);
    }
    if let Some(bed_type) = &args.bed_type {
        settings.insert("curr_bed_type", edge_config::Value::One(bed_type.clone()));
    } else if settings.get("curr_bed_type").is_none()
        && settings.get("machine_start_gcode").is_some()
    {
        println!("bed type not given; the default, Cool Plate, is assumed (--bed-type changes it)");
        settings.insert(
            "curr_bed_type",
            edge_config::Value::One("Cool Plate".into()),
        );
    }
    let print = PrintSettings::from_settings(&settings)?;
    // A bare model has no place on the bed of its own: it goes in the
    // middle, as the reference's arranger leaves a single part.
    let mut centred = None;
    if let (false, Some([min_x, min_y, max_x, max_y])) = (is_project, print.bed) {
        let x = (min_x + max_x) / 2.0 - f64::from(min[0] + max[0]) / 2.0;
        let y = (min_y + max_y) / 2.0 - f64::from(min[1] + max[1]) / 2.0;
        shift(&mut bodies, x, y);
        centred = Some(((min_x + max_x) / 2.0, (min_y + max_y) / 2.0));
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
    let mut layers = edge_slice::slice_regions(&borrowed, &planes, scaled(closing_radius))?;

    // The first layers are printed slightly smaller than modelled, tapering
    // to nothing over `elefant_foot_compensation_layers`. The modelled
    // outlines are kept: neighbouring layers are still compared with them.
    let elephant_foot = settings.float("elefant_foot_compensation")?.unwrap_or(0.0);
    let elephant_layers = if elephant_foot > 0.0 {
        usize::try_from(
            settings
                .integer("elefant_foot_compensation_layers")?
                .unwrap_or(1),
        )
        .unwrap_or(0)
    } else {
        0
    }
    .min(layers.len());
    let mut modelled: Vec<Vec<ExPolygon>> = Vec::with_capacity(elephant_layers);
    for (index, layer) in layers.iter_mut().take(elephant_layers).enumerate() {
        let height = if index == 0 {
            first_layer_height
        } else {
            layer_height
        };
        let outer = walls.params(height, index == 0)?.outer;
        let amount = elephant_foot * (1.0 - index as f64 / elephant_layers as f64);
        let shrunk = edge_slice::compensate_elephant_foot(
            &layer.regions,
            scaled(outer.width + outer.spacing),
            scaled(amount),
        )?;
        modelled.push(std::mem::replace(&mut layer.regions, shrunk));
    }
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
                .params_at(height, index, layers.len())
                .map_err(|e| e.to_string())?;
            edge_perimeter::classic(&layer.regions, &params).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<Perimeters>, String>>()?;
    let perimeter_time = started.elapsed();

    // The part's outlines as modelled: what supports, brim and skirt stand
    // off, and what neighbouring layers are compared by, so a first layer
    // shrunk against elephant foot does not count as an overhang above it.
    let outlines: Vec<&[ExPolygon]> = (0..layers.len())
        .map(|index| {
            modelled
                .get(index)
                .map_or(layers[index].regions.as_slice(), Vec::as_slice)
        })
        .collect();

    let started = Instant::now();
    let support = SupportSettings::from_settings(&settings)?;
    // Laid out from the middle of what is being printed.
    let middle = bodies
        .iter()
        .filter_map(|body| body.solid.bounds())
        .reduce(|(lo, hi), (min, max)| {
            (
                std::array::from_fn(|i| lo[i].min(min[i])),
                std::array::from_fn(|i| hi[i].max(max[i])),
            )
        })
        .map_or(Point2::from_mm(0.0, 0.0), |(lo, hi)| {
            Point2::from_mm(
                f64::from(lo[0] + hi[0]) / 2.0,
                f64::from(lo[1] + hi[1]) / 2.0,
            )
        });
    let supports = edge_support::generate(
        &outlines,
        middle,
        first_layer_height,
        layer_height,
        &support,
    )?;
    let support_time = started.elapsed();

    // Skirt and brim, per layer, in the order they are printed.
    let adhesion = AdhesionSettings::from_settings(&settings)?;
    let beside = &outlines[..layers.len().min(adhesion.skirt_height.max(1))];
    let supports_beside: Vec<&[ExtrusionPath]> = supports
        .iter()
        .take(beside.len())
        .map(Vec::as_slice)
        .collect();
    // The brim is worked out object by object: each one's first layer and
    // how tall it stands. A bare model is the one object.
    let own: Vec<&Mesh> = if objects.is_empty() {
        bodies.iter().map(|body| &body.solid).collect()
    } else {
        objects.iter().collect()
    };
    let mut footprints: Vec<(Vec<ExPolygon>, f64)> = Vec::new();
    if adhesion.brim != BrimKind::None {
        for mesh in own {
            let Some((low, high)) = mesh.bounds() else {
                continue;
            };
            let body = [edge_slice::Body {
                solid: mesh,
                negative: None,
            }];
            let first = edge_slice::slice_regions(&body, &planes[..1], scaled(closing_radius))?;
            if let Some(layer) = first.into_iter().next() {
                footprints.push((layer.regions, f64::from(high[2] - low[2])));
            }
        }
    }
    let footings: Vec<Footing<'_>> = footprints
        .iter()
        .map(|(footprint, height)| Footing {
            footprint,
            height: *height,
        })
        .collect();
    let brim = edge_adhesion::brim(&footings, &adhesion, first_layer_height)?;
    let mut around: Vec<Vec<ExtrusionLoop>> = vec![Vec::new(); layers.len()];
    for (index, lines) in around.iter_mut().enumerate() {
        if adhesion.has_skirt(index) {
            let height = if index == 0 {
                first_layer_height
            } else {
                layer_height
            };
            *lines = edge_adhesion::skirt(beside, &brim, &supports_beside, &adhesion, height)?;
        }
    }
    let skirt_loops = around.first().map_or(0, Vec::len);
    if let Some(first) = around.first_mut() {
        first.extend(brim);
    }

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
    let surfaces = edge_infill::classify(
        &slices,
        &outlines,
        &inside_walls,
        &infill.shells(layer_height)?,
    )?;
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
    if elephant_layers > 0 {
        println!(
            "elephant foot compensation {elephant_foot} mm over the first {elephant_layers} layer(s)"
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
    if support.enabled {
        let lines = || supports.iter().flatten();
        let dense = lines().filter(|line| line.role == Role::SupportInterface);
        println!(
            "supports on {} layer(s): {} lines, {:.2} m, of which {:.2} m in the dense tops; {:.2} cm³",
            supports.iter().filter(|layer| !layer.is_empty()).count(),
            lines().count(),
            lines().map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
            dense.map(ExtrusionPath::length_mm).sum::<f64>() / 1e3,
            lines().map(ExtrusionPath::volume_mm3).sum::<f64>() / 1e3,
        );
    }
    if let Some(first) = around.first().filter(|first| !first.is_empty()) {
        println!(
            "{skirt_loops} skirt loop(s) on {} layer(s), {} brim loop(s): {:.2} m on the first layer",
            adhesion.skirt_height.min(layers.len()),
            first.len() - skirt_loops,
            first.iter().map(ExtrusionLoop::length_mm).sum::<f64>() / 1e3,
        );
    }
    if let Some((x, y)) = centred {
        println!("placed in the middle of the bed, at {x} x {y} mm");
    }
    if let Some(pattern) = &infill.unsupported_pattern {
        println!(
            "note: sparse pattern \"{pattern}\" is not implemented; grid was used in its place"
        );
    }
    println!(
        "load {:.1} ms, slice {:.1} ms, perimeters {:.1} ms, supports {:.1} ms, surfaces {:.1} ms, \
         fill {:.1} ms on {} threads",
        load_time.as_secs_f64() * 1e3,
        slice_time.as_secs_f64() * 1e3,
        perimeter_time.as_secs_f64() * 1e3,
        support_time.as_secs_f64() * 1e3,
        classify_time.as_secs_f64() * 1e3,
        (infill_time - classify_time).as_secs_f64() * 1e3,
        rayon::current_num_threads(),
    );

    if let Some(path) = &args.gcode {
        let started = Instant::now();
        // Layer tops, measured from the bottom of the model as the bed.
        let plan: Vec<edge_gcode::Layer<'_>> = (0..layers.len())
            .map(|index| edge_gcode::Layer {
                z: first_layer_height + index as f64 * layer_height,
                height: if index == 0 {
                    first_layer_height
                } else {
                    layer_height
                },
                around: &around[index],
                support: &supports[index],
                loops: &perimeters[index].loops,
                islands: &perimeters[index].islands,
                paths: &fills[index],
                infill_area: &perimeters[index].infill,
            })
            .collect();
        // Nothing may be sent to a printer that would run it off its bed.
        if let Some([min_x, min_y, max_x, max_y]) = print.bed {
            let points = plan.iter().flat_map(|layer| {
                let loops = layer.around.iter().chain(layer.loops);
                loops.flat_map(|line| &line.polygon.points).chain(
                    layer
                        .support
                        .iter()
                        .chain(layer.paths)
                        .flat_map(|line| &line.points),
                )
            });
            let mut reach: Option<[f64; 4]> = None;
            for point in points {
                let (x, y) = (unscaled(point.x), unscaled(point.y));
                let [a, b, c, d] = reach.unwrap_or([x, y, x, y]);
                reach = Some([a.min(x), b.min(y), c.max(x), d.max(y)]);
            }
            if let Some([a, b, c, d]) = reach
                && (a < min_x || b < min_y || c > max_x || d > max_y)
            {
                return Err(format!(
                    "the print reaches from {a:.1} x {b:.1} to {c:.1} x {d:.1} mm, off the bed, which runs \
                     from {min_x} x {min_y} to {max_x} x {max_y} mm; no G-code was written"
                )
                .into());
            }
        }
        // Written whole or not at all: a file cut short by an error must
        // never be left where it could be sent to a printer.
        let mut text = Vec::new();
        let summary = edge_gcode::write(&mut text, &plan, &print)?;
        fs::write(path, &text)?;
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
        if print.machine_start_gcode.is_some() {
            println!(
                "note: start, end and layer-change G-code come from the printer profile; not yet run on a printer"
            );
        } else {
            println!(
                "note: no printer profile, so this G-code has a generic preamble and is not printer-ready"
            );
        }
        for note in print
            .unsupported
            .iter()
            .chain(&adhesion.unsupported)
            .chain(&support.unsupported)
        {
            println!("note: {note}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plates_are_laid_out_in_a_grid_running_back_along_y() {
        let settings = Settings::from_json(
            r#"{"printable_area": ["0.5x1", "270.5x1", "270.5x271", "0.5x271"]}"#,
        )
        .unwrap();
        let origin = |index, count| plate_origin(index, count, Some(&settings));
        // A 270 x 270 mm bed, plates a fifth of a bed apart.
        assert_eq!(origin(0, 3), Some((0.0, 0.0)));
        assert_eq!(origin(1, 3), Some((324.0, 0.0)));
        // Three plates make two columns, so the third starts a new row.
        assert_eq!(origin(2, 3), Some((0.0, -324.0)));
        assert_eq!(origin(4, 9), Some((324.0, -324.0)));
        // The first plate needs no bed size; any other does.
        assert_eq!(plate_origin(0, 2, None), Some((0.0, 0.0)));
        assert_eq!(plate_origin(1, 2, None), None);
        assert_eq!(plate_origin(1, 2, Some(&Settings::default())), None);
    }

    #[test]
    fn shifting_moves_solid_and_negative_parts_together() {
        let cube = edge_mesh::Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ));
        let mut bodies = [Body {
            solid: cube.clone(),
            negative: Some(cube),
        }];
        shift(&mut bodies, 10.0, -5.0);
        for mesh in [&bodies[0].solid, bodies[0].negative.as_ref().unwrap()] {
            let (min, max) = mesh.bounds().unwrap();
            assert_eq!((min, max), ([10.0, -5.0, 0.0], [11.0, -4.0, 1.0]));
        }
    }
}
