use std::{error::Error, fmt::Write as _, fs, path::PathBuf, process::ExitCode, time::Instant};

use clap::{Args, Parser, Subcommand};
use edge_3mf::{PartKind, Project};
use edge_config::{Kind, Settings, Vendor};
use edge_geom::unscaled;
use edge_mesh::Mesh;
use edge_slice::Layer;

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
    /// Vendor whose process preset supplies the layer heights; needs --process.
    #[arg(long, requires = "process")]
    vendor: Option<String>,
    /// Process preset name within --vendor.
    #[arg(long, requires = "vendor")]
    process: Option<String>,
    #[command(flatten)]
    profiles: ProfilesArgs,
    /// Worker threads; defaults to all cores.
    #[arg(long)]
    threads: Option<usize>,
    /// Write one layer's contours to this SVG file.
    #[arg(long)]
    svg: Option<PathBuf>,
    /// Layer index for --svg; defaults to the middle layer.
    #[arg(long, requires = "svg")]
    svg_layer: Option<usize>,
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

/// A model ready to slice and, for a project, the settings it was saved with.
fn load(args: &SliceArgs) -> Result<(Mesh, Option<Settings>), Failure> {
    let path = &args.model;
    let named = |e: &dyn Error| format!("{}: {e}", path.display());
    let is_3mf = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"));
    if !is_3mf {
        if args.plate.is_some() {
            return Err("--plate only applies to 3MF files".into());
        }
        return Ok((edge_mesh::stl::load(path).map_err(|e| named(&e))?, None));
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
    Ok((project.mesh(plate), project.settings))
}

/// Layer and first-layer height with where each came from, in order of
/// precedence: command line, process preset, the project's settings, default.
fn layer_heights(
    args: &SliceArgs,
    project: Option<&Settings>,
) -> Result<[(f64, &'static str); 2], Failure> {
    let preset = match (&args.vendor, &args.process) {
        (Some(vendor), Some(process)) => Some(
            args.profiles
                .vendor(vendor)?
                .resolve(Kind::Process, process)?,
        ),
        _ => None,
    };
    let pick = |flag: Option<f64>, key: &str| -> Result<Option<(f64, &'static str)>, Failure> {
        if let Some(value) = flag {
            return Ok(Some((value, "command line")));
        }
        for (settings, source) in [
            (preset.as_ref(), "process preset"),
            (project, "project settings"),
        ] {
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
    if !(layer.0 > 0.0 && first.0 > 0.0) {
        return Err("layer heights must be positive".into());
    }
    Ok([layer, first])
}

fn slice(args: SliceArgs) -> Result<(), Failure> {
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let started = Instant::now();
    let (mesh, project_settings) = load(&args)?;
    let load_time = started.elapsed();
    let Some((min, max)) = mesh.bounds() else {
        return Err(format!("{}: nothing printable to slice", args.model.display()).into());
    };
    let [
        (layer_height, layer_source),
        (first_layer_height, first_source),
    ] = layer_heights(&args, project_settings.as_ref())?;

    let started = Instant::now();
    let planes = edge_slice::layer_planes(
        min[2].into(),
        max[2].into(),
        first_layer_height,
        layer_height,
    );
    let layers = edge_slice::slice(&mesh, &planes);
    let slice_time = started.elapsed();

    let contours: usize = layers.iter().map(|l| l.polygons.len()).sum();
    let points: usize = layers
        .iter()
        .flat_map(|l| &l.polygons)
        .map(|p| p.points.len())
        .sum();
    let leaky = layers.iter().filter(|l| l.open_chains > 0).count();
    println!(
        "{}: {} triangles, {} vertices, {:.3} x {:.3} x {:.3} mm",
        args.model.display(),
        mesh.triangles.len(),
        mesh.vertices.len(),
        max[0] - min[0],
        max[1] - min[1],
        max[2] - min[2],
    );
    println!(
        "layer height {layer_height} mm ({layer_source}), first layer {first_layer_height} mm ({first_source})"
    );
    println!(
        "{} layers, {contours} contours, {points} points",
        layers.len()
    );
    if leaky > 0 {
        println!("warning: {leaky} layers have open contours; the mesh is not watertight there");
    }
    println!(
        "load {:.1} ms, slice {:.1} ms on {} threads",
        load_time.as_secs_f64() * 1e3,
        slice_time.as_secs_f64() * 1e3,
        rayon::current_num_threads(),
    );

    if let Some(path) = args.svg {
        let index = args.svg_layer.unwrap_or(layers.len() / 2);
        let layer = layers.get(index).ok_or_else(|| {
            format!(
                "--svg-layer {index} is out of range; there are {} layers",
                layers.len()
            )
        })?;
        fs::write(&path, svg(layer))?;
        println!(
            "layer {index} (z {:.3} mm) written to {}",
            layer.z,
            path.display()
        );
    }
    Ok(())
}

/// Renders a layer in millimetres with +y up; holes show through via even-odd fill.
fn svg(layer: &Layer) -> String {
    let points = || {
        layer
            .polygons
            .iter()
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

    let mut path = String::new();
    for polygon in &layer.polygons {
        for (i, p) in polygon.points.iter().enumerate() {
            let command = if i == 0 { 'M' } else { 'L' };
            write!(path, "{command}{:.4} {:.4}", unscaled(p.x), -unscaled(p.y)).unwrap();
        }
        path.push('Z');
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{:.4} {:.4} {:.4} {:.4}\">\
         <path d=\"{path}\" fill=\"#9db7d5\" fill-rule=\"evenodd\" stroke=\"#1d3557\" stroke-width=\"{:.4}\"/></svg>\n",
        min_x - margin,
        -max_y - margin,
        width + 2.0 * margin,
        height + 2.0 * margin,
        width.max(height) / 500.0,
    )
}
