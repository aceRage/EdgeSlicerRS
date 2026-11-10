use std::{error::Error, fmt::Write as _, fs, path::PathBuf, time::Instant};

use clap::{Args, Parser, Subcommand};
use edge_geom::unscaled;
use edge_slice::Layer;

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
}

#[derive(Args)]
struct SliceArgs {
    /// STL file, binary or ASCII.
    model: PathBuf,
    /// Layer height in mm.
    #[arg(long, default_value_t = 0.2)]
    layer_height: f64,
    /// First layer height in mm; defaults to the layer height.
    #[arg(long)]
    first_layer_height: Option<f64>,
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

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::Slice(args) => slice(args),
    }
}

fn slice(args: SliceArgs) -> Result<(), Box<dyn Error>> {
    let first_layer_height = args.first_layer_height.unwrap_or(args.layer_height);
    if !(args.layer_height > 0.0 && first_layer_height > 0.0) {
        return Err("layer heights must be positive".into());
    }
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()?;
    }

    let started = Instant::now();
    let mesh =
        edge_mesh::stl::load(&args.model).map_err(|e| format!("{}: {e}", args.model.display()))?;
    let load_time = started.elapsed();
    let Some((min, max)) = mesh.bounds() else {
        return Err(format!("{}: no triangles", args.model.display()).into());
    };

    let started = Instant::now();
    let planes = edge_slice::layer_planes(
        min[2].into(),
        max[2].into(),
        first_layer_height,
        args.layer_height,
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
